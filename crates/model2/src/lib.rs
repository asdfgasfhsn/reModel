//! Sega Model 2 board model, driving the reference i960 interpreter.
//!
//! This is the "slow path": it runs the original program code so its
//! behaviour can be observed (which code runs, what hardware it touches)
//! and later used as the oracle for the recompiled build.

pub mod audio;
pub mod bus;
pub mod capture;
pub mod drive;
pub mod eeprom;
pub mod enhance;
pub mod geo;
pub mod geo_native;
pub mod geo_native_2c;
pub mod gun;
pub mod idle;
pub mod input;
pub mod light;
pub mod pacing;
pub mod pack;
pub mod photo;
pub mod protection;
pub mod raster;
pub mod sharc;
pub mod smooth;
pub mod tgp2a;
pub mod tgp2a_hle;
pub mod video;

pub mod recomp;
pub mod runahead;

pub use bus::Model2Bus;
use i960::cpu::{Cpu, Stop};
/// The i960 JIT (native builds; docs/jit.md), for the front ends' settings.
#[cfg(not(target_arch = "wasm32"))]
pub use jit;
use romset::RomSet;
use std::collections::BTreeSet;

/// Average cycles per instruction, for timing timers and vblank against
/// instruction count. The i960KB averages a little under this on game code.
pub const CYCLES_PER_INSN: u64 = 3;
pub(crate) const TGP_WORDS_PER_INSN: u64 = 2;

/// A recompiled i960 block: runs from its start address until it
/// transfers control, ends a frame or uses up `Model2Bus::budget`.
pub type Block = fn(&mut Cpu, &mut Model2Bus) -> Result<(), Stop>;

pub struct Machine {
    pub cpu: Cpu,
    pub bus: Model2Bus,
    /// Video frames completed (a copy of `bus.frames`).
    pub frames: u64,
    /// `program_hash` of the ROM set (save states carry it).
    program_hash: u64,
    /// Recompiled blocks by address / 4 over the program ROM, if this build
    /// has recompiled code for the game's ROM.
    native: Option<Vec<Option<Block>>>,
    /// The i960 JIT, when it is on (`enable_jit`, docs/jit.md): it takes
    /// the place of recompiled code.
    #[cfg(not(target_arch = "wasm32"))]
    jit: Option<Box<jit::Jit<Model2Bus>>>,
    /// Every instruction address executed.
    pub coverage: Coverage,
    /// Targets of indirect branches/calls actually taken, by source.
    pub indirect: BTreeSet<(u32, u32)>,
    /// Execution counts per IP over the most recent frame (hot-spot view).
    frame_profile: Profile,
    /// States loaded into it so far: a copy from it must start afresh
    /// after one (`runahead`).
    loads: u64,
    /// The game's idle loops and their skipping (`idle`).
    idle: idle::IdleSkip,
}

impl Machine {
    pub fn new(roms: &RomSet) -> Self {
        let mut bus = Model2Bus::new(roms);
        let mut cpu = Cpu::new();
        cpu.reset(&mut bus);
        let native = recomp::i960_blocks(program_hash(roms)).map(|blocks| {
            let mut table = vec![None; (PROGRAM_END / 4) as usize];
            // Only word-aligned starts can be dispatched; a block at an
            // odd address (from a build before `remodel recomp` dropped
            // them) would otherwise take its word's slot.
            for (a, f) in blocks.into_iter().filter(|&(a, _)| a & 3 == 0) {
                table[(a / 4) as usize] = Some(f);
            }
            table
        });
        Machine {
            cpu,
            bus,
            frames: 0,
            program_hash: program_hash(roms),
            native,
            #[cfg(not(target_arch = "wasm32"))]
            jit: None,
            coverage: Coverage::default(),
            indirect: BTreeSet::new(),
            frame_profile: Profile::default(),
            loads: 0,
            idle: idle::IdleSkip::new(roms.game),
        }
    }

