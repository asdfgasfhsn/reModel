//! Audio files for replacement soundtracks: WAV and MPEG-1 Layer II readers
//! and a band-limited resampler.
//!
//! WAV: integer PCM of 8, 16, 24 or 32 bits, IEEE float of 32 or 64 bits,
//! plain or `WAVE_FORMAT_EXTENSIBLE`, any rate and channel count (mono is
//! duplicated to both sides; with more than two channels the first two are
//! used). A `smpl` chunk's first loop becomes the default loop region.
//! MP2: a raw stream of MPEG-1 Layer II frames, decoded with `crates/mpeg`.

use std::path::Path;
use std::sync::Arc;

/// Decoded audio: stereo frames at the file's own rate, full scale ±1.0.
#[derive(Clone, Debug, Default)]
pub struct Audio {
    pub rate: u32,
    /// Channels in the file (output is always stereo).
    pub channels: u16,
    /// A short description of the source format, for diagnostics.
    pub format: String,
    pub frames: Vec<[f32; 2]>,
    /// Loop region from the file itself (`smpl` chunk), in frames, end exclusive.
    pub file_loop: Option<(u64, u64)>,
}

impl Audio {
    pub fn seconds(&self) -> f64 {
        self.frames.len() as f64 / self.rate.max(1) as f64
    }
}

/// Load a `.wav` or `.mp2` file, chosen by its contents.
pub fn load(path: &Path) -> Result<Audio, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let r = if bytes.starts_with(b"RIFF") || bytes.starts_with(b"RF64") {
        parse_wav(&bytes)
    } else if mpeg::sync(&bytes, 0).is_some() {
        decode_mp2(&bytes)
    } else {
        Err("not a WAV or MPEG-1 Layer II file (convert it with `remodel soundtrack-import`)".into())
    };
    r.map_err(|e| format!("{}: {e}", path.display()))
}

fn u16_at(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes([b[i], b[i + 1]])
}

fn u32_at(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]])
}

/// Parse a RIFF/WAVE file.
pub fn parse_wav(b: &[u8]) -> Result<Audio, String> {
    if b.len() < 12 || &b[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".into());
    }
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut data: Option<&[u8]> = None;
    let mut file_loop = None;
    let mut i = 12;
    while i + 8 <= b.len() {
        let id = &b[i..i + 4];
        let size = u32_at(b, i + 4) as usize;
        let body = &b[i + 8..(i + 8).saturating_add(size).min(b.len())];
        match id {
            b"fmt " if body.len() >= 16 => {
                let mut tag = u16_at(body, 0);
                if tag == 0xfffe && body.len() >= 26 {
                    tag = u16_at(body, 24); // first two bytes of the sub-format GUID
                }
                fmt = Some((tag, u16_at(body, 2), u32_at(body, 4), u16_at(body, 14)));
            }
            b"data" => data = Some(body),
            b"smpl" if body.len() >= 36 + 24 && u32_at(body, 28) > 0 => {
                let (start, end) = (u32_at(body, 36 + 8) as u64, u32_at(body, 36 + 12) as u64);
                if end > start {
                    file_loop = Some((start, end + 1)); // `smpl` loop ends are inclusive
                }
            }
            _ => {}
        }
        // RF64/huge files may carry 0xFFFFFFFF as the data size: stop there.
        if id == b"data" && size == u32::MAX as usize {
            break;
        }
        i = i + 8 + size + (size & 1);
    }
    let (tag, channels, rate, bits) = fmt.ok_or("no fmt chunk")?;
    let data = data.ok_or("no data chunk")?;
    if channels == 0 || rate == 0 {
        return Err("zero channels or sample rate".into());
    }
    let sample: fn(&[u8]) -> f32 = match (tag, bits) {
        (1, 8) => |s| (s[0] as f32 - 128.0) / 128.0,
        (1, 16) => |s| i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
        (1, 24) => |s| (i32::from_le_bytes([0, s[0], s[1], s[2]]) >> 8) as f32 / 8_388_608.0,
        (1, 32) => |s| i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2_147_483_648.0,
        (3, 32) => |s| f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
        (3, 64) => |s| f64::from_le_bytes(s[..8].try_into().unwrap()) as f32,
        _ => return Err(format!("unsupported WAV encoding (format {tag:#x}, {bits} bits); convert it to 16-bit PCM")),
    };
    let width = bits as usize / 8;
    let block = width * channels as usize;
    let frames = data
        .chunks_exact(block)
        .map(|f| {
            let l = sample(&f[..width]);
            [l, if channels > 1 { sample(&f[width..2 * width]) } else { l }]
        })
        .collect();
    let kind = if tag == 3 { "float" } else { "PCM" };
    Ok(Audio { rate, channels, format: format!("WAV {bits}-bit {kind}"), frames, file_loop })
}

