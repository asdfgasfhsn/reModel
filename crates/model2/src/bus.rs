//! Model 2B-CRX and 2C-CRX i960 address space.
//!
//! Provenance: the layout below is a working hypothesis taken from MAME's
//! public driver notes, which do not run this game correctly. Treat every
//! register meaning as unverified until the game's own code confirms it.
//!
//! Memory regions are plain byte arrays. Hardware registers are modelled
//! only as far as needed for the game to make progress; every access to
//! anything that isn't plain memory is counted so the runner can report
//! what the game is waiting on.

use crate::drive::Drive;
use crate::eeprom::Eeprom;
use crate::geo::{self, Geometrizer};
use crate::gun::GunBoard;
use crate::protection;
use crate::sharc;
use crate::tgp2a::{Shared, Tgp2a};
use i960::cpu::Bus;
use mb86235::cpu::{External, Step, Tgp};
use romset::{Board, Role, RomSet};
use std::collections::{BTreeMap, VecDeque};

/// Main CPU clock: 25 MHz.
pub const CPU_HZ: u64 = 25_000_000;
/// One video frame in CPU cycles: 656x424 total pixels at a 16 MHz pixel
/// clock gives 57.524 Hz, which matches measured captures of Model 2 boards
/// (see docs/hw/peripherals.md).
pub const FRAME_CYCLES: u64 = 434_600;

// IRQ controller request/enable bits.
pub const IRQ_VBLANK: u32 = 1 << 0;
pub const IRQ_TIMER0: u32 = 1 << 2;
pub const IRQ_UART: u32 = 1 << 10;

/// i960 external line driven by each request bit group.
fn irq_line(bits: u32) -> [bool; 4] {
    [bits & 0x1 != 0, bits & 0x2 != 0, bits & 0x3fc != 0, bits & 0xc00 != 0]
}

#[derive(Default, Clone, Copy)]
pub struct Access {
    pub reads: u64,
    pub writes: u64,
    pub last_write: u32,
}

struct Mem {
    data: Vec<u8>,
    writable: bool,
    /// Writes so far: a version number for caching what's derived from it.
    writes: u64,
}

impl Mem {
    /// Load contents and write count saved by [`Model2Bus`]'s state.
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        let len = self.data.len();
        state::State::load(&mut self.data, r)?;
        if self.data.len() != len {
            return Err(state::Error("memory size differs".into()));
        }
        state::State::load(&mut self.writes, r)
    }
    fn zeroed(size: usize) -> Self {
        Mem { data: vec![0; size], writable: true, writes: 0 }
    }
    fn from(data: Vec<u8>, writable: bool) -> Self {
        Mem { data, writable, writes: 0 }
    }
}

/// Index of buffer RAM in `mems`; shared with the TGP's external bus.
const BUFFER_RAM: usize = 3;
/// The RAMs code can be copied to and run from (0x200000 and work RAM at
/// 0x500000), whose writes the JIT counts.
const CODE_RAMS: [usize; 2] = [1, 2];
/// Bytes per write counter (`jit::GEN_PAGE`).
const CODE_GEN_SHIFT: u32 = 10;
/// The original Model 2's I/O registers (`Model2Bus::m2_io`).
const M2_IO_REGS: usize = 64;
/// Indexes of palette RAM and render RAM in `mems`.
const PALETTE_RAM: usize = 6;
const RENDER_RAM: usize = 10;
/// How far the TGP may run while the i960 waits on an empty output FIFO.
const TGP_STALL_LIMIT: u64 = 2_000_000;

/// The TGP's view of its external bus.
pub struct TgpExt<'a> {
    buffer: &'a mut [u8],
    data: &'a [u32],
    /// The geometrizer's command words in buffer RAM, which a store ends
    /// (`Geometrizer::commands`).
    commands: &'a mut BTreeMap<u32, u32>,
}

impl External for TgpExt<'_> {
    #[inline]
    fn read(&mut self, addr: u32) -> u32 {
        match addr {
            0x40_0000..=0x7f_ffff => {
                let o = (addr as usize & 0x7fff) * 4;
                u32::from_le_bytes(self.buffer[o..o + 4].try_into().unwrap())
            }
            0x80_0000..=0x9f_ffff => self.data.get((addr - 0x80_0000) as usize).copied().unwrap_or(0),
            _ => 0,
        }
    }
    #[inline]
    fn write(&mut self, addr: u32, v: u32) {
        if let 0x40_0000..=0x7f_ffff = addr {
            let o = (addr as usize & 0x7fff) * 4;
            self.buffer[o..o + 4].copy_from_slice(&v.to_le_bytes());
            if !self.commands.is_empty() {
                self.commands.remove(&(o as u32));
            }
        }
    }
}

/// Region table entry: [start, end] inclusive, index into `mems`. A window
/// larger than its backing store mirrors it.
struct Map {
    start: u32,
    end: u32,
    mem: usize,
    /// Where in the memory `start` is.
    base: usize,
}

pub struct Timer {
    reload: u32,
    started_at: u64,
    running: bool,
}

pub struct Model2Bus {
    /// The board revision (from the game's manifest): 2B has SHARCs where
    /// 2C has MB86235s, and moves the sound UART.
    pub board: Board,
    mems: Vec<Mem>,
    /// Index in `mems` of the first memory-test-only RAM (the rest follow).
    test_rams: usize,
    /// Model 2A: index in `mems` of its texture RAM (the luma tables follow).
    video_2a: Option<usize>,
    /// Model 2A: the TGP's data ROM on its external bus.
    tgp_rom: Vec<u32>,
    map: Vec<Map>,
    /// For each 64 KB page, 1 + the index of the map entry in it (0: none).
    pages: Vec<u8>,
    /// Current time in CPU cycles; advanced by the machine.
    pub now: u64,
    /// Video frames completed (vblanks).
    pub frames: u64,
    next_vblank: u64,
    /// Instructions recompiled code may still run before returning.
    pub budget: u64,

    pub irq_request: u32,
    pub irq_enable: u32,
    /// Lines newly asserted since the machine last looked.
    pub raised_lines: [bool; 4],
    pub timers: [Timer; 4],

    // Geometry coprocessor (MB86235) interface.
    copro_ctrl: u32,
    copro_uploading: bool,
    pub copro_program: Vec<u64>,
    copro_upload_half: Option<u32>,
    pub copro_booted: bool,
    /// The second MB86235: transforms display lists for the rasteriser.
    pub geo: Geometrizer,
    /// "textures" ROM (texture headers and point lists) as u16 words;
    /// shared, so a renderer can hold it while the machine runs on.
    pub texture_rom: std::sync::Arc<[u16]>,
    /// The geometry processor, running the uploaded program.
    pub tgp: Tgp,
    /// Model 2B's copro, a SHARC, in place of `tgp`.
    pub sharc: Option<Box<sharc::Copro>>,
    /// Model 2A's TGP interface and stand-in (`tgp2a`), in place of `tgp`.
    pub tgp2a: Option<Box<Tgp2a>>,
    /// "copro_data" ROM as 32-bit words, at TGP external address 0x800000.
    copro_data: Vec<u32>,
    /// First instruction the TGP model couldn't execute, if any.
    pub tgp_fault: Option<u32>,
    /// The last run stopped waiting for input.
    tgp_stalled: bool,
    /// `tgp` on the JIT (docs/jit.md, "Copro JITs"), when it is on.
    #[cfg(not(target_arch = "wasm32"))]
    tgp_jit: crate::copro_jit::TgpJit,
    pub copro_empty_reads: u64,
    /// Running FNV-1a hash of every value the i960 read from the copro, to
    /// compare runs.
    pub copro_result_hash: u64,
    /// Where the CPU read the copro output FIFO while it was empty (real
    /// hardware stalls there until a result arrives), by IP.
    pub copro_empty_by_ip: BTreeMap<u32, u64>,
    /// IP of the instruction being executed, set by the machine.
    pub cur_ip: u32,
    /// For the JIT (docs/jit.md): a write counter per KB of the RAMs code
    /// can run from ([`CODE_RAMS`]), bumped by every write there; empty
    /// (and not counted) unless [`Model2Bus::count_code_writes`] was called.
    code_gens: Vec<u32>,
    /// For the JIT: the plain memory its blocks reach without a call
    /// (`jit::FastPage`, docs/jit.md "Plain memory inline"), made with
    /// `code_gens` and again after a state load; empty until then.
    #[cfg(not(target_arch = "wasm32"))]
    fast: Vec<jit::FastPage>,
    pub geo_words: u64,
    /// When set, geo port writes (address, value) are recorded here.
    pub geo_trace: Option<Vec<(u32, u32)>>,
    /// Write watch (debugging): an address, and the (cycle, ip, address,
    /// value, size) of every memory write touching its 32-bit word.
    pub watch: Option<(u32, Vec<WatchHit>)>,
    /// The catalogue's texture upload watch (`crate::texwatch`); render
    /// side only, never part of the state.
    pub tex_watch: Option<Box<crate::texwatch::TexWatch>>,
    /// Everything written to the sound link UART: (cycle, control register,
    /// byte). Data bytes are the commands for the sound boards.
    pub uart_log: Vec<(u64, bool, u8)>,
    /// The sound link UART's transmitter, for its TxRDY interrupt.
    uart: Uart,
    /// Video status bit 2 at 0x98000C. Hypothesis (from the boot code, which
    /// times how long it takes to change): toggles once per frame, or once
    /// per two frames in 30 Hz mode (`video_30hz`).
    pub frame_toggle: bool,
    /// 30 Hz mode: bit 0 of what the game wrote to 0x98000C (`vstriker`
    /// alone sets it, docs/games/vstriker.md, "Frame rate"). The frame bit
    /// then toggles, and the geometrizer starts a frame, on every second
    /// vblank only. Saved with `frame_toggle` in one byte.
    pub video_30hz: bool,
    board_ctrl: u8,

    // 315-5649 I/O: inputs are active low.
    pub inputs: [u8; 4],
    /// The latched gear lever (`input::apply_player`, for games with a
    /// `Controls::shifter`). Players' state, not the machine's.
    pub gear: crate::input::GearLatch,
    /// Where the keys have turned the analog inputs that ramp
    /// (`input::apply_player`, `romset::KeyRamp`). Players' state too.
    pub key_ramps: crate::input::KeyRamps,
    io_analog_channel: usize,
    pub analog: [u8; 8],
    /// Diagnostics, not machine state: when each analog channel was last
    /// read (CPU cycle) and the value it gave (`remodel latency`).
    pub analog_reads: [(u64, u8); 8],
    /// 315-5649 output latches, by register.
    io_latch: [u8; 16],
    pub eeprom: Eeprom,
    /// The game's controls (from its manifest): what `input` sets.
    pub controls: &'static romset::Controls,
    /// The cabinet's drive board, if the game has one: commands on I/O
    /// register 4, replies on register 3 (in place of port `inputs[2]`).
    pub drive: Option<Drive>,
    /// The communication board in the expansion slot at `0x1A10000`, if
    /// the game has one fitted (`model2::link`); empty slots read as all
    /// ones.
    pub link: Option<crate::link::Link>,
    /// Debugging (`REMODEL_LINK`, `docs/hw/link-board.md`): every CPU
    /// access to `0x1A00000`-`0x1A1FFFF` (shared RAM and the board's
    /// registers). Never part of the state; `None` costs a test per access.
    pub link_trace: Option<Vec<LinkHit>>,
    /// The light gun interface, for games with guns: commands on I/O
    /// registers 9 and 10, replies on 12, status on 13.
    pub gun: Option<GunBoard>,
    /// The protection device at 0x1D80000, for games whose manifest has one.
    protection: Option<Box<protection::Device>>,
    /// The original Model 2's I/O registers at 0x1C00000 (a byte each,
    /// every other byte address; docs/hw/model2.md), in place of the
    /// 315-5649's: what the game last wrote to each.
    m2_io: Option<Box<[u8; M2_IO_REGS]>>,