    pub fn step(&mut self) -> Result<(), Stop> {
        // Interrupt entry happens inside `step`, so attribute bus accesses
        // to the instruction actually executed (approximate on IRQ entry).
        self.bus.cur_ip = self.cpu.ip;
        let (ip, flow) = self.cpu.step(&mut self.bus)?;
        self.coverage.insert(ip);
        self.frame_profile.count(ip);
        if matches!(flow, i960::Flow::IndirectJump | i960::Flow::IndirectCall | i960::Flow::IndirectLink) {
            self.indirect.insert((ip, self.cpu.ip));
        }

        if self.bus.retire_insn(&mut self.cpu) {
            self.frames = self.bus.frames;
            self.frame_profile.clear();
        }
        Ok(())
    }

    /// True if the i960 runs recompiled code (this build has it for the
    /// game's ROM and it hasn't been turned off) or JIT-translated code.
    pub fn is_native(&self) -> bool {
        self.native.is_some() || self.jit_on()
    }

    /// Run the interpreter only, even if recompiled code is available (and
    /// turn the JIT off).
    pub fn disable_native(&mut self) {
        self.native = None;
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.jit = None;
        }
    }

    /// True if the i960 runs on the JIT.
    pub fn jit_on(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.jit.is_some();
        #[cfg(target_arch = "wasm32")]
        false
    }

    /// Run the i960 on the JIT (docs/jit.md): its code translated to the
    /// host's machine code as it runs, in place of any recompiled code.
    /// Fails where Cranelift has no back end for the host.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn enable_jit(&mut self, config: jit::Config) -> Result<(), String> {
        let mut j = jit::Jit::new(config)?;
        j.set_stops(self.idle.starts());
        self.bus.count_code_writes();
        self.native = None;
        self.jit = Some(Box::new(j));
        self.bus.set_copro_jit(config.copro, config.copro_check)
    }

    /// The copro JITs' counters (docs/jit.md, "Copro JITs"), by chip, with
    /// the runs compared in a lockstep.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn copro_jit_stats(&self) -> Vec<(&'static str, jit::CoproStats, u64)> {
        self.bus.copro_jit_stats()
    }

    /// The JIT's counters, when it is on.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn jit_stats(&self) -> Option<jit::Stats> {
        self.jit.as_ref().map(|j| j.stats())
    }

    /// What translated blocks leave to the interpreter, by mnemonic.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn jit_fallbacks(&self) -> Vec<(&'static str, u64)> {
        self.jit.as_ref().map(|j| j.fallback_kinds()).unwrap_or_default()
    }

    /// Translate the whole program now, as the static recompiler cuts it
    /// (`insns` and its entry points, from `analyze`): a measurement of
    /// what translating everything ahead of time costs. Returns the number
    /// of blocks, or `None` with the JIT off.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn jit_translate_program(
        &mut self,
        insns: &std::collections::BTreeMap<u32, i960::Insn>,
        entries: &BTreeSet<u32>,
    ) -> Option<usize> {
        let bus = &self.bus;
        self.jit.as_mut().map(|j| j.translate_program(insns, entries, bus))
    }

    /// Run the SHARCs (Model 2B) on their interpreter, even if the build has
    /// recompiled code for their programs.
    pub fn disable_native_sharc(&mut self) {
        if let Some(c) = self.bus.sharc.as_deref_mut() {
            c.interpret = true;
            c.attach();
        }
        self.bus.geo.set_sharc_interpret(true);
    }

    /// Run up to `max` instructions, stopping after the one that ends a
    /// frame. Returns how many ran. Identical in effect to calling `step`
    /// that many times, but runs recompiled code where there is some, and
    /// skips through the game's idle loops (`idle`).
    pub fn run_slice(&mut self, max: u64) -> Result<u64, Stop> {
        let (start, frames) = (self.cpu.icount, self.frames);
        if self.native.is_none() && !self.jit_on() {
            while self.cpu.icount - start < max && self.frames == frames {
                if !self.idle_skip(max - (self.cpu.icount - start))? {
                    self.step()?;
                }
            }
            return Ok(self.cpu.icount - start);
        }
        while self.cpu.icount - start < max && self.frames == frames {
            if !self.idle_skip(max - (self.cpu.icount - start))? {
                self.dispatch(max - (self.cpu.icount - start))?;
            }
        }
        Ok(self.cpu.icount - start)
    }

    /// Run one recompiled block (or, where none starts, one interpreted
    /// instruction). Returns how many instructions ran.
    pub fn run_block(&mut self) -> Result<u64, Stop> {
        let start = self.cpu.icount;
        self.dispatch(u64::MAX)?;
        Ok(self.cpu.icount - start)
    }

    fn dispatch(&mut self, budget: u64) -> Result<(), Stop> {
        self.bus.budget = budget;
        self.cpu.take_interrupts(&mut self.bus);
        let ip = self.cpu.ip;
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(j) = self.jit.as_deref_mut() {
            match j.run(&mut self.cpu, &mut self.bus) {
                Some(r) => r?,
                None => {
                    self.bus.cur_ip = ip;
                    let (ip, _) = self.cpu.step_insn(&mut self.bus)?;
                    self.coverage.insert(ip);
                    self.bus.retire_insn(&mut self.cpu);
                }
            }
            self.frames = self.bus.frames;
            return Ok(());
        }
        let block = self.native.as_ref().and_then(|t| t.get((ip / 4) as usize).copied().flatten());
        match block {
            Some(block) if ip & 3 == 0 => block(&mut self.cpu, &mut self.bus)?,
            _ => {
                self.bus.cur_ip = ip;
                let (ip, _) = self.cpu.step_insn(&mut self.bus)?;
                self.coverage.insert(ip);
                self.bus.retire_insn(&mut self.cpu);
            }
        }
        self.frames = self.bus.frames;
        Ok(())
    }

    /// Run up to `max` instructions; returns the stop reason if any.
    pub fn run(&mut self, max: u64) -> Option<Stop> {
        for _ in 0..max {
            if let Err(stop) = self.step() {
                return Some(stop);
            }
        }
        None
    }

    /// The whole machine's state (see `remodel-state`): load it into a
    /// machine built from the same ROMs with [`Machine::load_state`].
    pub fn save_state(&mut self) -> Vec<u8> {
        use state::State as _;
        self.bus.geo.sync();
        let mut w = state::Writer::default();
        w.buf.extend_from_slice(format!("REMODEL-STATE-{STATE_FORMAT}\n").as_bytes());
        self.program_hash.save(&mut w);
        self.cpu.save(&mut w);
        self.bus.save(&mut w);
        w.buf
    }

    /// Restore a state from [`Machine::save_state`]. On error the machine
    /// may be partly loaded: build a new one.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        use state::State as _;
        self.loads += 1;
        let mut r = state::Reader::new(data);
        match split_header(data, STATE_PREFIX) {
            None => return Err("not a reModel save state".into()),
            Some((n, _)) if n > STATE_FORMAT => return Err(too_new("machine state", None, n, STATE_FORMAT)),
            Some((n, _)) if n < STATE_FORMAT => {
                return Err(format!("machine state format {n} is older than this build's {STATE_FORMAT}"))
            }
            Some(_) => r.bytes(STATE_PREFIX.len() + STATE_FORMAT.to_string().len() + 1).map_err(|e| e.to_string())?,
        };
        let mut hash = 0u64;
        hash.load(&mut r).map_err(|e| e.to_string())?;
        if hash != self.program_hash {
            return Err("save state is for a different ROM set".into());
        }
        self.cpu.load(&mut r).map_err(|e| e.to_string())?;
        self.bus.load(&mut r).map_err(|e| e.to_string())?;
        if !r.done() {
            return Err("save state has trailing data".into());
        }
        self.frames = self.bus.frames;
        self.forget_ram_code();
        Ok(())
    }

    /// Make this machine's state `src`'s, as `load_state(&src.save_state())`
    /// would, but by copying (see [`Model2Bus::copy_from`], whose `marks`
    /// this takes): run-ahead's fork of the real machine
    /// ([`runahead::RunAhead`]). Diagnostics (coverage, profiles) stay
    /// this machine's own. Both must be built from the same ROM set.
    pub fn copy_from(&mut self, src: &mut Machine, marks: &mut bus::CopyMarks) {
        use state::State as _;
        assert_eq!(self.program_hash, src.program_hash, "copy between machines of different games");
        let mut w = state::Writer { buf: std::mem::take(&mut marks.cpu) };
        w.buf.clear();
        src.cpu.save(&mut w);
        self.cpu.load(&mut state::Reader::new(&w.buf)).expect("a CPU's own state loads");
        marks.cpu = w.buf;
        self.bus.copy_from(&mut src.bus, marks);
        self.frames = src.frames;
        self.forget_ram_code();
    }

    /// RAM was replaced without the bus seeing writes: code the JIT
    /// translated from it must be checked against it again (each block
    /// compares its words on its next entry; run-ahead copies every frame,
    /// so dropping them all would mean translating them every frame).
    fn forget_ram_code(&mut self) {
        self.bus.code_written_everywhere();
    }

    /// Run the way `src` runs: the interpreter or recompiled code, and the
    /// geometrizer's settings ([`geo::Geometrizer::configure_like`]).
    pub fn configure_like(&mut self, src: &mut Machine) {
        if !src.is_native() {
            self.disable_native();
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let (Some(j), false) = (src.jit.as_deref(), self.jit_on()) {
            // Same settings; a JIT that worked there works here.
            let _ = self.enable_jit(j.config());
        }
        self.set_idle_skip(src.idle_skip_on());
        if let (Some(c), Some(s)) = (self.bus.sharc.as_deref_mut(), src.bus.sharc.as_deref()) {
            if c.interpret != s.interpret {
                c.interpret = s.interpret;
                c.attach();
            }
        }
        self.bus.geo.configure_like(&mut src.bus.geo);
    }

    /// How many times a state has been loaded into it (even partly).
    pub fn loads(&self) -> u64 {
        self.loads
    }

    /// Hottest instruction addresses in the current frame window.
    pub fn hot_spots(&self, n: usize) -> Vec<(u32, u64)> {
        let mut v = self.frame_profile.counts();
        v.sort_by_key(|&(a, c)| (std::cmp::Reverse(c), a));
        v.truncate(n);
        v
    }
}

