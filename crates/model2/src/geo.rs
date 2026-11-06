//! Model 2B/2C geometrizer: a DSP running a program the game uploads,
//! fed with a display list from buffer RAM once per frame. Its output FIFO
//! is the command stream for the fixed-function rasteriser. On 2C it is a
//! second MB86235 (protocol per docs/hw/geometrizer.md sections 1-2); on
//! 2B an ADSP-21062 SHARC, booted through its host interface, whose
//! program emits the same stream (docs/hw/model2b.md).

use adsp2106x::cpu::{Bus as SharcBus, Sharc, Step as SharcStep, Width};
use mb86235::cpu::{External, Step, Tgp};
use std::collections::BTreeMap;

/// The geometrizer's processor, which sets how it is programmed and fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Core {
    /// Model 2C: an MB86235, its program uploaded as 64-bit words.
    Tgp,
    /// Model 2B: an ADSP-21062 SHARC, booted through its IOP registers.
    Sharc,
}

impl Core {
    pub fn of(board: romset::Board) -> Self {
        match board {
            romset::Board::Model2B => Core::Sharc,
            _ => Core::Tgp,
        }
    }
}

/// IOP register number of EPB0, the SHARC's DMA buffer a host boots it
/// through (the `0x804000` port during an upload).
const EPB0: u32 = adsp2106x::cpu::EPB0;

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
/// Most SHARC instructions the frame-start pulse may take before the
/// program waits for FLAG0 to fall.
const SHARC_PULSE_STEPS: u64 = 4096;
/// Geometrizer PCs kept for the first-error dump.
const PC_TRACE_LEN: usize = 4096;

/// The geometrizer's external bus: polygon RAM below 0x800000, the
/// polygons ROM from 0x800000 (word addressed), mirrored to fill the window
/// (4 MB in topskatr, 16 MB in skisuprg).
pub struct GeoExt<'a> {
    ram: &'a mut [u32],
    rom: &'a [u32],
    /// Word-address mask for the ROM: its size rounded up to a power of two.
    rom_mask: u32,
}

impl<'a> GeoExt<'a> {
    fn new(ram: &'a mut [u32], rom: &'a [u32]) -> Self {
        let rom_mask = (rom.len().next_power_of_two().max(1) - 1) as u32;
        GeoExt { ram, rom, rom_mask }
    }
}

impl External for GeoExt<'_> {
    #[inline]
    fn read(&mut self, addr: u32) -> u32 {
        if addr >= 0x80_0000 {
            self.rom.get((addr & self.rom_mask) as usize).copied().unwrap_or(0)
        } else {
            self.ram[addr as usize % self.ram.len()]
        }
    }
    #[inline]
    fn write(&mut self, addr: u32, v: u32) {
        if addr < 0x80_0000 {
            let n = self.ram.len();
            self.ram[addr as usize % n] = v;
        }
    }
}

/// The 2B geometrizer SHARC's external bus (from its program): the list
/// FIFO at `0x400000`, the rasteriser FIFO at `0x410000`, polygon RAM
/// writes at `0x420000 + address`, and polygon memory reads through an
/// address register at `0x430000` with the data at `0x430001`. Its fast
/// polygon RAM (model addresses with bit 24 set) is the SHARC's own
/// internal memory and never reaches the bus.
struct SharcGeoBus<'a> {
    poly: GeoExt<'a>,
    /// Polygon memory address register (`0x430000`).
    latch: &'a mut u32,
    list: ListDma<'a>,
    out: &'a mut Vec<u32>,
    /// Set by a list read or a rasteriser word other than a NOP: the
    /// program is still working on the frame.
    active: &'a mut bool,
    other: &'a mut BTreeMap<u32, u64>,
}

/// The list DMA: buffer RAM from the read pointer, following jump words.
struct ListDma<'a> {
    buffer: &'a [u8],
    jumps: &'a [u32],
    pos: usize,
    words: &'a mut usize,
    fed: &'a mut Vec<u32>,
}

impl ListDma<'_> {
    /// The next list word, or None once the frame's cap is reached.
    fn next(&mut self) -> Option<u32> {
        let mask = self.buffer.len() - 1;
        while *self.words < MAX_LIST_WORDS {
            let w = u32::from_le_bytes(self.buffer[self.pos..self.pos + 4].try_into().unwrap());
            *self.words += 1;
            if self.jumps.binary_search(&(self.pos as u32)).is_ok() {
                self.pos = w as usize & mask & !3;
                continue;
            }
            self.pos = (self.pos + 4) & mask;
            self.fed.push(w);
            return Some(w);
        }
        None
    }
}