    /// Non-memory accesses, by word address.
    pub io_log: AccessLog,
    /// Accesses that hit nothing, by word address.
    pub unmapped: BTreeMap<u32, Access>,

    /// An idle-loop probe is watching the i960's accesses (`crate::idle`).
    probing: bool,
    probe: Probe,
}

/// What the i960 touched during an idle-loop probe ([`Model2Bus::probe_begin`]).
#[derive(Default)]
pub(crate) struct Probe {
    /// It wrote, or read something that may change before the next event.
    pub impure: bool,
    /// It read something the copro changes (buffer RAM, its output
    /// status): stable only while the copro is idle.
    pub needs_copro_idle: bool,
    /// Registers it read, for their access counts.
    pub io_reads: Vec<u32>,
}

/// Backup SRAM's contents at power-on: all ones, with the operator
/// settings the game's manifest gives (`Game::backup_ram`).
/// (On the original Model 2 they go to its I/O board's store instead.)
fn backup_ram(game: &romset::Game) -> Vec<u8> {
    let mut ram = vec![0xff; 0x4000];
    for &(at, bytes) in game.backup_ram.iter().filter(|_| game.board != Board::Model2) {
        ram[at..at + bytes.len()].copy_from_slice(bytes);
    }
    ram
}

impl Model2Bus {
    pub fn new(roms: &RomSet) -> Self {
        let mut program = roms.program.clone();
        program.resize(0x20_0000, 0);
        let mut main_data = roms.bank(Role::MainData);
        main_data.resize(0x200_0000, 0);
        let main_data_high = roms.bank(Role::MainDataHigh);

        let mut mems = vec![
            Mem::from(program, false),
            Mem::zeroed(0x4_0000),
            Mem::zeroed(0x10_0000),
            Mem::zeroed(0x2_0000),
            Mem::zeroed(0x40),
            Mem::zeroed(0x20_0000),
            Mem::zeroed(0x2_0000),
            Mem::zeroed(0x4000),
            Mem::from(backup_ram(roms.game), true),
            Mem::from(main_data, false),
            Mem::zeroed(0x70_0000),
        ];
        let mut map = vec![
            Map { start: 0x0000_0000, end: 0x001f_ffff, mem: 0, base: 0 },
            Map { start: 0x0020_0000, end: 0x0023_ffff, mem: 1, base: 0 },
            Map { start: 0x0050_0000, end: 0x005f_ffff, mem: 2, base: 0 },
            Map { start: 0x0090_0000, end: 0x0097_ffff, mem: 3, base: 0 },
            Map { start: 0x00e0_0000, end: 0x00e0_0037, mem: 4, base: 0 },
            // Tile RAM and character RAM, 1 MB, repeating once through the
            // 2 MB window: Last Bronx's gauge routine writes its tile
            // patterns at 0x1181000 and draws them from tile 0x80
            // (docs/hw/tilemap.md).
            Map { start: 0x0100_0000, end: 0x010f_ffff, mem: 5, base: 0 },
            Map { start: 0x0110_0000, end: 0x011f_ffff, mem: 5, base: 0 },
            Map { start: 0x0180_0000, end: 0x0181_ffff, mem: 6, base: 0 },
            Map { start: 0x01a0_0000, end: 0x01a0_3fff, mem: 7, base: 0 },
            Map { start: 0x01a1_0000, end: 0x01a1_3fff, mem: 7, base: 0 },
            Map { start: 0x01d0_0000, end: 0x01d0_3fff, mem: 8, base: 0 },
            Map { start: 0x0200_0000, end: 0x03ff_ffff, mem: 9, base: 0 },
        ];
        // The original Model 2 (docs/hw/model2.md): the program ROM's
        // second 128 KB is also at 0x220000, where the code is linked to
        // run (`daytona` calls it there); 0x200000-0x21FFFF stays RAM,
        // which the boot code fills with the first 128 KB.
        if roms.game.board == Board::Model2 {
            map[1].end = 0x0021_ffff;
            map.push(Map { start: 0x0022_0000, end: 0x0023_ffff, mem: 0, base: 0x2_0000 });
        }
        // Render RAM: the two texture banks, then the luma tables. Some
        // boards have 1 MB texture banks, each repeating through its 2 MB
        // window (`Game::texture_ram_mirrored`).
        if roms.game.texture_ram_mirrored {
            for (start, base) in
                [(0x1100_0000, 0), (0x1110_0000, 0), (0x1120_0000, 0x20_0000), (0x1130_0000, 0x20_0000)]
            {
                map.push(Map { start, end: start + 0xf_ffff, mem: 10, base });
            }
            map.push(Map { start: 0x1140_0000, end: 0x116f_ffff, mem: 10, base: 0x40_0000 });
        } else {
            map.push(Map { start: 0x1100_0000, end: 0x116f_ffff, mem: 10, base: 0 });
        }
        // The second main data bank, mapped only for games with chips in
        // it (segawski, waverunr), so the others keep their memory map unchanged.
        if !main_data_high.is_empty() {
            map.push(Map { start: 0x0600_0000, end: 0x07ff_ffff, mem: mems.len(), base: 0 });
            mems.push(Mem::from(main_data_high, false));
        }
        // Model 2A's video RAM at 0x12000000 (docs/hw/model2a.md): texture
        // RAM, 8 MB of which the game fills below 0x12180000 and from
        // 0x12580000, and a small block above it.
        let mut video_2a = None;
        if roms.game.board.has_tgp_2a() {
            video_2a = Some(mems.len());
            for (start, size) in [(0x1200_0000u32, 0x80_0000usize), (0x1280_0000, 0x1_0000)] {
                map.push(Map { start, end: start + size as u32 - 1, mem: mems.len(), base: 0 });
                mems.push(Mem::zeroed(size));
            }
        }
        // The original Model 2's I/O board buffer at 0x1C00200: 128 bytes
        // the game reads and writes a byte a 16-bit word, around the
        // commands of register 0x20 (docs/hw/model2.md). Plain RAM here.
        // The game keeps its operator settings there, not in the SRAM, so
        // `Game::backup_ram` fills this store (byte N at 0x1C00200 + 2N).
        if roms.game.board == Board::Model2 {
            map.push(Map { start: 0x01c0_0200, end: 0x01c0_03ff, mem: mems.len(), base: 0 });
            let mut store = vec![0; 0x200];
            for &(at, bytes) in roms.game.backup_ram {
                for (i, &b) in bytes.iter().enumerate() {
                    store[2 * (at + i)] = b;
                }
            }
            mems.push(Mem::from(store, true));
        }
        // Two 64 KB RAMs on the video side that only the memory test uses:
        // IC55 at 0x10A00000 and IC54 at 0x10C00000 (segawski, skisuprg and
        // waverunr test them byte-wide, comparing the low four bits, with
        // render mode bit 0 set). Kept last, apart from the rest of RAM
        // in save states and the state hash, so neither changes for runs
        // that never touch them (docs/hw/geometrizer.md 1.1).
        let test_rams = mems.len();
        for start in [0x10a0_0000, 0x10c0_0000] {
            map.push(Map { start, end: start + 0xffff, mem: mems.len(), base: 0 });
            mems.push(Mem::zeroed(0x1_0000));
        }
        let mut pages = vec![0u8; 0x1_0000];
        for (i, m) in map.iter().enumerate() {
            for p in m.start >> 16..=m.end >> 16 {
                assert_eq!(pages[p as usize], 0, "two map entries share page {p:#x}");
                pages[p as usize] = i as u8 + 1;
            }
        }
        let timer = || Timer { reload: 0xfffff, started_at: 0, running: false };
        let board = roms.game.board;
        let copro_data: Vec<u32> =
            roms.bank(Role::CoproData).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        Model2Bus {
            board,
            mems,
            test_rams,
            video_2a,
            tgp_rom: roms.bank(Role::TgpData).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect(),
            map,
            pages,
            now: 0,
            frames: 0,
            next_vblank: FRAME_CYCLES,
            budget: 0,
            irq_request: 0,
            irq_enable: 0,
            raised_lines: [false; 4],
            timers: [timer(), timer(), timer(), timer()],
            copro_ctrl: 0,
            copro_uploading: false,
            copro_program: Vec::new(),
            copro_upload_half: None,
            copro_booted: false,
            tgp: Tgp::new(),
            sharc: (board == Board::Model2B).then(|| Box::new(sharc::Copro::with_data(&copro_data))),
            tgp2a: board.has_tgp_2a().then(|| Box::new(Tgp2a::new(copro_data.clone()))),
            copro_data,
            tgp_fault: None,
            tgp_stalled: false,
            #[cfg(not(target_arch = "wasm32"))]
            // Run two words at a time: short blocks (docs/jit.md).
            tgp_jit: crate::copro_jit::TgpJit::new("copro", 16),
            texture_rom: roms
                .bank(Role::Textures)
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_le_bytes(*c))
                .collect::<Vec<_>>()
                .into(),
            geo: Geometrizer::new(polygon_rom(roms), geo::Core::of(board)),
            copro_empty_reads: 0,
            copro_result_hash: 0xcbf2_9ce4_8422_2325,
            copro_empty_by_ip: BTreeMap::new(),
            cur_ip: 0,
            code_gens: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            fast: Vec::new(),
            geo_words: 0,
            geo_trace: None,
            watch: None,
            tex_watch: None,
            uart_log: Vec::new(),
            frame_toggle: false,
            video_30hz: false,
            board_ctrl: 0,
            uart: Uart::default(),
            inputs: [0xff; 4],
            gear: Default::default(),
            key_ramps: Default::default(),
            io_analog_channel: 0,
            analog: [0x80; 8],
            analog_reads: [(0, 0); 8],
            io_latch: [0; 16],
            eeprom: Eeprom::new(),
            controls: &roms.game.controls,
            drive: roms.game.drive_board.map(Drive::new),
            link: roms.game.link_board.map(crate::link::Link::new),
            link_trace: None,
            gun: roms
                .game
                .controls
                .guns
                .iter()
                .any(|g| matches!(g.source, romset::GunSource::Counters { .. }))
                .then(GunBoard::default),
            m2_io: (board == Board::Model2).then(|| Box::new([0; M2_IO_REGS])),
            protection: roms.game.protection.as_ref().map(|p| Box::new(protection::Device::new(p, &roms.program))),
            io_log: AccessLog::default(),
            unmapped: BTreeMap::new(),
            probing: false,
            probe: Probe::default(),
        }
    }

    #[inline]
    fn find(&self, addr: u32) -> Option<(usize, usize)> {
        let m = &self.map[self.pages[(addr >> 16) as usize].checked_sub(1)? as usize];
        if addr < m.start || addr > m.end {
            return None;
        }
        Some((m.mem, (m.base + (addr - m.start) as usize) % self.mems[m.mem].data.len()))
    }

    /// Run the TGP for up to `n` words, stopping when it waits for input.
    // Called on every i960 instruction: the checks inline into the
    // recompiled program crates, the stepping doesn't.
    #[inline]
    pub fn run_tgp(&mut self, n: u64) {
        if !self.copro_booted || self.tgp_fault.is_some() {
            return;
        }
        // A stalled TGP stays stalled until its input FIFO has a word; this
        // is called on every i960 instruction, so don't step it until then.
        if self.tgp_stalled && self.tgp.fifo_in.is_empty() {
            return;
        }
        self.step_tgp(n);
    }

    #[inline(never)]
    fn step_tgp(&mut self, n: u64) {
        let mut ext =
            TgpExt { buffer: &mut self.mems[BUFFER_RAM].data, data: &self.copro_data, commands: self.geo.commands() };
        #[cfg(not(target_arch = "wasm32"))]
        let jit = self
            .tgp_jit
            .run(&mut self.tgp, &mut ext, n, &|a| (0x40_0000..=0x7f_ffff).contains(&a).then_some(a & 0x7fff));
        #[cfg(target_arch = "wasm32")]
        let jit = None;
        let r = match (jit, crate::recomp::copro(self.tgp.program_hash)) {
            (Some(r), _) => r,
            (None, Some(step)) => {
                (0..n).map(|_| step(&mut self.tgp, &mut ext)).find(|&r| r != Step::Ran).unwrap_or(Step::Ran)
            }
            (None, None) => self.tgp.run(&mut ext, n),
        };
        match r {
            Step::Unsupported(pc) => self.tgp_fault = Some(pc),
            r => self.tgp_stalled = r == Step::Stalled,
        }
    }

    /// Start processing this frame's display list (called at vblank). It
    /// runs alongside the i960; `geo.sync()` waits for it.
    pub fn run_geometrizer(&mut self) {
        self.geo.start_frame(&self.mems[BUFFER_RAM].data);
    }

    /// Everything that happens between two i960 instructions: the TGP runs,
    /// time and timers advance, vblank and interrupt lines are raised.
    /// True if this instruction ended a frame.
    #[inline]
    pub fn retire_insn(&mut self, cpu: &mut i960::cpu::Cpu) -> bool {
        // The TGP runs at 20 MHz against the i960's 25 MHz: about two of its
        // words per i960 instruction at our cycles-per-instruction estimate.
        self.run_tgp(crate::TGP_WORDS_PER_INSN);
        if let Some(c) = &mut self.sharc {
            c.run(sharc::STEPS_PER_INSN, &mut self.mems[BUFFER_RAM].data, self.geo.commands());
        }
        self.now += crate::CYCLES_PER_INSN;
        self.tick_timers();
        // The UART's TxRDY pin holds request bit 10 while it's enabled: set
        // again whenever the game clears it with the transmitter ready.
        if self.irq_enable & IRQ_UART != 0 && self.irq_request & IRQ_UART == 0 && self.uart.tx_ready(self.now) {
            self.request_irq(IRQ_UART);
        }
        let vblank = self.now >= self.next_vblank;
        if vblank {
            self.next_vblank += FRAME_CYCLES;
            self.frames += 1;
            if self.tex_watch.is_some() {
                self.watch_textures();
            }
            // In 30 Hz mode the frame bit and the geometrizer's start come
            // on every second vblank (the even ones); the vblank interrupt
            // still comes on each (*unconfirmed*).
            if !self.video_30hz || self.frames.is_multiple_of(2) {
                self.frame_toggle = !self.frame_toggle;
                self.run_geometrizer();
            }
            if self.link.as_ref().is_some_and(|l| l.fake().is_some()) {
                self.link_mirror();
            }
            self.request_irq(IRQ_VBLANK);
        }
        for line in 0..4 {
            if std::mem::take(&mut self.raised_lines[line]) {
                cpu.raise_line(line as u32);
            }
        }
        vblank
    }

    /// The texture upload watch's look at the frame that just ended.
    fn watch_textures(&mut self) {
        let mut writes = self.mems[RENDER_RAM].writes;
        if let Some(b) = self.video_2a {
            writes += self.mems[b].writes + self.mems[b + 1].writes;
        }
        if let Some(w) = self.tex_watch.as_deref_mut() {
            w.frame_end(self.frames, &self.mems[RENDER_RAM].data, writes);
        }
    }

    /// Raise IRQ request bits (only those already enabled latch).
    pub fn request_irq(&mut self, bits: u32) {
        let before = irq_line(self.irq_request);
        self.irq_request |= bits & self.irq_enable;
        let after = irq_line(self.irq_request);
        for i in 0..4 {
            if after[i] && !before[i] {
                self.raised_lines[i] = true;
            }
        }
    }

    /// How many i960 instructions from now retire with nothing happening
    /// but time passing and the copro running: no vblank, no timer
    /// expiring, no sound UART interrupt (all `retire_insn` does besides).
    /// The retire after them is the first that may.
    pub(crate) fn quiet_retires(&self) -> u64 {
        // Retire r (from 1) leaves `now` at now + CYCLES_PER_INSN * r.
        let first = |at: u64| at.saturating_sub(self.now).div_ceil(crate::CYCLES_PER_INSN).max(1);
        let mut q = first(self.next_vblank);
        for t in self.timers.iter().filter(|t| t.running) {
            q = q.min(first(t.started_at + t.reload as u64));
        }
        if self.irq_enable & IRQ_UART != 0 && self.irq_request & IRQ_UART == 0 && self.uart.tx_enabled {
            q = q.min(first(self.uart.ready_at));
        }
        q - 1
    }

    /// True if the copro can't change anything the i960 reads until the
    /// i960 gives it work: no program, stopped, or waiting on its input.
    pub(crate) fn copro_idle(&self) -> bool {
        let tgp = !self.copro_booted || self.tgp_fault.is_some() || (self.tgp_stalled && self.tgp.fifo_in.is_empty());
        tgp && self.sharc.as_ref().is_none_or(|c| c.idle())
    }

    /// Retire `n` quiet instructions (within [`Model2Bus::quiet_retires`])
    /// at once, `k` passes of an idle loop that read the registers
    /// `io_reads` each: the copro runs as far as `n` single retires take
    /// it, batched, and time moves on. The 2C TGP's batch is exact: `n`
    /// runs of a few words stop where one long one does (it stalls only
    /// on an empty input FIFO, which nothing fills meanwhile, and stays
    /// stalled), and so is the SHARC's (`sharc::Copro::run_calls`).
    pub(crate) fn idle_advance(&mut self, n: u64, k: u64, io_reads: &[u32]) {
        self.run_tgp(crate::TGP_WORDS_PER_INSN * n);
        if let Some(c) = &mut self.sharc {
            c.run_calls(n, sharc::STEPS_PER_INSN, &mut self.mems[BUFFER_RAM].data, self.geo.commands());
        }
        self.now += crate::CYCLES_PER_INSN * n;
        for &a in io_reads {
            self.io_log.entry(a & !3).or_default().reads += k;
        }
    }

    /// Start watching the i960's accesses for an idle-loop probe.
    pub(crate) fn probe_begin(&mut self) {
        self.probe.impure = false;
        self.probe.needs_copro_idle = false;
        self.probe.io_reads.clear();
        self.probing = true;
    }

    /// Stop watching: what the probe saw. [`Model2Bus::probe_return`] gives
    /// it back, to keep its buffer.
    pub(crate) fn probe_end(&mut self) -> Probe {
        self.probing = false;
        std::mem::take(&mut self.probe)
    }

    pub(crate) fn probe_return(&mut self, p: Probe) {
        self.probe = p;
    }

    /// A read during a probe. Plain memory other than buffer RAM is
    /// written only by the i960, and a few registers change only at events
    /// (`quiet_retires`) or by the i960's own writes; anything else may
    /// change at any time (timer counts, inputs, FIFO data) or has side
    /// effects.
    #[cold]
    #[inline(never)]
    fn probe_read(&mut self, addr: u32) {
        match self.find(addr) {
            Some((m, _)) => self.probe.needs_copro_idle |= m == BUFFER_RAM,
            None => match addr & !3 {
                // Video status (vblank), interrupt requests and enable,
                // copro control and status, board control.
                0x0098_000c | 0x00e8_0000 | 0x00e8_0004 | 0x0098_0000 | 0x0098_0014 | 0x00f8_0000 => {
                    self.probe.io_reads.push(addr)
                }
                // The copro's output FIFO empty flag.
                0x0098_0004 => {
                    self.probe.needs_copro_idle = true;
                    self.probe.io_reads.push(addr);
                }
                _ => self.probe.impure = true,
            },
        }
    }

    /// Advance timers to `now`, raising their IRQs on expiry.
    #[inline]
    pub fn tick_timers(&mut self) {
        for i in 0..4 {
            let t = &mut self.timers[i];
            if t.running && self.now - t.started_at >= t.reload as u64 {
                t.running = false;
                t.reload = 0xfffff;
                self.request_irq(IRQ_TIMER0 << i);
            }
        }
    }

    fn log(&mut self, addr: u32, write: Option<u32>) {
        let e = self.io_log.entry(addr & !3).or_default();
        match write {
            Some(v) => {
                e.writes += 1;
                e.last_write = v;
            }
            None => e.reads += 1,
        }
    }

    /// Model 2B's registers where they differ from 2C's.
    fn io_read32_2b(&mut self, addr: u32) -> Option<u32> {
        let c = self.sharc.as_mut()?;
        Some(match addr & !3 {
            0x0088_4000..=0x0088_7fff => {
                // The i960 stalls until a result arrives: let the copro run.
                if c.fifo_out.is_empty() {
                    c.run_for_result(&mut self.mems[BUFFER_RAM].data, self.geo.commands());
                }
                pop_result(
                    &mut c.fifo_out,
                    &mut self.copro_result_hash,
                    &mut self.copro_empty_reads,
                    &mut self.copro_empty_by_ip,
                    self.cur_ip,
                )
            }
            0x0098_0004 => c.fifo_out.is_empty() as u32,
            // Copro status. indy500's T.G.P. test reads it once and calls
            // the chips IC31 (bit 1) and IC37 (bit 0) bad if their bit is
            // set. As on 2C, all ones until the programs are uploaded, then
            // 0 (*unconfirmed*: no other 2B game reads it).
            0x0098_0014 => copro_status(c.booted && self.geo.booted),
            0x008c_0000..=0x008c_03ff => c.host_read((addr - 0x008c_0000) >> 2),
            0x0084_0000..=0x0084_03ff => 0,
            // Sound UART (8251 type): status, then data, a register a word.
            // Status: transmitter ready and empty, nothing received.
            0x009c_0004 => 0x05,
            0x009c_0000 => 0,
            // 2C's UART isn't here on 2B. (The 2B program writes mode
            // bytes beside it, to 0x1C80004, which stays unmapped.)
            0x01c8_0000..=0x01c8_0003 => 0,
            _ => return None,
        })
    }

    /// Model 2B's registers where they differ from 2C's. False if `addr`
    /// isn't one.
    fn io_write32_2b(&mut self, addr: u32, v: u32) -> bool {
        let Some(c) = self.sharc.as_mut() else { return false };
        match addr & !3 {
            0x0088_4000..=0x0088_7fff if c.held => c.host_write(adsp2106x::cpu::EPB0, v),
            0x0088_4000..=0x0088_7fff => c.fifo_in.push_back(v),
            0x0088_0000..=0x0088_3fff => c.fifo_in.push_back(tag_command(v, addr)),
            0x008c_0000..=0x008c_03ff => c.host_write((addr - 0x008c_0000) >> 2, v),
            0x0084_0000..=0x0084_03ff => {
                self.geo.host_write((addr - 0x0084_0000) >> 2, v);
            }
            0x0098_0000 => {
                c.control(v & 0x8000_0000 != 0);
                self.copro_ctrl = v;
            }
            0x009c_0000 | 0x009c_0004 => {
                let control = addr & 4 != 0;
                self.uart_log.push((self.now, control, v as u8));
                self.uart.write(control, v as u8, self.now);
            }
            0x01c8_0000..=0x01c8_0003 => {}
            _ => return false,
        }
        true
    }

    /// Model 2A's registers where they differ from 2C's.
    fn io_read32_2a(&mut self, addr: u32) -> Option<u32> {
        let t = self.tgp2a.as_mut()?;
        let bus =
            &mut Shared { buffer: &mut self.mems[BUFFER_RAM].data, commands: self.geo.commands(), rom: &self.tgp_rom };
        Some(match addr & !3 {
            0x0088_4000..=0x0088_7fff => t.data_read(self.frames, self.cur_ip, bus),
            0x0098_0004 => t.output_empty() as u32,
            // As on 2C: all ones while the TGP holds no program.
            0x0098_0014 => copro_status(!t.program.is_empty()),
            _ => return None,
        })
    }

    /// Model 2A's registers where they differ from 2C's. False if `addr`
    /// isn't one.
    fn io_write32_2a(&mut self, addr: u32, v: u32) -> bool {
        let Some(t) = self.tgp2a.as_mut() else { return false };
        let bus =
            &mut Shared { buffer: &mut self.mems[BUFFER_RAM].data, commands: self.geo.commands(), rom: &self.tgp_rom };
        match addr & !3 {
            0x0088_4000..=0x0088_7fff => t.data_write(addr & 0x3fff, v, self.frames, self.cur_ip, bus),
            0x0088_0000..=0x0088_3fff => t.command_write(addr & 0x3fff, v, self.frames, self.cur_ip, bus),
            0x0098_0000 => {
                t.control(self.copro_ctrl, v);
                self.copro_ctrl = v;
            }
            _ => return false,
        }
        true
    }

    /// The original Model 2's I/O registers (docs/hw/model2.md): register
    /// N at 0x1C00000 + 2N, in byte lanes 0 and 2 of its word. *From
    /// `daytona`'s code:* 0-7 the analog channels (read directly, no
    /// channel select), 8, 9, 10 and 14 switch inputs (active low), 11 DIP
    /// switches (`vcop`'s; read as all off), 15 an
    /// output latch, 17 the drive board's command; 32 a command register
    /// (the game waits for it to read 0) and 33 a status (it waits for
    /// 0x40). The commands complete at once here.
    fn m2_io_reg_read(&mut self, reg: usize) -> u8 {
        let latched = self.m2_io.as_ref().map_or(0, |r| r[reg % M2_IO_REGS]);
        match reg {
            0..=7 => {
                let v = self.analog[reg];
                self.analog_reads[reg] = (self.now, v);
                v
            }
            8 => self.inputs[0],
            9 => self.inputs[1],
            10 => self.inputs[2],
            // DIP switches, active low: all off (`vcop` reads its reload
            // and enemy type from bits 7 and 6, docs/games/vcop.md).
            11 => 0xff,
            14 => self.inputs[3],
            32 => 0,
            33 => 0x40,
            _ => latched,
        }
    }

    fn io_read32_m2(&mut self, addr: u32) -> Option<u32> {
        self.m2_io.as_ref()?;
        // The light gun registers at 0x1C00100 (`vcop`, docs/hw/model2.md):
        // register N, a byte at 0x1C00100 + 2N, is gun counter N, laid out
        // as the 2C gun interface's (players' Y and X in 0-7, the "sees
        // no screen" bits in 8), but read directly.
        if let Some(g) = &self.gun {
            if (0x01c0_0100..0x01c0_0120).contains(&addr) {
                let n = ((addr & !3) - 0x01c0_0100) as usize / 2;
                return Some(g.counters[n] as u32 | (g.counters[n + 1] as u32) << 16);
            }
        }
        if !(0x01c0_0000..0x01c0_0000 + 2 * M2_IO_REGS as u32).contains(&addr) {
            return None;
        }
        let reg = ((addr & !3) - 0x01c0_0000) as usize / 2;
        Some(self.m2_io_reg_read(reg) as u32 | (self.m2_io_reg_read(reg + 1) as u32) << 16)
    }

    fn io_write32_m2(&mut self, addr: u32, v: u32, lanes: u32) -> bool {
        if self.m2_io.is_none() || !(0x01c0_0000..0x01c0_0000 + 2 * M2_IO_REGS as u32).contains(&addr) {
            return false;
        }
        let reg = ((addr & !3) - 0x01c0_0000) as usize / 2;
        for k in 0..2 {
            if lanes & (1 << (2 * k)) != 0 {
                let b = (v >> (16 * k)) as u8;
                if let Some(r) = self.m2_io.as_mut() {
                    r[reg + k] = b;
                }
                if reg + k == 17 {
                    if let Some(d) = &mut self.drive {
                        d.command(b);
                    }
                }
            }
        }
        true
    }

    fn io_read32(&mut self, addr: u32) -> Option<u32> {
        if let Some(v) = self.io_read32_2b(addr).or_else(|| self.io_read32_2a(addr)).or_else(|| self.io_read32_m2(addr))
        {
            self.log(addr, None);
            return Some(v);
        }
        let v = match addr & !3 {
            0x0080_0000..=0x0080_7fff => self.geo.port_read(addr - 0x0080_0000),
            0x0088_4000..=0x0088_7fff => {
                // The i960 stalls until a result arrives: let the TGP run.
                if self.tgp.fifo_out.is_empty() {
                    self.run_tgp(TGP_STALL_LIMIT);
                }
                pop_result(
                    &mut self.tgp.fifo_out,
                    &mut self.copro_result_hash,
                    &mut self.copro_empty_reads,
                    &mut self.copro_empty_by_ip,
                    self.cur_ip,
                )
            }
            0x0098_0000 => self.copro_ctrl,
            0x0098_0004 => self.tgp.fifo_out.is_empty() as u32,
            0x0098_000c => (self.frame_toggle as u32) << 2,
            // All ones while the copro holds no program: from reset until
            // an upload's first word, and again once a new upload starts.
            // bel uploads its program only if it reads bit 0 set at boot.
            0x0098_0014 => copro_status(!self.copro_program.is_empty()),
            0x0098_0008..=0x0098_003f => 0,
            // Board control byte set during reset; meaning unknown.
            0x00f8_0000 => self.board_ctrl as u32,
            0x00e8_0000 => self.irq_request,
            // Reads of the second word of the pair: the requests that are
            // enabled. Last Bronx's interrupt dispatcher loads the pair
            // (`ldl 0xE80000`) and services the bits of the second word
            // until none is left, so it can't be the mask it was written as
            // (docs/games/lastbrnx.md); *unconfirmed* what the
            // hardware returns. No other game reads it.
            0x00e8_0004 => self.irq_request & self.irq_enable,
            // A timer's count reads one word above where it is written:
            // timer N (written at 0xF00000 + 4N) counts at 0xF00004 + 4N.
            // Every game that reads the timers does so with `ldl` and uses
            // the second word just after writing the timer of the first:
            // `vf2` times a frame on timer 2 (`st 0xF00008` at vblank, then
            // the second word of `ldl 0xF00008`), `lastbrnx` and
            // `dynabb97` profile on timers 0 and 1 the same way
            // (docs/hw/peripherals.md). What a lone read of 0xF00000
            // returns is *unconfirmed* (no game makes one); it wraps to
            // timer 3 here.
            0x00f0_0000..=0x00f0_000f => {
                let t = &self.timers[(((addr >> 2) + 3) & 3) as usize];
                if t.running {
                    t.reload.saturating_sub((self.now - t.started_at) as u32)
                } else {
                    t.reload
                }
            }
            0x01c0_0000..=0x01c0_001f => {
                let lo = self.io_reg_read((((addr - 0x01c0_0000) >> 2) * 2) as usize) as u32;
                let hi = self.io_reg_read((((addr - 0x01c0_0000) >> 2) * 2 + 1) as usize) as u32;
                lo | (hi << 16)
            }
            // UART: status reports transmitter ready and empty, nothing received.
            0x01c8_0000..=0x01c8_0003 => 0x0005_0000,
            // Communication (link) board registers with no board fitted:
            // they read as all ones. Bit 0 of 0x1A14000 set is "no
            // communication board" to stcc's and overrev's network checks,
            // and overrev's stand-alone boot then waits for bit 7 of
            // 0x1A14002 to be set (docs/games/overrev.md).
            0x01a1_4000 => self.link.as_ref().map_or(u32::MAX, crate::link::Link::status),
            0x1000_0000..=0x105f_ffff => 0,
            _ => return None,
        };
        self.log(addr, None);
        Some(v)
    }

    /// 315-5649 register read. Inputs are active low. Unverified (MAME):
    /// port A bit 0 ("control mode") routes EEPROM DO onto port B bit 5, with
    /// bits 7, 6 and 4 reading high.
    fn io_reg_read(&mut self, reg: usize) -> u8 {
        match reg {
            0 | 5 => self.io_latch[reg],
            1 if self.io_latch[0] & 1 != 0 => (self.inputs[0] & 0x0f) | 0xd0 | ((self.eeprom.dout() as u8) << 5),
            1 => self.inputs[0],
            2 => self.inputs[1],
            3 => self.drive.as_ref().map_or(self.inputs[2], Drive::reply),
            6 => self.inputs[3],
            8 => 0xff,
            12 if self.gun.is_some() => self.gun.as_ref().map_or(0, GunBoard::reply),
            13 if self.gun.is_some() => GunBoard::STATUS,
            0xf => {
                let v = self.analog[self.io_analog_channel & 7];
                self.analog_reads[self.io_analog_channel & 7] = (self.now, v);
                self.io_analog_channel = (self.io_analog_channel + 1) & 7;
                v
            }
            _ => 0,
        }
    }

    fn io_reg_write(&mut self, reg: usize, v: u8) {
        self.io_latch[reg & 15] = v;
        match reg {
            // Port A: bit 5 DI, bit 6 CS, bit 7 CLK.
            0 => self.eeprom.set_lines(v & 0x40 != 0, v & 0x80 != 0, v & 0x20 != 0),
            4 => {
                if let Some(d) = &mut self.drive {
                    d.command(v);
                }
            }
            9 => {
                if let Some(g) = &mut self.gun {
                    g.command(v, self.io_latch[10]);
                }
            }
            0xf => self.io_analog_channel = (v & 7) as usize,
            _ => {}
        }
    }

    /// `lanes` is the byte-enable mask of the access within its word.
    fn io_write32(&mut self, addr: u32, v: u32, lanes: u32) -> bool {
        if let (Some(t), 0x0080_0000..=0x0080_7fff) = (self.geo_trace.as_mut(), addr) {
            t.push((addr, v));
        }
        if self.io_write32_2b(addr, v) || self.io_write32_2a(addr, v) || self.io_write32_m2(addr, v, lanes) {
            self.log(addr, Some(v));
            return true;
        }
        match addr & !3 {
            0x0080_0000..=0x0080_7fff => {
                if addr >= 0x0080_4000 {
                    self.geo_words += 1;
                }
                let buffer = &mut self.mems[BUFFER_RAM].data;
                if !self.geo.port_write(addr - 0x0080_0000, v, buffer) {
                    return false;
                }
            }
            0x0088_4000..=0x0088_7fff => {
                if self.copro_uploading {
                    // Program RAM takes 64-bit words, low half first.
                    match self.copro_upload_half.take() {
                        None => self.copro_upload_half = Some(v),
                        Some(lo) => self.copro_program.push(((v as u64) << 32) | lo as u64),
                    }
                } else {
                    self.tgp.fifo_in.push_back(v);
                }
            }
            0x0088_0000..=0x0088_3fff => {
                self.tgp.fifo_in.push_back(tag_command(v, addr));
            }
            0x0098_0000 => {
                let rising = v & 0x8000_0000 != 0 && self.copro_ctrl & 0x8000_0000 == 0;
                let falling = v & 0x8000_0000 == 0 && self.copro_ctrl & 0x8000_0000 != 0;
                if rising {
                    self.copro_uploading = true;
                    self.copro_program.clear();
                }
                if falling {
                    self.copro_uploading = false;
                    self.copro_booted = true;
                    self.tgp.boot(&self.copro_program);
                    self.tgp_stalled = false;
                    #[cfg(not(target_arch = "wasm32"))]
                    self.tgp_jit.reset();
                }
                self.copro_ctrl = v;
            }
            0x0098_0008 => self.geo.control_write(v),
            // Model 2B only for now: `vcop`, `skytargt` and `dynamcop` (2, 2A) write the same
            // two values and are left at 60 Hz (docs/games/vstriker.md, "Frame rate").
            0x0098_000c => self.video_30hz = self.sharc.is_some() && v & 1 != 0,
            0x0098_0004..=0x0098_003f => {}
            0x00f8_0000 => self.board_ctrl = v as u8,
            0x00e8_0000 => self.irq_request &= v,
            0x00e8_0004 => self.irq_enable = v,
            0x00f0_0000..=0x00f0_000f => {
                let t = &mut self.timers[((addr >> 2) & 3) as usize];
                t.reload = v & 0xff_ffff;
                t.started_at = self.now;
                t.running = true;
            }
            0x01c0_0000..=0x01c0_001f => {
                // Two 8-bit registers per word, in byte lanes 0 and 2.
                let reg = (((addr - 0x01c0_0000) >> 2) * 2) as usize;
                for k in 0..2 {
                    if lanes & (1 << (2 * k)) != 0 {
                        self.io_reg_write(reg + k, (v >> (16 * k)) as u8);
                    }
                }
            }
            0x01c8_0000..=0x01c8_0003 => {
                // Sound link UART: data in lane 0, mode/command in lane 2.
                for (lane, control) in [(0, false), (2, true)] {
                    if lanes & (1 << lane) != 0 {
                        let b = (v >> (8 * lane)) as u8;
                        self.uart_log.push((self.now, control, b));
                        self.uart.write(control, b, self.now);
                    }
                }
            }
            0x1000_0000..=0x105f_ffff => {}
            _ => return false,
        }
        self.log(addr, Some(v));
        true
    }

    #[inline]
    fn mem_read(&self, addr: u32, n: usize) -> Option<u32> {
        let (m, off) = self.find(addr)?;
        let d = &self.mems[m].data;
        let mut v = 0u32;
        for i in 0..n {
            v |= (d[(off + i) % d.len()] as u32) << (8 * i);
        }
        Some(v)
    }

    #[inline]
    fn mem_write(&mut self, addr: u32, v: u32, n: usize) -> bool {
        let Some((m, off)) = self.find(addr) else { return false };
        if let Some((w, log)) = &mut self.watch {
            if addr & !3 == *w & !3 {
                log.push((self.now, self.cur_ip, addr, v, n));
            }
        }
        if !self.code_gens.is_empty() {
            self.count_code_write(m, off, n);
        }
        let mem = &mut self.mems[m];
        if mem.writable {
            mem.writes += 1;
            let len = mem.data.len();
            for i in 0..n {
                mem.data[(off + i) % len] = (v >> (8 * i)) as u8;
            }
            if let Some(v2a) = self.video_2a.filter(|&b| m == b || m == b + 1) {
                self.mirror_video_2a(m - v2a, off & !3);
            }
        }
        true
    }

    /// Model 2A's video RAM holds one 16-bit value in each 32-bit word
    /// (docs/hw/model2a.md, 3); the rasteriser reads the 2C's layout, the
    /// same values back to back, so the word at byte offset `o` of video
    /// RAM block `block` (0 texture RAM, 1 the luma tables) is kept there
    /// too. Texture RAM's banks are 4 MB apart here and 2 MB there, and a
    /// bank holds 512K words (the sheet's 1 MB): the 2 MB of 32-bit words
    /// in its window repeat (`vf2` fills only the window's upper half,
    /// words 0x80000 up; docs/hw/model2a.md, 3).
    fn mirror_video_2a(&mut self, block: usize, o: usize) {
        let b = self.video_2a.unwrap_or(0) + block;
        let w = [self.mems[b].data[o], self.mems[b].data[o + 1]];
        let k = o / 4;
        let at = if block == 0 { (k >> 20) * 0x20_0000 + (k & 0x7_ffff) * 2 } else { 0x40_0000 + k * 2 };
        let render = &mut self.mems[RENDER_RAM];
        if let Some(d) = render.data.get_mut(at..at + 2) {
            d.copy_from_slice(&w);
            render.writes += 1;
        }
    }

    /// Start counting writes to the RAMs code can run from (the JIT's
    /// check that code it translated from RAM is still there).
    pub fn count_code_writes(&mut self) {
        if self.code_gens.is_empty() {
            let len: usize = CODE_RAMS.iter().map(|&m| self.mems[m].data.len() >> CODE_GEN_SHIFT).sum();
            self.code_gens = vec![0; len];
        }
        #[cfg(not(target_arch = "wasm32"))]
        self.make_fast_pages();
    }

    /// The JIT's view of plain memory below `jit::FAST_SPAN`: a page whose
    /// 64 KB are all one memory, in order (no wrap of a mirror inside it),
    /// is read inline; written inline too if that memory is writable and
    /// a write does nothing but store, count (`Mem::writes`) and bump its
    /// code RAM counter (`count_code_write` adds 2 for a write within one
    /// counter's KB: its first and last byte's). Not Model 2A's video RAM,
    /// whose writes are mirrored.
    #[cfg(not(target_arch = "wasm32"))]
    fn make_fast_pages(&mut self) {
        let mut fast = vec![jit::FastPage::NONE; jit::FAST_PAGES];
        for (p, e) in fast.iter_mut().enumerate() {
            let first = (p as u32) << jit::FAST_SHIFT;
            let last = first + (jit::FAST_PAGE - 1);
            let (Some((m, off)), Some((m2, off2))) = (self.find(first), self.find(last)) else { continue };
            if m != m2 || off2 != off + (jit::FAST_PAGE - 1) as usize {
                continue;
            }
            let video = self.video_2a.is_some_and(|b| m == b || m == b + 1);
            let gen = self.code_gen(m, off).filter(|_| !self.code_gens.is_empty());
            let gens = self.code_gens.as_mut_ptr();
            let mem = &mut self.mems[m];
            e.data = mem.data[off..].as_mut_ptr();
            if mem.writable && !video {
                e.writes = &mut mem.writes;
                if let Some(g) = gen {
                    // SAFETY: the counters of this page's 64 KB are in
                    // `code_gens` (`code_gen` of its first byte, the rest
                    // after it).
                    e.gen = unsafe { gens.add(g) };
                    e.gen_add = 2;
                }
            }
        }
        self.fast = fast;
    }

    /// Run the copro programs on their JITs, or not (docs/jit.md, "Copro
    /// JITs") as `c` says: `copro`, and with `copro_check` each beside its
    /// interpreter (a lockstep), translating on the workers if
    /// `background`; noting their blocks in `list` and translating those
    /// it has at once (docs/jit.md, "Hot-block list").
    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_copro_jit(
        &mut self,
        c: &jit::Config,
        list: Option<&std::sync::Arc<jit::HotList>>,
    ) -> Result<(), String> {
        if let Some(t) = self.tgp2a.as_deref_mut() {
            t.set_jit(c, list)?;
        }
        // Model 2C's copro (2A's `tgp` and 2B's never run).
        if self.tgp2a.is_none() && self.sharc.is_none() {
            self.tgp_jit.set(c.copro, c, list)?;
        }
        // Model 2B's.
        if let Some(s) = self.sharc.as_deref_mut() {
            s.set_jit(c, list)?;
        }
        self.geo.set_tgp_jit(c, list)
    }

    /// The copro JITs' counters, by chip, with the runs compared in a
    /// lockstep.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn copro_jit_stats(&self) -> Vec<(&'static str, jit::CoproStats, u64)> {
        let mut v = Vec::new();
        if let Some((s, n)) = self.tgp2a.as_deref().and_then(|t| t.jit_stats()) {
            v.push(("MB86234", s, n));
        }
        if let Some(j) = &self.tgp_jit.code {
            v.push(("MB86235 copro", j.stats(), self.tgp_jit.checked));
        }
        if let Some(s) = self.sharc.as_deref() {
            if let Some(j) = &s.jit.code {
                v.push(("ADSP-21062 copro", j.stats(), s.jit.checked));
            }
        }
        if let Some((s, n)) = self.geo.tgp_jit_stats() {
            v.push(("MB86235 geometrizer", s, n));
        }
        if let Some((s, n)) = self.geo.sharc_jit_stats() {
            v.push(("ADSP-21062 geometrizer", s, n));
        }
        v
    }

    /// Move every write counter, as if all of the code RAMs had been
    /// written (after their contents were replaced directly).
    pub fn code_written_everywhere(&mut self) {
        self.code_gens.iter_mut().for_each(|g| *g = g.wrapping_add(1));
    }

    /// The write counter of byte `off` of memory `m`, if it has one.
    #[inline]
    fn code_gen(&self, m: usize, off: usize) -> Option<usize> {
        let below = if m == CODE_RAMS[1] { self.mems[CODE_RAMS[0]].data.len() >> CODE_GEN_SHIFT } else { 0 };
        CODE_RAMS.contains(&m).then_some(below + (off >> CODE_GEN_SHIFT))
    }

    #[inline]
    fn count_code_write(&mut self, m: usize, off: usize, n: usize) {
        let len = self.mems[m].data.len();
        for o in [off, (off + n - 1) % len] {
            if let Some(i) = self.code_gen(m, o) {
                self.code_gens[i] = self.code_gens[i].wrapping_add(1);
            }
        }
    }

    fn unmapped(&mut self, addr: u32, write: Option<u32>) {
        let e = self.unmapped.entry(addr & !3).or_default();
        match write {
            Some(v) => {
                e.writes += 1;
                e.last_write = v;
            }
            None => e.reads += 1,
        }
    }

    #[inline]
    fn read(&mut self, addr: u32, n: usize) -> u32 {
        let v = self.read_untraced(addr, n);
        if addr >> 17 == 0x01a0_0000 >> 17 {
            let hit = LinkHit { cycle: self.now, ip: self.cur_ip, addr, value: v, size: n as u8, write: false };
            if let Some(t) = &mut self.link_trace {
                t.push(hit);
            }
        }
        v
    }

    #[inline]
    fn read_untraced(&mut self, addr: u32, n: usize) -> u32 {
        if self.probing {
            self.probe_read(addr);
        }
        if let Some(v) = self.mem_read(addr, n) {
            return v;
        }
        if let Some(w) = self.io_read32(addr) {
            let shift = (addr & 3) * 8;
            let mask = if n == 4 { u32::MAX } else { (1u32 << (8 * n)) - 1 };
            return (w >> shift) & mask;
        }
        if let Some(p) = self.protection.as_mut().filter(|p| (protection::START..=p.end()).contains(&addr)) {
            let v = p.read(addr - protection::START, n);
            self.log(addr, None);
            return v;
        }
        self.unmapped(addr, None);
        0
    }

    #[inline]
    fn write(&mut self, addr: u32, v: u32, n: usize) {
        if addr >> 17 == 0x01a0_0000 >> 17 {
            let hit = LinkHit { cycle: self.now, ip: self.cur_ip, addr, value: v, size: n as u8, write: true };
            if let Some(t) = &mut self.link_trace {
                t.push(hit);
            }
        }
        if self.probing {
            self.probe.impure = true;
        }
        if self.mem_write(addr, v, n) {
            return;
        }
        let shift = (addr & 3) * 8;
        let shifted = v << shift;
        let lanes = ((1u32 << n) - 1) << (addr & 3);
        if self.io_write32(addr, shifted, lanes) {
            return;
        }
        if let Some(p) = self.protection.as_mut().filter(|p| (protection::START..=p.end()).contains(&addr)) {
            p.write(addr - protection::START, v, n);
            self.log(addr, Some(v));
            return;
        }
        if self.link.is_some() && addr & !3 == 0x01a1_4000 {
            self.link_register_write(addr, v);
        }
        self.unmapped(addr, Some(v));
    }

    /// A write to the link board's registers (experimental: the scripted
    /// partner of `REMODEL_LINK=fake`, `docs/hw/link-board.md`).
    fn link_register_write(&mut self, addr: u32, v: u32) {
        let Some((id, total)) = self.link.as_ref().and_then(crate::link::Link::fake) else { return };
        if addr == 0x01a1_4000 && v & 0xff == 1 {
            if let Some((m, off)) = self.find(0x01a1_0000) {
                let d = &mut self.mems[m].data;
                (d[off], d[off + 2], d[off + 3]) = (1, id, total);
            }
        }
    }

    /// Experimental partner that does what the game does (`REMODEL_LINK=
    /// fake`): at each vblank the game's own block (`0x1A12000`, 0x1C0
    /// bytes) is copied into the first TOTAL receive blocks (`0x1A121C0` +
    /// 0x1C0 each), as if every node were in the same state as it.
    fn link_mirror(&mut self) {
        let Some((_, total)) = self.link.as_ref().and_then(crate::link::Link::fake) else { return };
        let Some((m, base)) = self.find(0x01a1_2000) else { return };
        let sent = self.mems[m].data[base..base + 0x1c0].to_vec();
        let frame = self.frames;
        // Receive block 0 is the game's own block, echoed at once; the
        // others come from the partner, `delay` frames late.
        let got = self.link.as_mut().and_then(|l| l.deliver(frame, &sent));
        let drop_at = self.link.as_ref().map_or(0, |l| l.drop_at);
        let d = &mut self.mems[m].data;
        if drop_at != 0 && frame >= drop_at {
            if let Some((m, off)) = self.find(0x01a1_0000) {
                self.mems[m].data[off] = 0;
            }
            return;
        }
        d[base + 0x1c0..base + 0x380].copy_from_slice(&sent);
        if let Some(got) = got {
            for k in 1..total.min(4) as usize {
                let at = base + 0x1c0 * (k + 1);
                d[at..at + 0x1c0].copy_from_slice(&got);
            }
        }
    }

    /// The whole memory `addr` falls in (diagnostics: dumping a region).
    pub fn memory_at(&self, addr: u32) -> Option<&[u8]> {
        self.find(addr).map(|(m, _)| self.mems[m].data.as_slice())
    }

    /// Backup SRAM at 0x01D00000.
    pub fn backup_ram(&self) -> &[u8] {
        &self.mems[8].data
    }

    /// Tile RAM, character RAM and tilemap registers: 0x01000000-0x011FFFFF.
    pub fn tile_ram(&self) -> &[u8] {
        &self.mems[5].data
    }

    /// Buffer RAM shared with the geometrizer and TGP: 0x00900000-0x0091FFFF.
    pub fn buffer_ram(&self) -> &[u8] {
        &self.mems[BUFFER_RAM].data
    }

    /// 3D screen offsets from the H/V sync registers (0x01040000 and
    /// 0x01060000): (84 + H, 130 + V), per docs/hw/geometrizer.md 4.4.
    pub fn sync_offsets(&self) -> (f32, f32) {
        let t = &self.mems[5].data;
        let h = i16::from_le_bytes([t[0x4_0000], t[0x4_0001]]) as f32;
        let v = i16::from_le_bytes([t[0x6_0000], t[0x6_0001]]) as f32;
        (84.0 + h, 130.0 + v)
    }

    /// Tile RAM's write counter (a version number: the catalogue's
    /// measure of how much of the 2D layer the game rewrote).
    pub fn tile_ram_writes(&self) -> u64 {
        self.mems[5].writes
    }

    /// The main data ROM (0x02000000), where the game's own art and data
    /// are copied from.
    pub fn data_rom(&self) -> &[u8] {
        &self.mems[9].data
    }

    /// Render RAM from 0x11000000: texture banks, luma tables, framebuffers.
    pub fn render_ram(&self) -> &[u8] {
        &self.mems[10].data
    }

    /// Palette and colour tables: 0x01800000-0x0181FFFF.
    pub fn palette_ram(&self) -> &[u8] {
        &self.mems[6].data
    }

    /// The level (0-255) the display shows as black: on the original Model
    /// 2 the pedestal its games build their colour tables on
    /// ([`crate::video::black_pedestal`]), else 0. Renderers apply it to
    /// the finished frame ([`crate::video::lift_black`]).
    pub fn black_level(&self) -> f32 {
        if self.board != Board::Model2 {
            return 0.0;
        }
        crate::video::black_pedestal(self.palette_ram()).unwrap_or(0.0)
    }

    /// Versions of render RAM and palette RAM: they change whenever the
    /// memory does, so a renderer can skip re-uploading it.
    pub fn video_ram_versions(&self) -> (u64, u64) {
        (self.mems[RENDER_RAM].writes, self.mems[PALETTE_RAM].writes)
    }

    /// Hashes of the writable RAM apart from buffer RAM (the game's own
    /// state), and of buffer RAM (the display list), for comparing two runs
    /// frame by frame (`remodel latency`).
    pub fn ram_hashes(&self) -> (u64, u64) {
        let (mut ram, mut buffer) = (Fnv(0xcbf2_9ce4_8422_2325), Fnv(0xcbf2_9ce4_8422_2325));
        for (i, m) in self.mems[..self.test_rams].iter().enumerate().filter(|(_, m)| m.writable) {
            let h = if i == BUFFER_RAM { &mut buffer } else { &mut ram };
            m.data.chunks(8).for_each(|c| h.word(c.iter().fold(0, |a, &b| a << 8 | b as u64)));
        }
        (ram.0, buffer.0)
    }

    /// Hash of all RAM, the last raster stream and the copro results. Two
    /// runs with the same inputs must agree; used to check that speed-ups
    /// and the recompiler don't change behaviour.
    pub fn state_hash(&mut self) -> u64 {
        self.geo.sync();
        let mut h = Fnv(0xcbf2_9ce4_8422_2325);
        let (core, test) = self.mems.split_at(self.test_rams);
        for m in core.iter().filter(|m| m.writable).chain(test.iter().filter(|m| m.writes > 0)) {
            m.data.chunks(8).for_each(|c| h.word(c.iter().fold(0, |a, &b| a << 8 | b as u64)));
        }
        self.geo.engine().stream.iter().for_each(|&w| h.word(w as u64));
        h.word(self.copro_result_hash);
        h.0
    }
}

