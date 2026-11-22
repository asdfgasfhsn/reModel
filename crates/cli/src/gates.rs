//! Quality gates (docs/uplift-studio.md, "Gates"): checks that give a
//! number, a limit and a verdict, so "done" is not a matter of looking.
//!
//! This file is the asset level: pure arithmetic on a candidate image and
//! its original (no ROM, no GPU, milliseconds each), the limits, the
//! verdicts and the files a run writes. The project glue is `asset check`
//! (`studio_gates.rs`); the pack, scene and clip levels are `pack_gates.rs`.
//!
//! Verdicts: `error` blocks (exit code 1), `warn` and `info` do not, `pass`
//! is a gate that held. A gate that does not apply (`alpha` on a tile with
//! nothing to cut out, `changed` on a tile) says `info` and why.

#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks, clippy::manual_checked_ops)]

use crate::studio_img::{seam_axis, Image, Wrap};
use serde_json::{json, Value};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- limits
//
// First limits, from the calibration E1 in the retired quality-gates spec:
// 303 textures and 5,970 tiles of the Top Skater showcase against their
// originals (2026-11-20), re-checked on a second game (docs/uplift-studio.md,
// "Gates", "Limits"). What E1 wrongly flagged of approved images is beside
// each one. A project overrides a limit in `gates.txt` with a reason.

/// `file`: the biggest whole-number scale from the original (E1: every wrong
/// aspect and non-whole scale caught, 0% of approved wrongly flagged).
pub const FILE_MAX_SCALE: usize = 8;
/// `file`: the biggest PNG, in bytes.
pub const FILE_MAX_BYTES: u64 = 4 << 20;
/// `alpha`: the share of the cut-out (union of the two at 0.5) that may
/// disagree, in percent (E1: 2.3% of approved textures flagged, 4 of them
/// real; every lost alpha caught). Error for textures, warn for tiles (14.6%).
pub const ALPHA_CUTOUT_PCT: f64 = 2.0;
/// `alpha.soft`: the share of pixels part-transparent where the original had
/// none, in percent (E1: 0.7% flagged; every soft alpha caught).
pub const ALPHA_SOFT_PCT: f64 = 0.1;
/// `seam`: levels (0-255) across a repeated or mirrored edge, and how many
/// times the typical step inside the image (E1: 4.2% flagged; 31% of a
/// 24-level step caught, none of a 6-level one). The test is
/// `studio_img::seam_axis`.
pub const SEAM_LEVELS: f32 = 12.0;
pub const SEAM_RATIO: f32 = 2.5;
/// `changed`: at least this share of the original's visible texels, in
/// percent, must differ (E1: under 5% flagged 2.3% of approved textures and
/// 90% of unchanged copies).
pub const CHANGED_MIN_PCT: f64 = 5.0;
/// `changed`: a texel differs when a colour channel is this many levels
/// away (E1 did not say; set by the calibration on the second game).
pub const CHANGED_LEVEL: i32 = 12;
/// `noise`: the noise estimate (levels, Immerkaer's) over this warns (E1:
/// 1.3% flagged; heavy noise only).
pub const NOISE_MAX: f64 = 16.0;
/// `flicker`: pixels involved in replacement/original changes per 100 frames
/// drawn. E1 did not calibrate clip gates: this is a first guess, a 70 by 70
/// object changing once per 100 frames.
pub const FLICKER_PX_PER_100_FRAMES: f64 = 5000.0;
/// `coverage`: the share of a scene's pixels that differ from the original
/// once the pack is drawn, in percent. Uncalibrated: a project sets its own.
pub const COVERAGE_MIN_PCT: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verdict {
    Pass,
    Info,
    Warn,
    Error,
}

impl Verdict {
    pub fn name(self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Info => "info",
            Verdict::Warn => "warn",
            Verdict::Error => "error",
        }
    }
}

/// One gate's result on one thing (an asset, the pack, a scene).
#[derive(Clone, Debug, PartialEq)]
pub struct GateResult {
    pub asset: String,
    pub gate: &'static str,
    pub value: f64,
    pub limit: f64,
    pub verdict: Verdict,
    /// The pass criterion as a number, e.g. `cut-out disagreement at most 2% (is 0.4%)`.
    pub rule: String,
    /// The next step for a failure (empty when it passed).
    pub fix: String,
    pub axis: Option<&'static str>,
}

