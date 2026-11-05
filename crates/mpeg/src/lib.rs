//! MPEG-1 Audio Layer II decoder, written from ISO/IEC 11172-3 (§2.4 and
//! Annex B).
//!
//! Handles mono, stereo, joint stereo and dual channel at every MPEG-1 Layer II
//! bitrate and sample rate. Free-format streams (bitrate index 0) are not
//! supported. A frame's CRC word is skipped, not checked. Output is `f32` with
//! full scale at ±1.0 and is not clipped.

mod window;

use std::f64::consts::PI;

/// PCM samples per channel in one Layer II frame.
pub const FRAME_SAMPLES: usize = 1152;

/// Layer II bitrates in kbit/s by index (0 = free format).
const BITRATES: [u32; 15] = [0, 32, 48, 56, 64, 80, 96, 112, 128, 160, 192, 224, 256, 320, 384];
const RATES: [u32; 3] = [44100, 48000, 32000];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Stereo,
    JointStereo,
    DualChannel,
    Mono,
}

/// A parsed MPEG-1 Layer II frame header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    /// A 16-bit CRC follows the header.
    pub crc: bool,
    /// Bits per second.
    pub bitrate: u32,
    pub sample_rate: u32,
    pub padding: bool,
    pub mode: Mode,
    /// Joint stereo: intensity stereo from subband 4, 8, 12 or 16.
    pub mode_ext: u8,
    pub emphasis: u8,
}

impl Header {
    /// Parse the 4-byte header at the start of `b`. Only MPEG-1 Layer II with
    /// a fixed bitrate is accepted.
    pub fn parse(b: &[u8]) -> Option<Header> {
        let &[b0, b1, b2, b3, ..] = b else { return None };
        // Sync (12 bits), ID = 1 (MPEG-1), layer = 0b10 (Layer II).
        if b0 != 0xff || b1 & 0xfe != 0xfc {
            return None;
        }
        let br = (b2 >> 4) as usize;
        let sr = (b2 >> 2 & 3) as usize;
        if br == 0 || br == 15 || sr == 3 {
            return None;
        }
        let mode = match b3 >> 6 {
            0 => Mode::Stereo,
            1 => Mode::JointStereo,
            2 => Mode::DualChannel,
            _ => Mode::Mono,
        };
        Some(Header {
            crc: b1 & 1 == 0,
            bitrate: BITRATES[br] * 1000,
            sample_rate: RATES[sr],
            padding: b2 & 2 != 0,
            mode,
            mode_ext: b3 >> 4 & 3,
            emphasis: b3 & 3,
        })
    }

    /// Frame length in bytes, header included.
    pub fn frame_len(&self) -> usize {
        (144 * self.bitrate / self.sample_rate) as usize + self.padding as usize
    }

    pub fn channels(&self) -> usize {
        if self.mode == Mode::Mono {
            1
        } else {
            2
        }
    }

    /// Bit allocation table (Annex B, Table B.2a-d), chosen by sample rate
    /// and bitrate per channel.
    fn table(&self) -> Table {
        let per_ch = self.bitrate / self.channels() as u32 / 1000;
        if per_ch <= 48 {
            if self.sample_rate == 32000 {
                Table::D
            } else {
                Table::C
            }
        } else if per_ch <= 80 || self.sample_rate == 48000 {
            Table::A
        } else {
            Table::B
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// No valid header at the start of the data.
    NoSync,
    /// Fewer bytes than the header's frame length.
    Truncated,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Error::NoSync => "no MPEG-1 Layer II frame header",
            Error::Truncated => "truncated frame",
        })
    }
}

impl std::error::Error for Error {}

/// Offset of the first frame header at or after `from` that is followed by
/// another header exactly one frame later (or by the end of the data).
pub fn sync(data: &[u8], from: usize) -> Option<usize> {
    (from..data.len().saturating_sub(3)).find(|&i| match Header::parse(&data[i..]) {
        Some(h) => {
            let next = i + h.frame_len();
            next >= data.len() || Header::parse(&data[next..]).is_some_and(|n| n.sample_rate == h.sample_rate)
        }
        None => false,
    })
}

/// Quantisation class (Table B.4): number of steps, bits per codeword and
/// whether three samples share one codeword.
#[derive(Clone, Copy)]
struct Class {
    steps: u32,
    bits: u32,
    grouped: bool,
}

const fn class(steps: u32, bits: u32, grouped: bool) -> Class {
    Class { steps, bits, grouped }
}