/// Save file header: `REMODEL-SAVE-<format>\n`, then (format 2 on) a
/// `made-by <app version>\n` line, informational; the rest is zlib-compressed.
const SAVE_PREFIX: &[u8] = b"REMODEL-SAVE-";

/// The save file layout this build writes and the newest it reads. An
/// integer bumped when the layout changes, not the app's version. Format 1
/// (no `made-by` line) is still read; docs/usage.md, "Save states".
pub const SAVE_FORMAT: u32 = 2;

/// The inner machine state's layout (`REMODEL-STATE-<n>`), bumped when any
/// saved field is added, removed or reordered.
pub const STATE_FORMAT: u32 = 1;

static APP_VERSION: std::sync::OnceLock<&'static str> = std::sync::OnceLock::new();

/// The version string the front end was built as, recorded in the save
/// files it writes (for information; never read back into the machine).
pub fn set_app_version(v: &'static str) {
    let _ = APP_VERSION.set(v);
}

/// Split `<prefix><n>\n` off `data`: the format number and what follows.
fn split_header<'a>(data: &'a [u8], prefix: &[u8]) -> Option<(u32, &'a [u8])> {
    let rest = data.strip_prefix(prefix)?;
    let end = rest.iter().take(12).position(|&b| b == b'\n')?;
    let n = std::str::from_utf8(&rest[..end]).ok()?.parse::<u32>().ok().filter(|&n| n >= 1)?;
    Some((n, &rest[end + 1..]))
}

