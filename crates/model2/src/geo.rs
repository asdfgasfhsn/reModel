//! Model 2B/2C geometrizer: a second MB86235 running a program the game
//! uploads, fed with a display list from buffer RAM once per frame. Its
//! output FIFO is the command stream for the fixed-function rasteriser.
//! Protocol per docs/hw/geometrizer.md sections 1-2.

use mb86235::cpu::{External, Step, Tgp};

/// Cap on list words streamed per frame (MAME stops after 0x8000 commands;
/// this bounds words, which is looser).
const MAX_LIST_WORDS: usize = 0x8000 * 8;
/// Cap on geometrizer words executed per frame.
const MAX_STEPS_PER_FRAME: u64 = 4_000_000;
/// The list DMA keeps the geometrizer's input FIFO this full. The program
/// polls the FIFO-empty flag rather than blocking on FI, so words must be
/// waiting before it looks.
const FIFO_FILL: usize = mb86235::cpu::FIFO_DEPTH;
/// How long F0 is held high at the start of a frame, in geometrizer words.
const F0_PULSE_STEPS: u64 = 256;
/// Entry of topskatr's geometrizer error loop, reached when a list word
/// fails the command check. Recording the path into it is a debugging aid;
/// other games' programs will differ.
const TS_COMMAND_ERROR_PC: u32 = 0x34d;
/// Rasteriser NOP. The idle loop's delay slot emits one per pass, which the
/// rasteriser discards.
const RASTER_NOP: u32 = 0xff00_0000;
/// Steps with no input consumed and no output produced after which the
/// program is taken to be idle (finished the list, waiting for F0).
const IDLE_STEPS: u64 = 4096;

/// The geometrizer's external bus: polygon RAM below 0x800000, the
/// polygons ROM from 0x800000 (word addressed).
struct GeoExt<'a> {
    ram: &'a mut [u32],
    rom: &'a [u32],
}

impl External for GeoExt<'_> {
    fn read(&mut self, addr: u32) -> u32 {
        if addr >= 0x80_0000 {
            self.rom.get((addr & 0xf_ffff) as usize).copied().unwrap_or(0)
        } else {
            self.ram[addr as usize % self.ram.len()]
        }
    }
    fn write(&mut self, addr: u32, v: u32) {
        if addr < 0x80_0000 {
            let n = self.ram.len();
            self.ram[addr as usize % n] = v;
        }
    }
}

pub struct Geometrizer {
    pub tgp: Tgp,
    pub program: Vec<u64>,
    uploading: bool,
    upload_half: Option<u32>,
    pub booted: bool,
    /// Byte offsets into buffer RAM.
    pub write_ptr: u32,
    pub read_ptr: u32,
    poly_ram: Vec<u32>,
    poly_rom: Vec<u32>,
    /// Rasteriser stream produced for the most recent frame.
    pub stream: Vec<u32>,
    /// List words consumed for the most recent frame.
    pub list_words: usize,
    /// Command words that failed the `fn | fn << 8` check.
    pub bad_commands: u64,
    pub fault: Option<u32>,
    pub frames: u64,
    /// List words fed during the most recent frame (diagnostics).
    pub fed: Vec<u32>,
    /// First frame that ended with the program in its command-check error
    /// loop, with the words fed that frame.
    pub first_error: Option<(u64, Vec<u32>)>,
    /// Recent geometrizer PCs (ring), dumped with the first error.
    pub pc_trace: std::collections::VecDeque<(u32, usize)>,
    pub first_error_trace: Vec<(u32, usize)>,
}

impl Geometrizer {
    pub fn new(poly_rom: Vec<u32>) -> Self {
        Geometrizer {
            tgp: Tgp::new(),
            program: Vec::new(),
            uploading: false,
            upload_half: None,
            booted: false,
            write_ptr: 0,
            read_ptr: 0,
            poly_ram: vec![0; 0x10_0000],
            poly_rom,
            stream: Vec::new(),
            list_words: 0,
            bad_commands: 0,
            fault: None,
            frames: 0,
            fed: Vec::new(),
            first_error: None,
            pc_trace: std::collections::VecDeque::new(),
            first_error_trace: Vec::new(),
        }
    }

    fn append(&mut self, buffer: &mut [u8], word: u32) {
        let o = (self.write_ptr as usize) & (buffer.len() - 1) & !3;
        buffer[o..o + 4].copy_from_slice(&word.to_le_bytes());
        self.write_ptr = (self.write_ptr + 4) & 0xf_ffff;
    }

