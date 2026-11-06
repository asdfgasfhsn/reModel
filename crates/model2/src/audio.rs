//! Live audio: the sound board (68000 + SCSP) and, on games that have one,
//! the DSB2 or DSB1 music board, run alongside the main board and fed the bytes the
//! game sends on the sound link as it sends them.
//!
//! Each byte reaches the boards at the output sample matching the CPU cycle
//! it was written at, so timing matches the offline replays (`remodel
//! sound-render`, `music-render`). The two outputs are mixed at adjustable
//! gains: on the cabinet they meet in the analog amplifier at levels the
//! ROMs don't tell us (docs/hw/sound.md 3.6), so the default balance is a
//! listening choice.

use crate::bus::{Model2Bus, CPU_HZ};
use romset::{Role, RomSet};

/// Output sample rate.
pub const RATE: u32 = 44_100;

/// Default mix, in dB: sound board (effects, announcer) and music board.
/// Summed at unity, the music averaged about 18 dB above the sound board
/// in a race and buried the announcer. +8 / -8 was judged right by ear on
/// the native build. Unconfirmed against a cabinet: a listening choice.
pub const DEFAULT_MIX: (f32, f32) = (8.0, -8.0);

/// A music board: the DSB2 (68000, `topskatr`) or the DSB1 (Z80, `stcc`).
/// Both take the sound link's bytes and give 44.1 kHz stereo.
enum Music {
    Dsb2(dsb::Dsb2),
    Dsb1(dsb::Dsb1),
}

impl Music {
    fn midi_in(&mut self, byte: u8) {
        match self {
            Music::Dsb2(m) => m.midi_in(byte),
            Music::Dsb1(m) => m.midi_in(byte),
        }
    }

    fn run(&mut self, samples: usize, out: &mut Vec<i16>) {
        match self {
            Music::Dsb2(m) => m.run(samples, out),
            Music::Dsb1(m) => m.run(samples, out),
        }
    }

    fn set_soundtrack(&mut self, st: dsb::Soundtrack) {
        match self {
            Music::Dsb2(m) => m.set_soundtrack(st),
            Music::Dsb1(m) => m.set_soundtrack(st),
        }
    }
}

impl state::State for Music {
    fn save(&self, w: &mut state::Writer) {
        match self {
            Music::Dsb2(m) => m.save(w),
            Music::Dsb1(m) => m.save(w),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        match self {
            Music::Dsb2(m) => m.load(r),
            Music::Dsb1(m) => m.load(r),
        }
    }
}

pub struct Live {
    sound: Option<sound::SoundBoard>,
    music: Option<Music>,
    /// Output samples produced so far (per channel).
    samples: u64,
    a: Vec<i16>,
    b: Vec<i16>,
    /// Linear gains for the sound board and the music board.
    gains: (f32, f32),
    /// The game's level (`Game::volume_db`), added to both.
    volume_db: f32,
}

fn gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

impl Live {
    /// The boards `roms` has: none, the sound board, or the sound board and
    /// a music board (DSB2 or DSB1).
    pub fn new(roms: &RomSet) -> Self {
        let prog = roms.region16be(Role::AudioCpu);
        let sound = (!prog.is_empty()).then(|| sound::SoundBoard::new(prog, roms.region16be(Role::Samples)));
        let mpeg = || -> Vec<u8> {
            roms.game
                .files
                .iter()
                .zip(&roms.chips)
                .filter(|(f, _)| f.role == Role::MpegData)
                .flat_map(|(_, c)| c.iter().copied())
                .collect()
        };
        let chip = |role| roms.game.files.iter().position(|f| f.role == role).map(|i| &roms.chips[i]);
        let music = if let Some(prog) = chip(Role::MpegCpu) {
            let mut d = dsb::Dsb2::new(prog, mpeg());
            d.set_diagnostics(false);
            Some(Music::Dsb2(d))
        } else if let Some(prog) = chip(Role::Dsb1Cpu) {
            let mut d = dsb::Dsb1::new(prog, mpeg());
            d.set_diagnostics(false);
            Some(Music::Dsb1(d))
        } else {
            None
        };
        let volume_db = roms.game.volume_db;
        let gains = (gain(DEFAULT_MIX.0 + volume_db), gain(DEFAULT_MIX.1 + volume_db));
        Live { sound, music, samples: 0, a: Vec::new(), b: Vec::new(), gains, volume_db }
    }

    /// Set the mix: sound board and music board gains in dB (the game's
    /// own level is added to both).
    pub fn set_mix(&mut self, sound_db: f32, music_db: f32) {
        self.gains = (gain(sound_db + self.volume_db), gain(music_db + self.volume_db));
    }

    /// Replace music tracks with the files of a custom soundtrack.
    pub fn set_soundtrack(&mut self, st: dsb::Soundtrack) {
        if let Some(m) = &mut self.music {
            m.set_soundtrack(st);
        }
    }

    /// Output samples rendered so far (at [`RATE`]), since boot.
    pub fn samples(&self) -> u64 {
        self.samples
    }

