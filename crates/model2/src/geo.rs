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
/// Geometrizer PCs kept for the first-error dump.
const PC_TRACE_LEN: usize = 4096;

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

/// The geometrizer's TGP and everything it owns. Runs a frame at a time,
/// on a worker thread (or inline, see [`Geometrizer::set_threaded`]), from
/// a copy of buffer RAM taken at vblank.
pub struct Engine {
    pub tgp: Tgp,
    poly_ram: Vec<u32>,
    poly_rom: Vec<u32>,
    /// Buffer RAM as it was when the frame started.
    buffer: Vec<u8>,
    /// Rasteriser stream produced for the most recent frame.
    pub stream: Vec<u32>,
    /// List words consumed for the most recent frame.
    pub list_words: usize,
    pub fault: Option<u32>,
    pub frames: u64,
    /// Rasteriser texture point RAM (u16[64K]), written by raster cmd 4
    /// with address bit 23 set. Persists across frames.
    pub texture_point_ram: Vec<u16>,
    /// List words fed during the most recent frame (diagnostics).
    pub fed: Vec<u32>,
    /// First frame that ended with the program in its command-check error
    /// loop, with the words fed that frame.
    pub first_error: Option<(u64, Vec<u32>)>,
    /// Recent geometrizer PCs and input FIFO depths (a ring indexed by
    /// step count), dumped with the first error.
    pc_trace: Vec<(u32, usize)>,
    pub first_error_trace: Vec<(u32, usize)>,
    /// Host time spent running frames, in total and for the slowest one.
    pub busy: std::time::Duration,
    pub slowest: std::time::Duration,
}

enum Slot {
    Idle(Box<Engine>),
    Running(std::thread::JoinHandle<Box<Engine>>),
    Empty,
}

/// The i960's side of the geometrizer: command and data ports, list
/// pointers and program upload. The engine runs behind it.
pub struct Geometrizer {
    pub program: Vec<u64>,
    uploading: bool,
    upload_half: Option<u32>,
    pub booted: bool,
    /// Byte offsets into buffer RAM.
    pub write_ptr: u32,
    pub read_ptr: u32,
    /// Command words that failed the `fn | fn << 8` check.
    pub bad_commands: u64,
    /// Host time the emulation thread spent waiting for the worker.
    pub waited: std::time::Duration,
    engine: Slot,
    threaded: bool,
    /// Remote mode: jobs go here instead of to the local engine.
    remote: Option<Box<dyn FnMut(Job)>>,
}

/// Work for an engine elsewhere (another worker or process): the program
/// the game uploaded, or one frame's list. An engine fed the same jobs in
/// order gives the same results as the local one ([`Engine::run_job`]).
pub enum Job {
    Boot(Vec<u64>),
    Frame { buffer: Vec<u8>, booted: bool, read_ptr: u32 },
}

/// Host time measurement that works where `std::time::Instant` doesn't
/// (WebAssembly in a browser has no clock in std: it measures nothing).
struct Stopwatch(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Stopwatch {
    fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Stopwatch(std::time::Instant::now());
        #[cfg(target_arch = "wasm32")]
        return Stopwatch();
    }

    fn elapsed(&self) -> std::time::Duration {
        #[cfg(not(target_arch = "wasm32"))]
        return self.0.elapsed();
        #[cfg(target_arch = "wasm32")]
        return std::time::Duration::ZERO;
    }
}

impl Geometrizer {
    pub fn new(poly_rom: Vec<u32>) -> Self {
        Geometrizer {
            program: Vec::new(),
            uploading: false,
            upload_half: None,
            booted: false,
            write_ptr: 0,
            read_ptr: 0,
            bad_commands: 0,
            waited: std::time::Duration::ZERO,
            // WebAssembly without threads support can't spawn.
            threaded: cfg!(not(target_arch = "wasm32")),
            remote: None,
            engine: Slot::Idle(Box::new(Engine::new(poly_rom))),
        }
    }

    /// Wait for a frame running on the worker, if any.
    pub fn sync(&mut self) {
        if !matches!(self.engine, Slot::Running(_)) {
            return;
        }
        if let Slot::Running(h) = std::mem::replace(&mut self.engine, Slot::Empty) {
            let t = Stopwatch::start();
            self.engine = Slot::Idle(h.join().expect("geometrizer worker panicked"));
            self.waited += t.elapsed();
        }
    }

    /// The engine's state. Call `sync` first if a frame may be running.
    pub fn engine(&self) -> &Engine {
        match &self.engine {
            Slot::Idle(e) => e,
            _ => panic!("geometrizer frame still running: call sync() first"),
        }
    }

    fn engine_mut(&mut self) -> &mut Engine {
        self.sync();
        match &mut self.engine {
            Slot::Idle(e) => e,
            _ => unreachable!(),
        }
    }

    /// Send jobs to `sink` instead of running them: an engine elsewhere (a
    /// browser worker) runs them with [`Engine::run_job`], and the local
    /// engine stays idle. Its results then come from there.
    pub fn set_remote(&mut self, sink: Box<dyn FnMut(Job)>) {
        self.sync();
        self.remote = Some(sink);
    }

    /// Run frames on a worker thread (the default where threads exist) or
    /// inline in `start_frame`. Results are identical: the engine works on
    /// its own copy of buffer RAM and is only seen after `sync`.
    pub fn set_threaded(&mut self, threaded: bool) {
        self.sync();
        self.threaded = threaded;
    }

