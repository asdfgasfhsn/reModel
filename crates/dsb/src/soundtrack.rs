//! Replacement soundtracks: a manifest (`tracks.txt`) that maps the game's
//! music tracks to audio files. The firmware still decides when a track
//! starts, stops, loops, changes volume and fades; the decoder plays the
//! file in place of the ROM range the firmware asked for. Format:
//! `docs/mods.md`.

use crate::audio::{self, Audio, LoopMode, Voice};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// A point in a replacement file.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pos {
    Seconds(f64),
    /// Frames at the file's own rate.
    Samples(u64),
}

impl std::fmt::Display for Pos {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Pos::Seconds(s) => write!(f, "{s} s"),
            Pos::Samples(n) => write!(f, "{n} samples"),
        }
    }
}

impl Pos {
    fn frames(self, rate: u32) -> u64 {
        match self {
            Pos::Seconds(s) => (s * rate as f64).round() as u64,
            Pos::Samples(n) => n,
        }
    }
}

/// One `[track N]` entry of the manifest.
#[derive(Clone, Debug, PartialEq)]
pub struct TrackSpec {
    pub track: u8,
    pub file: PathBuf,
    pub loop_mode: LoopMode,
    pub loop_start: Option<Pos>,
    pub loop_end: Option<Pos>,
    /// Linear gain applied on top of the game's volume.
    pub gain: f32,
    /// MPEG ROM range `[start, end)` to replace; `None` takes it from the
    /// firmware's track table.
    pub range: Option<(u32, u32)>,
}

impl TrackSpec {
    fn new(track: u8) -> Self {
        TrackSpec {
            track,
            file: PathBuf::new(),
            loop_mode: LoopMode::Game,
            loop_start: None,
            loop_end: None,
            gain: 1.0,
            range: None,
        }
    }
}

/// A parsed manifest.
#[derive(Clone, Debug, Default)]
pub struct Manifest {
    pub title: Option<String>,
    pub tracks: Vec<TrackSpec>,
}

fn parse_pos(v: &str) -> Result<Pos, String> {
    let v = v.trim();
    if let Some(n) = v.strip_suffix("samples").or_else(|| v.strip_suffix("smp")) {
        return n.trim().parse().map(Pos::Samples).map_err(|_| format!("bad sample count `{v}`"));
    }
    let secs = v.strip_suffix('s').unwrap_or(v).trim();
    // [[h:]m:]s.frac
    let mut t = 0.0;
    for part in secs.split(':') {
        let x: f64 = part.parse().map_err(|_| format!("bad time `{v}` (use seconds, m:ss.s or `N samples`)"))?;
        if x.is_nan() || x < 0.0 {
            return Err(format!("bad time `{v}`"));
        }
        t = t * 60.0 + x;
    }
    Ok(Pos::Seconds(t))
}

fn parse_gain(v: &str) -> Result<f32, String> {
    let l = v.trim().to_ascii_lowercase();
    let r = match l.strip_suffix("db") {
        Some(d) => d.trim().parse::<f32>().map(|d| 10f32.powf(d / 20.0)),
        None => l.parse::<f32>(),
    };
    r.ok().filter(|g| g.is_finite() && *g >= 0.0).ok_or_else(|| format!("bad gain `{v}` (e.g. `0.8` or `-3 dB`)"))
}

fn parse_addr(v: &str) -> Result<u32, String> {
    let v = v.trim();
    let r = match v.strip_prefix("0x").or_else(|| v.strip_prefix("0X")) {
        Some(h) => u32::from_str_radix(h, 16),
        None => v.parse(),
    };
    r.map_err(|_| format!("bad address `{v}`"))
}

