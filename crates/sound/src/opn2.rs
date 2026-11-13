//! Yamaha OPN2 (YM3438 / YM2612) FM synthesiser: six four-operator
//! channels, two timers and an LFO, written from the chips' published
//! register map and the behaviour Yamaha's OPN-family manuals describe.
//!
//! The model runs one step per chip sample (the master clock / 144). What
//! it covers, and what it approximates:
//!
//! - **Timers** A (10 bits, one count a sample) and B (8 bits, one count
//!   every 16 samples), register `0x27`'s load, flag-enable and reset bits,
//!   and the status byte (bit 0 timer A, bit 1 timer B, bit 7 busy). Busy
//!   is the board's to time (it knows the CPU clock): see
//!   [`crate::pcm_board`].
//! - **Operators**: F-number and block, multiple, detune (approximate: the
//!   chip's exact detune table is not reproduced), the four-stage envelope
//!   (AR, D1R, D2R, RR, sustain level, key scaling) with Yamaha's
//!   counter-and-pattern rate scheme, total level, the eight algorithms,
//!   feedback on operator 1, and amplitude modulation enable.
//! - **LFO** (register `0x22`) with per-channel AMS/PMS, stereo enables,
//!   the DAC replacing channel 6 (`0x2A`/`0x2B`), and channel 3's special
//!   mode (per-operator frequencies).
//! - *Not modelled*: SSG-EG (register `0x90`+), CSM key-on, the chips'
//!   9-bit DAC quantisation and the YM2612's DAC ladder distortion.
//!
//! The original Model 2 sound programs seen so far (`daytona`, `vcop`,
//! `desert`) use the chip only for timer B and silence its channels at
//! reset (`docs/hw/model2.md` §5), so nothing here has been checked
//! against a game's music.

/// Operator output range (14-bit signed).
const OUT_MAX: i32 = 8191;
/// Envelope attenuation: 10 bits, 0.09375 dB a step.
const ENV_MAX: u16 = 1023;

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum Stage {
    Attack,
    Decay1,
    Decay2,
    #[default]
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
            0 => Stage::Attack,
            1 => Stage::Decay1,
            2 => Stage::Decay2,
            3 => Stage::Release,
            _ => return Err(state::Error(format!("bad OPN2 envelope stage {v}"))),
        };
        Ok(())
    }
}

#[derive(Clone, Default)]
struct Operator {
    /// 20-bit phase.
    phase: u32,
    att: u16,
    stage: Stage,
    key: bool,
    /// Last output, for feedback and modulation.
    out: i32,
}

#[derive(Clone, Default)]
struct Channel {
    op: [Operator; 4],
    /// Operator 1's last two outputs, for feedback.
    fb: [i32; 2],
}

pub struct Opn2 {
    /// Register files of the two parts (channels 1-3 and 4-6).
    regs: [[u8; 256]; 2],
    addr: [u8; 2],
    /// Latched high F-number bytes (`0xA4`-`0xA6`, `0xAC`-`0xAE`) per part.
    latch: [[u8; 8]; 2],
    /// F-number and block per channel (11 + 3 bits), and channel 3's
    /// special-mode frequencies for operators 1-3.
    freq: [u16; 6],
    freq3: [u16; 3],
    ch: [Channel; 6],
    /// Timer flags (status bits 0, 1), counters, timer B's prescaler.
    flags: u8,
    count_a: u16,
    count_b: u16,
    pre_b: u8,
    /// Envelope clock (every three samples) and its divider.
    eg_count: u32,
    eg_div: u8,
    lfo_count: u32,
    lfo_phase: u8,
    sin: Vec<i32>,
    gain: Vec<i32>,
}

/// Algorithms: for each, which operators (S1-S4) modulate S2, S3, S4 (as
/// bit masks of S1..S3), and which are carriers (bit mask of S1..S4).
const ALG: [([u8; 3], u8); 8] = [
    ([0b001, 0b010, 0b100], 0b1000),
    ([0b000, 0b011, 0b100], 0b1000),
    ([0b000, 0b010, 0b101], 0b1000),
    ([0b001, 0b000, 0b110], 0b1000),
    ([0b001, 0b000, 0b100], 0b1010),
    ([0b001, 0b001, 0b001], 0b1110),
    ([0b001, 0b000, 0b000], 0b1110),
    ([0b000, 0b000, 0b000], 0b1111),
];