impl GateResult {
    fn new(asset: &str, gate: &'static str, value: f64, limit: f64, verdict: Verdict, rule: String) -> GateResult {
        GateResult { asset: asset.into(), gate, value, limit, verdict, rule, fix: String::new(), axis: None }
    }

    fn fix(mut self, f: &str) -> GateResult {
        if self.verdict >= Verdict::Warn {
            self.fix = f.into();
        }
        self
    }

    /// A gate that does not apply here.
    pub fn info(asset: &str, gate: &'static str, why: impl Into<String>) -> GateResult {
        GateResult::new(asset, gate, 0.0, 0.0, Verdict::Info, why.into())
    }

    pub fn to_json(&self) -> Value {
        json!({
            "asset": self.asset, "gate": self.gate, "value": round3(self.value), "limit": round3(self.limit),
            "verdict": self.verdict.name(), "rule": self.rule,
            "axis": self.axis, "fix": if self.fix.is_empty() { Value::Null } else { json!(self.fix) },
        })
    }

    /// `gates.tsv` columns: asset, gate, value, limit, verdict, rule.
    pub fn tsv(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.asset,
            self.gate,
            round3(self.value),
            round3(self.limit),
            self.verdict.name(),
            self.rule
        )
    }
}

pub const TSV_HEADER: &str = "asset\tgate\tvalue\tlimit\tverdict\trule";

fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// The worst verdict of some results (`Pass` for none).
pub fn worst(rs: &[GateResult]) -> Verdict {
    rs.iter().map(|r| r.verdict).max().unwrap_or(Verdict::Pass)
}

// ------------------------------------------------------ limit overrides

/// A project's `gates.txt`: `GATE<TAB>VALUE<TAB>REASON` per line (`#`
/// comments). The gate is a name from the tables (`seam`, `noise`,
/// `changed`, `alpha`, `alpha.soft`, `budget`, `flicker`, `coverage`); the
/// value replaces the default limit, and the report lists each override.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Limits {
    set: BTreeMap<String, (f64, String)>,
}

pub const LIMIT_GATES: [&str; 9] =
    ["file", "alpha", "alpha.soft", "seam", "changed", "noise", "budget", "flicker", "coverage"];

impl Limits {
    pub fn parse(text: &str) -> Result<Limits, String> {
        let mut set = BTreeMap::new();
        for (n, l) in text.lines().enumerate() {
            let l = l.trim();
            if l.is_empty() || l.starts_with('#') {
                continue;
            }
            let f: Vec<&str> = l.splitn(3, '\t').collect();
            let [gate, value, reason] = f[..] else {
                return Err(format!("gates.txt line {}: GATE, VALUE and a REASON, separated by tabs", n + 1));
            };
            if !LIMIT_GATES.contains(&gate) {
                let known: Vec<String> = LIMIT_GATES.iter().map(|s| s.to_string()).collect();
                return Err(format!(
                    "gates.txt line {}: {}",
                    n + 1,
                    crate::answer::Answer::unknown("gate", gate, &known).message()
                ));
            }
            let v: f64 =
                value.trim().parse().map_err(|_| format!("gates.txt line {}: `{value}` is not a number", n + 1))?;
            if reason.trim().is_empty() {
                return Err(format!("gates.txt line {}: an override needs a reason", n + 1));
            }
            set.insert(gate.to_string(), (v, reason.trim().to_string()));
        }
        Ok(Limits { set })
    }

    pub fn load(project: &std::path::Path) -> Result<Limits, String> {
        match std::fs::read_to_string(project.join("gates.txt")) {
            Ok(t) => Limits::parse(&t),
            Err(_) => Ok(Limits::default()),
        }
    }

    pub fn get(&self, gate: &str, default: f64) -> f64 {
        self.set.get(gate).map_or(default, |s| s.0)
    }

    /// `(gate, value, reason)` of every override.
    pub fn overrides(&self) -> Vec<(String, f64, String)> {
        self.set.iter().map(|(g, (v, r))| (g.clone(), *v, r.clone())).collect()
    }
}

// ------------------------------------------------------------ the asset

