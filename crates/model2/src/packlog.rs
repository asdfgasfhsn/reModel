//! The texture pack diagnostic (`--pack-log FILE`; docs/texture-packs.md,
//! "Flicker"): per drawn frame, for every texture, tile pattern and region
//! the renderer looked up in the pack, whether it drew the replacement
//! (a hit) or the original (a miss) and why, and a report of the ones that
//! alternate between the two over a run, ranked by the pixels involved.
//!
//! Render-side only and free when off: the renderer keeps a
//! `Option<PackLog>` and gathers nothing without one.
//!
//! The log is text, one block per frame drawn:
//!
//! ```text
//! F <frame>
//! t <key> <surface> <WxH> <outcome> <polygons> <pixels> <cx> <cy> <bank:x:y>
//! k <entry> <key> <outcome> <pixels> <cx> <cy>
//! r <name> <outcome> <cells>
//! ```
//!
//! `t` is a 3D texture (by key; pixels in output pixels, the centre of its
//! polygons in output pixels, weighted by pixels), `k` a tile pattern by its
//! map entry (tile number and colour bank; the key changes with the
//! palette), `r` a region override. An outcome is `hit` (the replacement
//! drawn), `up` (a hit whose image was uploaded this frame), `held` (a
//! hit by a stand-in: [`Fixes`]) or `miss:REASON`.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::Path;

/// The render-side stabilisations of a pack's replacements, each toggleable
/// (`--pack-fixes`, default all) so a run can be compared with and without.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fixes {
    /// A texture whose key is not in the pack, but whose surface (the same
    /// texels in another colour or luma table) has an image under another
    /// key, or whose sheet rectangle drew a replacement a moment ago, draws
    /// that image.
    pub surface: bool,
    /// An image that is evicted or not yet decoded keeps a stand-in (the
    /// same surface's, or the last one drawn for the same sheet rectangle)
    /// rather than showing the original meanwhile; for tile patterns, the
    /// last image of the same pattern, colours changed or not.
    pub hold: bool,
    /// The pack's images are decoded in the background from the start, and
    /// uploaded when the game is idle, instead of at first draw.
    pub preload: bool,
}

impl Default for Fixes {
    fn default() -> Self {
        Fixes { surface: true, hold: true, preload: true }
    }
}

impl Fixes {
    /// None of them: the renderer as it was.
    pub const NONE: Fixes = Fixes { surface: false, hold: false, preload: false };

    /// `all`, `none` or names (`surface`, `hold`, `preload`)
    /// separated by commas.
    pub fn parse(s: &str) -> Result<Fixes, String> {
        match s {
            "all" | "on" => return Ok(Fixes::default()),
            "none" | "off" => return Ok(Fixes::NONE),
            _ => {}
        }
        let mut f = Fixes::NONE;
        for n in s.split(',') {
            match n.trim() {
                "surface" => f.surface = true,
                "hold" => f.hold = true,
                "preload" => f.preload = true,
                o => return Err(format!("unknown pack fix `{o}` (all, none, surface, hold, preload)")),
            }
        }
        Ok(f)
    }
}

/// Frames a hold or a surface fallback may stand in for a missing image.
pub const HOLD_FRAMES: u64 = 8;

/// One distinct texture drawn in a frame.
#[derive(Clone, Debug, Default)]
pub struct TexRec {
    pub key: u64,
    pub surface: u64,
    pub width: u32,
    pub height: u32,
    /// `hit`, `up`, `held`, or `miss:REASON`.
    pub outcome: String,
    pub polys: u32,
    pub pixels: f64,
    /// Pixel-weighted sums of the polygons' centres.
    pub sx: f64,
    pub sy: f64,
    pub rect: [u32; 3],
}

/// One distinct tile pattern (map entry) drawn in a frame.
#[derive(Clone, Debug, Default)]
pub struct TileRec {
    pub entry: u32,
    pub key: u64,
    pub outcome: String,
    /// Game pixels, and the sums of their positions.
    pub pixels: f64,
    pub sx: f64,
    pub sy: f64,
}

/// The log being written.
pub struct PackLog {
    out: std::io::BufWriter<std::fs::File>,
    frame: u64,
    block: bool,
}

