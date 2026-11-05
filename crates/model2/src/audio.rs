//! Live audio: the sound board (68000 + SCSP) and, on games that have one,
//! the DSB2 music board, run alongside the main board and fed the bytes the
//! game sends on the sound link as it sends them.
//!
//! Each byte reaches the boards at the output sample matching the CPU cycle
//! it was written at, so timing matches the offline replays (`remodel
//! sound-render`, `music-render`). The two outputs are summed: on the
//! cabinet they meet in the analog amplifier (docs/hw/sound.md 3.6).

use crate::bus::{Model2Bus, CPU_HZ};
use romset::{Role, RomSet};

/// Output sample rate.
pub const RATE: u32 = 44_100;

pub struct Live {
    sound: Option<sound::SoundBoard>,
    music: Option<dsb::Dsb2>,
    /// Output samples produced so far (per channel).
    samples: u64,
    a: Vec<i16>,
    b: Vec<i16>,
}

impl Live {
    /// The boards `roms` has: none, the sound board, or both.
    pub fn new(roms: &RomSet) -> Self {
        let prog = roms.region16be(Role::AudioCpu);
        let sound = (!prog.is_empty()).then(|| sound::SoundBoard::new(prog, roms.region16be(Role::Samples)));
        let music = roms.game.files.iter().position(|f| f.role == Role::MpegCpu).map(|i| {
            let mpeg: Vec<u8> = roms
                .game
                .files
                .iter()
                .zip(&roms.chips)
                .filter(|(f, _)| f.role == Role::MpegData)
                .flat_map(|(_, c)| c.iter().copied())
                .collect();
            let mut d = dsb::Dsb2::new(&roms.chips[i], mpeg);
            d.set_diagnostics(false);
            d
        });
        Live { sound, music, samples: 0, a: Vec::new(), b: Vec::new() }
    }

    /// Replace music tracks with the files of a custom soundtrack.
    pub fn set_soundtrack(&mut self, st: dsb::Soundtrack) {
        if let Some(m) = &mut self.music {
            m.set_soundtrack(st);
        }
    }

    pub fn has_sound(&self) -> bool {
        self.sound.is_some() || self.music.is_some()
    }

    /// Render up to the bus's current time, delivering the sound link bytes
    /// written since the last call (taken from `bus.uart_log`). Appends
    /// interleaved stereo i16 at [`RATE`] to `out`.
    pub fn run(&mut self, bus: &mut Model2Bus, out: &mut Vec<i16>) {
        for (t, control, byte) in std::mem::take(&mut bus.uart_log) {
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
        self.render_to(bus.now, out);
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
        out.extend(self.a.iter().zip(&self.b).map(|(&x, &y)| x.saturating_add(y)));
    }
}
