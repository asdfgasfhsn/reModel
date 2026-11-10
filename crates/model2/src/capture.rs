//! Captured frames: everything the renderers read for one frame, saved so
//! a renderer can be run and compared offline (`remodel run --capture`,
//! `remodel gpu-check`).
//!
//! A capture directory holds `texture-rom.bin` (the texture ROM, shared by
//! every frame) and one subdirectory per frame with the raster stream, the
//! video RAMs and the sync offsets, all little-endian. Derived from the
//! game: keep captures under `out/`.

use crate::raster::Sources;
use std::path::Path;

pub struct Frame {
    pub frame: u64,
    /// The geometrizer's raster stream.
    pub stream: Vec<u32>,
    pub texture_point_ram: Vec<u16>,
    pub render_ram: Vec<u8>,
    pub palette: Vec<u8>,
    pub tiles: Vec<u8>,
    pub xoff: f32,
    pub yoff: f32,
}

fn words16(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect()
}

fn bytes16(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

impl Frame {
    /// The current frame of `bus` (waits for the geometrizer to finish it).
    pub fn grab(bus: &mut crate::Model2Bus) -> Frame {
        bus.geo.sync();
        let (xoff, yoff) = bus.sync_offsets();
        let geo = bus.geo.engine();
        Frame {
            frame: bus.frames,
            stream: geo.stream.clone(),
            texture_point_ram: geo.texture_point_ram.clone(),
            render_ram: bus.render_ram().to_vec(),
            palette: bus.palette_ram().to_vec(),
            tiles: bus.tile_ram().to_vec(),
            xoff,
            yoff,
        }
    }

    /// The renderer's view of this frame.
    pub fn sources<'a>(&'a self, texture_rom: &'a [u16]) -> Sources<'a> {
        Sources {
            texture_rom,
            texture_point_ram: &self.texture_point_ram,
            render_ram: &self.render_ram,
            palette: &self.palette,
            xoff: self.xoff,
            yoff: self.yoff,
        }
    }

    /// Save as `root/NNNNNN/`, writing `root/texture-rom.bin` if missing.
    pub fn save(&self, root: &Path, texture_rom: &[u16]) -> std::io::Result<()> {
        let rom = root.join("texture-rom.bin");
        if !rom.exists() {
            std::fs::create_dir_all(root)?;
            std::fs::write(rom, bytes16(texture_rom))?;
        }
        let dir = root.join(format!("{:06}", self.frame));
        std::fs::create_dir_all(&dir)?;
        let stream: Vec<u8> = self.stream.iter().flat_map(|w| w.to_le_bytes()).collect();
        std::fs::write(dir.join("stream.bin"), stream)?;
        std::fs::write(dir.join("points.bin"), bytes16(&self.texture_point_ram))?;
        std::fs::write(dir.join("render.bin"), &self.render_ram)?;
        std::fs::write(dir.join("palette.bin"), &self.palette)?;
        std::fs::write(dir.join("tiles.bin"), &self.tiles)?;
        std::fs::write(dir.join("meta.txt"), format!("frame {}\nxoff {}\nyoff {}\n", self.frame, self.xoff, self.yoff))
    }

    pub fn load(dir: &Path) -> std::io::Result<Frame> {
        let read = |f: &str| std::fs::read(dir.join(f));
        let meta = std::fs::read_to_string(dir.join("meta.txt"))?;
        let field = |k: &str| {
            meta.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.trim().parse::<f64>().ok())).unwrap_or(0.0)
        };
        Ok(Frame {
            frame: field("frame ") as u64,
            stream: read("stream.bin")?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect(),
            texture_point_ram: words16(&read("points.bin")?),
            render_ram: read("render.bin")?,
            palette: read("palette.bin")?,
            tiles: read("tiles.bin")?,
            xoff: field("xoff ") as f32,
            yoff: field("yoff ") as f32,
        })
    }
}

/// The texture ROM saved in capture directory `root`.
pub fn load_texture_rom(root: &Path) -> std::io::Result<Vec<u16>> {
    Ok(words16(&std::fs::read(root.join("texture-rom.bin"))?))
}

fn words32(b: &[u8]) -> Vec<u32> {
    b.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect()
}

