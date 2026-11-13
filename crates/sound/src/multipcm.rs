//! Sega's MultiPCM (315-5560) sample player, as the original Model 2 sound
//! board uses it: 28 voices of 8-bit PCM from a sample ROM with a header
//! table at its start, each with pitch, level, pan, an envelope and an LFO.
//!
//! There is no public manual. The register semantics come from how
//! `daytona`'s sound program drives the chip and from the sample ROMs'
//! own headers (`docs/hw/multipcm.md`, which marks what is confirmed and
//! what is inferred). Where the program leaves a behaviour open (envelope
//! timings, the LFO, the pan curve) the model borrows the published
//! figures of Yamaha's YMF278B (OPL4), whose wave-table part takes the
//! same 12-byte sample headers; those are guesses for this chip.
//!
//! Interface (one byte port each): data at offset 0 (reads: status, bit 0
//! busy, never set here), slot select at 1, register select at 2. Slot
//! numbers are 0-31 with every eighth (7, 15, 23, 31) unused: 28 voices.
//! Per-slot registers:
//!
//! | reg | use |
//! |---|---|
//! | 0 | pan: high nibble, signed (0 centre, +1..+7 to the right, -1..-7 to the left) |
//! | 1 | sample number: its 12-byte header in the ROM's table at 0 |
//! | 2 | F-number bits 5-0 in bits 7-2 |
//! | 3 | octave (signed high nibble), F-number bits 9-6 (low nibble) |
//! | 4 | bit 7: key on (1) / off (0) |
//! | 5 | total level (attenuation) in bits 7-1; bit 0: set it at once, else glide |
//! | 6 | LFO frequency (bits 5-3), vibrato depth (bits 2-0) |
//! | 7 | tremolo depth (bits 2-0) |
//!
//! The pitch is `2^(octave - 1) * (1 + F / 1024)` samples of the ROM per
//! output sample at 44.1 kHz: octave 1, F 0 plays a sample at its stored
//! rate (inferred from the game's use of its samples: most are keyed
//! there, and its announcer's words are natural speech there,
//! `docs/hw/multipcm.md`).
//!
//! Sample addresses are 22 bits. The board banks the upper half of the
//! 2 MB space: `0x000000-0x0FFFFF` reads the ROM's first MB, `0x100000-
//! 0x1FFFFF` the MB the bank register selects ([`MultiPcm::set_bank`]).

/// Voices.
pub const VOICES: usize = 28;
/// Output rate (a sample keyed at octave 1, F 0 plays at its stored rate).
pub const RATE: f64 = 44_100.0;

const MB: u32 = 0x10_0000;
/// Envelope attenuation: 1/32 dB units with 8 fractional bits.
const ENV_FRAC: u32 = 8;
const ENV_DB: u32 = 32 << ENV_FRAC;
/// Silence: 96 dB.
const ENV_MAX: u32 = 96 * ENV_DB;
/// Gain table range: 1/32 dB units up to 128 dB.
const GAIN_STEPS: usize = 128 * 32;
/// Total level: 0.375 dB a step (OPL4's figure; unconfirmed), in 1/32 dB.
const TL_UNIT: u32 = 12;
/// Level glide: one TL step every this many samples (unconfirmed).
const TL_GLIDE: u8 = 16;

/// Envelope stage.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Stage {
    #[default]
    Off,
    Attack,
    Decay1,
    Decay2,
    Release,
}

impl state::State for Stage {
    fn save(&self, w: &mut state::Writer) {
        (*self as u8).save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        let mut v = 0u8;
        v.load(r)?;
        *self = match v {
            0 => Stage::Off,
            1 => Stage::Attack,
            2 => Stage::Decay1,
            3 => Stage::Decay2,
            4 => Stage::Release,
            _ => return Err(state::Error(format!("bad MultiPCM envelope stage {v}"))),
        };
        Ok(())
    }
}

/// One voice.
#[derive(Clone, Default)]
pub struct Voice {
    pub regs: [u8; 8],
    stage: Stage,
    /// Sample start (22-bit chip address), loop and end offsets (end is
    /// the last sample played before the loop).
    start: u32,
    lp: u32,
    end: u32,
    /// Envelope rates and decay level from the header: AR, D1R, DL, D2R,
    /// rate scaling, RR.
    env: [u8; 6],
    /// Position: offset from `start`, and 16-bit fraction.
    pos: u32,
    frac: u32,
    /// Envelope attenuation, `ENV_DB` per dB.
    att: u32,
    /// Total level now (gliding to the register's) in TL steps, and the
    /// glide's sample counter.
    tl: u8,
    glide: u8,
    lfo_phase: u32,
}