/// What the asset gates look at for one image.
pub struct Asset<'a> {
    pub name: &'a str,
    /// The file as read, when it is a PNG that decodes (the error otherwise).
    pub image: Result<&'a Image, String>,
    pub bytes: u64,
    pub original: Option<&'a Image>,
    /// Wrap of the two axes (`Clamp` where the game never draws past the size).
    pub wrap: (Wrap, Wrap),
    /// The hash of the file, and the one recorded at approval (if any) when
    /// the file is the approved one.
    pub hash: u64,
    pub approved: Option<u64>,
}

impl Asset<'_> {
    pub fn is_tile(&self) -> bool {
        self.name.starts_with("tile_")
    }
}

/// FNV-1a over a file's bytes: the identity `stale` compares.
pub fn hash_bytes(b: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &x in b {
        h = (h ^ x as u64).wrapping_mul(0x100000001b3);
    }
    h
}

pub fn hash_hex(h: u64) -> String {
    format!("{h:016x}")
}

/// The asset gates, in order: `file`, `alpha`, `alpha.soft`, `seam`,
/// `changed`, `noise`, `stale`. The gates that compare with the original
/// run only when `file` holds.
pub fn check(a: &Asset, limits: &Limits) -> Vec<GateResult> {
    let name = a.name;
    let mut out = Vec::new();
    let stale = stale_gate(a);
    let img = match &a.image {
        Ok(i) => *i,
        Err(e) => {
            out.push(
                GateResult::new(name, "file", 0.0, 1.0, Verdict::Error, format!("must decode as a PNG: {e}"))
                    .fix("write the image again as a PNG"),
            );
            out.extend(stale);
            return out;
        }
    };
    let file = file_gate(a, img, limits);
    let file_ok = file.verdict != Verdict::Error;
    out.push(file);
    let comparable = match (file_ok, a.original) {
        (true, Some(o)) => Some(o),
        _ => None,
    };
    let why = if !file_ok { "skipped: `file` failed" } else { "skipped: no original to compare with" };
    match comparable {
        Some(o) => {
            let small = downsample(img, img.width / o.width);
            out.extend(alpha_gates(name, a.is_tile(), img, &small, o, limits));
            out.push(seam_gate(a, img, o, limits));
            out.push(changed_gate(a, &small, o, limits));
        }
        None => {
            for g in ["alpha", "alpha.soft", "seam", "changed"] {
                out.push(GateResult::info(name, g, why));
            }
        }
    }
    out.push(noise_gate(name, img, limits));
    out.extend(stale);
    out
}

fn file_gate(a: &Asset, img: &Image, limits: &Limits) -> GateResult {
    let max_scale = limits.get("file", FILE_MAX_SCALE as f64);
    let rule = |s: String| {
        format!("original's aspect, a whole scale from 1 to {max_scale}, at most {} MB: {s}", FILE_MAX_BYTES >> 20)
    };
    let name = a.name;
    if a.bytes > FILE_MAX_BYTES {
        return GateResult::new(
            name,
            "file",
            a.bytes as f64 / (1 << 20) as f64,
            (FILE_MAX_BYTES >> 20) as f64,
            Verdict::Error,
            rule(format!("{:.1} MB", a.bytes as f64 / (1 << 20) as f64)),
        )
        .fix("save at a smaller scale or with less detail");
    }
    let Some(o) = a.original else {
        return GateResult::new(
            name,
            "file",
            0.0,
            max_scale,
            Verdict::Pass,
            rule("decodes; no original to measure the scale against".into()),
        );
    };
    let (w, h, ow, oh) = (img.width, img.height, o.width, o.height);
    if ow == 0 || oh == 0 || w % ow != 0 || h % oh != 0 || w / ow != h / oh {
        return GateResult::new(
            name,
            "file",
            w as f64 / ow.max(1) as f64,
            max_scale,
            Verdict::Error,
            rule(format!("{w}x{h} against {ow}x{oh}")),
        )
        .fix(&format!("make it {ow}x{oh} times a whole number (2x is {}x{})", ow * 2, oh * 2));
    }
    let s = w / ow;
    if s as f64 > max_scale {
        return GateResult::new(name, "file", s as f64, max_scale, Verdict::Error, rule(format!("scale {s}")))
            .fix(&format!("shrink it to at most {}x{}", ow * max_scale as usize, oh * max_scale as usize));
    }
    GateResult::new(name, "file", s as f64, max_scale, Verdict::Pass, rule(format!("scale {s}, {w}x{h}")))
}

