//! `studio sharing`, the collecting half: what every scene of a project draws,
//! by object block, texture key and palette (no ROM, no GPU: the scenes'
//! captures are drawn by the software renderer's identification pass, as
//! `texmap` does, without the picture).
//!
//! The store is `PROJECT/sharing/`, derived from the ROM like the scenes:
//!
//! ```text
//! scenes/NAME.tsv   one scene's sightings: block, key, pal, pixels (a header line holds its stamp)
//! parts.tsv         every (key, pal) seen: key, pal, w, h, surface, mean r g b of the original through the
//!                   palette, opaque share, ramp
//! orig/             tex_KEY~PAL_WxH.png: that original, through that palette
//! ```
//!
//! A scene is read again only when its `scene.txt` changed (the stamp).

#![allow(clippy::chunks_exact_to_as_chunks)]

use model2::capture::{self, Frame};
use model2::idmap::{self, Key, Registry, FIRST_ID};
use model2::raster;
use model2::video::{self, View};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// A mean colour, 0 to 255 a channel.
pub type Tone = [f32; 3];

/// What one scene drew of one texture in one object block.
#[derive(Clone, Debug, PartialEq)]
pub struct Sight {
    pub scene: String,
    /// The 3D window and object block (`window << 20 | block`): what the
    /// game draws as one object.
    pub block: u32,
    pub key: u64,
    pub pal: u32,
    pub pixels: u64,
}

/// A texture seen through one palette.
#[derive(Clone, Debug, PartialEq)]
pub struct PartInfo {
    pub key: u64,
    pub pal: u32,
    pub w: u32,
    pub h: u32,
    pub surface: u64,
    /// The mean colour of the original's opaque texels through this palette.
    pub orig: Tone,
    pub opaque: f32,
    pub ramp: String,
}

impl PartInfo {
    pub fn name(&self) -> String {
        model2::enhance::variant_name(self.key, self.pal, self.w, self.h)
    }
}

/// The mean colour of the texels with alpha of at least 128, and their share.
pub fn mean_tone(rgba: &[u8]) -> (Tone, f32) {
    let (mut s, mut n) = ([0f64; 3], 0usize);
    for p in rgba.chunks_exact(4) {
        if p[3] >= 128 {
            for c in 0..3 {
                s[c] += p[c] as f64;
            }
            n += 1;
        }
    }
    let total = (rgba.len() / 4).max(1);
    if n == 0 {
        return ([0.0; 3], 0.0);
    }
    ([(s[0] / n as f64) as f32, (s[1] / n as f64) as f32, (s[2] / n as f64) as f32], n as f32 / total as f32)
}

/// Everything the scenes drew.
#[derive(Default)]
pub struct Collected {
    pub sights: Vec<Sight>,
    pub parts: BTreeMap<(u64, u32), PartInfo>,
    /// Scenes read from the store, read afresh, and that could not be read.
    pub cached: usize,
    pub fresh: usize,
    pub failed: Vec<(String, String)>,
}

pub fn store_dir(project: &Path) -> PathBuf {
    project.join("sharing")
}

fn fnv(s: &str) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
    }
    h
}

fn stamp_of(scene_dir: &Path) -> String {
    format!("{:016x}", fnv(&std::fs::read_to_string(scene_dir.join("scene.txt")).unwrap_or_default()))
}

/// One frame's sightings and the (key, pal) parts it met (not yet in `known`).
fn draw_scene(
    game: &romset::Game,
    scene_dir: &Path,
    known: &Mutex<BTreeMap<(u64, u32), PartInfo>>,
    images: &Mutex<Vec<(String, model2::enhance::Rgba)>>,
) -> Result<Vec<(u32, u64, u32, u64)>, String> {
    let cap = scene_dir.join("capture");
    let frame_dir = crate::scene::capture_frame_dir(&cap).ok_or("no capture")?;
    let rom = capture::load_texture_rom(&cap).map_err(|e| format!("{}: {e}", cap.display()))?;
    let f = Frame::load(&frame_dir).map_err(|e| format!("{}: {e}", frame_dir.display()))?;
    let view = View::native().with_hud_windows(game.hud_windows, game.hud_element_windows);
    let src = f.sources(&rom);
    let mut polys = raster::build_shaded(&f.stream, &src, None);
    video::HudLayout::new(&f.tiles, &f.palette, &view).anchor(&mut polys, &view);
    let placed = raster::place(&mut polys, &view);
    let (vis, uv) = raster::visible_uv(&polys, &placed, &src, &view);
    let mut reg = Registry::default();
    let ids = idmap::identify(&polys, &placed, &vis, &uv, None, &view, &src, &mut reg);
    let mut count: HashMap<(u32, u32), u64> = HashMap::new();
    for (o, &id) in ids.ids.iter().enumerate() {
        if id >= FIRST_ID {
            *count.entry((ids.class[o], id)).or_insert(0) += 1;
        }
    }
    let mut out = Vec::new();
    for ((class, id), pixels) in count {
        let Some(e) = reg.entry(id) else { continue };
        let Key::Tex(key) = e.key else { continue };
        {
            let mut k = known.lock().unwrap();
            k.entry((key, e.pal)).or_insert_with(|| {
                let (orig, opaque) = mean_tone(&e.image.data);
                images.lock().unwrap().push((
                    model2::enhance::variant_name(key, e.pal, e.w, e.h),
                    model2::enhance::Rgba { width: e.image.width, height: e.image.height, data: e.image.data.clone() },
                ));
                PartInfo {
                    key,
                    pal: e.pal,
                    w: e.w,
                    h: e.h,
                    surface: e.surface,
                    orig,
                    opaque,
                    ramp: e.detail.split(' ').find_map(|t| t.strip_prefix("ramp=")).unwrap_or("").to_string(),
                }
            });
        }
        out.push((class, key, e.pal, pixels));
    }
    out.sort();
    Ok(out)
}