    pub fn has_sound(&self) -> bool {
        self.sound.is_some() || self.music.is_some()
    }

    /// Render up to the bus's current time, delivering the sound link bytes
    /// written since the last call (taken from `bus.uart_log`). Appends
    /// interleaved stereo i16 at [`RATE`] to `out`.
    pub fn run(&mut self, bus: &mut Model2Bus, out: &mut Vec<i16>) {
        let link = std::mem::take(&mut bus.uart_log);
        self.feed(&link, bus.now, out);
    }

    /// `run` for boards that live apart from the bus (a browser worker, a
    /// replay): `link` is the sound link log since the last call, as in
    /// `Model2Bus::uart_log` (cycle, control register, byte), and `until`
    /// the bus time to render up to.
    pub fn feed(&mut self, link: &[(u64, bool, u8)], until: u64, out: &mut Vec<i16>) {
        for &(t, control, byte) in link {
            if !control {
                self.render_to(t, out);
                if let Some(s) = &mut self.sound {
                    s.midi_in(byte);
                }
                if let Some(m) = &mut self.music {
                    m.midi_in(byte);
                }
            }
        }
        self.render_to(until, out);
    }

    fn render_to(&mut self, cycle: u64, out: &mut Vec<i16>) {
        let target = (cycle as u128 * RATE as u128 / CPU_HZ as u128) as u64;
        let n = target.saturating_sub(self.samples) as usize;
        if n == 0 {
            return;
        }
        self.samples = target;
        self.a.clear();
        self.b.clear();
        match &mut self.sound {
            Some(s) => s.run(n, &mut self.a),
            None => self.a.resize(n * 2, 0),
        }
        match &mut self.music {
            Some(m) => m.run(n, &mut self.b),
            None => self.b.resize(n * 2, 0),
        }
        let (ga, gb) = self.gains;
        out.extend(
            self.a
                .iter()
                .zip(&self.b)
                .map(|(&x, &y)| (x as f32 * ga + y as f32 * gb).round().clamp(-32768.0, 32767.0) as i16),
        );
    }
}

/// Linear resampler for the live mix: interleaved stereo i16 at [`RATE`]
/// in, f32 out, `step` input samples per output sample. A step above 1
/// plays faster (fewer samples out) and raises the pitch: front ends use it
/// to match the game's speed to the display, and to correct clock drift.
#[derive(Default)]
pub struct Resampler {
    pos: f64,
    prev: [f32; 2],
}

impl Resampler {
    pub fn run(&mut self, input: &[i16], step: f64, out: &mut Vec<f32>) {
        for frame in input.as_chunks::<2>().0 {
            let cur = [frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0];
            while self.pos < 1.0 {
                let t = self.pos as f32;
                let p = self.prev;
                out.extend([p[0] + (cur[0] - p[0]) * t, p[1] + (cur[1] - p[1]) * t]);
                self.pos += step;
            }
            self.pos -= 1.0;
            self.prev = cur;
        }
    }
}

/// The audio rate control for a front end: the factor by which to scale
/// the resampler's step (game samples per output sample) on top of the
/// speed the game is meant to run at, so the output queue neither runs
/// dry nor overflows when the game can't keep that speed.
///
/// Two parts. The game's measured pace: frames it ran against frames it
/// should have run, each a rate through two 150 ms low-pass stages. It is
/// exactly 1 while the game keeps up, and follows the game down when it
/// can't (a geometrizer slower than real time), so the audio then plays
/// in slow motion, lower in pitch, instead of breaking up. And a
/// correction from the queue's level (low-passed over 150 ms): at most
/// 0.5% near the target, which is the drift between the clocks, and more
/// once the level is more than 20% off (up to 20% slower when it runs
/// low, 3% faster when it piles up), which catches what the pace misses
/// while it settles. Modelled on sgt24h's measured frame costs in
/// WebAssembly (docs/web.md): 61 dropouts a minute in a race before, 1
/// after.
pub struct Pace {
    /// Frames run and frames due per second, after one and two low-pass
    /// stages.
    ran: [f64; 2],
    due: [f64; 2],
    /// The queue level (seconds), low-passed; None without a report.
    level: Option<f64>,
}

impl Default for Pace {
    fn default() -> Self {
        Pace { ran: [60.0; 2], due: [60.0; 2], level: None }
    }
}

impl Pace {
    /// Time constant of the pace's two stages, and the level's (seconds).
    const PACE_TAU: f64 = 0.15;
    const LEVEL_TAU: f64 = 0.15;
    /// Most correction near the target level (the clock drift).
    const DRIFT: f64 = 0.005;
    /// Beyond this fraction off the target, correct harder: up to
    /// `SLOWER` at an empty queue, `FASTER` at twice the target.
    const DEADBAND: f64 = 0.2;
    const SLOWER: f64 = 0.2;
    const FASTER: f64 = 0.03;