/// The image reduced by a whole `s` (box average, weighted by alpha).
pub fn downsample(img: &Image, s: usize) -> Image {
    if s <= 1 {
        return img.clone();
    }
    let (w, h) = (img.width / s, img.height / s);
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let (mut c, mut a) = ([0u32; 3], 0u32);
            for dy in 0..s {
                for dx in 0..s {
                    let o = ((y * s + dy) * img.width + x * s + dx) * 4;
                    let al = img.rgba[o + 3] as u32;
                    for k in 0..3 {
                        c[k] += img.rgba[o + k] as u32 * al;
                    }
                    a += al;
                }
            }
            let o = (y * w + x) * 4;
            for k in 0..3 {
                rgba[o + k] = if a == 0 { 0 } else { ((c[k] + a / 2) / a) as u8 };
            }
            rgba[o + 3] = ((a + (s * s / 2) as u32) / (s * s) as u32) as u8;
        }
    }
    Image { width: w, height: h, rgba }
}

fn alpha_gates(name: &str, tile: bool, full: &Image, small: &Image, o: &Image, limits: &Limits) -> Vec<GateResult> {
    let n = o.width * o.height;
    let (mut either, mut differ, mut soft_new, mut soft_old) = (0usize, 0usize, 0usize, 0usize);
    for i in 0..n {
        let (a, b) = (small.rgba[i * 4 + 3], o.rgba[i * 4 + 3]);
        let (va, vb) = (a >= 128, b >= 128);
        either += (va || vb) as usize;
        differ += (va != vb) as usize;
        soft_old += (b != 0 && b != 255) as usize;
    }
    // Soft alpha is counted on the image as written: reducing it blurs a hard edge.
    let nf = full.width * full.height;
    for i in 0..nf {
        let a = full.rgba[i * 4 + 3];
        soft_new += (a != 0 && a != 255) as usize;
    }
    let lim = limits.get("alpha", ALPHA_CUTOUT_PCT);
    let pct = if either == 0 { 0.0 } else { differ as f64 * 100.0 / either as f64 };
    let level = if tile { Verdict::Warn } else { Verdict::Error };
    let v = if pct > lim { level } else { Verdict::Pass };
    let mut cut = GateResult::new(
        name,
        "alpha",
        pct,
        lim,
        v,
        format!("cut-out (alpha 0.5) disagreement with the original at most {lim}% of the union: {pct:.2}%"),
    )
    .fix("repaint so transparent texels stay transparent: apply the original's alpha as a mask (`asset normalise` copies it)");
    if tile && v == Verdict::Warn {
        cut.rule += " (a warning for a tile)";
    }
    let slim = limits.get("alpha.soft", ALPHA_SOFT_PCT);
    let spct = soft_new as f64 * 100.0 / nf.max(1) as f64;
    let orig_soft = soft_old as f64 * 100.0 / n.max(1) as f64;
    let sv = if orig_soft <= slim && spct > slim { Verdict::Warn } else { Verdict::Pass };
    let soft = GateResult::new(
        name,
        "alpha.soft",
        spct,
        slim,
        sv,
        format!("part-transparent pixels at most {slim}% where the original has none: {spct:.2}% (original {orig_soft:.2}%)"),
    )
    .fix("threshold the alpha to 0 or 255 where the original has a hard cut-out");
    vec![cut, soft]
}