/// Decode a stream of MPEG-1 Layer II frames (leading junk such as an ID3 tag
/// is skipped).
pub fn decode_mp2(b: &[u8]) -> Result<Audio, String> {
    let mut at = mpeg::sync(b, 0).ok_or("no MPEG-1 Layer II frames")?;
    let mut dec = mpeg::Decoder::new();
    let mut pcm = Box::new([[0.0; mpeg::FRAME_SAMPLES]; 2]);
    let mut a = Audio::default();
    while let Ok(h) = dec.decode_frame(&b[at..], &mut pcm) {
        if a.rate == 0 {
            a.rate = h.sample_rate;
            a.channels = h.channels() as u16;
            a.format = format!("MP2 {} kbit/s", h.bitrate / 1000);
        } else if h.sample_rate != a.rate {
            break;
        }
        a.frames.extend((0..mpeg::FRAME_SAMPLES).map(|i| [pcm[0][i], pcm[1][i]]));
        at += h.frame_len();
    }
    if a.frames.is_empty() {
        return Err("no decodable MPEG-1 Layer II frames".into());
    }
    Ok(a)
}

/// A windowed-sinc interpolation kernel for one rate ratio, tabulated at
/// `PHASES` fractional positions (linearly interpolated between them).
#[derive(Debug)]
pub struct Kernel {
    /// Taps either side of the interpolation point.
    half: usize,
    /// `(PHASES + 1)` rows of `2 * half` taps, each row summing to 1.
    table: Vec<f32>,
}

const PHASES: usize = 256;
/// Zero crossings of the sinc either side, at the lower of the two rates.
const ZEROS: f64 = 16.0;

impl Kernel {
    pub fn new(in_rate: u32, out_rate: u32) -> Kernel {
        // Cut off a little below the lower Nyquist frequency.
        let fc = 0.95 * (out_rate as f64 / in_rate as f64).min(1.0);
        let half = (ZEROS / fc).ceil() as usize;
        let mut table = Vec::with_capacity((PHASES + 1) * 2 * half);
        for p in 0..=PHASES {
            let frac = p as f64 / PHASES as f64;
            let row: Vec<f64> = (0..2 * half)
                .map(|k| {
                    let x = k as f64 - (half as f64 - 1.0) - frac;
                    let t = x / half as f64;
                    let w =
                        0.42 + 0.5 * (std::f64::consts::PI * t).cos() + 0.08 * (2.0 * std::f64::consts::PI * t).cos();
                    let s = if x == 0.0 {
                        1.0
                    } else {
                        (std::f64::consts::PI * fc * x).sin() / (std::f64::consts::PI * fc * x)
                    };
                    if t.abs() >= 1.0 {
                        0.0
                    } else {
                        s * w
                    }
                })
                .collect();
            let sum: f64 = row.iter().sum();
            table.extend(row.iter().map(|v| (v / sum) as f32));
        }
        Kernel { half, table }
    }
}

/// How a replacement track loops.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LoopMode {
    /// Loop when the game arms looping for the track (the default).
    #[default]
    Game,
    /// Always loop.
    On,
    /// Never loop.
    Off,
}

/// A replacement track being played: resampled to the output rate on the
/// fly, looping between `loop_start` and `loop_end` (in source frames).
pub struct Voice {
    pub track: u8,
    audio: Arc<Audio>,
    kernel: Option<Arc<Kernel>>,
    pos: f64,
    step: f64,
    loop_start: u64,
    loop_end: u64,
    /// The position has wrapped at least once: taps before `loop_start`
    /// come from the end of the loop.
    wrapped: bool,
    pub gain: f32,
    pub mode: LoopMode,
}

/// What [`Voice::next`] produced.
pub enum Step {
    Sample([f32; 2]),
    /// A sample, and the position just wrapped to the loop start.
    Looped([f32; 2]),
    /// The end was reached without looping.
    Ended,
}

