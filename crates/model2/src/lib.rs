//! Sega Model 2 board model, driving the reference i960 interpreter.
//!
//! This is the "slow path": it runs the original program code so its
//! behaviour can be observed (which code runs, what hardware it touches)
//! and later used as the oracle for the recompiled build.

pub mod bus;
pub mod eeprom;

pub use bus::Model2Bus;
use bus::{FRAME_CYCLES, IRQ_VBLANK};
use i960::cpu::{Cpu, Stop};
use romset::RomSet;
use std::collections::{BTreeSet, HashMap};

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
    pub coverage: BTreeSet<u32>,
    /// Targets of indirect branches/calls actually taken, by source.
    pub indirect: BTreeSet<(u32, u32)>,
    /// Execution counts per IP over the most recent frame (hot-spot view).
    pub frame_profile: HashMap<u32, u64>,
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
            coverage: BTreeSet::new(),
            indirect: BTreeSet::new(),
            frame_profile: HashMap::new(),
        }
    }

    pub fn step(&mut self) -> Result<(), Stop> {
        // Interrupt entry happens inside `step`, so attribute bus accesses
        // to the instruction actually executed (approximate on IRQ entry).
        self.bus.cur_ip = self.cpu.ip;
        let (ip, flow) = self.cpu.step(&mut self.bus)?;
        self.coverage.insert(ip);
        *self.frame_profile.entry(ip).or_default() += 1;
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
        let mut v: Vec<(u32, u64)> = self.frame_profile.iter().map(|(&a, &c)| (a, c)).collect();
        v.sort_by_key(|&(a, c)| (std::cmp::Reverse(c), a));
        v.truncate(n);
        v
    }
}