impl Voice {
    pub fn sounding(&self) -> bool {
        self.stage != Stage::Off
    }
}

/// Counters for diagnostics (not saved).
#[derive(Clone, Default, Debug)]
pub struct Stats {
    pub key_ons: u64,
    pub key_offs: u64,
    /// Key-ons per sample number.
    pub samples_used: Vec<u64>,
    /// Writes to the unused slot numbers (7, 15, 23, 31).
    pub bad_slot: u64,
    /// Writes outside the three ports: (offset, value), first few.
    pub other: Vec<(u32, u8)>,
    pub bank_writes: u64,
}

pub struct MultiPcm {
    rom: Vec<u8>,
    pub voices: [Voice; VOICES],
    slot: u8,
    reg: u8,
    bank: u8,
    /// Envelope increments per sample by effective rate (0-63): decay
    /// (attenuation units) and attack (a 2^-24 fraction of the
    /// attenuation left).
    decay: [u32; 64],
    attack: [u32; 64],
    gain: Vec<i32>,
    /// LFO: phase step per sample (2^32 a cycle) by frequency; vibrato
    /// depth in cents and tremolo in 1/32 dB by depth.
    lfo_step: [u32; 8],
    vib_cents: [f32; 8],
    trem: [u32; 8],
    pub stats: Stats,
    /// Key-on log for `sound-render` traces: (output sample, voice,
    /// registers at key-on, bank). Not saved.
    pub key_log: Option<Vec<KeyOn>>,
    clock: u64,
}

/// A key-on for traces: output sample, voice, its registers, the bank.
pub type KeyOn = (u64, u8, [u8; 8], u8);

/// Voice index of a slot number (None for the unused 7, 15, 23, 31).
fn voice_of(slot: u8) -> Option<usize> {
    let s = slot & 0x1f;
    (s & 7 != 7).then(|| (s >> 3) as usize * 7 + (s & 7) as usize)
}

impl MultiPcm {
    /// A chip reading `rom` (its sample ROMs concatenated, the header
    /// table first).
    pub fn new(rom: Vec<u8>) -> Self {
        // Envelope times, inferred from OPL4's published tables and
        // unconfirmed for this chip: 96 dB of decay takes 39.3 s at
        // effective rate 4 and attack 2.83 s, halving every four rates;
        // rates 60 and up attack at once.
        let mut decay = [0; 64];
        let mut attack = [0; 64];
        for r in 4..64 {
            let scale = 2f64.powf(r as f64 / 4.0 - 1.0);
            let t = 39.28 / scale;
            decay[r] = ((ENV_MAX as f64) / (t * RATE)).round().max(1.0) as u32;
            // Exponential approach: 60 dB of the way in the attack time.
            let ta = 2.826 / scale;
            attack[r] = if r >= 60 { 1 << 24 } else { ((6.9 / (ta * RATE)) * (1u64 << 24) as f64) as u32 };
        }
        let gain = (0..=GAIN_STEPS).map(|i| (32768.0 * 10f64.powf(-(i as f64) / 32.0 / 20.0)) as i32).collect();
        let lfo_hz = [0.168, 2.019, 3.196, 4.206, 5.215, 5.888, 6.224, 7.066];
        let lfo_step = lfo_hz.map(|f: f64| (f / RATE * 4_294_967_296.0) as u32);
        let vib_cents = [0.0, 3.378, 5.065, 6.750, 10.114, 20.170, 40.106, 79.307];
        let trem = [0.0, 1.781, 2.906, 3.656, 4.406, 5.906, 7.406, 11.91f64].map(|db| (db * 32.0) as u32);
        MultiPcm {
            rom,
            voices: Default::default(),
            slot: 0,
            reg: 0,
            bank: 0,
            decay,
            attack,
            gain,
            lfo_step,
            vib_cents,
            trem,
            stats: Stats { samples_used: vec![0; 256], ..Default::default() },
            key_log: None,
            clock: 0,
        }
    }

    /// Status (data port read): bit 0 busy. Writes are taken at once, so
    /// never busy.
    pub fn status(&self) -> u8 {
        0
    }