impl PackLog {
    pub fn create(path: &Path) -> std::io::Result<PackLog> {
        if let Some(d) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(d)?;
        }
        let mut out = std::io::BufWriter::new(std::fs::File::create(path)?);
        writeln!(out, "# reModel pack log (docs/texture-packs.md, Flicker)")?;
        Ok(PackLog { out, frame: 0, block: false })
    }

    /// The game frame the next block is for.
    pub fn set_frame(&mut self, frame: u64) {
        self.frame = frame;
    }

    /// Start a block: the lookups of one drawn frame follow.
    pub fn begin(&mut self) {
        let _ = writeln!(self.out, "F\t{}", self.frame);
        self.block = true;
    }

    pub fn tex(&mut self, r: &TexRec) {
        let n = r.pixels.max(1.0);
        let _ = writeln!(
            self.out,
            "t\t{:016x}\t{:016x}\t{}x{}\t{}\t{}\t{:.0}\t{:.0}\t{:.0}\t{}:{}:{}",
            r.key,
            r.surface,
            r.width,
            r.height,
            r.outcome,
            r.polys,
            r.pixels,
            r.sx / n,
            r.sy / n,
            r.rect[0],
            r.rect[1],
            r.rect[2]
        );
    }

    pub fn tile(&mut self, r: &TileRec) {
        let n = r.pixels.max(1.0);
        let _ = writeln!(
            self.out,
            "k\t{:08x}\t{:016x}\t{}\t{:.0}\t{:.0}\t{:.0}",
            r.entry,
            r.key,
            r.outcome,
            r.pixels,
            r.sx / n,
            r.sy / n
        );
    }

    pub fn region(&mut self, name: &str, outcome: &str, cells: u32) {
        let _ = writeln!(self.out, "r\t{name}\t{outcome}\t{cells}");
    }

    pub fn flush(&mut self) {
        let _ = self.out.flush();
    }
}

impl Drop for PackLog {
    fn drop(&mut self) {
        self.flush();
    }
}

// ---------------------------------------------------------------- report

/// One subject's appearance in one block.
#[derive(Clone, Debug)]
struct Seen {
    block: usize,
    frame: u64,
    hit: bool,
    reason: String,
    pixels: f64,
    x: f64,
    y: f64,
}

#[derive(Default)]
struct Series {
    kind: char,
    label: String,
    seen: Vec<Seen>,
}

/// How many blocks a subject may be absent between two appearances and
/// still count as the same stretch on screen.
const GAP: usize = 2;

/// The flicker report over the logs in `paths` (the same run is one log;
/// several are reported one after the other): totals, and the subjects that
/// alternate between replacement and original, ranked by the pixels of the
/// frames either side of each change.
pub fn report(paths: &[&Path], top: usize) -> Result<String, String> {
    let mut out = String::new();
    for p in paths {
        let text = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        out += &format!("== {}\n", p.display());
        out += &report_text(&text, top);
    }
    Ok(out)
}

/// The flicker totals of a log, for a gate (`remodel pack-flicker --limit`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Flicker {
    /// Frames drawn (blocks in the log).
    pub blocks: usize,
    /// Subjects that change between replacement and original.
    pub subjects: usize,
    /// Changes in all.
    pub changes: u32,
    /// Pixels involved: for each change, the smaller of the pixel counts either side.
    pub px: f64,
}

impl Flicker {
    /// Pixels involved per 100 frames drawn (0 for an empty log).
    pub fn px_per_100_frames(&self) -> f64 {
        if self.blocks == 0 {
            0.0
        } else {
            self.px * 100.0 / self.blocks as f64
        }
    }
}

pub fn report_text(text: &str, top: usize) -> String {
    analyse(text, top).0
}

/// The flicker totals of the log `text`.
pub fn flicker(text: &str) -> Flicker {
    analyse(text, 0).1
}

