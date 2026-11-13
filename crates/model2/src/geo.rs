//! Model 2B/2C geometrizer: a DSP running a program the game uploads,
//! fed with a display list from buffer RAM once per frame. Its output FIFO
//! is the command stream for the fixed-function rasteriser. On 2C it is a
//! second MB86235 (protocol per docs/hw/geometrizer.md sections 1-2); on
//! 2B an ADSP-21062 SHARC, booted through its host interface, whose
//! program emits the same stream (docs/hw/model2b.md).

use adsp2106x::cpu::{Bus as SharcBus, Sharc, Step as SharcStep, Width};
use mb86235::cpu::{External, Step, Tgp};
use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

/// The geometrizer's processor, which sets how it is programmed and fed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Core {
    /// Model 2C: an MB86235, its program uploaded as 64-bit words.
    Tgp,
    /// Model 2B: an ADSP-21062 SHARC, booted through its IOP registers.
    Sharc,
    /// Model 2A: fixed-function hardware, no program (docs/hw/model2a.md).
    /// Always the native geometrizer, from the first frame.
    Fixed,
}

impl Core {
    pub fn of(board: romset::Board) -> Self {
        match board {
            romset::Board::Model2B => Core::Sharc,
            romset::Board::Model2A | romset::Board::Model2 => Core::Fixed,
            _ => Core::Tgp,
        }
    }
}

/// Which implementation of the uploaded geometrizer program runs a frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GeoMode {
    /// The uploaded program on the geometrizer's own processor, the
    /// "chip" (a 2B SHARC, a 2C TGP; recompiled if the build has it): the
    /// reference, and the default.
    #[default]
    Chip,
    /// The native implementation (`geo_native` on 2B, `geo_native_2c` on
    /// 2C) when it implements the uploaded program, else the chip.
    Native,
    /// The chip's output, with the native one computed beside it from the
    /// same input and compared word by word ([`Engine::check`]).
    Check,
}

impl GeoMode {
    /// `chip` (or by the processor's name, `sharc` or `tgp`), `native` or
    /// `check`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "chip" | "sharc" | "tgp" => Some(GeoMode::Chip),
            "native" => Some(GeoMode::Native),
            "check" => Some(GeoMode::Check),
            _ => None,
        }
    }
}

/// The first difference `GeoMode::Check` found in a frame.
#[derive(Clone, Debug)]
pub struct Mismatch {
    /// Frame (the engine's count) and index of the first differing word.
    pub frame: u64,
    pub index: usize,
    pub sharc_len: usize,
    pub native_len: usize,
    /// Words from `start` on, around the difference, from each.
    pub start: usize,
    pub sharc: Vec<u32>,
    pub native: Vec<u32>,
    /// The raster command word the difference falls under, and its index.
    pub command: Option<(usize, u32)>,
}

impl Mismatch {
    /// The difference at word `i` of frame `frame`'s streams.
    fn at(frame: u64, i: usize, chip: &[u32], native: &[u32]) -> Self {
        let start = i.saturating_sub(16);
        let command = chip[..i.min(chip.len())].iter().rposition(|&w| w >> 24 == 0xff).map(|k| (k, chip[k]));
        Mismatch {
            frame,
            index: i,
            sharc_len: chip.len(),
            native_len: native.len(),
            start,
            sharc: chip.iter().skip(start).take(32).copied().collect(),
            native: native.iter().skip(start).take(32).copied().collect(),
            command,
        }
    }
}

/// The index of the first word two streams differ at, if they do.
fn first_difference(a: &[u32], b: &[u32]) -> Option<usize> {
    let first = a.iter().zip(b).position(|(x, y)| x != y);
    first.or((a.len() != b.len()).then(|| a.len().min(b.len())))
}

/// What `GeoMode::Check` found, and how long each side took.
#[derive(Clone, Debug, Default)]
pub struct CheckReport {
    /// Frames compared, and those whose streams differed.
    pub frames: u64,
    pub differing: u64,
    /// Words compared in all.
    pub words: u64,
    /// Frames after which the program's state words differed (with the
    /// first: frame, data memory address, SHARC's, native's).
    pub state_differing: u64,
    pub first_state: Option<(u64, u32, u32, u32)>,
    pub first: Option<Mismatch>,
    /// Host time per frame, in milliseconds: (SHARC, native).
    pub times: Vec<(f32, f32)>,
    /// List words the two read differently (frames).
    pub list_differing: u64,
    /// With smooth shading's side channel on ([`Engine::set_light`]): the
    /// list scan (the chip path's side channel) against the native
    /// geometrizer's block facts, in `GeoMode::Check` and on every frame a
    /// native geometrizer runs.
    pub light: LightCheck,
}

/// The list scan's block facts (`crate::light::scan_list`, the chip path)
/// against the native geometrizer's (its light tap) for the same list,
/// frame by frame.
#[derive(Clone, Debug, Default)]
pub struct LightCheck {
    /// Frames compared; of them, those the scan gave up on, and those
    /// with another number of blocks.
    pub frames: u64,
    pub scan_failed: u64,
    pub count_differing: u64,
    /// Blocks compared: all facts equal (model address, direct data,
    /// computed normals, specular, focal lengths, light vector); equal
    /// where the scan knew them (the list hadn't set the light or the mode
    /// yet); focal lengths not known yet; differing.
    pub equal: u64,
    pub partial: u64,
    pub unknown: u64,
    pub differing: u64,
    /// The first differing block: frame, block, native's, scan's.
    pub first: Option<(u64, usize, crate::light::Block, crate::light::Block)>,
}

/// The polygon memory address register, as `CheckReport::first_state`
/// names it.
pub const POLY_ADDRESS_LATCH: u32 = 0x43_0000;
/// Polygon RAM word 0 as `CheckReport::first_state` names it on 2C (the
/// TGP's own words are below: `geo_native_2c::Native::compare`).
pub const POLY_RAM_STATE: u32 = 0x100_0000;

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
pub(crate) const RASTER_NOP: u32 = 0xff00_0000;

/// A raster stream's commands as the rasteriser reads them: (index,
/// command word), skipping the operands of the window (`0x3`), `0x8` and
/// memory write (`0x4`) commands, and data words between commands.
fn raster_commands(stream: &[u32]) -> impl Iterator<Item = (usize, u32)> + '_ {
    let mut i = 0;
    std::iter::from_fn(move || {
        while i < stream.len() {
            let (at, w) = (i, stream[i]);
            i += 1;
            if w >> 24 != 0xff {
                continue;
            }
            i += match w & 0xf {
                0x3 => 6,
                0x8 => 1,
                0x4 => 2 + stream.get(i + 1).map_or(0, |&n| n as usize),
                _ => 0,
            };
            return Some((at, w));
        }
        None
    })
}

/// The last window command in a raster stream (its command word and six
/// operands): the window the rasteriser is left with after it.
pub(crate) fn last_window(stream: &[u32]) -> Option<[u32; 7]> {
    let (at, _) = raster_commands(stream).filter(|&(_, w)| w & 0xf == 0x3).last()?;
    stream.get(at..at + 7).map(|s| s.try_into().unwrap())
}