    /// After `dt` seconds in which the game ran `ran` frames of the `due`
    /// it should have: the factor for the resampler's step. `queued`: the
    /// audio queued now (seconds), if known; `target`: what it should be.
    pub fn update(&mut self, dt: f64, ran: f64, due: f64, queued: Option<f64>, target: f64) -> f64 {
        // Past a long gap (a hidden tab, a stall) the pace stays as it was.
        let gap = dt > 0.1;
        let dt = dt.clamp(0.001, 0.1);
        let a = (dt / Self::PACE_TAU).min(1.0);
        if !gap {
            for (s, n) in [(&mut self.ran, ran), (&mut self.due, due)] {
                s[0] += (n / dt - s[0]) * a;
                s[1] += (s[0] - s[1]) * a;
            }
        }
        let pace = if self.due[1] > 0.0 { (self.ran[1] / self.due[1]).clamp(0.25, 1.5) } else { 1.0 };
        self.level = queued.map(|q| match self.level {
            Some(l) => l + (q - l) * (dt / Self::LEVEL_TAU).min(1.0),
            None => q,
        });
        let Some(level) = self.level else { return pace };
        let e = ((level - target) / target).clamp(-1.0, 1.0);
        let beyond = (e.abs() - Self::DEADBAND).max(0.0) / (1.0 - Self::DEADBAND);
        let extra = if e < 0.0 { -beyond * Self::SLOWER } else { beyond * Self::FASTER };
        pace * (1.0 + e * Self::DRIFT + extra)
    }
}

// Save states: the boards and the sample count; the mix is configuration.
impl state::State for Live {
    fn save(&self, w: &mut state::Writer) {
        self.samples.save(w);
        self.sound.is_some().save(w);
        if let Some(s) = &self.sound {
            s.save(w);
        }
        self.music.is_some().save(w);
        if let Some(m) = &self.music {
            m.save(w);
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.samples.load(r)?;
        let mismatch = || state::Error("save state is for a different set of sound boards".into());
        let mut saved = false;
        saved.load(r)?;
        match (&mut self.sound, saved) {
            (Some(s), true) => s.load(r)?,
            (None, false) => {}
            _ => return Err(mismatch()),
        }
        saved.load(r)?;
        match (&mut self.music, saved) {
            (Some(m), true) => m.load(r)?,
            (None, false) => {}
            _ => return Err(mismatch()),
        }
        Ok(())
    }
}

/// Load sound board state saved with `state::to_bytes(&live)`.
pub fn load_state(live: &mut Live, bytes: &[u8]) -> Result<(), String> {
    let mut r = state::Reader::new(bytes);
    state::State::load(live, &mut r).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampler_rates() {
        let input: Vec<i16> = (0..2000).flat_map(|i| [i as i16, -(i as i16)]).collect();
        for (step, expect) in [(1.0, 2000), (0.5, 4000), (1.25, 1600)] {
            let mut out = Vec::new();
            Resampler::default().run(&input, step, &mut out);
            assert!((out.len() as i64 / 2 - expect).abs() <= 1, "step {step}: {} frames", out.len() / 2);
        }
    }

    /// A 60 Hz display, one game frame per refresh while the game keeps
    /// up, `speed(second)` of them when it can't: the queue's lowest
    /// level after the first second, and the last factor.
    fn pace_run(seconds: f64, speed: impl Fn(f64) -> f64) -> (f64, f64) {
        let (dt, frame) = (1.0 / 60.0, 1.0 / 57.524);
        let step = 60.0 / 57.524;
        let mut pace = Pace::default();
        let (mut queued, mut lowest, mut credit, mut factor) = (0.07, f64::MAX, 0.0, 1.0);
        let mut t = 0.0;
        while t < seconds {
            credit += speed(t);
            let ran = if credit >= 1.0 { 1.0 } else { 0.0 };
            credit -= ran;
            queued += ran * frame / (step * factor) - dt;
            if t > 1.0 {
                lowest = lowest.min(queued);
            }
            factor = pace.update(dt, ran, 1.0, Some(queued.max(0.0)), 0.07);
            t += dt;
        }
        (lowest, factor)
    }

    #[test]
    fn pace_keeps_audio_flowing() {
        // Keeping up: no change beyond the drift correction.
        let (lowest, factor) = pace_run(10.0, |_| 1.0);
        assert!((factor - 1.0).abs() < 0.001 && lowest > 0.05, "steady: {factor} {lowest}");
        // Slowing to 80% and then 60% of the display's rate: the audio
        // slows to match and never runs dry.
        let (lowest, factor) = pace_run(20.0, |t| {
            if t < 3.0 {
                1.0
            } else if t < 10.0 {
                0.8
            } else {
                0.6
            }
        });
        assert!(lowest > 0.0, "ran dry: {lowest}");
        assert!((factor - 0.6).abs() < 0.03, "factor {factor}");
        // And back to full speed without piling up.
        let (_, factor) = pace_run(20.0, |t| if t < 10.0 { 0.7 } else { 1.0 });
        assert!((factor - 1.0).abs() < 0.01, "recovered: {factor}");
    }
}