fn analyse(text: &str, top: usize) -> (String, Flicker) {
    let mut series: BTreeMap<(char, String), Series> = BTreeMap::new();
    let (mut block, mut frame) = (0usize, 0u64);
    let mut blocks = 0usize;
    // Totals by kind: pixels drawn, pixels replaced, misses by reason.
    let mut tot: HashMap<char, (f64, f64)> = HashMap::new();
    let mut reasons: BTreeMap<(char, String), f64> = BTreeMap::new();
    let mut held_px = 0.0;
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f[0] {
            "F" if f.len() >= 2 => {
                block += 1;
                blocks += 1;
                frame = f[1].parse().unwrap_or(frame);
            }
            "t" if f.len() >= 10 => {
                // Subject: the surface and size, so the colours and
                // transient keys of one texture are one thing.
                let px: f64 = f[6].parse().unwrap_or(0.0);
                let (cx, cy): (f64, f64) = (f[7].parse().unwrap_or(0.0), f[8].parse().unwrap_or(0.0));
                add(
                    &mut series,
                    &mut tot,
                    &mut reasons,
                    &mut held_px,
                    't',
                    format!("{} {}", f[2], f[3]),
                    block,
                    frame,
                    f[4],
                    px,
                    cx,
                    cy,
                );
                let s = series.get_mut(&('t', format!("{} {}", f[2], f[3]))).unwrap();
                if !s.label.contains("key") {
                    s.label = format!("tex surf {} {} (key {})", f[2], f[3], f[1]);
                }
            }
            "k" if f.len() >= 7 => {
                let px: f64 = f[4].parse().unwrap_or(0.0);
                let (cx, cy): (f64, f64) = (f[5].parse().unwrap_or(0.0), f[6].parse().unwrap_or(0.0));
                add(
                    &mut series,
                    &mut tot,
                    &mut reasons,
                    &mut held_px,
                    'k',
                    f[1].to_string(),
                    block,
                    frame,
                    f[3],
                    px,
                    cx,
                    cy,
                );
                series.get_mut(&('k', f[1].to_string())).unwrap().label = format!("tile entry {}", f[1]);
            }
            "r" if f.len() >= 4 => {
                let cells: f64 = f[3].parse().unwrap_or(0.0);
                add(
                    &mut series,
                    &mut tot,
                    &mut reasons,
                    &mut held_px,
                    'r',
                    f[1].to_string(),
                    block,
                    frame,
                    f[2],
                    cells,
                    0.0,
                    0.0,
                );
                series.get_mut(&('r', f[1].to_string())).unwrap().label = format!("region {}", f[1]);
            }
            _ => {}
        }
    }
    let mut out = format!("blocks (frames drawn)   {blocks}\n");
    for (k, name, unit) in [('t', "3D textures", "px"), ('k', "tile patterns", "px"), ('r', "regions", "cells")] {
        if let Some((all, hit)) = tot.get(&k) {
            out += &format!(
                "{name:<14} drawn {all:>14.0} {unit}, replaced {hit:>14.0} ({:.1}%)\n",
                100.0 * hit / all.max(1.0)
            );
        }
    }
    if held_px > 0.0 {
        out += &format!("held by a stand-in      {held_px:.0} (counted as replaced)\n");
    }
    out += "misses by reason:\n";
    for ((k, r), v) in &reasons {
        out += &format!("  {} {r:<18} {v:>14.0}\n", kind_name(*k));
    }
    // Transitions.
    struct Row {
        kind: char,
        label: String,
        flips: u32,
        hit_blocks: u32,
        miss_blocks: u32,
        weight: f64,
        reasons: BTreeMap<String, u32>,
        frames: Vec<u64>,
        at: (f64, f64),
    }
    let mut rows = Vec::new();
    for s in series.values() {
        let mut r = Row {
            kind: s.kind,
            label: s.label.clone(),
            flips: 0,
            hit_blocks: 0,
            miss_blocks: 0,
            weight: 0.0,
            reasons: BTreeMap::new(),
            frames: Vec::new(),
            at: (0.0, 0.0),
        };
        let (mut ax, mut ay, mut an) = (0.0, 0.0, 0.0);
        for (i, v) in s.seen.iter().enumerate() {
            if v.hit {
                r.hit_blocks += 1;
            } else {
                r.miss_blocks += 1;
            }
            ax += v.x * v.pixels;
            ay += v.y * v.pixels;
            an += v.pixels;
            if i == 0 {
                continue;
            }
            let p = &s.seen[i - 1];
            if v.block - p.block > GAP + 1 || v.hit == p.hit {
                continue;
            }
            r.flips += 1;
            r.weight += p.pixels.min(v.pixels).max(1.0);
            let miss = if v.hit { p } else { v };
            *r.reasons.entry(miss.reason.clone()).or_default() += 1;
            if r.frames.len() < 6 {
                r.frames.push(v.frame);
            }
        }
        r.at = (ax / an.max(1.0), ay / an.max(1.0));
        if r.flips > 0 {
            rows.push(r);
        }
    }
    let total_flips: u32 = rows.iter().map(|r| r.flips).sum();
    let total_w: f64 = rows.iter().map(|r| r.weight).sum();
    out += &format!("\nflicker: {} subjects change between replacement and original, {total_flips} changes in all, {total_w:.0} px involved\n", rows.len());
    for k in ['t', 'k', 'r'] {
        let (n, f, w) =
            rows.iter().filter(|r| r.kind == k).fold((0, 0, 0.0), |a, r| (a.0 + 1, a.1 + r.flips, a.2 + r.weight));
        if n > 0 {
            out += &format!("  {:<14} {n:>5} subjects {f:>6} changes {w:>12.0} px\n", kind_name(k));
        }
    }
    rows.sort_by(|a, b| b.weight.total_cmp(&a.weight));
    out += "\nrank  changes  hit/miss blocks      px  at (x,y)    what [miss reasons] first frames\n";
    for (i, r) in rows.iter().take(top).enumerate() {
        let reasons: Vec<String> = r.reasons.iter().map(|(k, v)| format!("{k} x{v}")).collect();
        out += &format!(
            "{:>4} {:>8} {:>7}/{:<7} {:>8.0}  {:>4.0},{:<4.0}  {} [{}] {:?}\n",
            i + 1,
            r.flips,
            r.hit_blocks,
            r.miss_blocks,
            r.weight,
            r.at.0,
            r.at.1,
            r.label,
            reasons.join(", "),
            r.frames
        );
    }
    (out, Flicker { blocks, subjects: rows.len(), changes: total_flips, px: total_w })
}

