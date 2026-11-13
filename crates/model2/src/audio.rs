//! Live audio: the sound board (68000 + SCSP, or on the original Model 2
//! the 68000 with MultiPCM and FM chips) and, on games that have one,
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

/// The sound board: the SCSP one (Model 2A/2B/2C) or the original Model
/// 2's (MultiPCMs and an OPN2). Both take the sound link's bytes and give
/// 44.1 kHz stereo.
enum Sound {
    Scsp(Box<sound::SoundBoard>),
    Pcm(Box<sound::PcmBoard>),
}

impl Sound {
    fn midi_in(&mut self, byte: u8) {
        match self {
            Sound::Scsp(s) => s.midi_in(byte),
            Sound::Pcm(s) => s.midi_in(byte),
        }
    }

    fn run(&mut self, samples: usize, out: &mut Vec<i16>) {
        match self {
            Sound::Scsp(s) => s.run(samples, out),
            Sound::Pcm(s) => s.run(samples, out),
        }
    }
}

impl state::State for Sound {
    fn save(&self, w: &mut state::Writer) {
        match self {
            Sound::Scsp(s) => s.save(w),
            Sound::Pcm(s) => s.save(w),
        }
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        match self {
            Sound::Scsp(s) => s.load(r),
            Sound::Pcm(s) => s.load(r),
        }
    }
}

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
    sound: Option<Sound>,
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
        // The original Model 2's sound board is another design: a 68000
        // with MultiPCM and FM chips (docs/hw/model2.md §5).
        let prog = roms.region16be(Role::AudioCpu);
        let sound = (!prog.is_empty()).then(|| {
            if roms.game.board == romset::Board::Model2 {
                let (a, b) = (roms.multipcm(Role::MultiPcmA), roms.multipcm(Role::MultiPcmB));
                Sound::Pcm(Box::new(sound::PcmBoard::new(prog, a, b)))
            } else {
                Sound::Scsp(Box::new(sound::SoundBoard::new(prog, roms.region16be(Role::Samples))))
            }
        });
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

/// The audio rate control for a front end: the resampler's step (game
/// samples per output sample) that keeps the output queue from running
/// dry or piling up, without bending the pitch audibly while the game
/// keeps its speed (docs/web.md, "Audio rate").
///
/// The step is the speed the game is meant to run at (1 on the clock,
/// 60 / 57.524 following a 60 Hz display), times its pace once it is
/// persistently behind, times a small correction from the queue's level,
/// and it glides rather than jumps:
///
/// - Pace: frames run over frames due, each a rate through two low-pass
///   stages, at most 1, so catching up never raises the pitch. The audio
///   follows it (slow motion, lower in pitch, instead of breaking up) only
///   once it has been below `ON_PACE` for `HOLD`, and stops once it has
///   been back for as long; a stall the front end catches up from, or
///   jitter, leaves the pitch alone and the queue absorbs it. The factor
///   from it is smoothed over `SMOOTH`.
/// - Queue: the level, low-passed over `LEVEL_TAU`, against the target,
///   corrects by at most `QUEUE` either way, which is inaudible and enough
///   for the drift between the clocks. Anything more is a gap in the audio,
///   which the output re-primes from, rather than a bend in its pitch.
/// - Glide: the step moves at most `SLEW` plus `FOLLOW` times the way it
///   has to go each second: noise of a few cents moves it at most about
///   35 cents a second, a real slowdown within a fraction of a second.
///
/// The pace's time constants are multiples of the queue's target, the
/// slack there is to ride out a stall with: a phone's larger queue buys
/// more patience. The speed should be steady: a front end following a
/// display passes its locked refresh rate (`pacing::Refresh`), not a
/// jittery average. `make pace-sim` (crates/model2/tests/pace_sim.rs)
/// models a browser page through it; the numbers are in docs/web.md.
#[derive(Default)]
pub struct Pace {
    /// Frames due and frames run per second, after one and two low-pass
    /// stages; None before the first update.
    rates: Option<[[f64; 2]; 2]>,
    /// Following the game's pace, and how long (seconds, net) the pace has
    /// said otherwise since this was last decided.
    behind: bool,
    against: f64,
    /// The factor from the pace, smoothed (0 before the first update).
    follow: f64,
    /// The queue level (seconds), low-passed; None without a report.
    level: Option<f64>,
    /// The step last returned (0 before the first).
    step: f64,
}