/// What the list FIFO gives once the frame's cap is reached: END, the
/// word the games fill buffer RAM with.
const LIST_END: u32 = 0x0780_0f0f;

impl SharcBus for SharcGeoBus<'_> {
    fn read(&mut self, addr: u32, _: Width) -> u64 {
        match addr {
            0x40_0000 => {
                *self.active = true;
                self.list.next().unwrap_or(LIST_END) as u64
            }
            0x43_0001 => self.poly.read(*self.latch) as u64,
            _ => {
                *self.other.entry(addr).or_default() += 1;
                0
            }
        }
    }
    fn write(&mut self, addr: u32, _: Width, v: u64) {
        let v = v as u32;
        match addr {
            0x41_0000 => {
                if v != RASTER_NOP {
                    *self.active = true;
                }
                self.out.push(v);
            }
            0x42_0000..=0x42_ffff => self.poly.write(addr - 0x42_0000, v),
            0x43_0000 => *self.latch = v,
            // Written at each frame handshake; meaning unknown.
            0x43_0002 => {}
            _ => *self.other.entry(addr).or_default() += 1,
        }
    }
}

/// The geometrizer's DSP and everything it owns. Runs a frame at a time,
/// on a worker thread (or inline, see [`Geometrizer::set_threaded`]), from
/// a copy of buffer RAM taken at vblank.
pub struct Engine {
    pub tgp: Tgp,
    /// The 2B geometrizer, in place of `tgp`.
    pub sharc: Option<Box<Sharc>>,
    /// Its polygon memory address register.
    poly_latch: u32,
    /// Its external accesses outside the map above (diagnostics).
    pub sharc_other: BTreeMap<u32, u64>,
    poly_ram: Vec<u32>,
    poly_rom: Vec<u32>,
    /// Buffer RAM as it was when the frame started.
    buffer: Vec<u8>,
    /// Jump word offsets in `buffer` (see `Geometrizer::jumps`).
    jumps: Vec<u32>,
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
    pub core: Core,
    /// The uploaded program: 64-bit MB86235 words, or for a SHARC the host
    /// writes that boot it, each `IOP register << 32 | data`.
    pub program: Vec<u64>,
    uploading: bool,
    upload_half: Option<u32>,
    pub booted: bool,
    /// Byte offsets into buffer RAM.
    pub write_ptr: u32,
    pub read_ptr: u32,
    /// Command words that failed the `fn | fn << 8` check.
    pub bad_commands: u64,
    /// Jump words written through the command port: byte offset in buffer
    /// RAM -> the word. The list DMA follows these and only these: data
    /// words (negative floats) also have bit 31 set, so the bit alone
    /// can't mark a jump. One stops being a jump once anything overwrites
    /// it: the port's own appends drop it at once; writes from elsewhere
    /// (the i960 or the copro storing into buffer RAM directly, as stcc's
    /// do) are caught when the frame starts, by the word having changed.
    /// How the hardware tells them apart is unconfirmed.
    jumps: BTreeMap<u32, u32>,
    /// Host time the emulation thread spent waiting for the worker.
    pub waited: std::time::Duration,
    engine: Slot,
    threaded: bool,
    /// Keep a copy of each frame's output when the next one starts
    /// ([`Geometrizer::last_frame`]).
    keep_last: bool,
    last: (Vec<u32>, Vec<u16>),
    /// Remote mode: jobs go here instead of to the local engine.
    remote: Option<Box<dyn FnMut(Job)>>,
}