fn bytes32(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

/// Delta pages: a changed page is written whole.
const PAGE: usize = 256;

/// `cur` as the pages that differ from `prev`: the new length, then
/// `(offset, length, bytes)` runs of changed pages (u32s little-endian).
fn delta(prev: &[u8], cur: &[u8]) -> Vec<u8> {
    let mut out = (cur.len() as u32).to_le_bytes().to_vec();
    let mut start = None;
    let pages = cur.len().div_ceil(PAGE);
    for p in 0..=pages {
        let changed = p < pages && {
            let r = p * PAGE..((p + 1) * PAGE).min(cur.len());
            prev.get(r.clone()) != Some(&cur[r])
        };
        match (changed, start) {
            (true, None) => start = Some(p * PAGE),
            (false, Some(s)) => {
                let e = (p * PAGE).min(cur.len());
                out.extend((s as u32).to_le_bytes());
                out.extend(((e - s) as u32).to_le_bytes());
                out.extend_from_slice(&cur[s..e]);
                start = None;
            }
            _ => {}
        }
    }
    out
}

/// Apply [`delta`]'s output to `prev`.
fn undelta(prev: &mut Vec<u8>, d: &[u8]) -> std::io::Result<()> {
    let bad = || std::io::Error::new(std::io::ErrorKind::InvalidData, "bad delta");
    let rd = |k: usize| d.get(k..k + 4).map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize).ok_or_else(bad);
    prev.resize(rd(0)?, 0);
    let mut k = 4;
    while k < d.len() {
        let (off, len) = (rd(k)?, rd(k + 4)?);
        let src = d.get(k + 8..k + 8 + len).ok_or_else(bad)?;
        prev.get_mut(off..off + len).ok_or_else(bad)?.copy_from_slice(src);
        k += 8 + len;
    }
    Ok(())
}

/// Consecutive frames with smooth motion's motion data, for checking
/// motion vectors and temporal anti-aliasing offline (`remodel run
/// --capture-seq`, `remodel temporal-check`).
///
/// `root/seq.txt` names the game, `root/texture-rom.bin` is shared, and
/// each frame's `root/NNNNNN/` holds `stream.bin`, `meta.txt` (frame,
/// sync offsets, and `motion none|cut|yes` with the smooth frame's number),
/// with motion `s0.bin` (the list posed with the frame before's
/// transforms) and `valid.bin` (a byte per object block)
/// ([`crate::smooth::Motion`]), and the video RAMs (render, palette, tiles,
/// texture point RAM): whole (`X.bin`) in the first frame, after that the
/// 256-byte pages that changed since the frame before (`X.delta`), so a
/// sequence must be read from its first frame on ([`Seq::open`]).
pub struct SeqWriter {
    root: std::path::PathBuf,
    prev: Option<[Vec<u8>; 4]>,
}

/// One frame of a sequence: the frame, and its motion data (None: the
/// native geometrizer didn't run it, so there is none).
pub struct SeqFrame {
    pub dir: std::path::PathBuf,
    pub frame: Frame,
    pub motion: Option<crate::smooth::Motion>,
    /// The smooth frame's number ([`crate::smooth::Frame::frame`]).
    pub smooth_frame: Option<u64>,
}

/// The four video RAMs a frame keeps, by file name.
const RAMS: [&str; 4] = ["render", "palette", "tiles", "points"];

fn rams(f: &Frame) -> [Vec<u8>; 4] {
    [f.render_ram.clone(), f.palette.clone(), f.tiles.clone(), bytes16(&f.texture_point_ram)]
}

impl SeqWriter {
    /// Start a sequence in `root` (emptied first) for game `game`.
    pub fn new(root: &Path, game: &str, texture_rom: &[u16]) -> std::io::Result<SeqWriter> {
        let _ = std::fs::remove_dir_all(root);
        std::fs::create_dir_all(root)?;
        std::fs::write(root.join("texture-rom.bin"), bytes16(texture_rom))?;
        std::fs::write(root.join("seq.txt"), format!("game {game}\nformat 1\n"))?;
        Ok(SeqWriter { root: root.to_path_buf(), prev: None })
    }