/// The error for a file of a format this build can't read.
fn too_new(what: &str, made_by: Option<&str>, format: u32, reads: u32) -> String {
    let by = made_by.map_or("a newer reModel".to_string(), |v| format!("reModel {v}"));
    format!("this {what} was made by {by} with format {format}; this build reads format {reads} (and older): update reModel")
}

/// A save file: the machine's state and, if given, the sound boards' (see
/// [`audio::Live`]), compressed (about 1 MB for topskatr).
pub fn save_session(machine: &mut Machine, live: Option<&audio::Live>) -> Vec<u8> {
    save_session_parts(machine, live.map(state::to_bytes))
}

/// [`save_session`] with the sound boards' state already saved (for boards
/// running on another thread).
pub fn save_session_parts(machine: &mut Machine, live: Option<Vec<u8>>) -> Vec<u8> {
    let m = machine.save_state();
    let mut raw = (m.len() as u64).to_le_bytes().to_vec();
    raw.extend(m);
    raw.extend(live.unwrap_or_default());
    let made_by = APP_VERSION.get().copied().unwrap_or("unknown").replace(['\n', '\r'], " ");
    let mut out = format!("REMODEL-SAVE-{SAVE_FORMAT}\nmade-by {made_by}\n").into_bytes();
    out.extend(png::zlib::compress(&raw));
    out
}