impl Manifest {
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let mut m = Manifest::default();
        let mut cur: Option<usize> = None;
        for (n, raw) in text.lines().enumerate() {
            let err = |e: String| format!("line {}: {e}", n + 1);
            let line = raw.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if let Some(h) = line.strip_prefix('[') {
                let h = h.strip_suffix(']').ok_or_else(|| err("unclosed `[`".into()))?;
                let num = h.trim().strip_prefix("track").ok_or_else(|| err(format!("unknown section `[{h}]`")))?;
                let t: u8 =
                    num.trim().parse().ok().filter(|&t| t > 0).ok_or_else(|| err(format!("bad track `{h}`")))?;
                if m.tracks.iter().any(|s| s.track == t) {
                    return Err(err(format!("track {t} given twice")));
                }
                m.tracks.push(TrackSpec::new(t));
                cur = Some(m.tracks.len() - 1);
                continue;
            }
            let (k, v) = line.split_once('=').ok_or_else(|| err("expected `key = value`".into()))?;
            let (k, v) = (k.trim(), v.trim());
            // Shorthand outside a section: `track 3 = file.wav`.
            if let Some(num) = k.strip_prefix("track ") {
                let t: u8 =
                    num.trim().parse().ok().filter(|&t| t > 0).ok_or_else(|| err(format!("bad track `{k}`")))?;
                if m.tracks.iter().any(|s| s.track == t) {
                    return Err(err(format!("track {t} given twice")));
                }
                m.tracks.push(TrackSpec { file: v.into(), ..TrackSpec::new(t) });
                cur = None;
                continue;
            }
            let Some(i) = cur else {
                match k {
                    "title" => m.title = Some(v.to_string()),
                    _ => return Err(err(format!("unknown key `{k}` outside a [track N] section"))),
                }
                continue;
            };
            let s = &mut m.tracks[i];
            match k {
                "file" => s.file = v.into(),
                "loop" => {
                    s.loop_mode = match v {
                        "game" => LoopMode::Game,
                        "on" | "yes" | "always" => LoopMode::On,
                        "off" | "no" | "never" => LoopMode::Off,
                        _ => return Err(err(format!("bad loop `{v}` (game, on or off)"))),
                    }
                }
                "loop_start" => s.loop_start = Some(parse_pos(v).map_err(err)?),
                "loop_end" => s.loop_end = Some(parse_pos(v).map_err(err)?),
                "gain" => s.gain = parse_gain(v).map_err(err)?,
                "range" => {
                    let (a, b) = v.split_once('-').ok_or_else(|| err(format!("bad range `{v}` (START-END)")))?;
                    let (a, b) = (parse_addr(a).map_err(err)?, parse_addr(b).map_err(err)?);
                    if a >= b {
                        return Err(err(format!("empty range `{v}`")));
                    }
                    s.range = Some((a, b));
                }
                _ => return Err(err(format!("unknown key `{k}`"))),
            }
        }
        if let Some(s) = m.tracks.iter().find(|s| s.file.as_os_str().is_empty()) {
            return Err(format!("track {} has no `file`", s.track));
        }
        Ok(m)
    }
}

/// A replaced track, as resolved against the firmware.
#[derive(Clone, Debug)]
pub struct Replacement {
    pub spec: TrackSpec,
    /// Full path of the audio file.
    pub path: PathBuf,
    /// MPEG ROM range it stands in for.
    pub start: u32,
    pub end: u32,
    /// What the file contains, for diagnostics.
    pub info: String,
}

/// A loaded soundtrack, ready to hand to the decoder.
#[derive(Debug, Default)]
pub struct Soundtrack {
    pub title: Option<String>,
    pub replacements: Vec<Replacement>,
    /// Recently used audio, most recent last.
    cache: Vec<(u8, Arc<Audio>)>,
    /// Problems met while playing (a file that stopped loading).
    pub errors: Vec<String>,
}

/// Decoded tracks kept in memory: the playing one and the one before.
const CACHE: usize = 2;