const CLASSES: [Class; 17] = [
    class(3, 5, true),
    class(5, 7, true),
    class(7, 3, false),
    class(9, 10, true),
    class(15, 4, false),
    class(31, 5, false),
    class(63, 6, false),
    class(127, 7, false),
    class(255, 8, false),
    class(511, 9, false),
    class(1023, 10, false),
    class(2047, 11, false),
    class(4095, 12, false),
    class(8191, 13, false),
    class(16383, 14, false),
    class(32767, 15, false),
    class(65535, 16, false),
];

// Allocation code n (1-based) of a subband selects CLASSES[row[n - 1]]. The
// row length is 2^nbal - 1, which gives the width of the allocation field.
const AB_0_2: &[u8] = &[0, 2, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
const AB_3_10: &[u8] = &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 16];
const AB_11_22: &[u8] = &[0, 1, 2, 3, 4, 5, 16];
const AB_23_29: &[u8] = &[0, 1, 16];
const CD_0_1: &[u8] = &[0, 1, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15];
const CD_2_11: &[u8] = &[0, 1, 3, 4, 5, 6, 7];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Table {
    A,
    B,
    C,
    D,
}

impl Table {
    fn sblimit(self) -> usize {
        match self {
            Table::A => 27,
            Table::B => 30,
            Table::C => 8,
            Table::D => 12,
        }
    }

    fn row(self, sb: usize) -> &'static [u8] {
        match self {
            Table::A | Table::B => match sb {
                0..=2 => AB_0_2,
                3..=10 => AB_3_10,
                11..=22 => AB_11_22,
                _ => AB_23_29,
            },
            Table::C | Table::D => match sb {
                0..=1 => CD_0_1,
                _ => CD_2_11,
            },
        }
    }
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl Bits<'_> {
    /// Next `n` bits, MSB first; reads past the end return zeros.
    fn read(&mut self, n: u32) -> u32 {
        let mut v = 0;
        for _ in 0..n {
            let byte = self.data.get(self.pos >> 3).copied().unwrap_or(0);
            v = v << 1 | (byte >> (7 - (self.pos & 7)) & 1) as u32;
            self.pos += 1;
        }
        v
    }
}

/// Decoder state: the synthesis filterbank history of each channel.
pub struct Decoder {
    v: [[f64; 1024]; 2],
    pos: [usize; 2],
    /// Matrixing coefficients N[i][k] = cos((16 + i)(2k + 1)π / 64).
    n: Vec<[f64; 32]>,
    /// Synthesis window D[i] (Table B.3).
    d: Vec<f64>,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    pub fn new() -> Self {
        let n = (0..64)
            .map(|i| std::array::from_fn(|k| ((16 + i) as f64 * (2 * k + 1) as f64 * PI / 64.0).cos()))
            .collect();
        Decoder { v: [[0.0; 1024]; 2], pos: [0; 2], n, d: window::synthesis_window() }
    }

    /// Clear the filterbank history, as at the start of a new stream.
    pub fn reset(&mut self) {
        self.v = [[0.0; 1024]; 2];
    }