/// Load a file from [`save_session`]. Without `live`, any sound board state
/// in it is ignored; with `live` but none saved, the boards are left as
/// they are.
pub fn load_session(machine: &mut Machine, live: Option<&mut audio::Live>, data: &[u8]) -> Result<(), String> {
    let rest = load_session_parts(machine, data)?;
    match live {
        Some(l) if !rest.is_empty() => audio::load_state(l, &rest),
        _ => Ok(()),
    }
}

/// Load the machine from a [`save_session`] file; returns the sound
/// boards' state (empty if none was saved), for [`audio::load_state`].
pub fn load_session_parts(machine: &mut Machine, data: &[u8]) -> Result<Vec<u8>, String> {
    let (format, body) = split_header(data, SAVE_PREFIX).ok_or("not a reModel save file")?;
    let made_by = body.strip_prefix(b"made-by ").and_then(|b| {
        b.iter().take(80).position(|&c| c == b'\n').map(|e| String::from_utf8_lossy(&b[..e]).into_owned())
    });
    if format > SAVE_FORMAT {
        return Err(too_new("save state", made_by.as_deref(), format, SAVE_FORMAT));
    }
    // Format 1 has no `made-by` line.
    let body = if format >= 2 {
        let line = made_by.as_ref().ok_or("save file damaged: no `made-by` line")?;
        &body[b"made-by ".len() + line.len() + 1..]
    } else {
        body
    };
    let data = png::zlib::decompress(body).map_err(|e| format!("save file damaged: {e}"))?;
    let len = data.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize).ok_or("save file truncated")?;
    let m = data.get(8..8 + len).ok_or("save file truncated")?;
    machine.load_state(m)?;
    Ok(data[8 + len..].to_vec())
}

/// Machine state header: format name and number (`STATE_FORMAT`).
const STATE_PREFIX: &[u8] = b"REMODEL-STATE-";