fn seam_gate(a: &Asset, img: &Image, o: &Image, limits: &Limits) -> GateResult {
    let lim = limits.get("seam", SEAM_LEVELS as f64);
    let rule = |s: String| {
        format!("jump across a repeated or mirrored edge at most {lim} levels, or at most {SEAM_RATIO}x the typical step: {s}")
    };
    let (wu, wv) = a.wrap;
    if wu == Wrap::Clamp && wv == Wrap::Clamp {
        return GateResult::info(a.name, "seam", "no repeated or mirrored axis");
    }
    let mut worst: Option<(f64, &'static str, bool, bool)> = None;
    for (vertical, wrap, axis) in [(true, wu, "x"), (false, wv, "y")] {
        let m = seam_axis(img, vertical, wrap);
        // The same seam in the original is the game's own.
        let own = seam_axis(o, vertical, wrap).suspect;
        let over = m.jump as f64 > lim && m.ratio > SEAM_RATIO;
        let bad = over && !own;
        if wrap != Wrap::Clamp && worst.is_none_or(|w| (bad, m.jump as f64) > (w.2, w.0)) {
            worst = Some((m.jump as f64, axis, bad, over && own));
        }
    }
    let (jump, axis, bad, owned) = worst.unwrap_or((0.0, "x", false, false));
    let note = if owned { " (the original has the same seam)" } else { "" };
    let mut r = GateResult::new(
        a.name,
        "seam",
        jump,
        lim,
        if bad { Verdict::Warn } else { Verdict::Pass },
        rule(format!("{jump:.1} levels on {axis}{note}")),
    )
    .fix("set `seamless` for this asset in recipes.txt, or repaint the edges so they meet");
    r.axis = Some(axis);
    r
}

fn changed_gate(a: &Asset, small: &Image, o: &Image, limits: &Limits) -> GateResult {
    if a.is_tile() {
        return GateResult::info(a.name, "changed", "textures only");
    }
    let lim = limits.get("changed", CHANGED_MIN_PCT);
    let (mut seen, mut differ) = (0usize, 0usize);
    for i in 0..o.width * o.height {
        if o.rgba[i * 4 + 3] < 128 {
            continue;
        }
        seen += 1;
        let d = (0..3).map(|k| (small.rgba[i * 4 + k] as i32 - o.rgba[i * 4 + k] as i32).abs()).max().unwrap_or(0);
        differ += (d > CHANGED_LEVEL) as usize;
    }
    if seen == 0 {
        return GateResult::info(a.name, "changed", "the original has no visible texels");
    }
    let pct = differ as f64 * 100.0 / seen as f64;
    GateResult::new(
        a.name,
        "changed",
        pct,
        lim,
        if pct < lim { Verdict::Warn } else { Verdict::Pass },
        format!(
            "at least {lim}% of visible texels differ from the original by more than {CHANGED_LEVEL} levels: {pct:.1}%"
        ),
    )
    .fix("it is a copy of the original or close to it: restyle it, or leave it out of the pack")
}

/// Immerkaer's noise estimate (levels) on the luma of the opaque 3x3
/// neighbourhoods of `img`; None when there is none.
pub fn noise(img: &Image) -> Option<f64> {
    let (w, h) = (img.width, img.height);
    if w < 3 || h < 3 {
        return None;
    }
    let luma: Vec<f64> =
        img.rgba.chunks_exact(4).map(|p| 0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64).collect();
    let (mut sum, mut n) = (0.0, 0usize);
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let at = |dx: usize, dy: usize| (y + dy - 1) * w + x + dx - 1;
            if (0..3).any(|dy| (0..3).any(|dx| img.rgba[at(dx, dy) * 4 + 3] < 128)) {
                continue;
            }
            let l = |dx, dy| luma[at(dx, dy)];
            let c = l(0, 0) - 2.0 * l(1, 0) + l(2, 0) - 2.0 * l(0, 1) + 4.0 * l(1, 1) - 2.0 * l(2, 1) + l(0, 2)
                - 2.0 * l(1, 2)
                + l(2, 2);
            sum += c.abs();
            n += 1;
        }
    }
    (n > 0).then(|| (std::f64::consts::PI / 2.0).sqrt() * sum / (6.0 * n as f64))
}

fn noise_gate(name: &str, img: &Image, limits: &Limits) -> GateResult {
    let lim = limits.get("noise", NOISE_MAX);
    let Some(n) = noise(img) else {
        return GateResult::info(name, "noise", "too small or too transparent to measure");
    };
    GateResult::new(
        name,
        "noise",
        n,
        lim,
        if n > lim { Verdict::Warn } else { Verdict::Pass },
        format!("noise estimate at most {lim} levels: {n:.1}"),
    )
    .fix("denoise or redraw it: grain this strong shimmers when the surface moves")
}