/// Register offset of operator S1..S4 within a channel's block.
const SLOT: [usize; 4] = [0, 8, 4, 12];

impl Default for Opn2 {
    fn default() -> Self {
        Self::new()
    }
}

impl Opn2 {
    pub fn new() -> Self {
        let sin = (0..1024)
            .map(|i| ((i as f64 + 0.5) * std::f64::consts::TAU / 1024.0).sin() * OUT_MAX as f64)
            .map(|v| v.round() as i32)
            .collect();
        // Q16 gain for an attenuation in envelope steps (0.09375 dB), up to
        // 4096 steps.
        let gain = (0..4096).map(|i| (65536.0 * 10f64.powf(-(i as f64) * 0.09375 / 20.0)) as i32).collect();
        let mut c = Opn2 {
            regs: [[0; 256]; 2],
            addr: [0; 2],
            latch: [[0; 8]; 2],
            freq: [0; 6],
            freq3: [0; 3],
            ch: Default::default(),
            flags: 0,
            count_a: 0,
            count_b: 0,
            pre_b: 0,
            eg_count: 0,
            eg_div: 0,
            lfo_count: 0,
            lfo_phase: 0,
            sin,
            gain,
        };
        for ch in &mut c.ch {
            for op in &mut ch.op {
                op.att = ENV_MAX;
            }
        }
        // Panning resets to both sides.
        for part in &mut c.regs {
            part[0xb4..0xb7].fill(0xc0);
        }
        c
    }

    /// Status: timer flags (busy is added by the board).
    pub fn status(&self) -> u8 {
        self.flags
    }

    /// Write the address (`port` 0 or 2) or data (1 or 3) port.
    pub fn write(&mut self, port: u32, v: u8) {
        let part = (port >> 1 & 1) as usize;
        if port & 1 == 0 {
            self.addr[part] = v;
            return;
        }
        let a = self.addr[part];
        if part == 0 && a < 0x30 {
            self.write_global(a, v);
            return;
        }
        self.regs[part][a as usize] = v;
        let c = (a & 3) as usize;
        if c == 3 {
            return;
        }
        let ch = part * 3 + c;
        match a {
            0xa0..=0xa2 => self.freq[ch] = ((self.latch[part][c] as u16) << 8 | v as u16) & 0x3fff,
            0xa4..=0xa6 => self.latch[part][c] = v & 0x3f,
            // Channel 3's operator frequencies: A9 S1, AA S2, A8 S3.
            0xa8..=0xaa if part == 0 => {
                self.freq3[[2, 0, 1][c]] = ((self.latch[part][4 + c] as u16) << 8 | v as u16) & 0x3fff
            }
            0xac..=0xae if part == 0 => self.latch[part][4 + c] = v & 0x3f,
            _ => {}
        }
    }

    fn write_global(&mut self, a: u8, v: u8) {
        let old = self.regs[0][a as usize];
        self.regs[0][a as usize] = v;
        match a {
            0x27 => {
                if v & 1 != 0 && old & 1 == 0 {
                    self.count_a = self.timer_a();
                }
                if v & 2 != 0 && old & 2 == 0 {
                    self.count_b = self.regs[0][0x26] as u16;
                    self.pre_b = 0;
                }
                self.flags &= !(v >> 4 & 3);
            }
            0x28 => {
                let c = (v & 3) as usize;
                if c == 3 {
                    return;
                }
                let ch = c + if v & 4 != 0 { 3 } else { 0 };
                for s in 0..4 {
                    let on = v >> (4 + s) & 1 != 0;
                    let op = &mut self.ch[ch].op[s];
                    if on && !op.key {
                        op.phase = 0;
                        op.stage = Stage::Attack;
                    } else if !on && op.key {
                        op.stage = Stage::Release;
                    }
                    op.key = on;
                }
            }
            _ => {}
        }
    }

    fn timer_a(&self) -> u16 {
        (self.regs[0][0x24] as u16) << 2 | (self.regs[0][0x25] & 3) as u16
    }

    /// Any operator sounding, or the DAC on: else a step only runs the
    /// timers.
    fn active(&self) -> bool {
        self.regs[0][0x2b] & 0x80 != 0 || self.ch.iter().any(|c| c.op.iter().any(|o| o.key || o.att < ENV_MAX))
    }