/// Identifies the program ROM image, which recompiled i960 code is keyed by.
pub fn program_hash(roms: &RomSet) -> u64 {
    let p = &roms.program;
    let word = |a: u32| p.get(a as usize..a as usize + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    i960::codegen::image_hash(word, p.len() as u32)
}

/// End of the program ROM, where all recompilable code lives.
const PROGRAM_END: u32 = 0x20_0000;

/// Instruction addresses below this are tracked in flat arrays (the program
/// ROM and the RAM code is copied to); the rest fall back to a set.
const TRACKED_SPAN: u32 = 0x100_0000;

/// Set of executed instruction addresses, cheap enough to update on every
/// instruction.
#[derive(Default)]
pub struct Coverage {
    bits: Vec<u64>,
    other: BTreeSet<u32>,
    len: usize,
}

impl Coverage {
    pub fn insert(&mut self, ip: u32) {
        if ip >= TRACKED_SPAN {
            if self.other.insert(ip) {
                self.len += 1;
            }
            return;
        }
        if self.bits.is_empty() {
            self.bits = vec![0; (TRACKED_SPAN / 4 / 64) as usize];
        }
        let (w, b) = ((ip / 4 / 64) as usize, (ip / 4) % 64);
        if self.bits[w] & (1 << b) == 0 {
            self.bits[w] |= 1 << b;
            self.len += 1;
        }
    }

    pub fn len(&self) -> usize {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Addresses in ascending order.
    pub fn iter(&self) -> impl Iterator<Item = u32> + '_ {
        let low =
            self.bits.iter().enumerate().flat_map(|(w, &bits)| {
                (0..64).filter(move |b| bits & (1 << b) != 0).map(move |b| (w as u32 * 64 + b) * 4)
            });
        low.chain(self.other.iter().copied())
    }
}

/// Per-IP execution counts for one frame, cleared at each vblank.
#[derive(Default)]
struct Profile {
    counts: Vec<u32>,
    touched: Vec<u32>,
}

impl Profile {
    fn count(&mut self, ip: u32) {
        if ip >= TRACKED_SPAN {
            return;
        }
        if self.counts.is_empty() {
            self.counts = vec![0; (TRACKED_SPAN / 4) as usize];
        }
        let c = &mut self.counts[(ip / 4) as usize];
        if *c == 0 {
            self.touched.push(ip);
        }
        *c += 1;
    }

    /// `n` more executions of `ip`, already counted once.
    fn add(&mut self, ip: u32, n: u64) {
        if let Some(c) = self.counts.get_mut((ip / 4) as usize) {
            *c = c.saturating_add(n.min(u32::MAX as u64) as u32);
        }
    }

    fn counts(&self) -> Vec<(u32, u64)> {
        self.touched.iter().map(|&ip| (ip, self.counts[(ip / 4) as usize] as u64)).collect()
    }

    fn clear(&mut self) {
        for ip in self.touched.drain(..) {
            self.counts[(ip / 4) as usize] = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coverage_counts_each_address_once_in_order() {
        let mut c = Coverage::default();
        for ip in [0x48, 0x40, 0x48, 0x2000_0000, 0x40] {
            c.insert(ip);
        }
        assert_eq!(c.len(), 3);
        assert_eq!(c.iter().collect::<Vec<_>>(), [0x40, 0x48, 0x2000_0000]);
    }

    #[test]
    fn save_headers_name_their_format() {
        assert_eq!(split_header(b"REMODEL-SAVE-1\nxyz", SAVE_PREFIX), Some((1, &b"xyz"[..])));
        assert_eq!(split_header(b"REMODEL-SAVE-12\n", SAVE_PREFIX), Some((12, &b""[..])));
        for bad in [&b"REMODEL-SAVE-\n"[..], b"REMODEL-SAVE-0\n", b"REMODEL-SAVE-x\n", b"PK\x03\x04", b"REMODEL-SAVE-1"]
        {
            assert_eq!(split_header(bad, SAVE_PREFIX), None);
        }
        let e = too_new("save state", Some("v9.0.0"), 7, SAVE_FORMAT);
        assert!(e.contains("made by reModel v9.0.0 with format 7") && e.contains("reads format 2"), "{e}");
    }
}