/// Work for an engine elsewhere (another worker or process): the program
/// the game uploaded, or one frame's list. An engine fed the same jobs in
/// order gives the same results as the local one ([`Engine::run_job`]).
pub enum Job {
    /// The uploaded program, as [`Geometrizer::program`] holds it.
    Boot(Vec<u64>),
    /// `jumps`: byte offsets in `buffer` of jump words (ascending).
    Frame { buffer: Vec<u8>, booted: bool, read_ptr: u32, jumps: Vec<u32> },
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
    pub fn new(poly_rom: Vec<u32>, core: Core) -> Self {
        Geometrizer {
            core,
            program: Vec::new(),
            uploading: false,
            upload_half: None,
            booted: false,
            write_ptr: 0,
            read_ptr: 0,
            bad_commands: 0,
            jumps: BTreeMap::new(),
            waited: std::time::Duration::ZERO,
            // WebAssembly without threads support can't spawn.
            threaded: cfg!(not(target_arch = "wasm32")),
            keep_last: false,
            last: (Vec::new(), Vec::new()),
            remote: None,
            engine: Slot::Idle(Box::new(Engine::new(poly_rom, core))),
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

    /// Keep the output of each frame (raster stream, texture point RAM)
    /// once the next one starts, for a front end that draws without
    /// waiting for the frame in progress: [`Geometrizer::last_frame`].
    pub fn set_keep_last(&mut self, keep: bool) {
        self.keep_last = keep;
    }

    /// The output of the frame before the one last started (with
    /// [`Geometrizer::set_keep_last`]; empty otherwise).
    pub fn last_frame(&self) -> (&[u32], &[u16]) {
        (&self.last.0, &self.last.1)
    }

    /// Wait for a frame running on the worker until `deadline`: true if
    /// it finished (its results are then the engine's, as after `sync`),
    /// false if it's still running.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn wait_until(&mut self, deadline: std::time::Instant) -> bool {
        loop {
            match &self.engine {
                Slot::Running(h) if !h.is_finished() => {
                    if std::time::Instant::now() >= deadline {
                        return false;
                    }
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                _ => {
                    self.sync();
                    return true;
                }
            }
        }
    }

    /// Move the engine out (for running it elsewhere, as after
    /// [`Geometrizer::set_remote`]), leaving an empty one.
    pub fn take_engine(&mut self) -> Engine {
        std::mem::replace(self.engine_mut(), Engine::placeholder())
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
        let word = |o: u32| u32::from_le_bytes(buffer[o as usize..o as usize + 4].try_into().unwrap());
        self.jumps.retain(|&o, w| word(o) == *w);
        let jumps: Vec<u32> = self.jumps.keys().copied().collect();
        if let Some(remote) = &mut self.remote {
            remote(Job::Frame { buffer: buffer.to_vec(), booted, read_ptr, jumps });
            return;
        }
        let mut e = std::mem::replace(self.engine_mut(), Engine::placeholder());
        if self.keep_last {
            self.last.0.clear();
            self.last.0.extend_from_slice(&e.stream);
            self.last.1.clear();
            self.last.1.extend_from_slice(&e.texture_point_ram);
        }
        e.buffer.clear();
        e.buffer.extend_from_slice(buffer);
        e.jumps = jumps;
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
        if !self.jumps.is_empty() {
            self.jumps.remove(&(o as u32));
        }
        buffer[o..o + 4].copy_from_slice(&word.to_le_bytes());
        self.write_ptr = (self.write_ptr + 4) & 0xf_ffff;
    }

    /// Write to the geo port window 0x00800000-0x00807FFF (`offset` from
    /// 0x00800000). Returns false if the offset isn't handled.
    pub fn port_write(&mut self, offset: u32, v: u32, buffer: &mut [u8]) -> bool {
        match offset {
            0x4000..=0x7fff if self.uploading && self.core == Core::Sharc => {
                // The SHARC's EPB0: 16-bit halves of 48-bit words.
                self.program.push((EPB0 as u64) << 32 | v as u64);
            }
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
                let jump = v & 0x8000_0000 != 0;
                let keep = if jump { 0x800f_ffff } else { 0x000f_ffff };
                let at = (self.write_ptr as usize & (buffer.len() - 1) & !3) as u32;
                let word = (v & keep) | (fnc << 23) | (eye << 29);
                self.append(buffer, word);
                if jump {
                    self.jumps.insert(at, word);
                }
            }
            // Command/data length writes (+4, +8): ignored by hardware models.
            0x0000..=0x0fff => {}
            _ => return false,
        }
        true
    }

    /// A write to the 2B geometrizer SHARC's host window (`0x840000`): IOP
    /// register `reg`. Kept with the upload while the host holds the bus;
    /// false if it isn't (no game is known to do that).
    pub fn host_write(&mut self, reg: u32, v: u32) -> bool {
        if self.core == Core::Sharc && self.uploading {
            self.program.push((reg as u64) << 32 | v as u64);
            return true;
        }
        false
    }

    /// The jump words, for save states (offset, word).
    pub fn jumps(&self) -> Vec<(u32, u32)> {
        self.jumps.iter().map(|(&o, &w)| (o, w)).collect()
    }

    pub fn set_jumps(&mut self, jumps: Vec<(u32, u32)>) {
        self.jumps = jumps.into_iter().collect();
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
                None => self.engine_mut().boot(&program),
            }
            self.program = program;
            self.booted = true;
        }
        self.uploading = on;
    }
}