    /// Advance the timers by one chip sample.
    fn timers(&mut self) {
        let ctl = self.regs[0][0x27];
        if ctl & 1 != 0 {
            self.count_a += 1;
            if self.count_a >= 1024 {
                self.count_a = self.timer_a();
                if ctl & 4 != 0 {
                    self.flags |= 1;
                }
            }
        }
        self.pre_b = (self.pre_b + 1) & 15;
        if self.pre_b == 0 && ctl & 2 != 0 {
            self.count_b += 1;
            if self.count_b >= 256 {
                self.count_b = self.regs[0][0x26] as u16;
                if ctl & 8 != 0 {
                    self.flags |= 2;
                }
            }
        }
    }

    /// One chip sample: (left, right), the channels summed at 14-bit scale.
    pub fn step(&mut self) -> (i32, i32) {
        self.timers();
        // LFO: 128 steps a cycle.
        let lfo_on = self.regs[0][0x22] & 8 != 0;
        if lfo_on {
            // Samples per LFO step at 53.3 kHz for 3.98 ... 72.2 Hz.
            const PERIOD: [u32; 8] = [108, 77, 71, 67, 62, 44, 8, 5];
            self.lfo_count += 1;
            if self.lfo_count >= PERIOD[(self.regs[0][0x22] & 7) as usize] {
                self.lfo_count = 0;
                self.lfo_phase = (self.lfo_phase + 1) & 127;
            }
        } else {
            self.lfo_phase = 0;
        }
        self.eg_div += 1;
        let eg_tick = self.eg_div == 3;
        if eg_tick {
            self.eg_div = 0;
            self.eg_count = self.eg_count.wrapping_add(1);
        }
        if !self.active() {
            return (0, 0);
        }
        let (mut l, mut r) = (0, 0);
        for ch in 0..6 {
            let out = if ch == 5 && self.regs[0][0x2b] & 0x80 != 0 {
                (self.regs[0][0x2a] as i32 - 128) << 6
            } else {
                self.channel(ch, eg_tick, lfo_on)
            };
            let pan = self.regs[ch / 3][0xb4 + ch % 3];
            if pan & 0x80 != 0 {
                l += out;
            }
            if pan & 0x40 != 0 {
                r += out;
            }
        }
        (l, r)
    }

    /// Key code: block and the F-number's top bits (for key scaling and
    /// detune).
    fn key_code(freq: u16) -> u32 {
        let block = (freq >> 11 & 7) as u32;
        let f = freq & 0x7ff;
        let (f11, f10, f9, f8) = (f >> 10 & 1, f >> 9 & 1, f >> 8 & 1, f >> 7 & 1);
        let n3 = (f11 & (f10 | f9 | f8)) | ((f11 ^ 1) & f10 & f9 & f8);
        block << 2 | (f11 as u32) << 1 | n3 as u32
    }