    /// Write to the geo port window 0x00800000-0x00807FFF (`offset` from
    /// 0x00800000). Returns false if the offset isn't handled.
    pub fn port_write(&mut self, offset: u32, v: u32, buffer: &mut [u8]) -> bool {
        match offset {
            0x4000..=0x7fff => {
                if self.uploading {
                    // Program words: 64-bit, low dword first.
                    match self.upload_half.take() {
                        None => self.upload_half = Some(v),
                        Some(lo) => self.program.push(((v as u64) << 32) | lo as u64),
                    }
                } else {
                    self.append(buffer, v);
                }
            }
            0x1008 => self.write_ptr = v & 0xf_ffff,
            0x3008 => self.read_ptr = v & 0xf_ffff,
            0x0000..=0x0fff if offset & 0xf == 0 => {
                // Command: fn from the offset, eye select from offset bits 10-11.
                let fnc = (offset >> 4) & 0x3f;
                let eye = (offset >> 10) & 3;
                if (v & 0x1f) != (fnc & 0x1f) || ((v >> 8) & 0x1f) != (fnc & 0x1f) {
                    self.bad_commands += 1;
                }
                let keep = if v & 0x8000_0000 != 0 { 0x800f_ffff } else { 0x000f_ffff };
                self.append(buffer, (v & keep) | (fnc << 23) | (eye << 29));
            }
            // Command/data length writes (+4, +8): ignored by hardware models.
            0x0000..=0x0fff => {}
            _ => return false,
        }
        true
    }

    pub fn port_read(&self, offset: u32) -> u32 {
        match offset {
            0x2008 => self.write_ptr,
            0x3008 => self.read_ptr,
            _ => 0,
        }
    }

    /// Geometrizer control (0x00980008): bit 31 frames the program upload.
    pub fn control_write(&mut self, v: u32) {
        let on = v & 0x8000_0000 != 0;
        if on && !self.uploading {
            self.program.clear();
            self.upload_half = None;
        }
        if !on && self.uploading {
            self.tgp.boot(&self.program);
            self.booted = true;
        }
        self.uploading = on;
    }

    /// Run one frame: stream the list at the read pointer into the
    /// geometrizer until it emits END, collecting its output.
    pub fn run_frame(&mut self, buffer: &[u8]) {
        self.stream.clear();
        self.fed.clear();
        self.list_words = 0;
        if !self.booted || self.fault.is_some() {
            return;
        }
        self.frames += 1;
        let mut pos = self.read_ptr as usize & (buffer.len() - 1) & !3;
        let mut ext = GeoExt { ram: &mut self.poly_ram, rom: &self.poly_rom };
        let start = self.tgp.steps;
        // Frame-start handshake: the program waits for F0 to rise, clears
        // its input FIFO, then waits for F0 to fall before reading the list
        // (topskatr geometrizer 0x327-0x331; confirmed by running it).
        self.tgp.ext_flags |= 1;
        if let Step::Unsupported(pc) = self.tgp.run(&mut ext, F0_PULSE_STEPS) {
            self.fault = Some(pc);
            return;
        }
        self.tgp.ext_flags &= !1;
        // The handshake emits NOP, NOP, END: the previous frame's terminator.
        self.tgp.fifo_out.clear();
        let mut last_activity = self.tgp.steps;
        loop {
            while self.tgp.fifo_in.len() < FIFO_FILL && self.list_words < MAX_LIST_WORDS {
                let w = u32::from_le_bytes(buffer[pos..pos + 4].try_into().unwrap());
                self.tgp.fifo_in.push_back(w);
                self.fed.push(w);
                pos = (pos + 4) & (buffer.len() - 1);
                self.list_words += 1;
            }
            let mut fault = None;
            for _ in 0..256 {
                let pc = self.tgp.pc;
                match self.tgp.step(&mut ext) {
                    Step::Unsupported(p) => {
                        fault = Some(p);
                        break;
                    }
                    Step::Stalled => break,
                    Step::Ran => {}
                }
                if self.pc_trace.len() >= 4000 {
                    self.pc_trace.pop_front();
                }
                self.pc_trace.push_back((pc, self.tgp.fifo_in.len()));
                if pc == TS_COMMAND_ERROR_PC && self.first_error_trace.is_empty() {
                    self.first_error_trace = self.pc_trace.iter().copied().collect();
                    self.first_error = Some((self.frames, self.fed.clone()));
                }
            }
            if let Some(pc) = fault {
                self.fault = Some(pc);
                break;
            }
            let produced = self.tgp.fifo_out.iter().any(|&w| w != RASTER_NOP);
            if produced || self.tgp.fifo_in.len() < FIFO_FILL {
                last_activity = self.tgp.steps;
            }
            self.stream.extend(self.tgp.fifo_out.drain(..).filter(|&w| w != RASTER_NOP));
            if self.tgp.steps - last_activity > IDLE_STEPS || self.tgp.steps - start > MAX_STEPS_PER_FRAME {
                break;
            }
        }
        // The DMA stops at END; words read ahead of it are not consumed.
        self.tgp.fifo_in.clear();
    }
}