/// The polygon ROM as the geometrizer reads it (u32 words).
pub fn polygon_rom(roms: &RomSet) -> Vec<u32> {
    roms.bank(Role::Polygons).as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

/// The JIT's view of writes to code (docs/jit.md).
#[cfg(not(target_arch = "wasm32"))]
impl jit::JitHost for Model2Bus {
    fn code_generations(&self) -> &[u32] {
        &self.code_gens
    }
    fn code_generation_index(&self, addr: u32) -> Option<usize> {
        let (m, off) = self.find(addr)?;
        self.code_gen(m, off)
    }
    /// Not while an idle-loop probe or a write watch looks at accesses.
    #[inline]
    fn fast_pages(&mut self) -> *const jit::FastPage {
        if self.fast.is_empty() || self.probing || self.watch.is_some() || self.link_trace.is_some() {
            jit::no_fast_pages()
        } else {
            self.fast.as_ptr()
        }
    }
    /// Quiet: the copro does nothing when run (no program, stopped, or
    /// waiting on its input), no interrupt line was raised for the next
    /// retire to pass on, no event is due (`quiet_retires`), and the
    /// budget lasts.
    #[inline]
    fn quiet(&mut self) -> u64 {
        if !self.copro_idle() || self.raised_lines.iter().any(|&l| l) {
            return 0;
        }
        self.quiet_retires().min(self.budget.saturating_sub(1))
    }
    /// What that many `retire`s do then: time passes, the budget goes down.
    #[inline]
    fn retire_quiet(&mut self, _cpu: &mut i960::cpu::Cpu, n: u64) {
        self.now += crate::CYCLES_PER_INSN * n;
        self.budget -= n;
    }
}

/// Recompiled code returns at the end of each frame and when its budget
/// runs out.
impl i960::cpu::Host for Model2Bus {
    #[inline(always)]
    fn at(&mut self, ip: u32) {
        self.cur_ip = ip;
    }
    // Inline into the recompiled program crates, as it was when they were
    // compiled into this one.
    #[inline]
    fn retire(&mut self, cpu: &mut i960::cpu::Cpu) -> bool {
        let vblank = self.retire_insn(cpu);
        self.budget -= 1;
        !vblank && self.budget > 0
    }
}

impl Bus for Model2Bus {
    #[inline]
    fn read8(&mut self, a: u32) -> u8 {
        self.read(a, 1) as u8
    }
    #[inline]
    fn read16(&mut self, a: u32) -> u16 {
        self.read(a, 2) as u16
    }
    #[inline]
    fn read32(&mut self, a: u32) -> u32 {
        self.read(a, 4)
    }
    #[inline]
    fn write8(&mut self, a: u32, v: u8) {
        self.write(a, v as u32, 1)
    }
    #[inline]
    fn write16(&mut self, a: u32, v: u16) {
        self.write(a, v as u32, 2)
    }
    #[inline]
    fn write32(&mut self, a: u32, v: u32) {
        self.write(a, v, 4)
    }
    fn fetch(&self, a: u32) -> Option<u32> {
        self.mem_read(a, 4)
    }
    fn is_rom(&self, a: u32) -> bool {
        a < 0x20_0000
    }
}

/// Access counts by address, with a cheap hash (logged on every I/O access).
pub type AccessLog = std::collections::HashMap<u32, Access, std::hash::BuildHasherDefault<AddrHasher>>;

#[derive(Default)]
pub struct AddrHasher(u64);

impl std::hash::Hasher for AddrHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        bytes.iter().for_each(|&b| self.0 = (self.0 << 8 | b as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    }
    fn write_u32(&mut self, v: u32) {
        self.0 = (v as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    }
}

/// One CPU access to the link board's range (`Model2Bus::link_trace`).
#[derive(Clone, Copy, Debug)]
pub struct LinkHit {
    pub cycle: u64,
    pub ip: u32,
    pub addr: u32,
    pub value: u32,
    pub size: u8,
    pub write: bool,
}

/// A watched write: (cycle, ip, address, value, size in bytes).
pub type WatchHit = (u64, u32, u32, u32, usize);

struct Fnv(u64);

impl Fnv {
    fn word(&mut self, v: u64) {
        self.0 = (self.0 ^ v).wrapping_mul(0x100_0000_01b3);
    }
}

// Save states: writable memory (with its write counts), time, interrupts,
// timers, the copro and its upload state, the geometrizer, I/O and the
// EEPROM. ROMs, the memory map and diagnostics stay as they are.
impl state::State for Model2Bus {
    fn save(&self, w: &mut state::Writer) {
        self.save_parts(w, Parts::All);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.load_parts(r, Parts::All)
    }
}

/// What [`Model2Bus::save_parts`] covers: the whole state, or all of it
/// but memory, the copro and the geometrizer's engine (which
/// [`Model2Bus::copy_from`] copies directly).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Parts {
    All,
    Small,
}

/// What the last [`Model2Bus::copy_from`] into a bus copied, so the next
/// can skip memory neither side has written since; and a buffer kept for
/// the rest of the state.
#[derive(Default)]
pub struct CopyMarks {
    /// Per memory: (source writes, destination writes) after the copy.
    mems: Vec<(u64, u64)>,
    rest: Vec<u8>,
    /// For the CPU's state (`Machine::copy_from`).
    pub(crate) cpu: Vec<u8>,
    /// Bytes of memory the last copy moved (diagnostics).
    pub copied: usize,
}

/// A command word written through the 0x880000 window with its function
/// number (the address bits 11 to 4) merged into bits 30 to 23.
fn tag_command(v: u32, addr: u32) -> u32 {
    (v & 0x800f_ffff) | (((addr >> 4) & 0xff) << 23)
}

/// The copro status register: all ones until its programs are uploaded,
/// then 0.
fn copro_status(ready: bool) -> u32 {
    if ready {
        0
    } else {
        u32::MAX
    }
}

/// A read of the copro's output FIFO (the caller has run the copro if it
/// was empty): the next result, folded into the result hash; an empty read
/// is counted, in total and by the i960 instruction that made it, and
/// reads 0.
fn pop_result(
    fifo: &mut VecDeque<u32>,
    hash: &mut u64,
    empty_reads: &mut u64,
    empty_by_ip: &mut BTreeMap<u32, u64>,
    ip: u32,
) -> u32 {
    match fifo.pop_front() {
        Some(v) => {
            *hash = (*hash ^ v as u64).wrapping_mul(0x100_0000_01b3);
            v
        }
        None => {
            *empty_reads += 1;
            *empty_by_ip.entry(ip).or_default() += 1;
            0
        }
    }
}

impl Model2Bus {
    fn save_parts(&self, w: &mut state::Writer, parts: Parts) {
        use state::State as _;
        let all = parts == Parts::All;
        if all {
            for m in self.mems[..self.test_rams].iter().filter(|m| m.writable) {
                m.data.save(w);
                m.writes.save(w);
            }
        }
        [self.now, self.frames, self.next_vblank].save(w);
        self.irq_request.save(w);
        self.irq_enable.save(w);
        self.raised_lines.save(w);
        self.timers.save(w);
        self.copro_ctrl.save(w);
        self.copro_uploading.save(w);
        self.copro_program.save(w);
        self.copro_upload_half.save(w);
        self.copro_booted.save(w);
        if all {
            self.geo.save(w);
        } else {
            self.geo.save_front(w);
        }
        if all {
            self.tgp.save(w);
        }
        self.tgp_fault.save(w);
        self.tgp_stalled.save(w);
        self.copro_result_hash.save(w);
        self.uart_log.save(w);
        (self.frame_toggle as u8 | (self.video_30hz as u8) << 1).save(w);
        self.board_ctrl.save(w);
        self.inputs.save(w);
        self.io_analog_channel.save(w);
        self.analog.save(w);
        self.io_latch.save(w);
        state::State::save(&self.eeprom, w);
        // Only games with a drive board have one (older states load).
        if let Some(d) = &self.drive {
            d.save(w);
        }
        // Likewise the gun interface, for gun games only.
        if let Some(g) = &self.gun {
            g.save(w);
        }
        // Last, so states from before list jumps were modelled still load.
        self.geo.jumps().save(w);
        // After that, the memory-test RAMs.
        if all {
            for m in &self.mems[self.test_rams..] {
                m.data.save(w);
                m.writes.save(w);
            }
        }
        // Then the UART's transmitter.
        self.uart.save(w);
        // And on 2B, the copro SHARC.
        if let Some(c) = self.sharc.as_ref().filter(|_| all) {
            c.save(w);
        }
        // Then the protection device, for games with one.
        if let Some(p) = &self.protection {
            p.save(w);
        }
        // And on 2A, the TGP interface.
        if let Some(t) = &self.tgp2a {
            t.save(w);
        }
        // And on the original Model 2, its I/O registers.
        if let Some(r) = &self.m2_io {
            (**r).save(w);
        }
    }

    fn load_parts(&mut self, r: &mut state::Reader, parts: Parts) -> Result<(), state::Error> {
        use state::State as _;
        let (all, test_rams) = (parts == Parts::All, self.test_rams);
        if all {
            for m in self.mems[..test_rams].iter_mut().filter(|m| m.writable) {
                m.load(r)?;
            }
        }
        let mut t = [0u64; 3];
        t.load(r)?;
        [self.now, self.frames, self.next_vblank] = t;
        self.irq_request.load(r)?;
        self.irq_enable.load(r)?;
        self.raised_lines.load(r)?;
        self.timers.load(r)?;
        self.copro_ctrl.load(r)?;
        self.copro_uploading.load(r)?;
        self.copro_program.load(r)?;
        self.copro_upload_half.load(r)?;
        self.copro_booted.load(r)?;
        if all {
            self.geo.load(r)?;
        } else {
            self.geo.load_front(r)?;
        }
        if all {
            self.tgp.load(r)?;
            #[cfg(not(target_arch = "wasm32"))]
            self.tgp_jit.reset();
        }
        self.tgp_fault.load(r)?;
        self.tgp_stalled.load(r)?;
        self.copro_result_hash.load(r)?;
        self.uart_log.load(r)?;
        let mut video = 0u8;
        video.load(r)?;
        (self.frame_toggle, self.video_30hz) = (video & 1 != 0, video & 2 != 0);
        self.board_ctrl.load(r)?;
        self.inputs.load(r)?;
        self.io_analog_channel.load(r)?;
        self.analog.load(r)?;
        self.io_latch.load(r)?;
        state::State::load(&mut self.eeprom, r)?;
        if let Some(d) = &mut self.drive {
            d.load(r)?;
        }
        if let Some(g) = &mut self.gun {
            g.load(r)?;
        }
        let mut jumps: Vec<(u32, u32)> = Vec::new();
        if !r.done() {
            jumps.load(r)?;
        }
        self.geo.set_jumps(jumps);
        if all {
            for m in &mut self.mems[test_rams..] {
                if r.done() {
                    *m = Mem::zeroed(m.data.len());
                } else {
                    m.load(r)?;
                }
            }
        }
        self.uart = Uart::default();
        if !r.done() {
            self.uart.load(r)?;
        }
        if let Some(c) = self.sharc.as_mut().filter(|_| all) {
            c.load(r)?;
        }
        if let Some(p) = &mut self.protection {
            p.load(r)?;
        }
        if let Some(t) = &mut self.tgp2a {
            t.load(r)?;
        }
        if let Some(regs) = &mut self.m2_io {
            (**regs).load(r)?;
        }
        // Memory may have been reallocated.
        #[cfg(not(target_arch = "wasm32"))]
        if all && !self.fast.is_empty() {
            self.make_fast_pages();
        }
        Ok(())
    }

    /// Make this bus's state `src`'s, as saving `src`'s and loading it here
    /// would (for run-ahead, `crate::runahead`), but faster: memory is
    /// copied directly, and only where either bus has written it since the
    /// last copy into this one with the same `marks` (buffer RAM, which the
    /// copro and TGP write behind the counts, always); the geometrizer's
    /// engine is copied directly too ([`Geometrizer::copy_from`]). Both
    /// buses must be for the same ROM set.
    ///
    /// `marks` belong to this bus: start with new ones after changing it
    /// any other way (loading a state). The write counts a renderer's
    /// versions come from ([`Model2Bus::video_ram_versions`]) only ever
    /// grow here, past both buses' counts, so a renderer drawing this bus
    /// re-uploads what a copy changed (and a front end drawing both buses
    /// must forget what it uploaded when it switches). Otherwise the copy
    /// is exact: the same state hash.
    pub fn copy_from(&mut self, src: &mut Model2Bus, marks: &mut CopyMarks) {
        assert_eq!(self.mems.len(), src.mems.len(), "copy between buses of different games");
        marks.mems.resize(self.mems.len(), (u64::MAX, u64::MAX));
        marks.copied = 0;
        for (i, (d, s)) in self.mems.iter_mut().zip(&src.mems).enumerate() {
            if !s.writable {
                continue;
            }
            if i == BUFFER_RAM || marks.mems[i] != (s.writes, d.writes) {
                d.data.copy_from_slice(&s.data);
                // The counts are also versions for a renderer: render and
                // palette RAM's only grow here (see above); the others
                // are the source's, as a load would leave them.
                d.writes = if matches!(i, PALETTE_RAM | RENDER_RAM) { d.writes.max(s.writes) + 1 } else { s.writes };
                marks.copied += d.data.len();
            }
            marks.mems[i] = (s.writes, d.writes);
        }
        self.geo.copy_from(&mut src.geo);
        self.tgp.copy_from(&src.tgp);
        #[cfg(not(target_arch = "wasm32"))]
        self.tgp_jit.reset();
        if let (Some(d), Some(s)) = (self.sharc.as_deref_mut(), src.sharc.as_deref()) {
            d.copy_from(s, &mut marks.rest);
        }
        let mut w = state::Writer { buf: std::mem::take(&mut marks.rest) };
        w.buf.clear();
        src.save_parts(&mut w, Parts::Small);
        self.load_parts(&mut state::Reader::new(&w.buf), Parts::Small).expect("a bus's own state loads");
        marks.rest = w.buf;
    }
}

state::fields!(Timer { reload, started_at, running });

/// One byte on the sound link: 10 bits at 31.25 kbaud, in 25 MHz cycles.
const UART_BYTE_CYCLES: u64 = 8_000;

/// The sound link UART's transmit side (an 8251-type uPD71051), as far as
/// its TxRDY interrupt needs: bel sends its sound commands a byte per
/// interrupt (docs/hw/peripherals.md). The first control write after a
/// reset is the mode, later ones are commands; TxRDY is high while the
/// command enables the transmitter (bit 0) and it can take a byte, a byte
/// time after the last one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Uart {
    expect_mode: bool,
    tx_enabled: bool,
    ready_at: u64,
}