impl Pace {
    /// In multiples of the queue's target: the time constant of the pace's
    /// two stages, how long it must say the game is behind before the
    /// audio follows (or keeping up before it stops), and the time
    /// constant the factor from it is smoothed with.
    pub const PACE_TAU: f64 = 2.0;
    pub const HOLD: f64 = 3.0;
    pub const SMOOTH: f64 = 3.0;
    /// Pace below which the game is behind.
    pub const ON_PACE: f64 = 0.97;
    /// The queue level's time constant (seconds), and the most correction
    /// from it either way.
    pub const LEVEL_TAU: f64 = 0.5;
    pub const QUEUE: f64 = 0.01;
    /// The glide: relative change a second, plus how much of the way to go
    /// a second.
    pub const SLEW: f64 = 0.02;
    pub const FOLLOW: f64 = 4.0;
    /// Updates further apart than this (seconds: a hidden tab, a long
    /// stall) leave everything as it was.
    pub const GAP: f64 = 0.25;

    /// The step last returned (1 before the first).
    pub fn step(&self) -> f64 {
        if self.step > 0.0 {
            self.step
        } else {
            1.0
        }
    }

    /// After `dt` seconds in which `due` frames fell due and the game ran
    /// `ran` of them, meant to run at `speed` (over its own rate): the
    /// resampler's step. `queued`: the audio queued now (seconds), if
    /// known; `target`: what it should be.
    pub fn update(&mut self, dt: f64, speed: f64, ran: f64, due: f64, queued: Option<f64>, target: f64) -> f64 {
        if dt <= 0.0 || dt > Self::GAP {
            return self.step();
        }
        let rates = self.rates.get_or_insert([[due / dt; 2]; 2]);
        let a = (dt / (Self::PACE_TAU * target)).min(1.0);
        for (s, n) in rates.iter_mut().zip([due, ran]) {
            s[0] += (n / dt - s[0]) * a;
            s[1] += (s[0] - s[1]) * a;
        }
        let (due_rate, ran_rate) = (rates[0][1], rates[1][1]);
        let pace = if due_rate > 0.0 { (ran_rate / due_rate).clamp(0.25, 1.0) } else { 1.0 };
        // Net time against the current decision, so a pace flickering
        // about the threshold still adds up.
        let against = self.behind != (pace < Self::ON_PACE);
        self.against = (self.against + if against { dt } else { -dt }).max(0.0);
        if self.against >= Self::HOLD * target {
            self.behind = !self.behind;
            self.against = 0.0;
        }
        let follow = if self.behind { pace } else { 1.0 };
        if self.follow == 0.0 {
            self.follow = 1.0;
        }
        self.follow += (follow - self.follow) * (dt / (Self::SMOOTH * target)).min(1.0);
        self.level = queued.map(|q| match self.level {
            Some(l) => l + (q - l) * (dt / Self::LEVEL_TAU).min(1.0),
            None => q,
        });
        let correct = self.level.map_or(1.0, |l| 1.0 + ((l - target) / target).clamp(-1.0, 1.0) * Self::QUEUE);
        let want = speed.clamp(0.1, 4.0) * self.follow * correct;
        if self.step > 0.0 {
            let off = (want / self.step).ln();
            let most = (Self::SLEW + off.abs() * Self::FOLLOW) * dt;
            self.step *= off.clamp(-most, most).exp();
        } else {
            self.step = want;
        }
        self.step
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
}