fn kind_name(k: char) -> &'static str {
    match k {
        't' => "3D",
        'k' => "tile",
        _ => "region",
    }
}

#[allow(clippy::too_many_arguments)]
fn add(
    series: &mut BTreeMap<(char, String), Series>,
    tot: &mut HashMap<char, (f64, f64)>,
    reasons: &mut BTreeMap<(char, String), f64>,
    held_px: &mut f64,
    kind: char,
    subject: String,
    block: usize,
    frame: u64,
    outcome: &str,
    pixels: f64,
    x: f64,
    y: f64,
) {
    let hit = !outcome.starts_with("miss");
    let reason = outcome.strip_prefix("miss:").unwrap_or(outcome).to_string();
    let t = tot.entry(kind).or_default();
    t.0 += pixels;
    if hit {
        t.1 += pixels;
        if outcome == "held" {
            *held_px += pixels;
        }
    } else {
        *reasons.entry((kind, reason.clone())).or_default() += pixels;
    }
    let s = series.entry((kind, subject)).or_default();
    s.kind = kind;
    // One entry per block: pixels add, a miss in any part of it makes the
    // block a miss for the subject if it is most of it.
    match s.seen.last_mut() {
        Some(l) if l.block == block => {
            let (hp, mp) = if l.hit { (l.pixels, 0.0) } else { (0.0, l.pixels) };
            let (hp, mp) = if hit { (hp + pixels, mp) } else { (hp, mp + pixels) };
            l.hit = hp >= mp;
            if !hit && (mp >= hp || l.reason.is_empty()) {
                l.reason = reason;
            }
            l.x = (l.x * l.pixels + x * pixels) / (l.pixels + pixels).max(1.0);
            l.y = (l.y * l.pixels + y * pixels) / (l.pixels + pixels).max(1.0);
            l.pixels += pixels;
        }
        _ => s.seen.push(Seen { block, frame, hit, reason: if hit { String::new() } else { reason }, pixels, x, y }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixes_parse() {
        assert_eq!(Fixes::parse("all").unwrap(), Fixes::default());
        assert_eq!(Fixes::parse("none").unwrap(), Fixes::NONE);
        let f = Fixes::parse("surface,preload").unwrap();
        assert!(f.surface && f.preload && !f.hold);
        assert!(Fixes::parse("nope").is_err());
    }

    #[test]
    fn report_finds_alternating_subjects() {
        let mut log = String::new();
        for f in 0..10u64 {
            log += &format!("F\t{f}\n");
            // A texture that alternates, one that never does.
            let o = if f % 2 == 0 { "hit" } else { "miss:colour" };
            log += &format!("t\t000000000000000a\t00000000000000bb\t32x32\t{o}\t1\t500\t10\t20\t0:0:0\n");
            log += "t\t000000000000000c\t00000000000000dd\t64x64\thit\t1\t900\t10\t20\t0:0:1\n";
        }
        let r = report_text(&log, 5);
        assert!(r.contains("1 subjects change"), "{r}");
        assert!(r.contains("colour x9"), "{r}");
        assert!(!r.contains("dd"), "{r}");
    }

    #[test]
    fn flicker_totals_are_the_gate_number() {
        let mk = |alternate: bool| {
            let mut log = String::new();
            for f in 0..100u64 {
                log += &format!("F\t{f}\n");
                let o = if alternate && f % 2 == 0 { "miss:colour" } else { "hit" };
                log += &format!("t\t000000000000000a\t00000000000000bb\t32x32\t{o}\t1\t500\t10\t20\t0:0:0\n");
            }
            flicker(&log)
        };
        let steady = mk(false);
        assert_eq!((steady.changes, steady.px), (0, 0.0));
        assert_eq!(steady.px_per_100_frames(), 0.0);
        let f = mk(true);
        assert_eq!(f.blocks, 100);
        assert_eq!(f.changes, 99);
        assert_eq!(f.px, 99.0 * 500.0);
        assert_eq!(f.px_per_100_frames(), 99.0 * 500.0, "per 100 frames over 100 frames is the total");
        assert_eq!(Flicker::default().px_per_100_frames(), 0.0, "an empty log has none");
    }
}