impl Default for Uart {
    fn default() -> Self {
        Uart { expect_mode: true, tx_enabled: false, ready_at: 0 }
    }
}

impl Uart {
    fn write(&mut self, control: bool, b: u8, now: u64) {
        if !control {
            self.ready_at = now + UART_BYTE_CYCLES;
        } else if self.expect_mode {
            self.expect_mode = false;
        } else if b & 0x40 != 0 {
            // Internal reset: a mode byte comes next.
            *self = Uart { ready_at: self.ready_at, ..Uart::default() };
        } else {
            self.tx_enabled = b & 1 != 0;
        }
    }

    #[inline]
    fn tx_ready(&self, now: u64) -> bool {
        self.tx_enabled && now >= self.ready_at
    }
}

state::fields!(Uart { expect_mode, tx_enabled, ready_at });

#[cfg(test)]
mod tests {
    use super::*;

    /// A bus for `game` with no ROM contents (enough for memory and I/O).
    fn bus(game: &str) -> Model2Bus {
        let game = romset::games::find(game).unwrap();
        let roms = RomSet { game, program: vec![0; 0x1000], chips: vec![Vec::new(); game.files.len()] };
        let mut b = Model2Bus::new(&roms);
        b.geo.set_threaded(false);
        b
    }

    /// Tile RAM is 1 MB, repeating through its 2 MB window (`lastbrnx`'s
    /// gauge patterns are written at `0x1181000` and drawn from `0x1081000`).
    #[test]
    fn tile_ram_repeats_through_its_window() {
        let mut b = bus("overrevb");
        b.write32(0x0118_1000, 0x1234_5678);
        assert_eq!(b.read32(0x0108_1000), 0x1234_5678);
        b.write32(0x0100_0004, 0x9abc_def0);
        assert_eq!(b.read32(0x0110_0004), 0x9abc_def0);
    }