fn stale_gate(a: &Asset) -> Option<GateResult> {
    let name = a.name;
    Some(match a.approved {
        None => GateResult::info(name, "stale", "no approval recorded for this file"),
        Some(h) if h == a.hash => {
            GateResult::new(name, "stale", 0.0, 0.0, Verdict::Pass, "file hash equals the approved hash: same".into())
        }
        Some(h) => GateResult::new(
            name,
            "stale",
            1.0,
            0.0,
            Verdict::Error,
            format!("file hash equals the approved hash: it is {}, approved was {}", hash_hex(a.hash), hash_hex(h)),
        )
        .fix("the file changed after approval: check it again and approve it, or restore the approved one"),
    })
}

// ------------------------------------------------------------ reporting

/// The text of a list of results: one line each, failures marked.
pub fn text_lines(rs: &[GateResult]) -> String {
    let mut s = String::new();
    for r in rs {
        s += &format!("{:<6} {:<10} {:<10} {}\n", r.verdict.name(), r.gate, r.asset, r.rule);
        if !r.fix.is_empty() {
            s += &format!("       fix: {}\n", r.fix);
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio_img::blank;

    /// A smooth gradient with texture, 16x16, opaque.
    fn art(w: usize, h: usize, seed: u8) -> Image {
        let mut im = blank(w, h, [0, 0, 0, 255]);
        for y in 0..h {
            for x in 0..w {
                let o = (y * w + x) * 4;
                im.rgba[o] = ((x * 255 / w) as u8).wrapping_add(seed);
                im.rgba[o + 1] = (y * 255 / h) as u8;
                im.rgba[o + 2] = ((x + y) * 128 / (w + h)) as u8;
            }
        }
        im
    }

    fn scale(im: &Image, s: usize) -> Image {
        let mut out = blank(im.width * s, im.height * s, [0, 0, 0, 255]);
        for y in 0..out.height {
            for x in 0..out.width {
                let (a, b) = (((y / s) * im.width + x / s) * 4, (y * out.width + x) * 4);
                out.rgba[b..b + 4].copy_from_slice(&im.rgba[a..a + 4]);
            }
        }
        out
    }

    fn run(name: &str, cand: &Image, orig: Option<&Image>, wrap: (Wrap, Wrap)) -> Vec<GateResult> {
        let a = Asset { name, image: Ok(cand), bytes: 1000, original: orig, wrap, hash: 1, approved: None };
        check(&a, &Limits::default())
    }

    fn gate<'a>(rs: &'a [GateResult], g: &str) -> &'a GateResult {
        rs.iter().find(|r| r.gate == g).unwrap_or_else(|| panic!("no gate {g}"))
    }

    const CLAMP: (Wrap, Wrap) = (Wrap::Clamp, Wrap::Clamp);

    #[test]
    fn a_restyled_upscale_passes_everything() {
        let o = art(16, 16, 0);
        let mut c = scale(&art(16, 16, 90), 2);
        // ... with a different look: invert one channel.
        for p in c.rgba.chunks_exact_mut(4) {
            p[1] = 255 - p[1];
        }
        let rs = run("tex_0123456789abcdef_16x16", &c, Some(&o), CLAMP);
        for r in &rs {
            assert!(r.verdict <= Verdict::Info, "{r:?}");
        }
        assert_eq!(gate(&rs, "file").verdict, Verdict::Pass);
        assert_eq!(gate(&rs, "changed").verdict, Verdict::Pass);
    }

    #[test]
    fn file_fails_on_aspect_scale_and_size() {
        let o = art(16, 16, 0);
        let n = "tex_0123456789abcdef_16x16";
        for (w, h, why) in [(32, 16, "aspect"), (24, 24, "not whole"), (144, 144, "too big a scale")] {
            let rs = run(n, &art(w, h, 0), Some(&o), CLAMP);
            let f = gate(&rs, "file");
            assert_eq!(f.verdict, Verdict::Error, "{why}");
            assert!(!f.fix.is_empty());
            assert_eq!(gate(&rs, "alpha").verdict, Verdict::Info, "the rest are skipped: {why}");
        }
        let big = Asset {
            bytes: FILE_MAX_BYTES + 1,
            ..Asset { name: n, image: Ok(&o), bytes: 0, original: Some(&o), wrap: CLAMP, hash: 1, approved: None }
        };
        assert_eq!(gate(&check(&big, &Limits::default()), "file").verdict, Verdict::Error);
        let bad = Asset {
            image: Err("bad crc".into()),
            ..Asset { name: n, image: Ok(&o), bytes: 0, original: Some(&o), wrap: CLAMP, hash: 1, approved: None }
        };
        assert_eq!(gate(&check(&bad, &Limits::default()), "file").verdict, Verdict::Error);
        assert_eq!(gate(&run(n, &scale(&o, 8), Some(&o), CLAMP), "file").verdict, Verdict::Pass);
    }

    #[test]
    fn alpha_fails_on_a_lost_cutout_and_on_soft_edges() {
        let mut o = art(16, 16, 0);
        for i in 0..64 {
            o.rgba[i * 4 + 3] = 0; // a quarter cut out
        }
        let n = "tex_0123456789abcdef_16x16";
        let opaque = scale(&art(16, 16, 50), 2);
        let rs = run(n, &opaque, Some(&o), CLAMP);
        assert_eq!(gate(&rs, "alpha").verdict, Verdict::Error, "{:?}", gate(&rs, "alpha"));
        let rs = run("tile_0123456789abcdef", &opaque, Some(&o), CLAMP);
        assert_eq!(gate(&rs, "alpha").verdict, Verdict::Warn, "a tile only warns");
        // Kept cut-out: passes.
        let mut kept = scale(&art(16, 16, 50), 2);
        for y in 0..32 {
            for x in 0..32 {
                if (y / 2) * 16 + x / 2 < 64 {
                    kept.rgba[(y * 32 + x) * 4 + 3] = 0;
                }
            }
        }
        let rs = run(n, &kept, Some(&o), CLAMP);
        assert_eq!(gate(&rs, "alpha").verdict, Verdict::Pass);
        assert_eq!(gate(&rs, "alpha.soft").verdict, Verdict::Pass);
        // Soft alpha where the original had none.
        let mut soft = scale(&art(16, 16, 50), 2);
        for p in soft.rgba.chunks_exact_mut(4).take(50) {
            p[3] = 128 - 40;
        }
        let o2 = art(16, 16, 0);
        let rs = run(n, &soft, Some(&o2), CLAMP);
        assert_eq!(gate(&rs, "alpha.soft").verdict, Verdict::Warn);
    }

    #[test]
    fn seam_fails_on_a_step_across_a_repeated_edge_unless_the_original_has_it() {
        // Left edge black, right edge white, smooth between: a seam.
        let mut c = blank(32, 32, [0, 0, 0, 255]);
        for y in 0..32 {
            for x in 0..32 {
                let l = (x * 255 / 31) as u8;
                c.rgba[(y * 32 + x) * 4..(y * 32 + x) * 4 + 3].copy_from_slice(&[l, l, l]);
            }
        }
        let flat = blank(32, 32, [90, 90, 90, 255]);
        let r = (Wrap::Repeat, Wrap::Repeat);
        let rs = run("tex_0123456789abcdef_32x32", &c, Some(&flat), r);
        let s = gate(&rs, "seam");
        assert_eq!(s.verdict, Verdict::Warn, "{s:?}");
        assert_eq!(s.axis, Some("x"));
        assert!(s.value > SEAM_LEVELS as f64);
        // The original has it too: the game's own.
        let rs = run("tex_0123456789abcdef_32x32", &c, Some(&c), r);
        assert_eq!(gate(&rs, "seam").verdict, Verdict::Pass);
        // Drawn once: no repeated axis.
        assert_eq!(gate(&run("tex_0123456789abcdef_32x32", &c, Some(&flat), CLAMP), "seam").verdict, Verdict::Info);
    }

    #[test]
    fn changed_fails_on_a_copy_and_passes_on_a_restyle() {
        let o = art(16, 16, 0);
        let n = "tex_0123456789abcdef_16x16";
        let rs = run(n, &scale(&o, 2), Some(&o), CLAMP);
        let c = gate(&rs, "changed");
        assert_eq!(c.verdict, Verdict::Warn, "{c:?}");
        assert!(c.value < 1.0);
        assert_eq!(
            gate(&run("tile_0123456789abcdef", &scale(&o, 2), Some(&o), CLAMP), "changed").verdict,
            Verdict::Info
        );
    }

    #[test]
    fn noise_fails_on_grain_and_passes_on_smooth_art() {
        let smooth = art(32, 32, 0);
        assert!(noise(&smooth).unwrap() < 2.0, "{:?}", noise(&smooth));
        let mut grain = art(32, 32, 0);
        let mut s = 12345u32;
        for p in grain.rgba.chunks_exact_mut(4) {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = ((s >> 24) as i32 - 128) / 2;
            for k in 0..3 {
                p[k] = (p[k] as i32 + n).clamp(0, 255) as u8;
            }
        }
        assert!(noise(&grain).unwrap() > NOISE_MAX, "{:?}", noise(&grain));
        let o = art(16, 16, 0);
        let rs = run("tex_0123456789abcdef_16x16", &scale(&art(16, 16, 9), 2), Some(&o), CLAMP);
        assert_eq!(gate(&rs, "noise").verdict, Verdict::Pass);
        let mut g2 = scale(&art(16, 16, 9), 2);
        for (i, p) in g2.rgba.chunks_exact_mut(4).enumerate() {
            let n = if (i * 7 + i / 3) % 2 == 0 { 60 } else { -60 };
            for k in 0..3 {
                p[k] = (p[k] as i32 + n).clamp(0, 255) as u8;
            }
        }
        assert_eq!(gate(&run("tex_0123456789abcdef_16x16", &g2, Some(&o), CLAMP), "noise").verdict, Verdict::Warn);
    }

    #[test]
    fn stale_fails_when_the_hash_moved() {
        let o = art(16, 16, 0);
        let c = scale(&o, 2);
        let mk = |approved| {
            let a = Asset {
                name: "tex_0123456789abcdef_16x16",
                image: Ok(&c),
                bytes: 10,
                original: Some(&o),
                wrap: CLAMP,
                hash: 7,
                approved,
            };
            check(&a, &Limits::default())
        };
        assert_eq!(gate(&mk(Some(7)), "stale").verdict, Verdict::Pass);
        let r = mk(Some(8));
        assert_eq!(gate(&r, "stale").verdict, Verdict::Error);
        assert_eq!(worst(&r), Verdict::Error);
        assert_eq!(gate(&mk(None), "stale").verdict, Verdict::Info);
    }

    #[test]
    fn limits_parse_override_and_refuse_without_a_reason() {
        let l = Limits::parse("# c\nnoise\t100\tgrainy film look\nseam\t20\tink border\n").unwrap();
        assert_eq!(l.get("noise", NOISE_MAX), 100.0);
        assert_eq!(l.get("changed", 5.0), 5.0);
        assert_eq!(l.overrides().len(), 2);
        assert!(Limits::parse("noise\t24\t\n").is_err());
        assert!(Limits::parse("noise\t24\n").is_err());
        assert!(Limits::parse("nois\t24\tx\n").unwrap_err().contains("did you mean"));
        // An override moves the verdict.
        let mut grain = art(32, 32, 0);
        let mut s = 99u32;
        for p in grain.rgba.chunks_exact_mut(4) {
            s = s.wrapping_mul(1664525).wrapping_add(1013904223);
            let n = ((s >> 24) as i32 - 128) / 2;
            for k in 0..3 {
                p[k] = (p[k] as i32 + n).clamp(0, 255) as u8;
            }
        }
        assert_eq!(noise_gate("x", &grain, &Limits::default()).verdict, Verdict::Warn);
        assert_eq!(noise_gate("x", &grain, &l).verdict, Verdict::Pass);
    }

    #[test]
    fn json_and_tsv_carry_value_limit_and_verdict() {
        let r = GateResult::new("a", "noise", 20.0, 16.0, Verdict::Warn, "r".into()).fix("do it");
        let j = r.to_json();
        assert_eq!(j["gate"], "noise");
        assert_eq!(j["limit"], 16.0);
        assert_eq!(j["verdict"], "warn");
        assert_eq!(j["fix"], "do it");
        assert_eq!(r.tsv(), "a\tnoise\t20\t16\twarn\tr");
    }
}