fn parts_tsv(parts: &BTreeMap<(u64, u32), PartInfo>) -> String {
    let mut s = String::from("key\tpal\tw\th\tsurface\tr\tg\tb\topaque\tramp\n");
    for p in parts.values() {
        let _ = writeln!(
            s,
            "{:016x}\t{:08x}\t{}\t{}\t{:016x}\t{:.2}\t{:.2}\t{:.2}\t{:.4}\t{}",
            p.key, p.pal, p.w, p.h, p.surface, p.orig[0], p.orig[1], p.orig[2], p.opaque, p.ramp
        );
    }
    s
}

fn read_parts(path: &Path) -> BTreeMap<(u64, u32), PartInfo> {
    let mut m = BTreeMap::new();
    for l in std::fs::read_to_string(path).unwrap_or_default().lines().skip(1) {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 10 {
            continue;
        }
        let (Ok(key), Ok(pal)) = (u64::from_str_radix(f[0], 16), u32::from_str_radix(f[1], 16)) else { continue };
        let num = |i: usize| f[i].parse::<f32>().unwrap_or(0.0);
        m.insert(
            (key, pal),
            PartInfo {
                key,
                pal,
                w: f[2].parse().unwrap_or(0),
                h: f[3].parse().unwrap_or(0),
                surface: u64::from_str_radix(f[4], 16).unwrap_or(0),
                orig: [num(5), num(6), num(7)],
                opaque: num(8),
                ramp: f[9].to_string(),
            },
        );
    }
    m
}

fn read_scene_file(path: &Path, scene: &str, stamp: &str) -> Option<Vec<Sight>> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if lines.next()? != format!("# stamp {stamp}") {
        return None;
    }
    let mut v = Vec::new();
    for l in lines {
        let f: Vec<&str> = l.split('\t').collect();
        if f.len() < 4 {
            return None;
        }
        v.push(Sight {
            scene: scene.to_string(),
            block: f[0].parse().ok()?,
            key: u64::from_str_radix(f[1], 16).ok()?,
            pal: u32::from_str_radix(f[2], 16).ok()?,
            pixels: f[3].parse().ok()?,
        });
    }
    Some(v)
}

/// The sightings of every scene in `scenes` (`(name, directory)`), from the
/// store where it is current, else by drawing the capture; new parts and
/// their original images are saved. `refresh` draws them all again.
pub fn collect(
    game: &romset::Game,
    project: &Path,
    scenes: &[(String, PathBuf)],
    refresh: bool,
    progress: bool,
) -> Result<Collected, String> {
    let store = store_dir(project);
    for d in ["scenes", "orig"] {
        std::fs::create_dir_all(store.join(d)).map_err(|e| format!("{}: {e}", store.display()))?;
    }
    let known = Mutex::new(read_parts(&store.join("parts.tsv")));
    let images: Mutex<Vec<(String, model2::enhance::Rgba)>> = Mutex::new(Vec::new());
    let out = Mutex::new(Collected::default());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 6);
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some((name, dir)) = scenes.get(i) else { break };
                let file = store.join("scenes").join(format!("{name}.tsv"));
                let stamp = stamp_of(dir);
                let cached = (!refresh)
                    .then(|| read_scene_file(&file, name, &stamp))
                    .flatten()
                    .filter(|v| v.iter().all(|s| known.lock().unwrap().contains_key(&(s.key, s.pal))));
                if let Some(v) = cached {
                    let mut o = out.lock().unwrap();
                    o.sights.extend(v);
                    o.cached += 1;
                    continue;
                }
                match draw_scene(game, dir, &known, &images) {
                    Ok(rows) => {
                        let mut t = format!("# stamp {stamp}\n");
                        let mut v = Vec::new();
                        for (block, key, pal, pixels) in rows {
                            let _ = writeln!(t, "{block}\t{key:016x}\t{pal:08x}\t{pixels}");
                            v.push(Sight { scene: name.clone(), block, key, pal, pixels });
                        }
                        let _ = std::fs::write(&file, t);
                        let mut o = out.lock().unwrap();
                        o.sights.extend(v);
                        o.fresh += 1;
                        if progress {
                            eprintln!("sharing: {name} drawn ({} of {})", o.fresh + o.cached, scenes.len());
                        }
                    }
                    Err(e) => out.lock().unwrap().failed.push((name.clone(), e)),
                }
            });
        }
    });
    let parts = known.into_inner().unwrap();
    for (name, im) in images.into_inner().unwrap() {
        let p = store.join("orig").join(format!("{name}.png"));
        let _ = std::fs::write(&p, png::encode_rgba(im.width, im.height, &im.data));
    }
    std::fs::write(store.join("parts.tsv"), parts_tsv(&parts)).map_err(|e| e.to_string())?;
    let mut c = out.into_inner().unwrap();
    // The store keeps every part ever seen; what is analysed is what these scenes draw.
    let seen: std::collections::HashSet<(u64, u32)> = c.sights.iter().map(|s| (s.key, s.pal)).collect();
    c.parts = parts.into_iter().filter(|(k, _)| seen.contains(k)).collect();
    c.sights.sort_by(|a, b| (&a.scene, a.block, a.key, a.pal).cmp(&(&b.scene, b.block, b.key, b.pal)));
    Ok(c)
}
