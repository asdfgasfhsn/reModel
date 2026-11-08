//! Run-ahead: show the game a few frames ahead of where it is, to take
//! away that much of its own input lag (docs/play.md, "Run-ahead").
//!
//! After each real frame, a second machine (the *shadow*) takes the real
//! one's state and runs `frames` more frames with the same inputs; the
//! front end draws the shadow's last frame. A game that shows an input
//! `L` frames after reading it then shows it `L - frames` frames after,
//! as long as `frames` isn't more than `L` (beyond that, what is shown
//! jumps back whenever an input changes the guessed future).
//!
//! The real machine never runs a speculative frame, so nothing has to be
//! restored: its timeline, state hash, sound, EEPROM and save states are
//! exactly as without run-ahead, whenever a front end looks at them. The
//! shadow's own side effects go nowhere: its sound commands are dropped
//! after each frame, and nothing else (EEPROM, drive board, coverage)
//! leaves it, since a front end only draws it. The cost is a copy of the
//! real machine's state per shown frame ([`crate::Machine::copy_from`],
//! memory neither machine wrote since the last copy skipped) and the
//! extra frames, plus a second machine's memory.
//!
//! Smooth motion's in-betweens blend consecutive *shown* frames: the
//! shadow records only the last frame of each run
//! ([`crate::geo::Geometrizer::set_smooth_shown`]), and keeps that record
//! across copies, so its next shown frame blends with it. The real
//! machine's smooth motion is wasted while this runs: a front end can turn
//! it off there.

use crate::bus::CopyMarks;
use crate::geo::Stopwatch;
use crate::Machine;
use i960::cpu::Stop;
use romset::RomSet;
use std::time::Duration;

/// Most frames run ahead.
pub const MAX_FRAMES: u32 = 8;

pub struct RunAhead {
    shadow: Box<Machine>,
    frames: u32,
    marks: CopyMarks,
    /// The real machine's frame count and loads the shown frame was run
    /// ahead from.
    from: Option<(u64, u64)>,
    /// The real machine's loads when `marks` started.
    marks_loads: u64,
    pub stats: Stats,
}

/// What run-ahead has cost so far: forks of the real machine, and host
/// time waiting for the geometrizers' frames in progress (the real
/// machine's, whose state the copy needs: on its own thread it would
/// otherwise run alongside the next frame), copying the state and running
/// the frames ahead (the shadow's geometrizer work on its own thread not
/// counted).
#[derive(Clone, Copy, Debug, Default)]
pub struct Stats {
    pub forks: u64,
    pub waiting: Duration,
    pub copying: Duration,
    pub running: Duration,
    /// Memory bytes the last copy moved.
    pub copied: usize,
}

impl RunAhead {
    /// Run-ahead of `frames` (1 to [`MAX_FRAMES`]) for `real`, built from
    /// `roms`, running the way it does ([`Machine::configure_like`]).
    pub fn new(roms: &RomSet, real: &mut Machine, frames: u32) -> Self {
        let mut shadow = Box::new(Machine::new(roms));
        shadow.configure_like(real);
        RunAhead {
            shadow,
            frames: frames.clamp(1, MAX_FRAMES),
            marks: CopyMarks::default(),
            from: None,
            marks_loads: real.loads(),
            stats: Stats::default(),
        }
    }

    pub fn frames(&self) -> u32 {
        self.frames
    }

    pub fn set_frames(&mut self, frames: u32) {
        self.frames = frames.clamp(1, MAX_FRAMES);
    }

    /// Smooth motion for the shown frames (the shadow's geometrizer).
    pub fn set_smooth(&mut self, on: bool) {
        self.shadow.bus.geo.set_smooth(on);
    }

    /// After a real frame: fork `real` and run the frames ahead, with the
    /// inputs it has now. A stop in them is returned (the real machine
    /// will meet it too, that many frames later).
    pub fn run(&mut self, real: &mut Machine) -> Result<(), Stop> {
        let t = Stopwatch::start();
        real.bus.geo.sync();
        self.shadow.bus.geo.sync();
        let waited = t.elapsed();
        // What the last copy skipped may differ after a load into `real`.
        if self.marks_loads != real.loads() {
            self.marks = CopyMarks::default();
            self.marks_loads = real.loads();
        }
        self.from = None;
        self.shadow.copy_from(real, &mut self.marks);
        let copied = t.elapsed();
        for i in 1..=self.frames {
            self.shadow.bus.geo.set_smooth_shown(i == self.frames);
            let r = self.shadow.run_slice(u64::MAX);
            // Speculative sound never plays.
            self.shadow.bus.uart_log.clear();
            r?;
        }
        self.from = Some((real.frames, real.loads()));
        let s = &mut self.stats;
        s.forks += 1;
        s.waiting += waited;
        s.copying += copied - waited;
        s.running += t.elapsed() - copied;
        s.copied = self.marks.copied;
        Ok(())
    }

    /// True if the shadow holds frames run ahead from `real` as it is now
    /// (after [`RunAhead::run`], before `real` runs again or loads).
    pub fn is_ahead_of(&self, real: &Machine) -> bool {
        self.from == Some((real.frames, real.loads()))
    }

    /// The machine to draw: the shadow when it is ahead of `real`, else
    /// `real` itself. For drawing only: anything that saves, loads or runs
    /// takes `real`.
    pub fn shown<'a>(&'a mut self, real: &'a mut Machine) -> &'a mut Machine {
        if self.is_ahead_of(real) {
            &mut self.shadow
        } else {
            real
        }
    }

    /// The shadow, as last run (tests and checks).
    pub fn shadow(&mut self) -> &mut Machine {
        &mut self.shadow
    }
}
