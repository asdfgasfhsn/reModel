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

/// A board the sound link feeds: the sound board (the SCSP one on Model
/// 2A/2B/2C, or the original Model 2's, MultiPCMs and an OPN2) or a music
/// board (the DSB2, 68000, `topskatr`; the DSB1, Z80, `stcc`). Each takes
/// the link's bytes and gives 44.1 kHz stereo.
trait Board: state::State + Send {
    fn midi_in(&mut self, byte: u8);
    fn run(&mut self, samples: usize, out: &mut Vec<i16>);
    /// Replace music tracks (a music board's; the sound board has none).
    fn set_soundtrack(&mut self, _: dsb::Soundtrack) {}
}

/// `Board` for a chip crate's board by its own methods, and `$extra`.
macro_rules! board {
    ($t:ty $(, $extra:item)*) => {
        impl Board for $t {
            fn midi_in(&mut self, byte: u8) {
                <$t>::midi_in(self, byte)
            }
            fn run(&mut self, samples: usize, out: &mut Vec<i16>) {
                <$t>::run(self, samples, out)
            }
            $($extra)*
        }
    };
}

board!(sound::SoundBoard);
board!(sound::PcmBoard);
board!(
    dsb::Dsb2,
    fn set_soundtrack(&mut self, st: dsb::Soundtrack) {
        dsb::Dsb2::set_soundtrack(self, st)
    }
);
board!(
    dsb::Dsb1,
    fn set_soundtrack(&mut self, st: dsb::Soundtrack) {
        dsb::Dsb1::set_soundtrack(self, st)
    }
);

pub struct Live {
    sound: Option<Box<dyn Board>>,
    music: Option<Box<dyn Board>>,
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
        let sound = (!prog.is_empty()).then(|| -> Box<dyn Board> {
            if roms.game.board == romset::Board::Model2 {
                let (a, b) = (roms.multipcm(Role::MultiPcmA), roms.multipcm(Role::MultiPcmB));
                Box::new(sound::PcmBoard::new(prog, a, b))
            } else {
                Box::new(sound::SoundBoard::new(prog, roms.region16be(Role::Samples)))
            }
        });
        let chip = |role| roms.game.files.iter().position(|f| f.role == role).map(|i| &roms.chips[i]);
        let music: Option<Box<dyn Board>> = if let Some(prog) = chip(Role::MpegCpu) {
            let mut d = dsb::Dsb2::new(prog, roms.mpeg_data());
            d.set_diagnostics(false);
            Some(Box::new(d))
        } else if let Some(prog) = chip(Role::Dsb1Cpu) {
            let mut d = dsb::Dsb1::new(prog, roms.mpeg_data());
            d.set_diagnostics(false);
            Some(Box::new(d))
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