impl Engine {
    /// A fresh engine over the polygon ROM (u32 words).
    pub fn new(poly_rom: Vec<u32>, core: Core) -> Self {
        Engine {
            tgp: Tgp::new(),
            sharc: (core == Core::Sharc).then(|| Box::new(Sharc::new())),
            poly_latch: 0,
            sharc_other: BTreeMap::new(),
            poly_ram: vec![0; 0x10_0000],
            poly_rom,
            buffer: Vec::new(),
            jumps: Vec::new(),
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
            Job::Boot(program) => self.boot(&program),
            Job::Frame { buffer, booted, read_ptr, jumps } => {
                self.buffer = buffer;
                self.jumps = jumps;
                self.run_frame(booted, read_ptr);
            }
        }
    }

    fn placeholder() -> Self {
        Engine {
            tgp: Tgp::new(),
            sharc: None,
            poly_latch: 0,
            sharc_other: BTreeMap::new(),
            poly_ram: Vec::new(),
            poly_rom: Vec::new(),
            buffer: Vec::new(),
            jumps: Vec::new(),
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

    /// Load an uploaded program ([`Geometrizer::program`]).
    pub fn boot(&mut self, program: &[u64]) {
        match &mut self.sharc {
            None => self.tgp.boot(program),
            Some(s) => {
                // A fresh SHARC in host boot mode, then the host's writes.
                // A write it can't take leaves `fault` at 0x80000000 | the
                // IOP register (a PC can't be that high).
                **s = Sharc::new();
                self.fault = None;
                for &w in program {
                    if s.host_write((w >> 32) as u32, w as u32).is_err() {
                        self.fault = Some(0x8000_0000 | (w >> 32) as u32);
                    }
                }
            }
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
        if self.sharc.is_some() {
            self.process_sharc(&buffer, read_ptr);
        } else {
            self.process(&buffer, read_ptr);
        }
        self.busy += t.elapsed();
        self.slowest = self.slowest.max(t.elapsed());
        self.buffer = buffer;
        self.apply_raster_writes();
    }

    fn process(&mut self, buffer: &[u8], read_ptr: u32) {
        self.frames += 1;
        let mut pos = read_ptr as usize & (buffer.len() - 1) & !3;
        let mut ext = GeoExt::new(&mut self.poly_ram, &self.poly_rom);
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
        let step = crate::recomp::geo(self.tgp.program_hash).unwrap_or(|t, e| t.step(e));
        let mut last_activity = self.tgp.steps;
        loop {
            while self.tgp.fifo_in.len() < FIFO_FILL && self.list_words < MAX_LIST_WORDS {
                let w = u32::from_le_bytes(buffer[pos..pos + 4].try_into().unwrap());
                if self.jumps.binary_search(&(pos as u32)).is_ok() {
                    // A jump: the DMA continues at the target and the
                    // geometrizer never sees the word (stcc links its list
                    // from parts this way).
                    pos = w as usize & (buffer.len() - 1) & !3;
                    self.list_words += 1;
                    continue;
                }
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

impl Engine {
    /// [`Engine::process`] for the 2B SHARC. The handshake is the 2C
    /// program's (topskatr's geometrizer, docs/hw/geometrizer.md 1.2): at
    /// a frame start FLAG0 rises; the program abandons what it was doing,
    /// emits NOP and END (the last frame's terminator), and waits for FLAG0
    /// to fall before reading the list from its input FIFO.
    fn process_sharc(&mut self, buffer: &[u8], read_ptr: u32) {
        self.frames += 1;
        let Some(s) = self.sharc.as_deref_mut() else { return };
        let mut out = Vec::new();
        let mut active = false;
        let mut bus = SharcGeoBus {
            poly: GeoExt::new(&mut self.poly_ram, &self.poly_rom),
            latch: &mut self.poly_latch,
            list: ListDma {
                buffer,
                jumps: &self.jumps,
                pos: read_ptr as usize & (buffer.len() - 1) & !3,
                words: &mut self.list_words,
                fed: &mut self.fed,
            },
            out: &mut out,
            active: &mut active,
            other: &mut self.sharc_other,
        };
        let start = s.steps;
        let mut fault = None;
        // FLAG0 high until the program waits for it to fall again.
        s.flag_in |= 1;
        for k in 0..SHARC_PULSE_STEPS {
            match s.step(&mut bus) {
                SharcStep::Ran => {}
                SharcStep::Idle => break,
                SharcStep::Fault(f) => {
                    fault = Some(f);
                    break;
                }
            }
            if k > 0 && s.spinning() {
                break;
            }
        }
        s.flag_in &= !1;
        bus.out.clear();
        *bus.active = false;
        let mut last_activity = s.steps;
        while fault.is_none() {
            match s.step(&mut bus) {
                SharcStep::Ran => {}
                SharcStep::Idle => break,
                SharcStep::Fault(f) => fault = Some(f),
            }
            if std::mem::take(&mut *bus.active) {
                last_activity = s.steps;
            }
            // Every so often: done (idle, waiting on a flag, or out of time)?
            if s.steps % 256 == 0
                && (s.steps - last_activity > IDLE_STEPS || s.steps - start > MAX_STEPS_PER_FRAME || s.spinning())
            {
                break;
            }
        }
        if let Some(f) = fault {
            self.fault = Some(match f {
                adsp2106x::cpu::Fault::Undefined { pc, .. }
                | adsp2106x::cpu::Fault::Unimplemented { pc, .. }
                | adsp2106x::cpu::Fault::Iop { pc, .. }
                | adsp2106x::cpu::Fault::BadAddress { pc, .. }
                | adsp2106x::cpu::Fault::Stack { pc, .. } => pc,
            });
        }
        self.stream.extend(out.into_iter().filter(|&w| w != RASTER_NOP));
    }
}

// Save states. The front end and, after any running frame finishes, the
// engine (its TGP, polygon RAM, texture point RAM and last frame's
// output); the polygon ROM and diagnostics stay. In remote mode the engine
// lives elsewhere and isn't saved.
impl state::State for Geometrizer {
    fn save(&self, w: &mut state::Writer) {
        self.program.save(w);
        self.uploading.save(w);
        self.upload_half.save(w);
        self.booted.save(w);
        self.write_ptr.save(w);
        self.read_ptr.save(w);
        self.bad_commands.save(w);
        match &self.engine {
            Slot::Idle(e) => e.save(w),
            _ => panic!("geometrizer frame still running: call sync() before saving"),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.program.load(r)?;
        self.uploading.load(r)?;
        self.upload_half.load(r)?;
        self.booted.load(r)?;
        self.write_ptr.load(r)?;
        self.read_ptr.load(r)?;
        self.bad_commands.load(r)?;
        self.engine_mut().load(r)
    }
}

// The 2C fields, then a 2B engine's SHARC (2C states are unchanged).
impl state::State for Engine {
    fn save(&self, w: &mut state::Writer) {
        self.tgp.save(w);
        self.poly_ram.save(w);
        self.buffer.save(w);
        self.stream.save(w);
        self.list_words.save(w);
        self.fault.save(w);
        self.frames.save(w);
        self.texture_point_ram.save(w);
        if let Some(s) = &self.sharc {
            s.save(w);
            self.poly_latch.save(w);
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.tgp.load(r)?;
        self.poly_ram.load(r)?;
        self.buffer.load(r)?;
        self.stream.load(r)?;
        self.list_words.load(r)?;
        self.fault.load(r)?;
        self.frames.load(r)?;
        self.texture_point_ram.load(r)?;
        if let Some(s) = &mut self.sharc {
            s.load(r)?;
            self.poly_latch.load(r)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jump words come only from the command port, and stop being jumps
    /// once overwritten: by the data port at once, by anything else when
    /// the next frame starts.
    #[test]
    fn list_jumps_are_tracked() {
        let mut g = Geometrizer::new(Vec::new(), Core::Tgp);
        g.set_threaded(false);
        let mut buffer = vec![0u8; 0x2_0000];
        g.port_write(0x1008, 0x100, &mut buffer);
        g.port_write(0x0000, 0x8000_4000, &mut buffer);
        // A negative float through the data port is not a jump.
        g.port_write(0x4000, 0xbf80_0000, &mut buffer);
        g.port_write(0x1008, 0x200, &mut buffer);
        g.port_write(0x0000, 0x8000_8000, &mut buffer);
        assert_eq!(g.jumps(), vec![(0x100, 0x8000_4000), (0x200, 0x8000_8000)]);
        // Data appended over the first jump.
        g.port_write(0x1008, 0x100, &mut buffer);
        g.port_write(0x4000, 0x1234, &mut buffer);
        assert_eq!(g.jumps(), vec![(0x200, 0x8000_8000)]);
        // Written directly (as the copro or the i960 can): dropped at the
        // next frame.
        buffer[0x200..0x204].copy_from_slice(&0x0580_0b0bu32.to_le_bytes());
        g.start_frame(&buffer);
        assert!(g.jumps().is_empty());
    }
}