    /// Write `port` (0 data, 1 slot select, 2 register select).
    pub fn write(&mut self, port: u32, v: u8) {
        match port {
            0 => self.write_reg(v),
            1 => self.slot = v,
            2 => self.reg = v & 7,
            _ => {
                if self.stats.other.len() < 16 {
                    self.stats.other.push((port, v));
                }
            }
        }
    }

    /// The bank register: which MB of the ROM `0x100000-0x1FFFFF` reads.
    pub fn set_bank(&mut self, v: u8) {
        self.bank = v;
        self.stats.bank_writes += 1;
    }

    pub fn bank(&self) -> u8 {
        self.bank
    }

    fn write_reg(&mut self, v: u8) {
        let Some(n) = voice_of(self.slot) else {
            self.stats.bad_slot += 1;
            return;
        };
        let r = self.reg as usize;
        let was = self.voices[n].regs[r];
        self.voices[n].regs[r] = v;
        match r {
            4 if v & 0x80 != 0 => self.key_on(n),
            4 if was & 0x80 != 0 => {
                let voice = &mut self.voices[n];
                if voice.stage != Stage::Off {
                    voice.stage = Stage::Release;
                }
                self.stats.key_offs += 1;
            }
            5 if v & 1 != 0 => self.voices[n].tl = v >> 1,
            _ => {}
        }
    }

    fn header(&self, sample: u8) -> [u8; 12] {
        let a = sample as usize * 12;
        let mut h = [0xff; 12];
        if let Some(b) = self.rom.get(a..a + 12) {
            h.copy_from_slice(b);
        }
        h
    }

    fn key_on(&mut self, n: usize) {
        let sample = self.voices[n].regs[1];
        let h = self.header(sample);
        self.stats.key_ons += 1;
        self.stats.samples_used[sample as usize] += 1;
        if let Some(log) = &mut self.key_log {
            log.push((self.clock, n as u8, self.voices[n].regs, self.bank));
        }
        let v = &mut self.voices[n];
        // Header: start (22 bits; the top two, the format, are 0 for
        // 8-bit samples in every set seen), loop offset, end offset
        // stored inverted, then the LFO/vibrato byte, AR/D1R, DL/D2R,
        // rate scaling/RR and tremolo, which are loaded into the voice.
        v.start = (h[0] as u32) << 16 & 0x3f_0000 | (h[1] as u32) << 8 | h[2] as u32;
        v.lp = (h[3] as u32) << 8 | h[4] as u32;
        v.end = !((h[5] as u32) << 8 | h[6] as u32) & 0xffff;
        v.regs[6] = h[7];
        v.env = [h[8] >> 4, h[8] & 15, h[9] >> 4, h[9] & 15, h[10] >> 4, h[10] & 15];
        v.regs[7] = h[11];
        v.pos = 0;
        v.frac = 0;
        v.lfo_phase = 0;
        v.att = ENV_MAX;
        v.stage = Stage::Attack;
    }

    fn rom_byte(&self, addr: u32) -> i32 {
        let a = addr & 0x3f_ffff;
        let off = if a < MB { a } else { (self.bank as u32) * MB + (a & (MB - 1)) };
        self.rom.get(off as usize).map_or(0, |&b| b as i8 as i32)
    }

    /// Octave and F-number.
    fn pitch(v: &Voice) -> (i32, u32) {
        let oct = ((v.regs[3] as i8) >> 4) as i32;
        let f = ((v.regs[3] & 15) as u32) << 6 | (v.regs[2] >> 2) as u32;
        (oct, f)
    }

    /// Effective envelope rate: 4 * rate plus the key scaling (OPL4's
    /// rule, unconfirmed): `(octave + scaling) * 2 + F9`, 0-15, none at
    /// scaling 15.
    fn rate(v: &Voice, r: u8) -> usize {
        if r == 0 {
            return 0;
        }
        let krs = v.env[4] as i32;
        let corr = if krs == 15 {
            0
        } else {
            let (oct, f) = Self::pitch(v);
            ((oct + krs) * 2 + (f >> 9) as i32).clamp(0, 15)
        };
        (r as i32 * 4 + corr).min(63) as usize
    }