    /// A timer's count reads one word above where it was written (`vf2`
    /// writes 0xF00008 at vblank and times the frame with the second word
    /// of `ldl 0xF00008`).
    #[test]
    fn timer_counts_read_a_word_above() {
        let mut b = bus("vf2");
        b.write32(0x00f0_0008, 0xfffff);
        b.now += 2500;
        assert_eq!(b.read32(0x00f0_000c), 0xfffff - 2500);
        // Timer 3, not running, reads back as written (wrapping to 0xF00000).
        assert_eq!(b.read32(0x00f0_0000), 0xfffff);
    }

    /// The word after the requests (what `lastbrnx`'s `ldl 0xE80000` takes
    /// for them) is the enabled requests, and an acknowledge clears them.
    #[test]
    fn interrupt_pair_reads_the_requests() {
        let mut b = bus("overrevb");
        b.write32(0x00e8_0004, 0x401);
        b.request_irq(0x401);
        assert_eq!((b.read32(0x00e8_0000), b.read32(0x00e8_0004)), (0x401, 0x401));
        b.write32(0x00e8_0000, !1);
        assert_eq!((b.read32(0x00e8_0000), b.read32(0x00e8_0004)), (0x400, 0x400));
        b.write32(0x00e8_0004, 1);
        assert_eq!(b.read32(0x00e8_0004), 0, "a masked request isn't enabled");
    }