    /// Add `f`, the next frame, with its smooth frame (None: none ran).
    pub fn push(&mut self, f: &Frame, smooth: Option<&crate::smooth::Frame>) -> std::io::Result<()> {
        let dir = self.root.join(format!("{:06}", f.frame));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(dir.join("stream.bin"), bytes32(&f.stream))?;
        let motion = match smooth {
            None => "none".to_string(),
            Some(s) => {
                let m = s.motion();
                if !m.cut {
                    std::fs::write(dir.join("s0.bin"), bytes32(&m.s0))?;
                    std::fs::write(dir.join("valid.bin"), m.valid.iter().map(|&v| v as u8).collect::<Vec<u8>>())?;
                }
                format!("{} {}", if m.cut { "cut" } else { "yes" }, s.frame)
            }
        };
        let cur = rams(f);
        for (k, name) in RAMS.iter().enumerate() {
            match &self.prev {
                Some(p) => std::fs::write(dir.join(format!("{name}.delta")), delta(&p[k], &cur[k]))?,
                None => std::fs::write(dir.join(format!("{name}.bin")), &cur[k])?,
            }
        }
        self.prev = Some(cur);
        std::fs::write(
            dir.join("meta.txt"),
            format!("frame {}\nxoff {}\nyoff {}\nmotion {motion}\n", f.frame, f.xoff, f.yoff),
        )
    }
}

/// A sequence written by [`SeqWriter`], read frame by frame in order.
pub struct Seq {
    pub game: String,
    pub texture_rom: Vec<u16>,
    dirs: std::vec::IntoIter<std::path::PathBuf>,
    rams: [Vec<u8>; 4],
}

impl Seq {
    /// Whether `root` holds a sequence (rather than a `--capture`).
    pub fn is_seq(root: &Path) -> bool {
        root.join("seq.txt").exists()
    }

    pub fn open(root: &Path) -> std::io::Result<Seq> {
        let info = std::fs::read_to_string(root.join("seq.txt"))?;
        let game = info.lines().find_map(|l| l.strip_prefix("game ")).unwrap_or("").trim().to_string();
        let mut dirs: Vec<_> =
            std::fs::read_dir(root)?.flatten().map(|e| e.path()).filter(|p| p.join("meta.txt").exists()).collect();
        dirs.sort();
        Ok(Seq { game, texture_rom: load_texture_rom(root)?, dirs: dirs.into_iter(), rams: Default::default() })
    }

    /// Frames left to read.
    pub fn len(&self) -> usize {
        self.dirs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.dirs.len() == 0
    }

    fn read(&mut self, dir: std::path::PathBuf) -> std::io::Result<SeqFrame> {
        let read = |f: &str| std::fs::read(dir.join(f));
        for (k, name) in RAMS.iter().enumerate() {
            match read(&format!("{name}.bin")) {
                Ok(b) => self.rams[k] = b,
                Err(_) => undelta(&mut self.rams[k], &read(&format!("{name}.delta"))?)?,
            }
        }
        let meta = std::fs::read_to_string(dir.join("meta.txt"))?;
        let field = |k: &str| meta.lines().find_map(|l| l.strip_prefix(k)).map(str::trim);
        let num = |k: &str| field(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
        let mut words = field("motion ").unwrap_or("none").split_whitespace();
        let (kind, smooth_frame) = (words.next().unwrap_or("none"), words.next().and_then(|n| n.parse().ok()));
        let motion = match kind {
            "cut" => Some(crate::smooth::Motion { cut: true, ..Default::default() }),
            "yes" => Some(crate::smooth::Motion {
                s0: words32(&read("s0.bin")?),
                valid: read("valid.bin")?.iter().map(|&b| b != 0).collect(),
                cut: false,
            }),
            _ => None,
        };
        let frame = Frame {
            frame: num("frame ") as u64,
            stream: words32(&read("stream.bin")?),
            texture_point_ram: words16(&self.rams[3]),
            render_ram: self.rams[0].clone(),
            palette: self.rams[1].clone(),
            tiles: self.rams[2].clone(),
            xoff: num("xoff ") as f32,
            yoff: num("yoff ") as f32,
        };
        Ok(SeqFrame { dir, frame, motion, smooth_frame })
    }
}

impl Iterator for Seq {
    type Item = std::io::Result<SeqFrame>;

