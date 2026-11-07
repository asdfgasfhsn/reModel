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
pub mod input;
pub mod protection;
pub mod raster;
pub mod sharc;
pub mod smooth;
pub mod video;

pub mod recomp;

pub use bus::Model2Bus;
use i960::cpu::{Cpu, Stop};
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
    /// Every instruction address executed.
    pub coverage: Coverage,
    /// Targets of indirect branches/calls actually taken, by source.
    pub indirect: BTreeSet<(u32, u32)>,
    /// Execution counts per IP over the most recent frame (hot-spot view).
    frame_profile: Profile,
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
            coverage: Coverage::default(),
            indirect: BTreeSet::new(),
            frame_profile: Profile::default(),
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
    /// game's ROM and it hasn't been turned off).
    pub fn is_native(&self) -> bool {
        self.native.is_some()
    }

    /// Run the interpreter only, even if recompiled code is available.
    pub fn disable_native(&mut self) {
        self.native = None;
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
    /// that many times, but runs recompiled code where there is some.
    pub fn run_slice(&mut self, max: u64) -> Result<u64, Stop> {
        let (start, frames) = (self.cpu.icount, self.frames);
        if self.native.is_none() {
            while self.cpu.icount - start < max && self.frames == frames {
                self.step()?;
            }
            return Ok(self.cpu.icount - start);
        }
        while self.cpu.icount - start < max && self.frames == frames {
            self.dispatch(max - (self.cpu.icount - start))?;
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
        w.buf.extend_from_slice(STATE_MAGIC);
        self.program_hash.save(&mut w);
        self.cpu.save(&mut w);
        self.bus.save(&mut w);
        w.buf
    }

    /// Restore a state from [`Machine::save_state`]. On error the machine
    /// may be partly loaded: build a new one.
    pub fn load_state(&mut self, data: &[u8]) -> Result<(), String> {
        use state::State as _;
        let mut r = state::Reader::new(data);
        if r.bytes(STATE_MAGIC.len()).ok() != Some(STATE_MAGIC) {
            return Err("not a Remodel save state (or from another version)".into());
        }
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
        Ok(())
    }

    /// Hottest instruction addresses in the current frame window.
    pub fn hot_spots(&self, n: usize) -> Vec<(u32, u64)> {
        let mut v = self.frame_profile.counts();
        v.sort_by_key(|&(a, c)| (std::cmp::Reverse(c), a));
        v.truncate(n);
        v
    }
}

/// Save file header (the rest is zlib-compressed).
const SAVE_MAGIC: &[u8] = b"REMODEL-SAVE-1\n";

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
    let mut out = SAVE_MAGIC.to_vec();
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
    let body = data.strip_prefix(SAVE_MAGIC).ok_or("not a Remodel save file (or from another version)")?;
    let data = png::zlib::decompress(body).map_err(|e| format!("save file damaged: {e}"))?;
    let len = data.get(..8).map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize).ok_or("save file truncated")?;
    let m = data.get(8..8 + len).ok_or("save file truncated")?;
    machine.load_state(m)?;
    Ok(data[8 + len..].to_vec())
}

/// Save state header: format name and version.
const STATE_MAGIC: &[u8] = b"REMODEL-STATE-1\n";

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
}