    /// `copy_from` leaves the state a load of a save would (the same state
    /// hash), copying only what either side wrote since the last copy;
    /// the video RAM versions only grow.
    #[test]
    fn copy_is_a_save_and_load_of_what_changed() {
        for game in ["topskatr", "overrevb", "srallyc"] {
            let (mut real, mut shadow) = (bus(game), bus(game));
            for (a, v) in [(0x0050_0000, 1), (0x0100_0000, 2), (0x1100_0000, 3), (0x0090_0010, 4), (0x0180_0000, 5)] {
                real.write32(a, v);
            }
            real.inputs[0] = 0x7f;
            let mut marks = CopyMarks::default();
            shadow.copy_from(&mut real, &mut marks);
            let same = |a: &mut Model2Bus, b: &mut Model2Bus| {
                assert_eq!(a.state_hash(), b.state_hash());
                assert_eq!(a.inputs, b.inputs);
                let mut loaded = bus(game);
                state::State::load(&mut loaded, &mut state::Reader::new(&state::to_bytes(a))).unwrap();
                assert_eq!(loaded.state_hash(), b.state_hash());
            };
            same(&mut real, &mut shadow);
            let versions = shadow.video_ram_versions();
            assert!(versions.0 > real.video_ram_versions().0 && versions.1 > real.video_ram_versions().1);
            // The shadow runs ahead (writes), the real machine too.
            shadow.write32(0x0050_0004, 6);
            shadow.write32(0x1100_0004, 7);
            real.write32(0x0100_0004, 8);
            shadow.copy_from(&mut real, &mut marks);
            same(&mut real, &mut shadow);
            assert!(shadow.video_ram_versions().0 > versions.0, "render RAM changed: a new version");
            assert_eq!(shadow.video_ram_versions().1, versions.1, "palette RAM didn't");
            // Nothing written: only buffer RAM, which the copro writes
            // behind the counts, is copied.
            shadow.copy_from(&mut real, &mut marks);
            assert_eq!(marks.copied, 0x2_0000);
            same(&mut real, &mut shadow);
        }
    }