    fn channel(&mut self, ch: usize, eg_tick: bool, lfo_on: bool) -> i32 {
        let part = ch / 3;
        let c = ch % 3;
        let reg = |a: usize| self.regs[part][a + c];
        let alg = (reg(0xb0) & 7) as usize;
        let fb = (reg(0xb0) >> 3 & 7) as u32;
        let ams = (reg(0xb4) >> 4 & 3) as usize;
        let pms = (reg(0xb4) & 7) as usize;
        let special = ch == 2 && self.regs[0][0x27] & 0xc0 != 0;
        // LFO: a triangle for AM (0..126) and for PM (-1..1).
        let lp = self.lfo_phase as i32;
        let am_lfo = if lfo_on {
            if lp < 64 {
                lp * 2
            } else {
                (127 - lp) * 2
            }
        } else {
            0
        };
        let pm = if lfo_on && pms != 0 {
            let tri = if lp < 32 {
                lp as f32 / 32.0
            } else if lp < 96 {
                (64 - lp) as f32 / 32.0
            } else {
                (lp - 128) as f32 / 32.0
            };
            const CENTS: [f32; 8] = [0.0, 3.4, 6.7, 10.0, 14.0, 20.0, 40.0, 80.0];
            (CENTS[pms] * tri / 1200.0).exp2()
        } else {
            1.0
        };
        // AMS depth in envelope steps: 0, 1.4, 5.9, 11.8 dB.
        const AMS: [i32; 4] = [0, 15, 63, 126];
        let am = am_lfo * AMS[ams] / 126;
        let (mods, carriers) = ALG[alg];
        let mut outs = [0i32; 4];
        let mut sum = 0;
        for s in 0..4 {
            let base = SLOT[s] + c;
            let r = |a: usize| self.regs[part][a + base];
            let freq = if special && s < 3 { self.freq3[s] } else { self.freq[ch] };
            let kc = Self::key_code(freq);
            // Phase: (F-number << block) / 2, times the multiple, plus detune.
            let dt_mul = r(0x30);
            let mul = (dt_mul & 15) as u32;
            let dt = (dt_mul >> 4 & 7) as usize;
            let fnum = (freq & 0x7ff) as u32;
            let block = (freq >> 11 & 7) as u32;
            let mut inc = (fnum << block) >> 1;
            if pm != 1.0 {
                inc = (inc as f32 * pm) as u32;
            }
            // Detune: approximate, growing with the key code (a few
            // phase units at the top of the range).
            const DT: [u32; 4] = [0, 1, 2, 4];
            let d = DT[dt & 3] * (1 << (kc >> 2)) / 16 + if dt & 3 != 0 && kc >= 16 { DT[dt & 3] } else { 0 };
            inc = if dt & 4 != 0 { inc.saturating_sub(d) } else { inc + d };
            inc = if mul == 0 { inc >> 1 } else { inc * mul };
            let ks = r(0x50) >> 6;
            let ksr = kc >> (3 - ks);
            let rate = |raw: u32| if raw == 0 { 0 } else { (raw * 2 + ksr).min(63) };
            let ar = rate((r(0x50) & 31) as u32);
            let d1r = rate((r(0x60) & 31) as u32);
            let d2r = rate((r(0x70) & 31) as u32);
            let rr = rate(((r(0x80) & 15) as u32) << 1 | 1);
            let sl = {
                let v = (r(0x80) >> 4) as u16;
                if v == 15 {
                    31 << 5
                } else {
                    v << 5
                }
            };
            let tl = ((r(0x40) & 127) as i32) << 3;
            let am_on = r(0x60) & 0x80 != 0;
            let eg_count = self.eg_count;
            let op = &mut self.ch[ch].op[s];
            op.phase = op.phase.wrapping_add(inc) & 0xf_ffff;
            if eg_tick {
                let rate = match op.stage {
                    Stage::Attack => ar,
                    Stage::Decay1 => d1r,
                    Stage::Decay2 => d2r,
                    Stage::Release => rr,
                };
                let step = eg_step(rate, eg_count);
                if step != 0 {
                    match op.stage {
                        Stage::Attack => {
                            if rate >= 62 {
                                op.att = 0;
                            } else {
                                let a = op.att as i32;
                                op.att = (a - (((a + 1) * step as i32) >> 4)).max(0) as u16;
                            }
                        }
                        _ => op.att = (op.att + step as u16).min(ENV_MAX),
                    }
                }
                match op.stage {
                    Stage::Attack if op.att == 0 => op.stage = Stage::Decay1,
                    Stage::Decay1 if op.att >= sl => op.stage = Stage::Decay2,
                    _ => {}
                }
            }
            // Modulation input: the modulators' outputs (halved), or
            // operator 1's feedback.
            let m = if s == 0 {
                if fb == 0 {
                    0
                } else {
                    (self.ch[ch].fb[0] + self.ch[ch].fb[1]) >> (10 - fb)
                }
            } else {
                let mask = mods[s - 1];
                (0..3).filter(|&k| mask >> k & 1 != 0).map(|k| outs[k]).sum::<i32>() >> 1
            };
            let op = &self.ch[ch].op[s];
            let att = op.att as i32 + tl + if am_on { am } else { 0 };
            let idx = ((op.phase >> 10) as i32 + m) as usize & 1023;
            let out = if att >= 4096 { 0 } else { (self.sin[idx] * self.gain[att as usize]) >> 16 };
            outs[s] = out;
            self.ch[ch].op[s].out = out;
            if s == 0 {
                self.ch[ch].fb = [self.ch[ch].fb[1], out];
            }
            if carriers >> s & 1 != 0 {
                sum += out;
            }
        }
        sum.clamp(-OUT_MAX - 1, OUT_MAX)
    }
}