impl Voice {
    /// `loop_region` is clamped to the audio; `None` loops the whole file.
    pub fn new(track: u8, audio: Arc<Audio>, out_rate: u32, loop_region: Option<(u64, u64)>, gain: f32) -> Voice {
        let len = audio.frames.len() as u64;
        let (mut ls, mut le) = loop_region.or(audio.file_loop).unwrap_or((0, len));
        le = le.min(len);
        if ls >= le {
            (ls, le) = (0, len);
        }
        let kernel = (audio.rate != out_rate).then(|| Arc::new(Kernel::new(audio.rate, out_rate)));
        Voice {
            track,
            step: audio.rate as f64 / out_rate as f64,
            audio,
            kernel,
            pos: 0.0,
            loop_start: ls,
            loop_end: le,
            wrapped: false,
            gain,
            mode: LoopMode::Game,
        }
    }

    /// Source frame `j`, following the loop when `looping`.
    fn at(&self, mut j: i64, looping: bool) -> [f32; 2] {
        let (ls, le) = (self.loop_start as i64, self.loop_end as i64);
        if looping && le > ls {
            if j >= le {
                j = ls + (j - le) % (le - ls);
            } else if self.wrapped && j < ls {
                j += le - ls;
            }
        }
        if j < 0 || j as usize >= self.audio.frames.len() {
            [0.0; 2]
        } else {
            self.audio.frames[j as usize]
        }
    }

    /// The next output sample (before gain). `game_loop` is whether the game
    /// has armed looping; the voice's [`LoopMode`] decides what it means.
    pub fn next(&mut self, game_loop: bool) -> Step {
        let looping = match self.mode {
            LoopMode::Game => game_loop,
            LoopMode::On => true,
            LoopMode::Off => false,
        };
        let mut wrapped_now = false;
        if looping && self.pos >= self.loop_end as f64 && self.loop_end > self.loop_start {
            self.pos -= (self.loop_end - self.loop_start) as f64;
            self.wrapped = true;
            wrapped_now = true;
        }
        if self.pos >= self.audio.frames.len() as f64 {
            return Step::Ended;
        }
        let i = self.pos.floor() as i64;
        let s = match &self.kernel {
            None => self.at(i, looping),
            Some(k) => {
                let f = (self.pos - i as f64) * PHASES as f64;
                let p = (f as usize).min(PHASES - 1);
                let t = (f - p as f64) as f32;
                let n = 2 * k.half;
                let (r0, r1) = (&k.table[p * n..][..n], &k.table[(p + 1) * n..][..n]);
                let base = i - (k.half as i64 - 1);
                let mut acc = [0.0f32; 2];
                for (j, (a, b)) in r0.iter().zip(r1).enumerate() {
                    let w = a + (b - a) * t;
                    let s = self.at(base + j as i64, looping);
                    acc[0] += w * s[0];
                    acc[1] += w * s[1];
                }
                acc
            }
        };
        self.pos += self.step;
        if wrapped_now {
            Step::Looped(s)
        } else {
            Step::Sample(s)
        }
    }
}