/// The rasteriser keeps its window from frame to frame: a frame whose
/// stream draws an object before it sets any window draws it in the
/// window the frame before was left with (`vf2`'s player-select screen
/// sends no window command; docs/games/vf2.md). The renderers take each
/// frame's stream on its own, so that window is put at the start of a
/// stream that doesn't begin with its own, which also carries it through
/// frames that set none.
pub(crate) fn carry_window(stream: &mut Vec<u32>, window: Option<[u32; 7]>) {
    let Some(window) = window else { return };
    let first = raster_commands(stream).map(|(_, w)| w & 0xf).find(|&c| c == 0x3 || c == 0x1);
    if first != Some(0x3) {
        stream.splice(0..0, window);
    }
}

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
    ram: PolyRam<'a>,
    rom: &'a [u32],
    /// Word-address mask for the ROM: its size rounded up to a power of two.
    rom_mask: u32,
    /// Polygon RAM was written.
    pub(crate) wrote: bool,
}

/// Polygon RAM as a [`GeoExt`] sees it: its own, or (for smooth motion's
/// in-betweens, `crate::smooth`) a copy read through an overlay that
/// takes the writes, leaving the copy as it was, or (for its motion data)
/// a copy read as it is whatever is written, the accesses noted.
enum PolyRam<'a> {
    Own(&'a mut [u32]),
    Overlay(&'a [u32], &'a mut HashMap<u32, u32>),
    Before(&'a [u32], &'a mut Vec<crate::smooth::Access>),
}

impl<'a> GeoExt<'a> {
    pub(crate) fn new(ram: &'a mut [u32], rom: &'a [u32]) -> Self {
        let rom_mask = (rom.len().next_power_of_two().max(1) - 1) as u32;
        GeoExt { ram: PolyRam::Own(ram), rom, rom_mask, wrote: false }
    }

    /// Polygon RAM `ram`, unchanged: writes go to `overlay`.
    pub(crate) fn overlay(ram: &'a [u32], overlay: &'a mut HashMap<u32, u32>, rom: &'a [u32]) -> Self {
        let rom_mask = (rom.len().next_power_of_two().max(1) - 1) as u32;
        GeoExt { ram: PolyRam::Overlay(ram, overlay), rom, rom_mask, wrote: false }
    }

    /// Polygon RAM `ram`, read as it is whatever is written (the RAM the
    /// frame before left): reads and writes, and each object or direct
    /// data block begun ([`GeoExt::block`]), are noted in `log`.
    pub(crate) fn before(ram: &'a [u32], log: &'a mut Vec<crate::smooth::Access>, rom: &'a [u32]) -> Self {
        let rom_mask = (rom.len().next_power_of_two().max(1) - 1) as u32;
        GeoExt { ram: PolyRam::Before(ram, log), rom, rom_mask, wrote: false }
    }

    /// An object or direct data block begins (noted by [`GeoExt::before`]).
    #[inline]
    pub(crate) fn block(&mut self) {
        if let PolyRam::Before(_, log) = &mut self.ram {
            log.push(crate::smooth::Access::Block);
        }
    }
}

impl External for GeoExt<'_> {
    #[inline]
    fn read(&mut self, addr: u32) -> u32 {
        if addr >= 0x80_0000 {
            self.rom.get((addr & self.rom_mask) as usize).copied().unwrap_or(0)
        } else {
            match &mut self.ram {
                PolyRam::Own(ram) => ram[addr as usize % ram.len()],
                PolyRam::Overlay(ram, o) => {
                    let i = addr as usize % ram.len();
                    o.get(&(i as u32)).copied().unwrap_or(ram[i])
                }
                PolyRam::Before(ram, log) => {
                    let i = addr as usize % ram.len();
                    log.push(crate::smooth::Access::Read(i as u32));
                    ram[i]
                }
            }
        }
    }
    #[inline]
    fn write(&mut self, addr: u32, v: u32) {
        if addr < 0x80_0000 {
            self.wrote = true;
            match &mut self.ram {
                PolyRam::Own(ram) => {
                    let n = ram.len();
                    ram[addr as usize % n] = v;
                }
                PolyRam::Overlay(ram, o) => {
                    o.insert((addr as usize % ram.len()) as u32, v);
                }
                PolyRam::Before(ram, log) => log.push(crate::smooth::Access::Write((addr as usize % ram.len()) as u32)),
            }
        }
    }
}

/// The 2B geometrizer SHARC's external bus (from its program): the list
/// FIFO at `0x400000`, the rasteriser FIFO at `0x410000`, polygon RAM
/// writes at `0x420000 + address`, and polygon memory reads through an
/// address register at `0x430000` with the data at `0x430001`. Its fast
/// polygon RAM (model addresses with bit 24 set) is the SHARC's own
/// internal memory and never reaches the bus.
pub struct SharcGeoBus<'a> {
    poly: GeoExt<'a>,
    /// Polygon memory address register (`0x430000`).
    latch: &'a mut u32,
    list: ListDma<'a>,
    out: &'a mut Vec<u32>,
    /// Set by a list read or a rasteriser word other than a NOP: the
    /// program is still working on the frame.
    active: &'a mut bool,
    other: &'a mut BTreeMap<u32, u64>,
    /// When to stop running (`after_step`).
    phase: Phase,
}

/// The two parts of a 2B geometrizer frame (`Engine::process_sharc`).
enum Phase {
    /// FLAG0 is high: run until the program waits for it to fall, at most
    /// `SHARC_PULSE_STEPS`; from the core's `steps` at `start`.
    Pulse { start: u64 },
    /// The list: run until the program has gone `IDLE_STEPS` without
    /// reading the list or drawing, spins on a flag, or has run
    /// `MAX_STEPS_PER_FRAME`; checked every 256 instructions.
    Frame { start: u64, last_activity: u64 },
}

/// The list DMA: buffer RAM from the read pointer, following jump words.
pub struct ListDma<'a> {
    buffer: &'a [u8],
    jumps: &'a [u32],
    pos: usize,
    words: &'a mut usize,
    fed: &'a mut Vec<u32>,
}

impl<'a> ListDma<'a> {
    /// The list from byte offset `read_ptr` of `buffer`; `jumps` are the
    /// offsets of jump words (ascending), `words` counts what it reads and
    /// `fed` gets every word it hands out.
    pub(crate) fn new(
        buffer: &'a [u8],
        jumps: &'a [u32],
        read_ptr: u32,
        words: &'a mut usize,
        fed: &'a mut Vec<u32>,
    ) -> Self {
        ListDma { buffer, jumps, pos: read_ptr as usize & (buffer.len() - 1) & !3, words, fed }
    }