/// Envelope increment at `rate` (0-63) on envelope clock `count`: rates
/// below 48 step once every 2^(11 - rate/4) clocks, by a pattern for
/// rate % 4 that averages 1, 1.25, 1.5, 1.75; rates 48 and up every clock
/// by that pattern times 2^(rate/4 - 11).
fn eg_step(rate: u32, count: u32) -> u32 {
    const PATTERN: [[u32; 8]; 4] =
        [[0, 1, 0, 1, 0, 1, 0, 1], [0, 1, 0, 1, 1, 1, 0, 1], [0, 1, 1, 1, 0, 1, 1, 1], [0, 1, 1, 1, 1, 1, 1, 1]];
    if rate < 4 {
        return 0;
    }
    let row = PATTERN[(rate & 3) as usize];
    if rate < 48 {
        let shift = 11 - rate / 4;
        if count & ((1 << shift) - 1) != 0 {
            return 0;
        }
        row[(count >> shift & 7) as usize]
    } else {
        row[(count & 7) as usize] << (rate / 4 - 11)
    }
}

state::fields!(Operator { phase, att, stage, key, out });
state::fields!(Channel { op, fb });
state::fields!(Opn2 {
    regs,
    addr,
    latch,
    freq,
    freq3,
    ch,
    flags,
    count_a,
    count_b,
    pre_b,
    eg_count,
    eg_div,
    lfo_count,
    lfo_phase
});

#[cfg(test)]
mod tests {
    use super::*;

    fn w(c: &mut Opn2, part: u32, a: u8, v: u8) {
        c.write(part * 2, a);
        c.write(part * 2 + 1, v);
    }

    #[test]
    fn timer_b_period() {
        // TB = 0xFC: (256 - 252) * 16 samples between overflows.
        let mut c = Opn2::new();
        w(&mut c, 0, 0x26, 0xfc);
        w(&mut c, 0, 0x27, 0x2a);
        let mut at = Vec::new();
        for i in 0..1000 {
            c.step();
            if c.status() & 2 != 0 {
                at.push(i);
                w(&mut c, 0, 0x27, 0x2a);
            }
        }
        let gaps: Vec<_> = at.windows(2).map(|p| p[1] - p[0]).collect();
        assert!(gaps.iter().all(|&g| g == 64), "{gaps:?}");
    }

    #[test]
    fn timer_a_flag_needs_enable() {
        let mut c = Opn2::new();
        w(&mut c, 0, 0x24, 0xff);
        w(&mut c, 0, 0x25, 0x03);
        w(&mut c, 0, 0x27, 0x01);
        for _ in 0..10 {
            c.step();
        }
        assert_eq!(c.status() & 1, 0);
        w(&mut c, 0, 0x27, 0x05);
        c.step();
        c.step();
        assert_eq!(c.status() & 1, 1);
        w(&mut c, 0, 0x27, 0x15);
        assert_eq!(c.status() & 1, 0);
    }

    #[test]
    fn sine_carrier_pitch() {
        // Algorithm 7, S4 alone, multiple 1, no detune: F-number 0x400 at
        // block 4 is 0x400 * 2^3 / 2^20 cycles a sample.
        let mut c = Opn2::new();
        w(&mut c, 0, 0xb0, 0x07);
        w(&mut c, 0, 0x3c, 0x01);
        w(&mut c, 0, 0x4c, 0x00);
        w(&mut c, 0, 0x5c, 0x1f);
        w(&mut c, 0, 0x8c, 0x0f);
        for s in [0x40, 0x44, 0x48] {
            w(&mut c, 0, s, 0x7f);
        }
        w(&mut c, 0, 0xa4, 4 << 3 | 0x04);
        w(&mut c, 0, 0xa0, 0x00);
        w(&mut c, 0, 0x28, 0x80);
        let n = 1 << 16;
        let mut prev = 0;
        let mut rising = 0;
        let mut peak = 0;
        for _ in 0..n {
            let (l, _) = c.step();
            if prev <= 0 && l > 0 {
                rising += 1;
            }
            peak = peak.max(l);
            prev = l;
        }
        let expect = n as f64 * (0x400 * 8) as f64 / (1 << 20) as f64;
        assert!((rising as f64 - expect).abs() <= 2.0, "{rising} vs {expect}");
        assert!(peak > 8000, "{peak}");
        // Key off: release.
        w(&mut c, 0, 0x28, 0x00);
        for _ in 0..20000 {
            c.step();
        }
        assert!(!c.active());
    }

    #[test]
    fn eg_rates_scale() {
        // Total increments over 2^14 clocks double every four rates.
        let total = |r| (0..1 << 14).map(|n| eg_step(r, n)).sum::<u32>();
        assert_eq!(total(20) * 2, total(24));
        assert_eq!(total(44) * 2, total(48));
        assert_eq!(total(52) * 2, total(56));
    }
}