    fn next(&mut self) -> Option<Self::Item> {
        let dir = self.dirs.next()?;
        Some(self.read(dir))
    }
}

/// [`frames`]: each frame with its directory, in order.
pub type Frames = Box<dyn Iterator<Item = std::io::Result<(std::path::PathBuf, Frame)>>>;

/// The frames of capture directory `root` in order, with their
/// directories: a `--capture` (each frame whole) or a sequence ([`Seq`]).
pub fn frames(root: &Path) -> std::io::Result<Frames> {
    if Seq::is_seq(root) {
        return Ok(Box::new(Seq::open(root)?.map(|f| f.map(|f| (f.dir, f.frame)))));
    }
    let mut dirs: Vec<_> =
        std::fs::read_dir(root)?.flatten().map(|e| e.path()).filter(|p| p.join("meta.txt").exists()).collect();
    dirs.sort();
    Ok(Box::new(dirs.into_iter().map(|d| Frame::load(&d).map(|f| (d, f)))))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let root = std::env::temp_dir().join(format!("remodel-capture-{}", std::process::id()));
        let f = Frame {
            frame: 42,
            stream: vec![0xff00_0003, 7],
            texture_point_ram: vec![1, 2],
            render_ram: vec![3; 8],
            palette: vec![4; 4],
            tiles: vec![5; 2],
            xoff: 84.0,
            yoff: 130.5,
        };
        f.save(&root, &[9, 10]).unwrap();
        let g = Frame::load(&root.join("000042")).unwrap();
        assert_eq!((g.frame, &g.stream, g.xoff, g.yoff), (42, &f.stream, 84.0, 130.5));
        assert_eq!(load_texture_rom(&root).unwrap(), [9, 10]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn delta_pages_round_trip() {
        let a: Vec<u8> = (0..1000u32).map(|i| i as u8).collect();
        let mut b = a.clone();
        b[300] ^= 1;
        b[999] ^= 1;
        let d = delta(&a, &b);
        assert!(d.len() < 4 + 2 * 8 + 256 + 232 + 1, "only the changed pages: {}", d.len());
        let mut c = a.clone();
        undelta(&mut c, &d).unwrap();
        assert_eq!(c, b);
        // Shorter and longer.
        for n in [10, 1500] {
            let e: Vec<u8> = (0..n).map(|i| (i * 7) as u8).collect();
            let mut c = a.clone();
            undelta(&mut c, &delta(&a, &e)).unwrap();
            assert_eq!(c, e);
        }
        assert_eq!(delta(&a, &a).len(), 4);
    }

    #[test]
    fn sequence_round_trip() {
        let root = std::env::temp_dir().join(format!("remodel-seq-{}", std::process::id()));
        let mut f = Frame {
            frame: 7,
            stream: vec![0xff00_0003, 1, 2],
            texture_point_ram: vec![1, 2, 3],
            render_ram: vec![3; 600],
            palette: vec![4; 4],
            tiles: vec![5; 300],
            xoff: 1.0,
            yoff: 2.0,
        };
        let mut w = SeqWriter::new(&root, "topskatr", &[9]).unwrap();
        w.push(&f, None).unwrap();
        f.frame = 8;
        f.render_ram[599] = 1;
        f.tiles.truncate(10);
        f.texture_point_ram[0] = 0x1234;
        w.push(&f, None).unwrap();
        let mut s = Seq::open(&root).unwrap();
        assert_eq!((s.game.as_str(), s.len(), &s.texture_rom[..]), ("topskatr", 2, &[9u16][..]));
        let a = s.next().unwrap().unwrap();
        assert_eq!((a.frame.frame, a.frame.render_ram[599], a.motion.is_none()), (7, 3, true));
        let b = s.next().unwrap().unwrap();
        assert_eq!((b.frame.frame, b.frame.render_ram[599], b.frame.tiles.len()), (8, 1, 10));
        assert_eq!((b.frame.texture_point_ram[0], &b.frame.stream, b.frame.yoff), (0x1234, &f.stream, 2.0));
        assert!(s.next().is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