    /// The next list word, or None once the frame's cap is reached.
    pub(crate) fn next(&mut self) -> Option<u32> {
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

/// A command word with this bit set is a jump (`Geometrizer::jumps`).
const JUMP: u32 = 0x8000_0000;

/// What the list FIFO gives once the frame's cap is reached: END, the
/// word the games fill buffer RAM with.
pub(crate) const LIST_END: u32 = 0x0780_0f0f;

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
    #[inline(always)]
    fn after_step(&mut self, cpu: &mut Sharc) -> bool {
        match &mut self.phase {
            Phase::Pulse { start } => {
                // Instructions so far, this one included.
                let k = cpu.steps - *start;
                !(k > 1 && cpu.spinning()) && k < SHARC_PULSE_STEPS
            }
            Phase::Frame { start, last_activity } => {
                let steps = cpu.steps;
                if std::mem::take(&mut *self.active) {
                    *last_activity = steps;
                }
                // Every so often: done (idle, waiting on a flag, or out of time)?
                !(steps.is_multiple_of(256)
                    && (steps - *last_activity > IDLE_STEPS || steps - *start > MAX_STEPS_PER_FRAME || cpu.spinning()))
            }
        }
    }
    /// To the end of the pulse, or the next multiple of 256 instructions:
    /// before that `after_step` only notes activity, which only an
    /// external access makes (`after_access`), and the core can't be
    /// spinning inside a block.
    #[inline(always)]
    fn block_budget(&mut self, cpu: &Sharc) -> u64 {
        match self.phase {
            Phase::Pulse { start } => SHARC_PULSE_STEPS.saturating_sub(cpu.steps - start),
            Phase::Frame { .. } => 256 - cpu.steps % 256,
        }
    }
    #[inline(always)]
    fn after_access(&mut self, cpu: &mut Sharc) {
        if let Phase::Frame { last_activity, .. } = &mut self.phase {
            if std::mem::take(&mut *self.active) {
                *last_activity = cpu.steps;
            }
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
    /// Recompiled code for its program, if the build has it
    /// (docs/recompiler.md); `sharc_interpret` keeps it on the interpreter.
    sharc_native: Option<crate::recomp::SharcGeoRun>,
    pub sharc_interpret: bool,
    poly_ram: Vec<u32>,
    poly_rom: Arc<Vec<u32>>,
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
    /// Which implementation runs frames, and the native one if it
    /// implements the uploaded program (only with a mode other than Sharc).
    pub mode: GeoMode,
    pub native: Option<Box<crate::geo_native::Native>>,
    /// The 2C native geometrizer, likewise.
    pub native_tgp: Option<Box<crate::geo_native_2c::Native>>,
    /// Model 2A: the fixed-function geometrizer, which is `native_tgp` in
    /// its fixed form, whatever the mode.
    fixed: bool,
    /// List words the TGP's program read in the most recent frame (the
    /// DMA's read-ahead not counted; `GeoMode::Check`).
    list_consumed: usize,
    /// `GeoMode::Check`: the native side's own polygon RAM, and what the
    /// comparison found.
    check_poly: Vec<u32>,
    check_latch: u32,
    pub check: CheckReport,
    /// Smooth motion (`crate::smooth`), when it's on.
    smooth: Option<Box<Smooth>>,
    /// Smooth shading's side channel (`crate::light`), when it's on.
    light: Option<Box<LightSide>>,
    /// Kept for [`Engine::copy_from`]'s SHARC state.
    copy_buf: Vec<u8>,
}

/// What smooth motion keeps between frames ([`Engine::set_smooth`]).
#[derive(Default)]
struct Smooth {
    /// The last frame's transforms, to match the next frame's with.
    prev: Vec<crate::smooth::Xform>,
    /// Polygon RAM as it was before the frame being run, shared with its
    /// in-betweens; stale after a frame that may have written it.
    poly: Arc<Vec<u32>>,
    stale: bool,
    /// The frame being run: the native state and address latch before it,
    /// what it recorded, and whether it wrote polygon RAM.
    pending: Option<(crate::smooth::Native, u32, Vec<crate::smooth::Xform>, bool)>,
    /// The in-betweens of the last frame, if it can have any.
    frame: Option<Arc<crate::smooth::Frame>>,
    /// Host time spent on it in the frames (copies and matching; not
    /// drawing in-betweens, which the front end does).
    busy: std::time::Duration,
    /// The frames being run aren't shown: leave the last shown one's
    /// transforms and in-betweens as they are
    /// ([`Geometrizer::set_smooth_shown`]).
    skip: bool,
}

impl Smooth {
    /// Before a frame: polygon RAM `ram` as the frame starts, if the copy
    /// is stale.
    fn begin(&mut self, ram: &[u32]) {
        if std::mem::take(&mut self.stale) {
            match Arc::get_mut(&mut self.poly) {
                Some(v) => {
                    v.clear();
                    v.extend_from_slice(ram);
                }
                None => self.poly = Arc::new(ram.to_vec()),
            }
        }
    }

    /// Nothing to blend the next frame with (after a load, a mode change,
    /// an upload).
    fn reset(&mut self) {
        self.prev.clear();
        self.pending = None;
        self.frame = None;
        self.stale = true;
    }
}

/// What smooth shading's side channel keeps ([`Engine::set_light`]).
#[derive(Default)]
struct LightSide {
    /// The last frame's.
    frame: Option<Arc<crate::light::Frame>>,
    /// The chip path's list scan: the state it carries between frames,
    /// and the program it reads lists for (worked out once an upload).
    scan: crate::light::ScanState,
    program: Option<crate::light::ScanProgram>,
    /// The last frame's list scan.
    scanned: Option<Arc<crate::light::Frame>>,
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
    /// Jump words written through the command port, and fn 0 command
    /// words that may become one: byte offset in buffer RAM -> the word
    /// there when the last frame started. The list DMA follows the jumps
    /// among these (bit 31 set) and only those: data words (negative
    /// floats) also have bit 31 set, so the bit alone can't mark a jump.
    /// The i960 writing buffer RAM directly keeps a command word one, with
    /// the new value: fvipers writes fn 0 placeholders through the port
    /// and later patches them into jumps, or retargets jumps, that way.
    /// The port's own appends and the copro's stores into buffer RAM
    /// (stcc's matrices over old jumps; fvipers' and schamp's list parts)
    /// end one ([`Geometrizer::commands`]). How the hardware tells them
    /// apart is unconfirmed.
    jumps: BTreeMap<u32, u32>,
    /// Host time the emulation thread spent waiting for the worker.
    pub waited: std::time::Duration,
    engine: Slot,
    threaded: bool,
    /// Keep a copy of each frame's output when the next one starts
    /// ([`Geometrizer::last_frame`]).
    keep_last: bool,
    last: (Vec<u32>, Vec<u16>),
    /// That frame's smooth motion data, if it had any (render-only, not
    /// saved).
    last_smooth: Option<Arc<crate::smooth::Frame>>,
    /// That frame's lighting side channel, likewise.
    last_light: Option<Arc<crate::light::Frame>>,
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
pub(crate) struct Stopwatch(#[cfg(not(target_arch = "wasm32"))] std::time::Instant);

impl Stopwatch {
    pub(crate) fn start() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        return Stopwatch(std::time::Instant::now());
        #[cfg(target_arch = "wasm32")]
        return Stopwatch();
    }

    pub(crate) fn elapsed(&self) -> std::time::Duration {
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
            booted: core == Core::Fixed,
            write_ptr: 0,
            read_ptr: 0,
            bad_commands: 0,
            jumps: BTreeMap::new(),
            waited: std::time::Duration::ZERO,
            // WebAssembly without threads support can't spawn.
            threaded: cfg!(not(target_arch = "wasm32")),
            keep_last: false,
            last: (Vec::new(), Vec::new()),
            last_smooth: None,
            last_light: None,
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

    /// The smooth motion data of [`Geometrizer::last_frame`]'s frame
    /// ([`crate::smooth::Frame`], for its motion vectors), if smooth motion
    /// was on and the native geometrizer ran it.
    pub fn last_smooth(&self) -> Option<&Arc<crate::smooth::Frame>> {
        self.last_smooth.as_ref()
    }

    /// The lighting side channel of [`Geometrizer::last_frame`]'s frame
    /// ([`Engine::light_frame`]), if it was on.
    pub fn last_light(&self) -> Option<&Arc<crate::light::Frame>> {
        self.last_light.as_ref()
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

    /// Keep a 2B geometrizer's SHARC on the interpreter even when the build
    /// has recompiled code for its program (`make lockstep`'s reference).
    pub fn set_sharc_interpret(&mut self, interpret: bool) {
        let e = self.engine_mut();
        e.sharc_interpret = interpret;
        e.attach_sharc();
    }

    /// Run frames with `mode` (`GeoMode`): the SHARC, the native
    /// geometrizer where it implements the program, or both compared.
    pub fn set_mode(&mut self, mode: GeoMode) {
        self.engine_mut().set_mode(mode);
    }

    /// Smooth motion on or off ([`Engine::set_smooth`]; the local engine).
    pub fn set_smooth(&mut self, on: bool) {
        self.engine_mut().set_smooth(on);
    }

    /// Smooth shading's side channel on or off ([`Engine::set_light`]; the
    /// local engine).
    pub fn set_light(&mut self, on: bool) {
        self.engine_mut().set_light(on);
    }

    /// Start a frame on the worker thread from the list at the read
    /// pointer. Runs in the background until the next `sync`.
    pub fn start_frame(&mut self, buffer: &[u8]) {
        let (booted, read_ptr) = (self.booted, self.read_ptr);
        let word = |o: u32| u32::from_le_bytes(buffer[o as usize..o as usize + 4].try_into().unwrap());
        // The i960's own writes to buffer RAM keep a command word one
        // (fvipers writes fn 0 placeholders through the port and patches
        // them into jumps, or retargets jumps, in buffer RAM); a jump is a
        // command word with bit 31 set.
        for (&o, w) in self.jumps.iter_mut() {
            *w = word(o);
        }
        let jumps: Vec<u32> = self.jumps.iter().filter(|(_, w)| *w & JUMP != 0).map(|(&o, _)| o).collect();
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
            self.last_smooth = e.smooth_frame();
            self.last_light = e.light_frame();
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
                if jump || fnc == 0 {
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

    /// The command words the list DMA may follow as jumps, for the copro,
    /// whose stores into buffer RAM end them.
    pub fn commands(&mut self) -> &mut BTreeMap<u32, u32> {
        &mut self.jumps
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
        let fixed = core == Core::Fixed;
        Engine {
            fixed,
            tgp: Tgp::new(),
            sharc: (core == Core::Sharc).then(|| Box::new(Sharc::new())),
            poly_latch: 0,
            sharc_other: BTreeMap::new(),
            sharc_native: None,
            sharc_interpret: false,
            poly_ram: vec![0; 0x10_0000],
            poly_rom: Arc::new(poly_rom),
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
            mode: if fixed { GeoMode::Native } else { GeoMode::Chip },
            native: None,
            native_tgp: fixed.then(|| Box::new(crate::geo_native_2c::Native::fixed())),
            list_consumed: 0,
            check_poly: Vec::new(),
            check_latch: 0,
            check: CheckReport::default(),
            smooth: None,
            light: None,
            copy_buf: Vec::new(),
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
            fixed: false,
            tgp: Tgp::new(),
            sharc: None,
            poly_latch: 0,
            sharc_other: BTreeMap::new(),
            sharc_native: None,
            sharc_interpret: false,
            poly_ram: Vec::new(),
            poly_rom: Arc::new(Vec::new()),
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
            mode: GeoMode::Chip,
            native: None,
            native_tgp: None,
            list_consumed: 0,
            check_poly: Vec::new(),
            check_latch: 0,
            check: CheckReport::default(),
            smooth: None,
            light: None,
            copy_buf: Vec::new(),
        }
    }

    /// Use `mode` from the next frame. A native geometrizer for the program
    /// the SHARC holds takes over the SHARC's state ([`Engine::attach_native`]).
    pub fn set_mode(&mut self, mode: GeoMode) {
        if self.fixed {
            return;
        }
        if mode == self.mode && (self.native.is_some() || self.native_tgp.is_some()) {
            return;
        }
        // The chip hasn't run the native frames: give it their state first.
        if self.mode == GeoMode::Native {
            if let (Some(n), Some(s)) = (&self.native, self.sharc.as_deref_mut()) {
                n.export(s);
            }
            if let Some(n) = self.native_tgp.take() {
                if crate::geo_native_2c::fresh(&self.tgp) {
                    self.tgp = crate::geo_native_2c::started(&self.tgp);
                }
                n.export(&mut self.tgp);
            }
        }
        self.mode = mode;
        self.attach_native();
    }

    /// With a mode other than Sharc: the native geometrizer for the program
    /// in the SHARC, if `geo_native` implements it, starting from the
    /// SHARC's state (the frames since the upload are taken to be the
    /// engine's frame count: games upload once).
    pub fn attach_native(&mut self) {
        if self.fixed {
            // No program to recognise: its state is in the TGP's, as a
            // save puts it there.
            if let Some(n) = self.native_tgp.as_deref_mut() {
                n.import(&self.tgp, false);
            }
            return;
        }
        self.native = None;
        self.native_tgp = None;
        if let Some(s) = &mut self.smooth {
            s.reset();
        }
        if self.mode == GeoMode::Chip || self.fault.is_some() {
            return;
        }
        let Some(s) = self.sharc.as_deref() else {
            self.attach_native_tgp();
            return;
        };
        let image = adsp2106x::cpu::program_image(s);
        let Some(program) = crate::geo_native::recognise(&image) else { return };
        let mut n = crate::geo_native::Native::new(program, s.iop.get(0xf).copied().unwrap_or(0));
        if self.frames == 0 {
            // Just uploaded: the state its start-up code sets up (it runs
            // as the first frame starts), from a copy run until it waits.
            n.import(&started(s), 0);
        } else {
            n.import(s, self.frames);
        }
        self.native = Some(Box::new(n));
        if self.mode == GeoMode::Check {
            self.check_poly.clone_from(&self.poly_ram);
            self.check_latch = self.poly_latch;
        }
    }

    /// [`Engine::attach_native`] for a 2C TGP: the native geometrizer for
    /// its program by the program's hash, from the TGP's state (run
    /// through its start-up code if it hasn't run yet).
    fn attach_native_tgp(&mut self) {
        let Some(program) = crate::geo_native_2c::recognise(self.tgp.program_hash) else { return };
        let mut n = crate::geo_native_2c::Native::new(program);
        if crate::geo_native_2c::fresh(&self.tgp) {
            n.import(&crate::geo_native_2c::started(&self.tgp), true);
        } else {
            n.import(&self.tgp, false);
        }
        self.native_tgp = Some(Box::new(n));
        if self.mode == GeoMode::Check {
            self.check_poly.clone_from(&self.poly_ram);
        }
    }

    /// Load an uploaded program ([`Geometrizer::program`]).
    pub fn boot(&mut self, program: &[u64]) {
        if let Some(l) = self.light.as_deref_mut() {
            *l = LightSide::default();
        }
        match &mut self.sharc {
            None => {
                self.tgp.boot(program);
                self.attach_native();
            }
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
                self.attach_sharc();
                self.attach_native();
            }
        }
    }

    /// Use the build's recompiled code for the program the SHARC holds, if
    /// it has some and `sharc_interpret` isn't set.
    pub fn attach_sharc(&mut self) {
        self.sharc_native = None;
        let Some(s) = self.sharc.as_deref_mut() else { return };
        if self.sharc_interpret {
            s.watch_code(&[], &[]);
        } else {
            self.sharc_native = crate::recomp::attach_sharc(s, crate::recomp::SharcRole::Geo).and_then(|p| p.geo);
            if self.sharc_native.is_none() {
                s.watch_code(&[], &[]);
            }
        }
    }

    /// True if the 2B geometrizer runs recompiled code.
    pub fn sharc_is_native(&self) -> bool {
        self.sharc_native.is_some()
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
        let window = last_window(&self.stream);
        self.stream.clear();
        self.fed.clear();
        self.list_words = 0;
        if let Some(l) = self.light.as_deref_mut() {
            l.frame = None;
            l.scanned = None;
        }
        if !booted || self.fault.is_some() {
            return;
        }
        let t = Stopwatch::start();
        let buffer = std::mem::take(&mut self.buffer);
        match (self.mode, self.native.is_some(), self.native_tgp.is_some()) {
            (GeoMode::Native, true, _) => self.process_native(&buffer, read_ptr),
            (GeoMode::Check, true, _) => self.process_check(&buffer, read_ptr),
            (GeoMode::Native, _, true) => self.process_native_tgp(&buffer, read_ptr),
            (GeoMode::Check, _, true) => self.process_check_tgp(&buffer, read_ptr),
            _ if self.sharc.is_some() => self.process_sharc(&buffer, read_ptr),
            _ => self.process(&buffer, read_ptr),
        }
        self.busy += t.elapsed();
        self.slowest = self.slowest.max(t.elapsed());
        self.buffer = buffer;
        carry_window(&mut self.stream, window);
        self.apply_raster_writes();
        self.finish_smooth(read_ptr, window);
    }

    /// Make this engine's state `src`'s ([`Geometrizer::copy_from`]): what
    /// a save and load would carry over, copied directly, and for the
    /// native geometrizer its own state (a load imports it from the
    /// chip's), so no chip state is copied when it runs the frames. Smooth
    /// motion's record stays this engine's own: its next shown frame
    /// blends with its last one. Both engines must be in the same mode.
    fn copy_from(&mut self, src: &Engine) {
        let native = self.mode == GeoMode::Native && (src.native.is_some() || src.native_tgp.is_some());
        if !native {
            self.tgp.copy_from(&src.tgp);
            if let (Some(to), Some(from)) = (&mut self.sharc, &src.sharc) {
                to.copy_from(from, &mut self.copy_buf);
                self.sharc_native = src.sharc_native;
            }
        }
        self.native.clone_from(&src.native);
        self.native_tgp.clone_from(&src.native_tgp);
        self.poly_latch = src.poly_latch;
        self.poly_ram.clone_from(&src.poly_ram);
        self.buffer.clone_from(&src.buffer);
        self.jumps.clone_from(&src.jumps);
        self.stream.clone_from(&src.stream);
        self.list_words = src.list_words;
        self.fault = src.fault;
        self.frames = src.frames;
        self.texture_point_ram.clone_from(&src.texture_point_ram);
        self.list_consumed = src.list_consumed;
        if self.mode == GeoMode::Check {
            self.check_poly.clone_from(&src.check_poly);
            self.check_latch = src.check_latch;
        }
        if let Some(s) = &mut self.smooth {
            s.pending = None;
            s.stale = true;
        }
    }

    /// Smooth motion (docs/hw/geometrizer.md, "Smooth motion"): from the
    /// next frame, each frame the native geometrizer runs also notes its
    /// transforms and keeps what an in-between of it needs
    /// ([`Engine::smooth_frame`]). Render-only: the frames' output and the
    /// state are the same either way. Only frames the native geometrizer
    /// runs (`GeoMode::Native`) get in-betweens.
    pub fn set_smooth(&mut self, on: bool) {
        match (on, self.smooth.is_some()) {
            (true, false) => self.smooth = Some(Box::new(Smooth { stale: true, ..Default::default() })),
            (false, true) => self.smooth = None,
            _ => {}
        }
    }

    /// Smooth shading's side channel (`crate::light::Frame`): from the next
    /// frame, each frame also records what its geometrizer knew about its
    /// lighting ([`Engine::light_frame`]): a native geometrizer every
    /// block's facts and every polygon's normal and lighting (its light
    /// hooks), the chip a scan of the list for the block facts. Render-only:
    /// the frames' output and the state are the same either way; off, the
    /// hooks do nothing.
    pub fn set_light(&mut self, on: bool) {
        match (on, self.light.is_some()) {
            (true, false) => self.light = Some(Box::default()),
            (false, true) => self.light = None,
            _ => {}
        }
    }

    /// Whether the side channel is on ([`Engine::set_light`]).
    pub fn light_on(&self) -> bool {
        self.light.is_some()
    }

    /// The last frame's lighting side channel, if it is on and the frame
    /// had one (None: none, or the list scan gave up).
    pub fn light_frame(&self) -> Option<Arc<crate::light::Frame>> {
        self.light.as_ref()?.frame.clone()
    }

    /// A tap for a frame a native geometrizer runs, if the side channel is
    /// on.
    fn light_tap(&self) -> Option<crate::light::Tap> {
        self.light.is_some().then(crate::light::Tap::new)
    }

    /// The program the chip path's list scan reads lists for.
    fn scan_program(&mut self) -> Option<crate::light::ScanProgram> {
        use crate::light::{ScanBoard, ScanProgram, ScanVariant};
        if let Some(p) = self.light.as_ref()?.program {
            return Some(p);
        }
        let p = if self.fixed {
            ScanProgram { board: ScanBoard::A, variant: ScanVariant::Shared }
        } else if let Some(s) = self.sharc.as_deref() {
            let lastbrnx = crate::geo_native::recognise(&adsp2106x::cpu::program_image(s))
                == Some(crate::geo_native::Program::Lastbrnx);
            ScanProgram {
                board: ScanBoard::B,
                variant: if lastbrnx { ScanVariant::Lastbrnx } else { ScanVariant::Shared },
            }
        } else {
            use crate::geo_native_2c::Program;
            let variant = match crate::geo_native_2c::recognise(self.tgp.program_hash) {
                Some(Program::Overrev) => ScanVariant::Overrev,
                Some(Program::TopSkater) => ScanVariant::TopSkater,
                _ => ScanVariant::Shared,
            };
            ScanProgram { board: ScanBoard::C, variant }
        };
        self.light.as_deref_mut()?.program = Some(p);
        Some(p)
    }

    /// The list scan of the frame's list (the words fed), if it knows
    /// every command.
    fn scan_fed(&mut self) -> Option<crate::light::Frame> {
        let program = self.scan_program()?;
        let l = self.light.as_deref_mut()?;
        let blocks = crate::light::scan_list(&self.fed, program, &mut l.scan)?;
        Some(crate::light::Frame { frame: self.frames, blocks, ..Default::default() })
    }

    /// After a frame the chip ran, with the side channel on: the block
    /// facts from a scan of the list it was fed.
    fn scan_light(&mut self) {
        let scan = self.scan_fed().map(Arc::new);
        if let Some(l) = self.light.as_deref_mut() {
            l.frame = scan.clone();
            l.scanned = scan;
        }
    }

    /// After a frame a native geometrizer ran, with the side channel on:
    /// what its tap recorded is the frame's; the list is scanned too, and
    /// the scan's block facts compared with the hooks' ([`LightCheck`]).
    fn finish_light(&mut self, tap: Option<crate::light::Tap>) {
        let Some(tap) = tap else { return };
        let native = tap.finish(self.frames);
        let scan = self.scan_fed();
        self.compare_light(&native, scan.as_ref());
        if let Some(l) = self.light.as_deref_mut() {
            l.frame = Some(Arc::new(native));
            l.scanned = scan.map(Arc::new);
        }
    }

    /// `GeoMode::Check`, side channel on: the scan's block facts (the
    /// chip's frame, already scanned) against those the native
    /// geometrizer recorded beside it.
    fn check_light(&mut self, tap: Option<crate::light::Tap>) {
        let Some(tap) = tap else { return };
        let native = tap.finish(self.frames);
        let scan = self.light.as_ref().and_then(|l| l.frame.clone());
        self.compare_light(&native, scan.as_deref());
    }

    /// Count how the list scan's block facts agree with a native
    /// geometrizer's for one frame ([`CheckReport::light`]).
    fn compare_light(&mut self, native: &crate::light::Frame, scan: Option<&crate::light::Frame>) {
        use crate::light::Block;
        let c = &mut self.check.light;
        c.frames += 1;
        let Some(scan) = scan else {
            c.scan_failed += 1;
            return;
        };
        if scan.blocks.len() != native.blocks.len() {
            c.count_differing += 1;
        }
        for (k, (n, s)) in native.blocks.iter().zip(&scan.blocks).enumerate() {
            let (nf, sf) = (n.facts(), s.facts());
            if s.flags & Block::UNKNOWN != 0 {
                c.unknown += 1;
            } else if nf == sf {
                c.equal += 1;
            } else if s.flags & Block::PARTIAL != 0
                && (nf.0, nf.2) == (sf.0, sf.2)
                && n.flags & Block::DIRECT == s.flags & Block::DIRECT
            {
                c.partial += 1;
            } else {
                c.differing += 1;
                c.first.get_or_insert((self.frames, k, *n, *s));
            }
        }
    }

    /// The last frame's list scan (block facts only), if the side channel
    /// is on and the scan knew the list: the chip path's side channel, or
    /// beside a native geometrizer's for comparison.
    pub fn light_scan_frame(&self) -> Option<Arc<crate::light::Frame>> {
        self.light.as_ref()?.scanned.clone()
    }

    /// The last frame's in-betweens ([`crate::smooth::Frame::render`]),
    /// if smooth motion is on and the native geometrizer ran it.
    pub fn smooth_frame(&self) -> Option<Arc<crate::smooth::Frame>> {
        self.smooth.as_ref()?.frame.clone()
    }

    /// Host time smooth motion has added to the frames so far.
    pub fn smooth_busy(&self) -> std::time::Duration {
        self.smooth.as_ref().map_or(std::time::Duration::ZERO, |s| s.busy)
    }

    /// Before a frame the native geometrizer runs, with smooth motion on:
    /// polygon RAM's copy brought up to date. Returns a tap to record with.
    fn begin_smooth(&mut self) -> Option<crate::smooth::Tap> {
        let s = self.smooth.as_deref_mut().filter(|s| !s.skip)?;
        let t = Stopwatch::start();
        s.begin(&self.poly_ram);
        s.busy += t.elapsed();
        Some(crate::smooth::Tap::record())
    }

    /// After a frame: its in-betweens' plan, from what the frame recorded
    /// and the frame before's.
    fn finish_smooth(&mut self, read_ptr: u32, window: Option<[u32; 7]>) {
        let Some(s) = self.smooth.as_deref_mut() else { return };
        if s.skip {
            // Not shown: its polygon RAM writes go unrecorded.
            s.stale = true;
            return;
        }
        let t = Stopwatch::start();
        s.frame = None;
        match s.pending.take() {
            Some((native, latch, list, wrote)) => {
                let plan = crate::smooth::Plan::new(&s.prev, list.clone());
                s.prev = list;
                s.frame = Some(Arc::new(crate::smooth::Frame::new(
                    native,
                    self.buffer.clone(),
                    self.jumps.clone(),
                    read_ptr,
                    latch,
                    s.poly.clone(),
                    self.poly_rom.clone(),
                    self.texture_point_ram.clone(),
                    plan,
                    self.frames,
                    window,
                )));
                s.stale |= wrote;
            }
            // Not a native frame: nothing to blend the next with, and
            // polygon RAM may have changed.
            None => {
                s.prev.clear();
                s.stale = true;
            }
        }
        s.busy += t.elapsed();
    }

    /// [`Engine::process_sharc`] by the native geometrizer: the same list,
    /// polygon memory and output.
    fn process_native(&mut self, buffer: &[u8], read_ptr: u32) {
        self.frames += 1;
        if self.native.is_none() {
            return;
        }
        let mut tap = self.begin_smooth();
        let mut light = self.light_tap();
        let Some(n) = self.native.as_deref_mut() else { return };
        let before = tap.is_some().then(|| (crate::smooth::Native::B(Box::new(n.clone())), self.poly_latch));
        let mut out = Vec::new();
        let mut list = ListDma::new(buffer, &self.jumps, read_ptr, &mut self.list_words, &mut self.fed);
        let mut io = crate::geo_native::Io {
            list: &mut list,
            poly: GeoExt::new(&mut self.poly_ram, &self.poly_rom),
            latch: &mut self.poly_latch,
            out: &mut out,
            tap: tap.as_mut(),
            light: light.as_mut(),
        };
        n.run_frame(&mut io);
        let wrote = io.poly.wrote;
        self.stream.extend(out.into_iter().filter(|&w| w != RASTER_NOP));
        self.finish_light(light);
        if let (Some(s), Some((native, latch)), Some(tap)) = (self.smooth.as_deref_mut(), before, tap) {
            s.pending = Some((native, latch, tap.into_list(), wrote));
        }
    }

    /// `GeoMode::Check`: the SHARC's frame (the result), then the native
    /// one from the same input on its own state and polygon RAM, compared.
    /// After a difference the native side takes the SHARC's state again,
    /// so each frame is compared from the same start.
    fn process_check(&mut self, buffer: &[u8], read_ptr: u32) {
        let t = Stopwatch::start();
        self.process_sharc(buffer, read_ptr);
        let sharc_ms = t.elapsed().as_secs_f32() * 1e3;
        let mut light = self.light_tap();
        let Some(n) = self.native.as_deref_mut() else { return };
        let t = Stopwatch::start();
        let (mut out, mut words, mut fed) = (Vec::new(), 0, Vec::new());
        let mut list = ListDma::new(buffer, &self.jumps, read_ptr, &mut words, &mut fed);
        let mut io = crate::geo_native::Io {
            list: &mut list,
            poly: GeoExt::new(&mut self.check_poly, &self.poly_rom),
            latch: &mut self.check_latch,
            out: &mut out,
            tap: None,
            light: light.as_mut(),
        };
        n.run_frame(&mut io);
        let native: Vec<u32> = out.into_iter().filter(|&w| w != RASTER_NOP).collect();
        let native_ms = t.elapsed().as_secs_f32() * 1e3;
        let c = &mut self.check;
        c.frames += 1;
        c.words += self.stream.len() as u64;
        c.times.push((sharc_ms, native_ms));
        let mut resync = false;
        if words != self.list_words {
            c.list_differing += 1;
            resync = true;
        }
        if self.check_latch != self.poly_latch {
            c.state_differing += 1;
            resync = true;
            c.first_state.get_or_insert((self.frames, POLY_ADDRESS_LATCH, self.poly_latch, self.check_latch));
        }
        if let Some(i) = first_difference(&self.stream, &native) {
            c.differing += 1;
            resync = true;
            if c.first.is_none() {
                c.first = Some(Mismatch::at(self.frames, i, &self.stream, &native));
            }
        }
        // The program's state words: the transform, lights and parameters,
        // the vertex ring (0x30020-0x300FF) and the edge vertex.
        if let Some(s) = self.sharc.as_deref() {
            let (dm, pm) = n.data();
            let pm_base = n.program.pm_base();
            let state = (0x20..0x100u32)
                .map(|i| (0x3_0000 + i, s.mem.read32(0x3_0000 + i), dm[i as usize]))
                .chain((0x10..0x13u32).map(|i| (pm_base + i, (s.mem.read48(pm_base + i) >> 16) as u32, pm[i as usize])))
                .find(|(_, a, b)| a != b);
            if let Some((addr, a, b)) = state {
                c.state_differing += 1;
                resync = true;
                c.first_state.get_or_insert((self.frames, addr, a, b));
            }
        }
        if resync {
            if let Some(s) = self.sharc.as_deref() {
                n.import(s, self.frames);
            }
            self.check_poly.clone_from(&self.poly_ram);
            self.check_latch = self.poly_latch;
        }
        self.check_light(light);
    }

    /// [`Engine::process`] by the 2C native geometrizer: the same list,
    /// polygon memory and output.
    fn process_native_tgp(&mut self, buffer: &[u8], read_ptr: u32) {
        self.frames += 1;
        if self.native_tgp.is_none() {
            return;
        }
        let mut tap = self.begin_smooth();
        let mut light = self.light_tap();
        let Some(n) = self.native_tgp.as_deref_mut() else { return };
        let before = tap.is_some().then(|| crate::smooth::Native::C(Box::new(n.clone())));
        let (mut out, mut latch) = (Vec::new(), 0);
        let mut list = ListDma::new(buffer, &self.jumps, read_ptr, &mut self.list_words, &mut self.fed);
        let mut io = crate::geo_native::Io {
            list: &mut list,
            poly: GeoExt::new(&mut self.poly_ram, &self.poly_rom),
            latch: &mut latch,
            out: &mut out,
            tap: tap.as_mut(),
            light: light.as_mut(),
        };
        n.run_frame(&mut io);
        let wrote = io.poly.wrote;
        self.stream.extend(out.into_iter().filter(|&w| w != RASTER_NOP));
        self.finish_light(light);
        if let (Some(s), Some(native), Some(tap)) = (self.smooth.as_deref_mut(), before, tap) {
            s.pending = Some((native, 0, tap.into_list(), wrote));
        }
    }

    /// [`Engine::process_check`] for a 2C TGP: its frame (the result), then
    /// the native one from the same input on its own state and polygon RAM,
    /// compared: the stream, the list words read, and the program's state
    /// (A and B RAM, the matrix queue; [`crate::geo_native_2c::Native::compare`]).
    fn process_check_tgp(&mut self, buffer: &[u8], read_ptr: u32) {
        let t = Stopwatch::start();
        self.process(buffer, read_ptr);
        let chip_ms = t.elapsed().as_secs_f32() * 1e3;
        let mut light = self.light_tap();
        let Some(n) = self.native_tgp.as_deref_mut() else { return };
        let t = Stopwatch::start();
        let (mut out, mut words, mut fed, mut latch) = (Vec::new(), 0, Vec::new(), 0);
        let mut list = ListDma::new(buffer, &self.jumps, read_ptr, &mut words, &mut fed);
        let mut io = crate::geo_native::Io {
            list: &mut list,
            poly: GeoExt::new(&mut self.check_poly, &self.poly_rom),
            latch: &mut latch,
            out: &mut out,
            tap: None,
            light: light.as_mut(),
        };
        n.run_frame(&mut io);
        let native: Vec<u32> = out.into_iter().filter(|&w| w != RASTER_NOP).collect();
        let native_ms = t.elapsed().as_secs_f32() * 1e3;
        let mut resync = false;
        let c = &mut self.check;
        c.frames += 1;
        c.words += self.stream.len() as u64;
        c.times.push((chip_ms, native_ms));
        if fed.len() != self.list_consumed {
            c.list_differing += 1;
            resync = true;
        }
        if let Some(i) = first_difference(&self.stream, &native) {
            c.differing += 1;
            resync = true;
            if c.first.is_none() {
                c.first = Some(Mismatch::at(self.frames, i, &self.stream, &native));
            }
        }
        if let Some((addr, a, b)) = n.compare(&self.tgp) {
            c.state_differing += 1;
            resync = true;
            c.first_state.get_or_insert((self.frames, addr, a, b));
        }
        if self.check_poly != self.poly_ram {
            c.state_differing += 1;
            resync = true;
            let k = self.check_poly.iter().zip(&self.poly_ram).position(|(a, b)| a != b).unwrap_or(0);
            c.first_state.get_or_insert((self.frames, POLY_RAM_STATE + k as u32, self.poly_ram[k], self.check_poly[k]));
        }
        if resync {
            n.import(&self.tgp, false);
            self.check_poly.clone_from(&self.poly_ram);
        }
        self.check_light(light);
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
        self.list_consumed = self.fed.len() - self.tgp.fifo_in.len();
        self.tgp.fifo_in.clear();
        self.scan_light();
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
        let native = self.sharc_native;
        let Some(s) = self.sharc.as_deref_mut() else { return };
        // Recompiled code for this program if the build has it, else the
        // interpreter; they must behave identically.
        let run = |s: &mut Sharc, bus: &mut SharcGeoBus| match native {
            Some(f) => f(s, bus),
            None => s.run_bus(bus),
        };
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
            phase: Phase::Pulse { start: s.steps },
        };
        let start = s.steps;
        let mut fault = None;
        // FLAG0 high until the program waits for it to fall again.
        s.flag_in |= 1;
        if let SharcStep::Fault(f) = run(s, &mut bus) {
            fault = Some(f);
        }
        s.flag_in &= !1;
        bus.out.clear();
        *bus.active = false;
        if fault.is_none() {
            bus.phase = Phase::Frame { start, last_activity: s.steps };
            if let SharcStep::Fault(f) = run(s, &mut bus) {
                fault = Some(f);
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
        self.scan_light();
    }
}

/// A copy of a freshly uploaded SHARC run through its start-up code, to
/// where it first waits for a frame (FLAG0).
fn started(s: &Sharc) -> Sharc {
    struct Quiet;
    impl SharcBus for Quiet {
        fn read(&mut self, _: u32, _: Width) -> u64 {
            0
        }
        fn write(&mut self, _: u32, _: Width, _: u64) {}
        fn after_step(&mut self, cpu: &mut Sharc) -> bool {
            !cpu.spinning() && cpu.steps < SHARC_PULSE_STEPS
        }
    }
    let mut copy = Sharc::new();
    let bytes = state::to_bytes(s);
    if state::State::load(&mut copy, &mut state::Reader::new(&bytes)).is_ok() {
        copy.watch_code(&[], &[]);
        copy.steps = 0;
        let _ = copy.run_bus(&mut Quiet);
    }
    copy
}

// Save states. The front end and, after any running frame finishes, the
// engine (its TGP, polygon RAM, texture point RAM and last frame's
// output); the polygon ROM and diagnostics stay. In remote mode the engine
// lives elsewhere and isn't saved.
impl state::State for Geometrizer {
    fn save(&self, w: &mut state::Writer) {
        self.save_front(w);
        match &self.engine {
            Slot::Idle(e) => e.save(w),
            _ => panic!("geometrizer frame still running: call sync() before saving"),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.load_front(r)?;
        self.engine_mut().load(r)
    }
}

impl Geometrizer {
    /// The front end's part of the state (without the engine's).
    pub(crate) fn save_front(&self, w: &mut state::Writer) {
        use state::State as _;
        self.program.save(w);
        self.uploading.save(w);
        self.upload_half.save(w);
        self.booted.save(w);
        self.write_ptr.save(w);
        self.read_ptr.save(w);
        self.bad_commands.save(w);
    }

    pub(crate) fn load_front(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        use state::State as _;
        self.program.load(r)?;
        self.uploading.load(r)?;
        self.upload_half.load(r)?;
        self.booted.load(r)?;
        self.write_ptr.load(r)?;
        self.read_ptr.load(r)?;
        self.bad_commands.load(r)
    }

    /// Make the engine's state `src`'s, as a save and load would, by
    /// copying it ([`Engine::copy_from`]); after any frames running on
    /// either finish. The front end's part goes with the bus's state
    /// (`Model2Bus::copy_from`).
    pub(crate) fn copy_from(&mut self, src: &mut Geometrizer) {
        // A remote engine's state is elsewhere: the local one is idle.
        assert!(
            self.remote.is_none() && src.remote.is_none(),
            "a geometrizer whose engine runs elsewhere (set_remote) can't be copied"
        );
        self.sync();
        let Slot::Idle(to) = &mut self.engine else { unreachable!() };
        to.copy_from(src.engine_mut());
    }

    /// Run frames the way `src` does: inline or on a thread, keeping the
    /// last frame or not, the same mode, SHARC interpreter choice, smooth
    /// motion and lighting side channel (`crate::Machine::configure_like`).
    pub fn configure_like(&mut self, src: &mut Geometrizer) {
        self.set_threaded(src.threaded);
        self.keep_last = src.keep_last;
        let (mode, interpret, smooth, light) = {
            let e = src.engine_mut();
            (e.mode, e.sharc_interpret, e.smooth.is_some(), e.light.is_some())
        };
        let e = self.engine_mut();
        if e.sharc_interpret != interpret {
            e.sharc_interpret = interpret;
            e.attach_sharc();
        }
        if e.mode != mode {
            e.set_mode(mode);
        }
        e.set_smooth(smooth);
        e.set_light(light);
    }

    /// Smooth motion: whether the frames started from now on are shown
    /// (the default), so their in-betweens are wanted. A frame that isn't
    /// (run-ahead's frames before the one it shows) records nothing, and
    /// the next shown frame blends with the last shown one; see
    /// `crate::runahead`.
    pub fn set_smooth_shown(&mut self, shown: bool) {
        if let Some(s) = self.engine_mut().smooth.as_deref_mut() {
            s.skip = !shown;
        }
    }
}

// The 2C fields, then a 2B engine's SHARC (2C states are unchanged).
impl state::State for Engine {
    fn save(&self, w: &mut state::Writer) {
        match (self.mode, &self.native_tgp) {
            // The native state, where the program keeps it, so the state
            // loads the same way whichever runs it.
            (GeoMode::Native, Some(n)) => {
                let mut copy = if crate::geo_native_2c::fresh(&self.tgp) && !self.fixed {
                    crate::geo_native_2c::started(&self.tgp)
                } else {
                    let mut copy = Tgp::new();
                    let bytes = state::to_bytes(&self.tgp);
                    let _ = state::State::load(&mut copy, &mut state::Reader::new(&bytes));
                    copy
                };
                n.export(&mut copy);
                copy.save(w);
            }
            _ => self.tgp.save(w),
        }
        self.poly_ram.save(w);
        self.buffer.save(w);
        self.stream.save(w);
        self.list_words.save(w);
        self.fault.save(w);
        self.frames.save(w);
        self.texture_point_ram.save(w);
        if let Some(s) = &self.sharc {
            match (self.mode, &self.native) {
                // The native state, where the program keeps it, so the
                // state loads the same way whichever runs it.
                (GeoMode::Native, Some(n)) => {
                    let mut copy = Sharc::new();
                    let bytes = state::to_bytes(&**s);
                    if state::State::load(&mut copy, &mut state::Reader::new(&bytes)).is_ok() {
                        n.export(&mut copy);
                        copy.save(w);
                    } else {
                        s.save(w);
                    }
                }
                _ => s.save(w),
            }
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
            self.attach_sharc();
        }
        self.attach_native();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame that draws before it sets a window gets the last window of
    /// the frame before, and so does one that sets none; one that sets
    /// its own first is left alone.
    #[test]
    fn the_window_carries_over() {
        let win = |x: u32| [0xff00_0003, x, 1, 2, 3, 4, 5];
        let object = [0xff00_0001, 0x10, 0x20];
        // Operands that look like commands are skipped: a memory write's
        // data, and the window's own.
        let mut first = vec![0xff00_0004, 0x80_0000, 1, 0xff00_0003];
        first.extend(win(7));
        first.extend(object);
        first.extend(win(8));
        assert_eq!(last_window(&first), Some(win(8)));
        let mut none: Vec<u32> = object.to_vec();
        carry_window(&mut none, last_window(&first));
        assert_eq!(none[..7], win(8));
        assert_eq!(none[7..], object);
        let mut empty = vec![0xff00_0008, 0];
        carry_window(&mut empty, Some(win(9)));
        assert_eq!(last_window(&empty), Some(win(9)));
        let mut own = first.clone();
        carry_window(&mut own, Some(win(9)));
        assert_eq!(own, first);
        let mut nothing = object.to_vec();
        carry_window(&mut nothing, None);
        assert_eq!(nothing, object);
    }

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
        // A fn 0 placeholder, patched into a jump by the i960 writing buffer
        // RAM (fvipers), and the other jump retargeted the same way: both
        // command words, with the values the frame started with.
        g.port_write(0x1008, 0x300, &mut buffer);
        g.port_write(0x0000, 0, &mut buffer);
        buffer[0x300..0x304].copy_from_slice(&0x8000_0400u32.to_le_bytes());
        buffer[0x200..0x204].copy_from_slice(&0x8000_0500u32.to_le_bytes());
        g.start_frame(&buffer);
        assert_eq!(g.jumps(), vec![(0x200, 0x8000_0500), (0x300, 0x8000_0400)]);
        // A store from the copro ends one (stcc's matrices over old jumps).
        g.commands().remove(&0x200);
        assert_eq!(g.jumps(), vec![(0x300, 0x8000_0400)]);
    }
}