    fn envelope(&self, v: &mut Voice) {
        match v.stage {
            Stage::Off => {}
            Stage::Attack => {
                let k = self.attack[Self::rate(v, v.env[0])] as u64;
                let d = ((v.att as u64 * k) >> 24) as u32 + 1;
                v.att = v.att.saturating_sub(d);
                if v.att == 0 {
                    v.stage = Stage::Decay1;
                }
            }
            Stage::Decay1 => {
                // Decay level: 3 dB a step (15: 93 dB).
                let dl = v.env[2] as u32 * 3 * ENV_DB;
                v.att = (v.att + self.decay[Self::rate(v, v.env[1])]).min(ENV_MAX);
                if v.att >= dl {
                    v.stage = Stage::Decay2;
                }
            }
            Stage::Decay2 | Stage::Release => {
                let r = if v.stage == Stage::Decay2 { v.env[3] } else { v.env[5] };
                v.att = (v.att + self.decay[Self::rate(v, r)]).min(ENV_MAX);
                if v.att >= ENV_MAX {
                    v.stage = Stage::Off;
                }
            }
        }
    }

    /// One output sample: left and right, each the sum of the voices at
    /// 16-bit scale.
    pub fn sample(&mut self) -> (i32, i32) {
        self.clock += 1;
        let (mut l, mut r) = (0i32, 0i32);
        for n in 0..VOICES {
            if self.voices[n].stage == Stage::Off {
                continue;
            }
            let mut v = std::mem::take(&mut self.voices[n]);
            self.envelope(&mut v);
            // Level glide toward the register's total level.
            let target = v.regs[5] >> 1;
            if v.tl != target {
                v.glide += 1;
                if v.glide >= TL_GLIDE {
                    v.glide = 0;
                    v.tl = if v.tl < target { v.tl + 1 } else { v.tl - 1 };
                }
            }
            // LFO: a triangle for vibrato and for tremolo.
            let lfo = self.lfo_step[(v.regs[6] >> 3 & 7) as usize];
            v.lfo_phase = v.lfo_phase.wrapping_add(lfo);
            let tri = {
                let p = v.lfo_phase >> 16;
                // -32768..32767, a triangle.
                (if p < 0x8000 { p as i32 * 2 - 0x8000 } else { 0x17fff - p as i32 * 2 }) as f32 / 32768.0
            };
            let (oct, f) = Self::pitch(&v);
            // Octave 1, F 0 reads one sample per output sample (16-bit
            // fraction): 2^(octave - 1) * (1 + F / 1024).
            let mut step = (1024 + f) << 5;
            step = if oct >= 0 { step << oct } else { step >> -oct };
            let vib = v.regs[6] & 7;
            if vib != 0 {
                step = (step as f32 * (self.vib_cents[vib as usize] * tri / 1200.0).exp2()) as u32;
            }
            let trem = self.trem[(v.regs[7] & 7) as usize];
            let am = (trem as f32 * (tri + 1.0) / 2.0) as u32;
            // Sample, linearly interpolated.
            let len = v.end + 1;
            let next = if v.pos + 1 >= len { v.lp } else { v.pos + 1 };
            let s0 = self.rom_byte(v.start + v.pos);
            let s1 = self.rom_byte(v.start + next);
            let s = (s0 << 8) + ((((s1 - s0) << 8) * (v.frac >> 4) as i32) >> 12);
            v.frac += step;
            v.pos += v.frac >> 16;
            v.frac &= 0xffff;
            if v.pos >= len {
                let loop_len = len.saturating_sub(v.lp).max(1);
                v.pos = v.lp + (v.pos - len) % loop_len;
                if v.lp >= len {
                    v.stage = Stage::Off;
                }
            }
            let att = (v.att >> ENV_FRAC) + v.tl as u32 * TL_UNIT + am;
            let g = self.gain[(att as usize).min(GAIN_STEPS)];
            let x = (s * g) >> 15;
            let (gl, gr) = pan(v.regs[0] >> 4);
            l += (x * gl) >> 15;
            r += (x * gr) >> 15;
            self.voices[n] = v;
        }
        (l, r)
    }

    pub fn sounding(&self) -> usize {
        self.voices.iter().filter(|v| v.sounding()).count()
    }
}

/// Pan gains (Q15) for the register's signed nibble: each step away from
/// the centre takes 3 dB off the other side, 7 silences it, and -8 (the
/// reset value) both (OPL4's curve; unconfirmed).
fn pan(nibble: u8) -> (i32, i32) {
    const STEP: [i32; 8] = [32768, 23197, 16423, 11627, 8231, 5827, 4125, 0];
    let p = ((nibble << 4) as i8 >> 4) as i32;
    match p {
        -8 => (0, 0),
        0 => (32767, 32767),
        p if p > 0 => (STEP[p as usize], 32767),
        p => (32767, STEP[(-p) as usize]),
    }
}