    /// Decode the frame at the start of `data` into `out` (1152 samples per
    /// channel; a mono frame fills both channels). Returns its header; the
    /// frame occupies `header.frame_len()` bytes.
    // Subband/channel loops index several parallel tables, as the standard writes them.
    #[allow(clippy::needless_range_loop)]
    pub fn decode_frame(&mut self, data: &[u8], out: &mut [[f32; FRAME_SAMPLES]; 2]) -> Result<Header, Error> {
        let h = Header::parse(data).ok_or(Error::NoSync)?;
        let len = h.frame_len();
        if data.len() < len {
            return Err(Error::Truncated);
        }
        let mut r = Bits { data: &data[..len], pos: if h.crc { 48 } else { 32 } };
        let table = h.table();
        let sblimit = table.sblimit();
        let nch = h.channels();
        let bound = if h.mode == Mode::JointStereo { (4 * (h.mode_ext as usize + 1)).min(sblimit) } else { sblimit };

        // Bit allocation: CLASSES index + 1, or 0 for an unused subband.
        let mut alloc = [[0u8; 32]; 2];
        for sb in 0..sblimit {
            let row = table.row(sb);
            let nbal = (row.len() as u32 + 1).trailing_zeros();
            for ch in 0..if sb < bound { nch } else { 1 } {
                let code = r.read(nbal) as usize;
                alloc[ch][sb] = if code == 0 { 0 } else { row[code - 1] + 1 };
            }
            if sb >= bound {
                alloc[1][sb] = alloc[0][sb];
            }
        }
        let mut scfsi = [[0u8; 32]; 2];
        for sb in 0..sblimit {
            for ch in 0..nch {
                if alloc[ch][sb] != 0 {
                    scfsi[ch][sb] = r.read(2) as u8;
                }
            }
        }
        // Scale factors for the three parts of the frame (granules 0-3, 4-7, 8-11).
        let mut scale = [[[0.0f64; 3]; 32]; 2];
        for sb in 0..sblimit {
            for ch in 0..nch {
                if alloc[ch][sb] == 0 {
                    continue;
                }
                let idx = match scfsi[ch][sb] {
                    0 => [r.read(6), r.read(6), r.read(6)],
                    1 => {
                        let (a, b) = (r.read(6), r.read(6));
                        [a, a, b]
                    }
                    2 => [r.read(6); 3],
                    _ => {
                        let (a, b) = (r.read(6), r.read(6));
                        [a, b, b]
                    }
                };
                scale[ch][sb] = idx.map(|i| 2f64.powf(1.0 - i as f64 / 3.0));
            }
        }

        for gr in 0..12 {
            let part = gr / 4;
            let mut sub = [[[0.0f64; 32]; 3]; 2];
            for sb in 0..sblimit {
                for ch in 0..if sb < bound { nch } else { 1 } {
                    let a = alloc[ch][sb];
                    if a == 0 {
                        continue;
                    }
                    let c = CLASSES[a as usize - 1];
                    let v = if c.grouped {
                        let mut w = r.read(c.bits);
                        std::array::from_fn(|_| {
                            let s = w % c.steps;
                            w /= c.steps;
                            s
                        })
                    } else {
                        [r.read(c.bits), r.read(c.bits), r.read(c.bits)]
                    };
                    // Requantise to (-1, 1): C · (s''' + D) of §2.4.3.3.5.
                    let q = v.map(|s| (2.0 * s as f64 + 1.0 - c.steps as f64) / c.steps as f64);
                    let chans = if sb < bound { ch..ch + 1 } else { 0..nch };
                    for k in chans {
                        for (i, &qi) in q.iter().enumerate() {
                            sub[k][i][sb] = qi * scale[k][sb][part];
                        }
                    }
                }
            }
            for (ch, sub) in sub.iter().enumerate().take(nch) {
                for (i, s) in sub.iter().enumerate() {
                    let at = (gr * 3 + i) * 32;
                    self.synth(ch, s, &mut out[ch][at..at + 32]);
                }
            }
        }
        if nch == 1 {
            out[1] = out[0];
        }
        Ok(h)
    }

    /// One step of the synthesis filterbank (Figure A.2): 32 subband samples
    /// in, 32 PCM samples out.
    fn synth(&mut self, ch: usize, s: &[f64; 32], out: &mut [f32]) {
        let pos = (self.pos[ch] + 1024 - 64) & 1023;
        self.pos[ch] = pos;
        let v = &mut self.v[ch];
        for (i, n) in self.n.iter().enumerate() {
            v[pos + i] = n.iter().zip(s).map(|(n, s)| n * s).sum();
        }
        let d = &self.d;
        for (j, o) in out.iter_mut().enumerate() {
            let mut acc = 0.0;
            for i in 0..8 {
                acc += d[i * 64 + j] * v[(pos + i * 128 + j) & 1023];
                acc += d[i * 64 + 32 + j] * v[(pos + i * 128 + 96 + j) & 1023];
            }
            *o = acc as f32;
        }
    }
}

// Save states: the synthesis history; the matrix and window are constant.
state::fields!(Decoder { v, pos });

#[cfg(test)]
mod tests {
    use super::*;

    /// MSB-first bit writer for building test frames.
    #[derive(Default)]
    struct Writer {
        b: Vec<u8>,
        n: usize,
    }

    impl Writer {
        fn put(&mut self, v: u32, bits: u32) {
            for i in (0..bits).rev() {
                if self.n.is_multiple_of(8) {
                    self.b.push(0);
                }
                if v >> i & 1 != 0 {
                    *self.b.last_mut().unwrap() |= 0x80 >> (self.n % 8);
                }
                self.n += 1;
            }
        }
    }