impl Soundtrack {
    /// Read `dir/tracks.txt` and check every file against the firmware's
    /// track table (`table[n - 1]` is track n's ROM range). Each file is
    /// decoded once here to validate it.
    pub fn load(dir: &Path, table: &[(u32, u32)]) -> Result<Soundtrack, String> {
        let path = dir.join("tracks.txt");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let m = Manifest::parse(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        Soundtrack::resolve(m, dir, table)
    }

    pub fn resolve(m: Manifest, dir: &Path, table: &[(u32, u32)]) -> Result<Soundtrack, String> {
        let mut st = Soundtrack { title: m.title, ..Soundtrack::default() };
        for spec in m.tracks {
            let (start, end) = match spec.range {
                Some(r) => r,
                None => *table.get(spec.track as usize - 1).ok_or_else(|| {
                    format!(
                        "track {}: the firmware's track table has {} tracks; give a `range` for it",
                        spec.track,
                        table.len()
                    )
                })?,
            };
            let path = dir.join(&spec.file);
            let a = audio::load(&path)?;
            let info = format!(
                "{}, {} Hz {}, {:.1} s{}",
                a.format,
                a.rate,
                if a.channels == 1 { "mono" } else { "stereo" },
                a.seconds(),
                if a.file_loop.is_some() { ", smpl loop" } else { "" }
            );
            st.replacements.push(Replacement { spec, path, start, end, info });
        }
        Ok(st)
    }

    fn find(&self, start: u32, end: u32) -> Option<&Replacement> {
        let r = &self.replacements;
        r.iter().find(|r| (r.start, r.end) == (start, end)).or_else(|| r.iter().find(|r| r.start == start))
    }

    /// The voice for a play of `[start, end)`, if that range is replaced.
    /// `Some(Err)` when the file can no longer be loaded.
    pub fn voice(&mut self, start: u32, end: u32, out_rate: u32) -> Option<Result<Voice, String>> {
        let r = self.find(start, end)?.clone();
        let t = r.spec.track;
        let a = match self.cache.iter().position(|c| c.0 == t) {
            Some(i) => self.cache.remove(i).1,
            None => match audio::load(&r.path) {
                Ok(a) => Arc::new(a),
                Err(e) => {
                    self.errors.push(e.clone());
                    return Some(Err(e));
                }
            },
        };
        self.cache.push((t, a.clone()));
        if self.cache.len() > CACHE {
            self.cache.remove(0);
        }
        let region = match (r.spec.loop_start, r.spec.loop_end) {
            (None, None) => None,
            (s, e) => Some((
                s.map_or(a.file_loop.map_or(0, |l| l.0), |p| p.frames(a.rate)),
                e.map_or(a.file_loop.map_or(a.frames.len() as u64, |l| l.1), |p| p.frames(a.rate)),
            )),
        };
        let mut v = Voice::new(t, a, out_rate, region, r.spec.gain);
        v.mode = r.spec.loop_mode;
        Some(Ok(v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest() {
        let m = Manifest::parse(
            "# demo\n\
             title = My mix\n\
             track 4 = attract.wav\n\
             [track 1]\n\
             file = title #1 theme.wav\n\
             loop_start = 1:02.5\n\
             loop_end = 4410000 samples\n\
             gain = -6 dB\n\
             [track 12]\n\
             file = ending.mp2\n\
             loop = off\n\
             gain = 0.5\n\
             range = 0xCC7708-0xEC8948\n",
        )
        .unwrap();
        assert_eq!(m.title.as_deref(), Some("My mix"));
        assert_eq!(m.tracks.len(), 3);
        assert_eq!((m.tracks[0].track, m.tracks[0].file.to_str()), (4, Some("attract.wav")));
        let t1 = &m.tracks[1];
        assert_eq!(t1.file.to_str(), Some("title #1 theme.wav"));
        assert_eq!(t1.loop_start, Some(Pos::Seconds(62.5)));
        assert_eq!(t1.loop_end, Some(Pos::Samples(4_410_000)));
        assert!((t1.gain - 0.501).abs() < 0.001);
        assert_eq!(t1.loop_mode, LoopMode::Game);
        let t12 = &m.tracks[2];
        assert_eq!((t12.loop_mode, t12.gain, t12.range), (LoopMode::Off, 0.5, Some((0xcc7708, 0xec8948))));
        assert_eq!(parse_pos("12.5s"), Ok(Pos::Seconds(12.5)));
        assert_eq!(parse_pos("1:00:00"), Ok(Pos::Seconds(3600.0)));
    }

    #[test]
    fn manifest_errors() {
        for (text, want) in [
            ("[track 1]\nfile = a.wav\n[track 1]\nfile = b.wav", "line 3: track 1 given twice"),
            ("[track 1]\nfile = a.wav\nvolume = 3", "line 3: unknown key `volume`"),
            ("[track 2]\nloop = maybe", "line 2: bad loop"),
            ("[track 2]\ngain = loud", "line 2: bad gain"),
            ("[track 0]", "line 1: bad track"),
            ("[track 3]\nloop = on", "track 3 has no `file`"),
            ("file = a.wav", "line 1: unknown key `file` outside"),
            ("[track 1]\nfile = a\nrange = 0x20-0x10", "line 3: empty range"),
        ] {
            let e = Manifest::parse(text).unwrap_err();
            assert!(e.starts_with(want), "{text:?}: {e}");
        }
    }

    #[test]
    fn resolve_against_table() {
        let dir = std::env::temp_dir().join(format!("remodel-soundtrack-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = audio::encode_wav(22050, 1, 1, 16, &[0.25; 22050]);
        std::fs::write(dir.join("a.wav"), wav).unwrap();
        std::fs::write(dir.join("tracks.txt"), "track 2 = a.wav\n[track 5]\nfile = a.wav\nrange = 0x100-0x200\n")
            .unwrap();
        let table = [(0x1000, 0x2000), (0x3000, 0x4000)];
        let mut st = Soundtrack::load(&dir, &table).unwrap();
        assert_eq!(st.replacements.len(), 2);
        assert_eq!((st.replacements[0].start, st.replacements[0].end), (0x3000, 0x4000));
        assert!(st.replacements[0].info.contains("22050 Hz mono, 1.0 s"), "{}", st.replacements[0].info);
        assert!(st.voice(0x1000, 0x2000, 44100).is_none());
        assert!(matches!(st.voice(0x3000, 0x4000, 44100), Some(Ok(v)) if v.track == 2));
        assert!(matches!(st.voice(0x100, 0x200, 44100), Some(Ok(v)) if v.track == 5));
        // A track beyond the table needs an explicit range.
        std::fs::write(dir.join("tracks.txt"), "track 3 = a.wav\n").unwrap();
        assert!(Soundtrack::load(&dir, &table).unwrap_err().contains("give a `range`"));
        std::fs::write(dir.join("tracks.txt"), "track 1 = missing.wav\n").unwrap();
        assert!(Soundtrack::load(&dir, &table).unwrap_err().contains("missing.wav"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
