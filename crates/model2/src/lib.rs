//! Sega Model 2 board model, driving the reference i960 interpreter.
//!
//! This is the "slow path": it runs the original program code so its
//! behaviour can be observed (which code runs, what hardware it touches)
//! and later used as the oracle for the recompiled build.

pub mod bus;
pub mod eeprom;
pub mod geo;
pub mod input;
pub mod raster;
pub mod video;

pub use bus::Model2Bus;
use bus::{FRAME_CYCLES, IRQ_VBLANK};
use i960::cpu::{Cpu, Stop};
use romset::RomSet;
use std::collections::BTreeSet;

/// Average cycles per instruction, for timing timers and vblank against
/// instruction count. The i960KB averages a little under this on game code.
pub const CYCLES_PER_INSN: u64 = 3;
const TGP_WORDS_PER_INSN: u64 = 2;

pub struct Machine {
    pub cpu: Cpu,
    pub bus: Model2Bus,
    pub frames: u64,
    next_vblank: u64,
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
        Machine {
            cpu,
            bus,
            frames: 0,
            next_vblank: FRAME_CYCLES,
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

        // The TGP runs at 20 MHz against the i960's 25 MHz: about two of its
        // words per i960 instruction at our cycles-per-instruction estimate.
        self.bus.run_tgp(TGP_WORDS_PER_INSN);
        self.bus.now += CYCLES_PER_INSN;
        self.bus.tick_timers();
        if self.bus.now >= self.next_vblank {
            self.next_vblank += FRAME_CYCLES;
            self.frames += 1;
            self.bus.frame_toggle = !self.bus.frame_toggle;
            self.bus.run_geometrizer();
            self.bus.request_irq(IRQ_VBLANK);
            self.frame_profile.clear();
        }
        for line in 0..4 {
            if std::mem::take(&mut self.bus.raised_lines[line]) {
                self.cpu.raise_line(line as u32);
            }
        }
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

    /// Hottest instruction addresses in the current frame window.
    pub fn hot_spots(&self, n: usize) -> Vec<(u32, u64)> {
        let mut v = self.frame_profile.counts();
        v.sort_by_key(|&(a, c)| (std::cmp::Reverse(c), a));
        v.truncate(n);
        v
    }
}

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