    /// 48 kHz 192 kbit/s (table A, 576-byte frames). `alloc` gives the
    /// 16-bit class (code 15) to listed subbands, with scale factor index
    /// `sf[ch]` and sample codes from `sample(ch, sb, slot)`.
    fn frame(
        mode: u32,
        ext: u32,
        alloc_sb: &[usize],
        sf: [u32; 2],
        sample: impl Fn(usize, usize, usize) -> u32,
    ) -> Vec<u8> {
        let mut w = Writer::default();
        w.put(0xfffd, 16);
        w.put(0b1010, 4);
        w.put(0b01, 2);
        w.put(0, 2);
        w.put(mode, 2);
        w.put(ext, 2);
        w.put(0, 4);
        let nch = if mode == 3 { 1 } else { 2 };
        let bound = if mode == 1 { 4 * (ext as usize + 1) } else { 27 };
        let nbal = |sb: usize| (Table::A.row(sb).len() as u32 + 1).trailing_zeros();
        for sb in 0..27 {
            for _ in 0..if sb < bound { nch } else { 1 } {
                let code = if alloc_sb.contains(&sb) { (1 << nbal(sb)) - 1 } else { 0 };
                w.put(code, nbal(sb));
            }
        }
        for &_sb in alloc_sb {
            for _ in 0..nch {
                w.put(2, 2);
            }
        }
        for _ in alloc_sb {
            for &s in &sf[..nch] {
                w.put(s, 6);
            }
        }
        for gr in 0..12 {
            for sb in 0..27 {
                if !alloc_sb.contains(&sb) {
                    continue;
                }
                for ch in 0..if sb < bound { nch } else { 1 } {
                    for i in 0..3 {
                        w.put(sample(ch, sb, gr * 3 + i), 16);
                    }
                }
            }
        }
        w.b.resize(576, 0);
        w.b
    }

    #[test]
    fn parses_topskatr_header() {
        let h = Header::parse(&[0xff, 0xfd, 0x88, 0x00]).unwrap();
        assert!(!h.crc);
        assert_eq!((h.bitrate, h.sample_rate, h.mode, h.padding), (128_000, 32_000, Mode::Stereo, false));
        assert_eq!(h.frame_len(), 576);
        assert_eq!(h.table(), Table::A);
    }

    #[test]
    fn header_fields_and_rejects() {
        // 44.1 kHz, 128 kbit/s, padded, joint stereo ext 2, with CRC.
        let h = Header::parse(&[0xff, 0xfc, 0x82, 0x60]).unwrap();
        assert!(h.crc && h.padding);
        assert_eq!((h.sample_rate, h.mode, h.mode_ext), (44100, Mode::JointStereo, 2));
        assert_eq!(h.frame_len(), 418);
        assert_eq!(h.table(), Table::A);
        assert_eq!(Header::parse(&[0xff, 0xfd, 0xa0, 0x00]).unwrap().table(), Table::B); // 44.1k 192k stereo
        assert_eq!(Header::parse(&[0xff, 0xfd, 0x18, 0xc0]).unwrap().table(), Table::D); // 32k 32k mono
        assert_eq!(Header::parse(&[0xff, 0xfd, 0x14, 0xc0]).unwrap().table(), Table::C); // 48k 32k mono
        assert!(Header::parse(&[0xff, 0xff, 0x88, 0x00]).is_none()); // layer I
        assert!(Header::parse(&[0xff, 0xf5, 0x88, 0x00]).is_none()); // MPEG-2
        assert!(Header::parse(&[0xff, 0xfd, 0x08, 0x00]).is_none()); // free format
        assert!(Header::parse(&[0xff, 0xfd, 0xf8, 0x00]).is_none()); // bad bitrate
        assert!(Header::parse(&[0xff, 0xfd, 0x8c, 0x00]).is_none()); // bad rate
        assert!(Header::parse(&[0xff, 0xfd]).is_none());
    }

    #[test]
    fn silence_decodes_to_zero() {
        let f = frame(0, 0, &[], [0, 0], |_, _, _| 0);
        let mut d = Decoder::new();
        let mut out = [[1.0; FRAME_SAMPLES]; 2];
        d.decode_frame(&f, &mut out).unwrap();
        assert!(out.iter().flatten().all(|&x| x == 0.0));
        // Midpoint codes (zero after requantisation) in a used subband too.
        let f = frame(3, 0, &[0, 5], [0, 0], |_, _, _| 32767);
        d.decode_frame(&f, &mut out).unwrap();
        assert!(out.iter().flatten().all(|&x| x == 0.0));
    }