state::fields!(Voice { regs, stage, start, lp, end, env, pos, frac, att, tl, glide, lfo_phase });
state::fields!(MultiPcm { voices, slot, reg, bank });

#[cfg(test)]
mod tests {
    use super::*;

    /// A ROM with one header (sample 0): a looped square wave of period
    /// 64 at address 0x1000, envelope at once.
    fn rom() -> Vec<u8> {
        let mut rom = vec![0u8; 0x2000];
        let len = 0x400u32;
        rom[..12].copy_from_slice(&[
            0x00,
            0x10,
            0x00,
            0x00,
            0x00,
            !((len - 1) >> 8) as u8,
            !(len - 1) as u8,
            0,
            0xf0,
            0,
            0x0f,
            0,
        ]);
        for i in 0..len as usize {
            rom[0x1000 + i] = if i % 64 < 32 { 0x40 } else { 0xc0 };
        }
        rom
    }

    fn key(c: &mut MultiPcm, slot: u8, regs: &[(u8, u8)]) {
        c.write(1, slot);
        for &(r, v) in regs {
            c.write(2, r);
            c.write(0, v);
        }
    }

    /// Crossings of zero (rising) over `n` samples of the left channel.
    fn rising(c: &mut MultiPcm, n: usize) -> usize {
        let mut prev = 0;
        let mut count = 0;
        for _ in 0..n {
            let (l, _) = c.sample();
            if prev <= 0 && l > 0 {
                count += 1;
            }
            prev = l;
        }
        count
    }

    #[test]
    fn slots_skip_every_eighth() {
        let v: Vec<_> = (0..32).filter_map(voice_of).collect();
        assert_eq!(v, (0..28).collect::<Vec<_>>());
    }

    #[test]
    fn pitch_follows_octave_and_f_number() {
        // Period 64 at the stored rate (octave 1, F 0): 44100 / 64 rising
        // edges a second.
        let mut c = MultiPcm::new(rom());
        key(&mut c, 0, &[(1, 0), (3, 0x10), (2, 0), (5, 0x01), (0, 0), (4, 0x80)]);
        let n = rising(&mut c, 44100);
        assert!((n as i64 - 689).abs() <= 2, "{n}");
        // One octave up, then a fifth below that (F = 512 is x1.5).
        key(&mut c, 0, &[(3, 0x20), (4, 0x80)]);
        let n = rising(&mut c, 44100);
        assert!((n as i64 - 1378).abs() <= 2, "{n}");
        key(&mut c, 0, &[(3, 0x18), (2, 0), (4, 0x80)]);
        let n = rising(&mut c, 44100);
        assert!((n as i64 - 1034).abs() <= 2, "{n}");
        // Octave 0 is half the stored rate.
        key(&mut c, 0, &[(3, 0x00), (2, 0), (4, 0x80)]);
        let n = rising(&mut c, 44100);
        assert!((n as i64 - 345).abs() <= 2, "{n}");
    }

    #[test]
    fn level_and_release() {
        let mut c = MultiPcm::new(rom());
        key(&mut c, 0, &[(1, 0), (5, 0x01), (0, 0), (4, 0x80)]);
        let loud = (0..4410).map(|_| c.sample().0.abs()).max().unwrap();
        assert!(loud > 0x3f00, "{loud}");
        // TL 0x10 = 6 dB at 0.375 dB a step: about half.
        key(&mut c, 0, &[(5, 0x21)]);
        let half = (0..4410).map(|_| c.sample().0.abs()).max().unwrap();
        assert!((half as f64 / loud as f64 - 0.5).abs() < 0.03, "{half} {loud}");
        // Key off: RR 15 releases within a few ms.
        key(&mut c, 0, &[(4, 0)]);
        for _ in 0..4410 {
            c.sample();
        }
        assert_eq!(c.sounding(), 0);
    }

    #[test]
    fn bank_selects_upper_window() {
        let mut rom = vec![0u8; 4 * MB as usize];
        rom[3 * MB as usize + 5] = 0x55;
        let mut c = MultiPcm::new(rom);
        c.set_bank(3);
        assert_eq!(c.rom_byte(MB + 5), 0x55);
        assert_eq!(c.rom_byte(5), 0);
    }

    #[test]
    fn pan_sides() {
        assert_eq!(pan(0), (32767, 32767));
        assert_eq!(pan(7), (0, 32767));
        assert_eq!(pan(9), (32767, 0));
        assert_eq!(pan(8), (0, 0));
    }
}