    /// Model 2A: 32-bit program words to the data port during an upload,
    /// the ping after it answered with 8, texture RAM at 0x12000000, and
    /// none of it on the other boards.
    #[test]
    fn model_2a_tgp_and_video_ram() {
        let mut b = bus("srallyc");
        // The stand-in (this program is no program).
        b.tgp2a.as_mut().unwrap().use_hle = true;
        b.write32(0x0098_0000, 0x8000_0000);
        for w in [0xbf60_0010u32, 0xbf60_0006, 0x1c00_0001] {
            b.write32(0x0088_4000, w);
        }
        assert_eq!(b.read32(0x0098_0014), 0, "a program is there");
        b.write32(0x0098_0000, 0);
        b.write32(0x0088_0080, 0);
        assert_eq!(b.read32(0x0088_4000), 8);
        assert_eq!(b.read32(0x0088_4000), 0, "empty: nothing else is answered");
        let t = b.tgp2a.as_ref().unwrap();
        assert_eq!(t.program, [0xbf60_0010, 0xbf60_0006, 0x1c00_0001]);
        assert_eq!(t.empty_reads, 1);
        b.write32(0x1200_0004, 0x1133);
        b.write32(0x1280_0000, 5);
        assert_eq!((b.read32(0x1200_0004), b.read32(0x1280_0000)), (0x1133, 5));
        // The rasteriser's render RAM has the 16-bit words back to back
        // (texture bank 1 is 2 MB above bank 0 there, 4 MB here; luma
        // tables from 4 MB).
        b.write32(0x1240_0008, 0xabcd);
        b.write32(0x1280_0004, 0x77);
        let r = b.render_ram();
        assert_eq!(u16::from_le_bytes([r[2], r[3]]), 0x1133);
        assert_eq!(u16::from_le_bytes([r[0x20_0004], r[0x20_0005]]), 0xabcd);
        assert_eq!(r[0x40_0002], 0x77);
        let mut other = bus("topskatr");
        other.write32(0x1200_0004, 0x1133);
        assert_eq!(other.read32(0x1200_0004), 0, "unmapped there");
        assert!(other.tgp2a.is_none());
    }
}