    #[test]
    fn impulse_response_is_the_window() {
        // One sample in subband 0, slot 0: PCM sample 32s + j is
        // x · D[32s + j] · N[j + 32 (s odd)][0] for the next 16 slots.
        let f = frame(3, 0, &[0], [0, 0], |_, _, t| if t == 0 { 49151 } else { 32767 });
        let mut d = Decoder::new();
        let mut out = [[0.0; FRAME_SAMPLES]; 2];
        d.decode_frame(&f, &mut out).unwrap();
        let x = 2.0 * 32768.0 / 65535.0;
        let w = window::synthesis_window();
        for s in 0..16 {
            for j in 0..32 {
                let i = j + if s % 2 == 1 { 32 } else { 0 };
                let want = x * w[32 * s + j] * ((16 + i) as f64 * PI / 64.0).cos();
                assert!((out[0][32 * s + j] as f64 - want).abs() < 1e-6, "sample {}", 32 * s + j);
            }
        }
        assert!(out[0][512..].iter().all(|&y| y.abs() < 1e-9));
        // The window's centre tap, from the standard's table.
        assert!((w[256] - 1.144989014).abs() < 1e-9 && (w[32] + 0.000442505).abs() < 1e-9);
        assert_eq!(out[0], out[1]);
    }

    #[test]
    fn joint_stereo_shares_samples_above_bound() {
        // Bound 4: subband 10 is coded once and scaled per channel.
        let f = frame(1, 0, &[10], [0, 3], |_, _, t| 32767 + 1000 * (t as u32 % 7));
        let mut d = Decoder::new();
        let mut out = [[0.0; FRAME_SAMPLES]; 2];
        d.decode_frame(&f, &mut out).unwrap();
        let peak = out[0].iter().fold(0f32, |m, &x| m.max(x.abs()));
        assert!(peak > 0.01);
        for (l, r) in out[0].iter().zip(&out[1]) {
            assert!((l * 0.5 - r).abs() < 1e-6); // scale factor index 3 is half of index 0
        }
    }

    #[test]
    fn sinusoid_lands_in_its_subband() {
        // A tone in subband 6 (48 kHz: 4.5-5.25 kHz) comes out with its
        // energy there: correlate the output with a tone at the band centre.
        let f = |sb| frame(3, 0, &[sb], [0, 0], |_, _, t| (32767.0 + 16000.0 * ((t as f64) * PI * 0.5).cos()) as u32);
        let mut d = Decoder::new();
        let mut out = [[0.0; FRAME_SAMPLES]; 2];
        d.decode_frame(&f(6), &mut out).unwrap();
        d.decode_frame(&f(6), &mut out).unwrap();
        let power = |freq: f64| {
            let (mut c, mut s) = (0.0, 0.0);
            for (n, &y) in out[0].iter().enumerate() {
                let p = 2.0 * PI * freq * n as f64 / 48000.0;
                c += y as f64 * p.cos();
                s += y as f64 * p.sin();
            }
            c * c + s * s
        };
        let inband = (0..8).map(|k| power(4500.0 + 100.0 * k as f64)).fold(0.0, f64::max);
        let far = (0..8).map(|k| power(12000.0 + 100.0 * k as f64)).fold(0.0, f64::max);
        assert!(inband > 1000.0 * far, "{inband} vs {far}");
    }

    #[test]
    fn sync_finds_chained_frames() {
        let mut data = vec![0x12, 0xff, 0xfd];
        let f = frame(3, 0, &[], [0, 0], |_, _, _| 0);
        data.extend(&f);
        data.extend(&f);
        assert_eq!(sync(&data, 0), Some(3));
    }

    /// Decode the start of topskatr's first music track from the user's ROM
    /// dump (`REMODEL_ROM`, default `../../rom`): `cargo test -p remodel-mpeg -- --ignored`.
    #[test]
    #[ignore]
    fn topskatr_rom_decodes() {
        let dir = std::env::var("REMODEL_ROM").unwrap_or_else(|_| "../../rom".into());
        let rom = std::fs::read(format!("{dir}/mpr-19747.18s")).expect("ROM dump");
        let mut d = Decoder::new();
        let mut out = [[0.0; FRAME_SAMPLES]; 2];
        let (mut at, mut peak, mut sum, mut n) = (0x11000, 0f32, 0f64, 0);
        for _ in 0..500 {
            let h = d.decode_frame(&rom[at..], &mut out).unwrap();
            assert_eq!((h.bitrate, h.sample_rate, h.mode), (128_000, 32_000, Mode::Stereo));
            at += h.frame_len();
            for &x in out.iter().flatten() {
                peak = peak.max(x.abs());
                sum += (x as f64).powi(2);
                n += 1;
            }
        }
        let rms = (sum / n as f64).sqrt();
        assert!(peak < 1.1 && rms > 0.01, "peak {peak} rms {rms}");
    }
}