/// Encode stereo or mono frames as a WAV file, for tests: `fmt` is 1 (PCM)
/// or 3 (float), `bits` the sample width.
#[cfg(test)]
pub(crate) fn encode_wav(rate: u32, channels: u16, fmt: u16, bits: u16, samples: &[f32]) -> Vec<u8> {
    let width = bits as usize / 8;
    let mut data = Vec::new();
    for &s in samples {
        match (fmt, bits) {
            (1, 8) => data.push(((s * 127.0).round() as i32 + 128) as u8),
            (1, 16) => data.extend(((s * 32767.0).round() as i16).to_le_bytes()),
            (1, 24) => data.extend(&((s * 8_388_607.0).round() as i32).to_le_bytes()[..3]),
            (3, 32) => data.extend(s.to_le_bytes()),
            _ => unreachable!(),
        }
    }
    let mut b = Vec::new();
    b.extend(b"RIFF");
    b.extend((36 + data.len() as u32).to_le_bytes());
    b.extend(b"WAVEfmt ");
    b.extend(16u32.to_le_bytes());
    b.extend(fmt.to_le_bytes());
    b.extend(channels.to_le_bytes());
    b.extend(rate.to_le_bytes());
    b.extend((rate * (width * channels as usize) as u32).to_le_bytes());
    b.extend(((width * channels as usize) as u16).to_le_bytes());
    b.extend(bits.to_le_bytes());
    b.extend(b"data");
    b.extend((data.len() as u32).to_le_bytes());
    b.extend(data);
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sine(rate: u32, hz: f64, secs: f64, amp: f64) -> Vec<f32> {
        (0..(rate as f64 * secs) as usize)
            .map(|i| (amp * (2.0 * std::f64::consts::PI * hz * i as f64 / rate as f64).sin()) as f32)
            .collect()
    }

    #[test]
    fn wav_formats() {
        let mono = sine(22050, 441.0, 0.1, 0.5);
        for (fmt, bits, tol) in [(1, 8, 0.01), (1, 16, 1e-4), (1, 24, 1e-6), (3, 32, 0.0)] {
            let a = parse_wav(&encode_wav(22050, 1, fmt, bits, &mono)).unwrap();
            assert_eq!((a.rate, a.channels, a.frames.len()), (22050, 1, mono.len()), "{bits}");
            for (f, &s) in a.frames.iter().zip(&mono) {
                assert!((f[0] - s).abs() <= tol && f[0] == f[1], "{bits}-bit: {} vs {s}", f[0]);
            }
        }
        let stereo: Vec<f32> = (0..100).flat_map(|i| [i as f32 / 200.0, -(i as f32) / 200.0]).collect();
        let a = parse_wav(&encode_wav(48000, 2, 1, 16, &stereo)).unwrap();
        assert_eq!(a.frames.len(), 100);
        assert!((a.frames[50][0] - 0.25).abs() < 1e-4 && (a.frames[50][1] + 0.25).abs() < 1e-4);
        assert!(parse_wav(b"RIFF\0\0\0\0WAVE").is_err());
    }

    #[test]
    fn wav_extensible_and_smpl_loop() {
        let mut b = encode_wav(32000, 1, 1, 16, &[0.0; 1000]);
        // Turn the fmt chunk into WAVE_FORMAT_EXTENSIBLE (40 bytes) with a PCM sub-format.
        let mut ext = b[20..36].to_vec();
        ext[0..2].copy_from_slice(&0xfffeu16.to_le_bytes());
        ext.extend(22u16.to_le_bytes());
        ext.extend(16u16.to_le_bytes());
        ext.extend(0u32.to_le_bytes());
        ext.extend([1, 0, 0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71]);
        let mut smpl = vec![0u8; 36 + 24];
        smpl[28..32].copy_from_slice(&1u32.to_le_bytes());
        smpl[44..48].copy_from_slice(&100u32.to_le_bytes());
        smpl[48..52].copy_from_slice(&899u32.to_le_bytes());
        let mut out = b[..12].to_vec();
        out.extend(b"fmt ");
        out.extend((ext.len() as u32).to_le_bytes());
        out.extend(&ext);
        out.extend(b"smpl");
        out.extend((smpl.len() as u32).to_le_bytes());
        out.extend(&smpl);
        out.extend(b.split_off(36));
        let a = parse_wav(&out).unwrap();
        assert_eq!((a.rate, a.frames.len(), a.file_loop), (32000, 1000, Some((100, 900))));
    }

    /// Amplitude and frequency of `x` at `hz` (single-bin DFT).
    fn tone(x: &[f32], rate: f64, hz: f64) -> f64 {
        let (mut re, mut im) = (0.0, 0.0);
        for (i, &v) in x.iter().enumerate() {
            let a = 2.0 * std::f64::consts::PI * hz * i as f64 / rate;
            re += v as f64 * a.cos();
            im += v as f64 * a.sin();
        }
        2.0 * (re * re + im * im).sqrt() / x.len() as f64
    }

    #[test]
    fn resampling_keeps_pitch_and_level() {
        for rate in [22050, 32000, 48000, 96000] {
            let s = sine(rate, 1000.0, 1.0, 0.5);
            let a =
                Arc::new(Audio { rate, channels: 1, frames: s.iter().map(|&v| [v, v]).collect(), ..Audio::default() });
            let mut v = Voice::new(1, a, 44100, None, 1.0);
            let mut out = vec![];
            while let Step::Sample(s) = v.next(false) {
                out.push(s[0]);
            }
            assert!((out.len() as i64 - 44100).abs() <= 1, "{rate}: {} samples", out.len());
            // Skip the edges, where the kernel runs off the ends of the file.
            let mid = &out[2000..42000];
            let amp = tone(mid, 44100.0, 1000.0);
            assert!((amp - 0.5).abs() < 0.005, "{rate}: amplitude {amp}");
            let peak = mid.iter().fold(0.0f32, |m, v| m.max(v.abs()));
            assert!((peak - 0.5).abs() < 0.01, "{rate}: peak {peak}");
        }
    }

    #[test]
    fn resampling_rejects_aliases() {
        // 30 kHz at 96 kHz would alias to 14.1 kHz at 44.1 kHz.
        let s = sine(96000, 30000.0, 0.5, 0.5);
        let a = Arc::new(Audio {
            rate: 96000,
            channels: 1,
            frames: s.iter().map(|&v| [v, v]).collect(),
            ..Audio::default()
        });
        let mut v = Voice::new(1, a, 44100, None, 1.0);
        let mut out = vec![];
        while let Step::Sample(s) = v.next(false) {
            out.push(s[0]);
        }
        let alias = tone(&out[2000..20000], 44100.0, 14100.0);
        assert!(alias < 0.5 * 0.001, "alias {alias}");
    }

    #[test]
    fn same_rate_is_bit_exact() {
        let frames: Vec<[f32; 2]> = (0..500).map(|i| [i as f32 / 500.0, -(i as f32) / 500.0]).collect();
        let a = Arc::new(Audio { rate: 44100, channels: 2, frames: frames.clone(), ..Audio::default() });
        let mut v = Voice::new(1, a, 44100, None, 1.0);
        for f in &frames {
            assert!(matches!(v.next(false), Step::Sample(s) if s == *f));
        }
        assert!(matches!(v.next(false), Step::Ended));
    }

    #[test]
    fn loops_between_points_only_when_armed() {
        let frames: Vec<[f32; 2]> = (0..100).map(|i| [i as f32, 0.0]).collect();
        let a = Arc::new(Audio { rate: 44100, channels: 2, frames, ..Audio::default() });
        // Looping: 0..80, then 20..80 forever.
        let mut v = Voice::new(1, a.clone(), 44100, Some((20, 80)), 1.0);
        let mut got = vec![];
        let mut loops = 0;
        for _ in 0..200 {
            match v.next(true) {
                Step::Sample(s) => got.push(s[0]),
                Step::Looped(s) => {
                    loops += 1;
                    got.push(s[0])
                }
                Step::Ended => panic!("ended while looping"),
            }
        }
        let want: Vec<f32> = (0..80).chain((20..80).cycle()).take(200).map(|i| i as f32).collect();
        assert_eq!(got, want);
        assert_eq!(loops, 2);
        // Not armed: runs past the loop end to the end of the file.
        let mut v = Voice::new(1, a, 44100, Some((20, 80)), 1.0);
        let n = std::iter::from_fn(|| matches!(v.next(false), Step::Sample(_)).then_some(())).count();
        assert_eq!(n, 100);
    }

    #[test]
    fn resampled_loop_is_seamless() {
        // A 1 kHz sine whose loop region is a whole number of periods at
        // 48 kHz: the resampled output must stay a clean sine across the seam.
        let s = sine(48000, 1000.0, 1.0, 0.5);
        let a = Arc::new(Audio {
            rate: 48000,
            channels: 1,
            frames: s.iter().map(|&v| [v, v]).collect(),
            ..Audio::default()
        });
        let mut v = Voice::new(1, a, 44100, Some((4800, 24000)), 1.0);
        let mut out = vec![];
        for _ in 0..3 * 44100 {
            match v.next(true) {
                Step::Sample(s) | Step::Looped(s) => out.push(s[0]),
                Step::Ended => panic!(),
            }
        }
        // Compare against an ideal sine continuing through the loops.
        let start = 5000; // past the first 4800 source frames
        let mut worst = 0.0f64;
        for (i, &o) in out.iter().enumerate().skip(start) {
            let src = (i as f64 * 48000.0 / 44100.0 - 4800.0).rem_euclid(19200.0) + 4800.0;
            let ideal = 0.5 * (2.0 * std::f64::consts::PI * 1000.0 * src / 48000.0).sin();
            worst = worst.max((o as f64 - ideal).abs());
        }
        assert!(worst < 2e-3, "worst error {worst}");
    }
}