    /// Start a frame on the worker thread from the list at the read
    /// pointer. Runs in the background until the next `sync`.
    pub fn start_frame(&mut self, buffer: &[u8]) {
        let (booted, read_ptr) = (self.booted, self.read_ptr);
        if let Some(remote) = &mut self.remote {
            remote(Job::Frame { buffer: buffer.to_vec(), booted, read_ptr });
            return;
        }
        let mut e = std::mem::replace(self.engine_mut(), Engine::placeholder());
        e.buffer.clear();
        e.buffer.extend_from_slice(buffer);
        if !self.threaded {
            e.run_frame(booted, read_ptr);
            self.engine = Slot::Idle(Box::new(e));
            return;
        }
        let handle = std::thread::spawn(move || {
            let mut e = Box::new(e);
            e.run_frame(booted, read_ptr);
            e
        });
        self.engine = Slot::Running(handle);
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
            let program = std::mem::take(&mut self.program);
            match &mut self.remote {
                Some(remote) => remote(Job::Boot(program.clone())),
                None => self.engine_mut().tgp.boot(&program),
            }
            self.program = program;
            self.booted = true;
        }
        self.uploading = on;
    }
}

impl Engine {
    /// A fresh engine over the polygon ROM (u32 words).
    pub fn new(poly_rom: Vec<u32>) -> Self {
        Engine {
            tgp: Tgp::new(),
            poly_ram: vec![0; 0x10_0000],
            poly_rom,
            buffer: Vec::new(),
            stream: Vec::new(),
            list_words: 0,
            fault: None,
            frames: 0,
            texture_point_ram: vec![0; 0x1_0000],
            fed: Vec::new(),
            first_error: None,
            pc_trace: vec![(0, 0); PC_TRACE_LEN],
            first_error_trace: Vec::new(),
            busy: std::time::Duration::ZERO,
            slowest: std::time::Duration::ZERO,
        }
    }

    /// Run a job from a remote-mode [`Geometrizer`]. After a frame, `stream`
    /// and `texture_point_ram` hold its results.
    pub fn run_job(&mut self, job: Job) {
        match job {
            Job::Boot(program) => self.tgp.boot(&program),
            Job::Frame { buffer, booted, read_ptr } => {
                self.buffer = buffer;
                self.run_frame(booted, read_ptr);
            }
        }
    }

    fn placeholder() -> Self {
        Engine {
            tgp: Tgp::new(),
            poly_ram: Vec::new(),
            poly_rom: Vec::new(),
            buffer: Vec::new(),
            stream: Vec::new(),
            list_words: 0,
            fault: None,
            frames: 0,
            texture_point_ram: Vec::new(),
            fed: Vec::new(),
            first_error: None,
            pc_trace: Vec::new(),
            first_error_trace: Vec::new(),
            busy: std::time::Duration::ZERO,
            slowest: std::time::Duration::ZERO,
        }
    }

    /// Apply this frame's raster memory writes (cmd 4: addr, count, data)
    /// to the persistent texture point RAM.
    fn apply_raster_writes(&mut self) {
        let mut i = 0;
        while i < self.stream.len() {
            let w = self.stream[i];
            i += 1;
            if w >> 24 == 0xff && w & 0xf == 4 && i + 1 < self.stream.len() {
                let (addr, count) = (self.stream[i], self.stream[i + 1] as usize);
                let data = self.stream.get(i + 2..i + 2 + count).unwrap_or(&[]);
                if addr & 0x80_0000 != 0 {
                    for (k, d) in data.iter().enumerate() {
                        self.texture_point_ram[(addr as usize + k) & 0xffff] = *d as u16;
                    }
                }
                i += 2 + count;
            }
        }
    }

    /// Run one frame: stream the list at the read pointer into the
    /// geometrizer until it emits END, collecting its output.
    fn run_frame(&mut self, booted: bool, read_ptr: u32) {
        self.stream.clear();
        self.fed.clear();
        self.list_words = 0;
        if !booted || self.fault.is_some() {
            return;
        }
        let t = Stopwatch::start();
        let buffer = std::mem::take(&mut self.buffer);
        self.process(&buffer, read_ptr);
        self.busy += t.elapsed();
        self.slowest = self.slowest.max(t.elapsed());
        self.buffer = buffer;
        self.apply_raster_writes();
    }

    fn process(&mut self, buffer: &[u8], read_ptr: u32) {
        self.frames += 1;
        let mut pos = read_ptr as usize & (buffer.len() - 1) & !3;
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
        // Recompiled code for this program if the build has it, else the
        // interpreter; they must behave identically.
        let step = crate::recomp::lookup::<GeoExt>(self.tgp.program_hash).unwrap_or(|t, e| t.step(e));
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
                match step(&mut self.tgp, &mut ext) {
                    Step::Unsupported(p) => {
                        fault = Some(p);
                        break;
                    }
                    Step::Stalled => break,
                    Step::Ran => {}
                }
                self.pc_trace[self.tgp.steps as usize % PC_TRACE_LEN] = (pc, self.tgp.fifo_in.len());
                if pc == TS_COMMAND_ERROR_PC && self.first_error_trace.is_empty() {
                    let at = self.tgp.steps as usize % PC_TRACE_LEN + 1;
                    self.first_error_trace = [&self.pc_trace[at..], &self.pc_trace[..at]].concat();
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
