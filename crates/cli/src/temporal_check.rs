//! `remodel temporal-check`: check a sequence's motion data (from `run
//! --capture-seq`) before anything is drawn with it, and draw the
//! references temporal anti-aliasing and upscaling are measured against.
//!
//! Each frame goes through a list of [`Section`]s, which see the same
//! [`Cx`] (the frame built with its motion data, its software picture and
//! what shows at each pixel, the frame before's, and on sampled frames
//! the [`Refs`]). The CPU ones here:
//! - [`Pose`]: frame N's list posed with frame N-1's transforms (`s0`)
//!   against frame N-1's real stream, word for word, and the previous
//!   screen position of each vertex against the one frame N-1's stream
//!   gives the same place in the stream (its "layout twin");
//! - [`Vectors`]: per pixel, the motion vector a temporal renderer would
//!   write (`raster::visible`), followed into frame N-1's software
//!   picture: on screen, the same surface (1/z), the same colour;
//! - [`Baselines`]: on every `every`-th frame the plain frame and naive
//!   upscales scored against a 4x4 supersampled reference.
//!
//! - [`Gpu`] (unless `--no-gpu`): the GPU's velocity target against those
//!   vectors, and the plain, MSAA 4x, temporal (100%) and upscaled (67%,
//!   50%) frames drawn through the whole sequence, scored against the same
//!   reference, with their frame-to-frame stability, and a still frame
//!   repeated.
//!
//! `--bench` times the GPU instead ([`bench`]).

use model2::capture::{Seq, SeqFrame};
use model2::raster::{self, Centres, MotionIn, Placed, Poly, Sources, Visible};
use model2::video::{self, View};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Frames before the first sampled one (a temporal renderer's history
/// fills over them).
const WARMUP: usize = 16;
/// Supersampling of the reference (per axis).
const REF_SS: usize = 4;
/// Render scales of the naive upscales, in percent.
const SCALES: [u32; 2] = [67, 50];

pub struct Options {
    pub view: View,
    /// Sample (references, pictures) every this many frames.
    pub every: usize,
    pub out: PathBuf,
    /// Run the GPU section; only it (no pose, vectors, baselines).
    pub gpu: bool,
    pub gpu_only: bool,
    /// The temporal resolve's constants for the GPU section (None: the
    /// renderer's).
    pub tuning: Option<gpu::TemporalTuning>,
    /// Enlarged crops (x, y, w, h in output pixels) of every column on
    /// sampled frames.
    pub crops: Vec<(usize, usize, usize, usize)>,
    /// Keep the references here between runs (they take seconds each).
    pub ref_cache: Option<PathBuf>,
    /// Sample these frames (by number) too.
    pub at: Vec<u64>,
    /// The depth buffer on in every GPU column (and gref).
    pub depth: bool,
    /// Record what the temporal resolve decides per pixel on sampled
    /// frames: the error against gref above the plain frame's by rule,
    /// FRAME-COL-rules.png and -excess.png.
    pub inspect: bool,
    /// Print the resolve's decisions for TAA's pixels in this rectangle
    /// (x, y, w, h) on every frame, into OUT/dump.txt.
    /// The fifth number is the column's render scale (100: TAA, 67, 50).
    pub dump: Option<(usize, usize, usize, usize, usize)>,
}

/// What a frame has to take motion from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// Motion data, and the frame before in the sequence.
    Motion,
    /// A cut (`Plan::cut`): a temporal renderer resets.
    Cut,
    /// No motion data (the native geometrizer didn't run it), or the
    /// streams' layouts differ.
    NoMotion,
    /// The first frame of the sequence, or one after a gap.
    First,
}

/// The frame before, as the checks of the next one need it.
pub struct Prev {
    pub frame: u64,
    pub stream: Vec<u32>,
    pub centres: Vec<Centres>,
    /// Software 3D layer (RGB, coverage).
    pub layer: (Vec<u8>, Vec<bool>),
    pub vis: Visible,
}

/// The references of a sampled frame.
pub struct Refs {
    /// The whole frame, software, `REF_SS` x `REF_SS` supersampled.
    pub reference: Vec<u8>,
    /// Pixels the checkerboard (stipple) polygons change.
    pub stipple: Vec<bool>,
}

/// One frame as the sections see it.
pub struct Cx<'a> {
    pub f: &'a SeqFrame,
    pub src: &'a Sources<'a>,
    pub view: &'a View,
    pub kind: Kind,
    /// Built with the motion data (`prev` filled), HUD anchored, placed.
    pub polys: &'a [Poly],
    pub placed: &'a [Placed],
    /// Software 3D layer and what shows at each pixel.
    pub layer: &'a (Vec<u8>, Vec<bool>),
    pub vis: &'a Visible,
    /// The same with the previous positions taken from frame N-1's stream
    /// at the same place (when its layout is the same): exact if the game
    /// sent the same objects in the same order.
    pub twin: Option<(&'a [Placed], &'a Visible)>,
    pub prev: Option<&'a Prev>,
    pub refs: Option<&'a Refs>,
    /// On a sampled frame, the frame before's reference too (with the GPU
    /// section: the reference's own frame-to-frame change).
    pub prev_ref: Option<&'a Refs>,
    /// The next frame is a sampled one.
    pub next_sampled: bool,
    pub out: &'a Path,
}

/// The summary's lines and what failed.
#[derive(Default)]
pub struct Report {
    pub lines: Vec<String>,
    pub fails: Vec<String>,
}

impl Report {
    fn line(&mut self, s: String) {
        self.lines.push(s);
    }

    /// A pass/fail criterion: noted either way.
    fn check(&mut self, ok: bool, what: String) {
        self.lines.push(format!("  {} {what}", if ok { "pass" } else { "FAIL" }));
        if !ok {
            self.fails.push(what);
        }
    }
}

pub trait Section {
    /// Its columns in the per-frame table.
    fn header(&self) -> String;
    fn frame(&mut self, cx: &Cx) -> Result<String, String>;
    fn summary(&self, r: &mut Report);
}

fn pct(n: u64, of: u64) -> f64 {
    100.0 * n as f64 / of.max(1) as f64
}

fn f24(w: u32) -> f32 {
    f32::from_bits(w << 8)
}

/// Quantile `q` of sorted `v`.
fn quantile(v: &[f32], q: f64) -> f32 {
    v.get(((v.len() as f64 - 1.0) * q).round().max(0.0) as usize).copied().unwrap_or(0.0)
}

/// The position words of a stream's drawn object blocks (P0, P1 and each
/// polygon's new points), with the block (counted over every `0x1`
/// command, as `Motion::valid` is) each is in; and the number of blocks.
/// Blocks before the first window are drawn nowhere and left out.
pub fn positions(s: &[u32]) -> (Vec<(usize, usize)>, usize) {
    let mut pos = Vec::new();
    let rd = |i: usize| s.get(i).copied().unwrap_or(0xff00_000f);
    let (mut i, mut block, mut window) = (0, 0, false);
    while i < s.len() {
        let w = s[i];
        i += 1;
        if w >> 24 != 0xff {
            continue;
        }
        match w & 0xf {
            0x3 => {
                window = true;
                i += 6
            }
            0x8 => i += 1,
            0x4 => i += 2 + rd(i + 1) as usize,
            0x1 => {
                block += 1;
                let b = block - 1;
                if window {
                    pos.extend((i + 2..i + 8).map(|k| (k, b)));
                }
                i += 8;
                loop {
                    let attr = rd(i);
                    if attr >> 24 == 0xff || attr & 3 == 0 {
                        break;
                    }
                    let n = if attr & 1 != 0 { 9 } else { 6 };
                    if window {
                        pos.extend((i + 3..i + n).map(|k| (k, b)));
                    }
                    i += n;
                }
            }
            _ => {}
        }
    }
    pos.retain(|&(k, _)| k < s.len());
    (pos, block)
}

/// How far `b` is from the best affine image of `a` (least squares), in
/// units of the stream's rounding (24-bit floats: 2^-15 of each value):
/// about 1 or less when it is one. None with too few points to tell.
fn affine_residual(a: &[[f64; 3]], b: &[[f64; 3]]) -> Option<f64> {
    if a.len() != b.len() || a.len() < 6 {
        return None;
    }
    let n = a.len() as f64;
    let mean = |v: &[[f64; 3]]| -> [f64; 3] { std::array::from_fn(|k| v.iter().map(|q| q[k]).sum::<f64>() / n) };
    let (ma, mb) = (mean(a), mean(b));
    // Centred: B - mb = L (A - ma), L by the normal equations, solved by
    // Gauss-Jordan elimination, with a little ridge so flat objects
    // (shadows, decals) still fit within their plane.
    let mut m = [[0f64; 6]; 3];
    for (p, q) in a.iter().zip(b) {
        let x: [f64; 3] = std::array::from_fn(|k| p[k] - ma[k]);
        for i in 0..3 {
            for j in 0..3 {
                m[i][j] += x[i] * x[j];
                m[i][3 + j] += x[i] * (q[j] - mb[j]);
            }
        }
    }
    let trace = m[0][0] + m[1][1] + m[2][2];
    if trace.is_nan() || trace <= 0.0 {
        return None;
    }
    for (i, row) in m.iter_mut().enumerate() {
        row[i] += 1e-12 * trace;
    }
    for c in 0..3 {
        let piv = (c..3).max_by(|&i, &j| m[i][c].abs().total_cmp(&m[j][c].abs()))?;
        m.swap(c, piv);
        for i in 0..3 {
            if i != c {
                let f = m[i][c] / m[c][c];
                let pivot = m[c];
                for (x, p) in m[i][c..].iter_mut().zip(&pivot[c..]) {
                    *x -= f * p;
                }
            }
        }
    }
    let (mut err, mut noise) = (0.0, 0.0);
    for (p, q) in a.iter().zip(b) {
        for k in 0..3 {
            let fit: f64 = mb[k] + (0..3).map(|i| (p[i] - ma[i]) * m[i][3 + k] / m[i][i]).sum::<f64>();
            err += (fit - q[k]).powi(2);
            noise += ((p[k].abs() + q[k].abs()) * f64::powi(2.0, -15)).powi(2);
        }
    }
    Some((err / noise.max(1e-300)).sqrt())
}

/// Whether two 24-bit floats agree to `1e-3` of their size.
fn agree(a: u32, b: u32) -> bool {
    let (x, y) = (f24(a), f24(b));
    (x - y).abs() <= 1e-3 * x.abs().max(y.abs()).max(1e-3)
}

/// Pose agreement (word by word, and per vertex on screen).
#[derive(Default)]
pub struct Pose {
    frames: u64,
    words: u64,
    matched: u64,
    valid_words: u64,
    valid_matched: u64,
    /// Frames whose layouts differ from the frame before's.
    incomparable: u64,
    /// Per comparable frame: share matched in valid blocks.
    shares: Vec<(u64, f64)>,
    /// Worst blocks: (share, frame, block, words, valid).
    worst: Vec<(f64, u64, usize, usize, bool)>,
    /// Blocks that missed (some position word off), of them valid, and
    /// those whose s0 pose is another block of frame N-1 (examples:
    /// frame:block->block).
    missed: u64,
    missed_valid: u64,
    elsewhere: u64,
    elsewhere_examples: Vec<String>,
    /// The others: frame N-1's block an affine image of frame N's (moved
    /// as a whole), or not (reshaped: animated outside the transforms).
    affine: u64,
    reshaped: u64,
    /// Blocks s0 matches tested the same way, and of them found affine.
    calibration: (u64, u64),
    /// Vertex distances on screen (output pixels) between the plan's
    /// previous position and the layout twin's.
    vdist: Vec<f32>,
}

impl Section for Pose {
    fn header(&self) -> String {
        format!("{:>7} {:>7} {:>7}", "pose%", "valid%", "vtx<1")
    }

    fn frame(&mut self, cx: &Cx) -> Result<String, String> {
        let (Kind::Motion, Some(m), Some(prev)) = (cx.kind, cx.f.motion.as_ref(), cx.prev) else {
            return Ok(format!("{:>7} {:>7} {:>7}", "-", "-", "-"));
        };
        let (pos, blocks) = positions(&prev.stream);
        if m.s0.len() != prev.stream.len() || positions(&m.s0).0 != pos {
            self.incomparable += 1;
            return Ok(format!("{:>7} {:>7} {:>7}", "layout", "-", "-"));
        }
        let valid = |b: usize| m.valid.len() != blocks || m.valid[b];
        let (mut n, mut ok, mut vn, mut vok) = (0u64, 0u64, 0u64, 0u64);
        let mut per_block: Vec<(usize, usize)> = vec![(0, 0); blocks];
        for &(k, b) in &pos {
            let a = agree(m.s0[k], prev.stream[k]);
            n += 1;
            ok += a as u64;
            per_block[b].0 += 1;
            per_block[b].1 += a as usize;
            if valid(b) {
                vn += 1;
                vok += a as u64;
            }
        }
        // A block that misses: is its s0 pose another block of frame N-1
        // (the plan paired its transforms with another copy of the model)?
        let mut words_of: Vec<Vec<usize>> = vec![Vec::new(); blocks];
        for &(k, b) in &pos {
            words_of[b].push(k);
        }
        for (b, &(words, good)) in per_block.iter().enumerate().filter(|(_, c)| c.0 > 0 && c.1 < c.0) {
            self.worst.push((good as f64 / words as f64, cx.f.frame.frame, b, words, valid(b)));
            self.missed += 1;
            self.missed_valid += valid(b) as u64;
            let mine: Vec<u32> = words_of[b].iter().map(|&k| m.s0[k]).collect();
            let other = (0..blocks).find(|&j| {
                j != b
                    && words_of[j].len() == mine.len()
                    && words_of[j].iter().zip(&mine).filter(|&(&k, &w)| agree(w, prev.stream[k])).count() * 100
                        >= 99 * mine.len()
            });
            if let Some(j) = other {
                self.elsewhere += 1;
                if self.elsewhere_examples.len() < 6 {
                    self.elsewhere_examples.push(format!("{}:{b}->{j}", cx.f.frame.frame));
                }
            } else {
                // Is frame N-1's block an affine image of frame N's (the
                // object moved as a whole: a transform could say how), or
                // not (its model changed: animation outside the transforms)?
                let pts = |s: &[u32]| -> Vec<[f64; 3]> {
                    words_of[b].as_chunks::<3>().0.iter().map(|c| c.map(|k| f24(s[k]) as f64)).collect()
                };
                match affine_residual(&pts(&cx.f.frame.stream), &pts(&prev.stream)) {
                    Some(r) if r < 4.0 => self.affine += 1,
                    Some(_) => self.reshaped += 1,
                    None => {}
                }
            }
        }
        // Calibration of that test: blocks whose pose s0 does give (same
        // model, moved by transforms) should all come out affine.
        let agreeing = per_block.iter().enumerate().filter(|(_, c)| c.0 >= 18 && c.1 == c.0).take(50);
        for (b, _) in agreeing {
            let pts = |s: &[u32]| -> Vec<[f64; 3]> {
                words_of[b].as_chunks::<3>().0.iter().map(|c| c.map(|k| f24(s[k]) as f64)).collect()
            };
            if let Some(r) = affine_residual(&pts(&cx.f.frame.stream), &pts(&prev.stream)) {
                self.calibration.0 += 1;
                self.calibration.1 += (r < 4.0) as u64;
            }
        }
        self.worst.sort_by(|a, b| a.0.total_cmp(&b.0).then(b.3.cmp(&a.3)));
        self.worst.truncate(12);
        self.frames += 1;
        (self.words, self.matched, self.valid_words, self.valid_matched) =
            (self.words + n, self.matched + ok, self.valid_words + vn, self.valid_matched + vok);
        self.shares.push((cx.f.frame.frame, pct(vok, vn)));
        // Per vertex on screen.
        let mut vtx = "-".to_string();
        if let Some((twin, _)) = cx.twin {
            let (w, h) = (cx.view.width as f32, cx.view.height as f32);
            let (mut within, mut all) = (0u64, 0u64);
            for (a, b) in cx.placed.iter().zip(twin).filter(|(a, b)| !a.prev.is_empty() && !b.prev.is_empty()) {
                for ((v, p), q) in a.points.iter().zip(&a.prev).zip(&b.prev) {
                    if !(0.0..w).contains(&v.x) || !(0.0..h).contains(&v.y) {
                        continue;
                    }
                    let d = ((p[0] / p[2] - q[0] / q[2]).powi(2) + (p[1] / p[2] - q[1] / q[2]).powi(2)).sqrt();
                    let d = if d.is_finite() { d } else { f32::MAX };
                    self.vdist.push(d);
                    within += (d < 1.0) as u64;
                    all += 1;
                }
            }
            vtx = format!("{:.1}", pct(within, all));
        }
        Ok(format!("{:>7.2} {:>7.2} {vtx:>7}", pct(ok, n), pct(vok, vn)))
    }

    fn summary(&self, r: &mut Report) {
        r.line(format!(
            "pose          {} comparable frames ({} with another layout): s0 position words within 1e-3 of frame N-1's: {:.2}% of all, {:.2}% in valid blocks",
            self.frames,
            self.incomparable,
            pct(self.matched, self.words),
            pct(self.valid_matched, self.valid_words)
        ));
        let mut d = self.vdist.clone();
        d.sort_by(f32::total_cmp);
        let within = |t: f32| pct(d.partition_point(|&x| x < t) as u64, d.len() as u64);
        r.line(format!(
            "  on-screen vertices with motion, against the layout twin ({}): {:.2}% within 0.25 px, {:.2}% within 1 px, {:.2}% within 4 px; p99 {:.2} px",
            d.len(),
            within(0.25),
            within(1.0),
            within(4.0),
            quantile(&d, 0.99)
        ));
        r.line(format!(
            "  blocks missing frame N-1's pose: {} ({} valid); posed exactly as another block of frame N-1: {} (e.g. frame:block->block {})",
            self.missed,
            self.missed_valid,
            self.elsewhere,
            self.elsewhere_examples.join(" ")
        ));
        r.line(format!(
            "  of the others, frame N-1's block is an affine image of frame N's (moved as a whole, so the plan took another motion): {}; not (reshaped: model or vertices changed): {} (blocks that do match: {} of {} affine)",
            self.affine, self.reshaped, self.calibration.1, self.calibration.0
        ));
        let low: Vec<String> =
            self.shares.iter().filter(|s| s.1 < 99.0).take(40).map(|(f, s)| format!("{f}:{s:.1}")).collect();
        if !low.is_empty() {
            r.line(format!(
                "  frames under 99% in valid blocks ({}): {}",
                self.shares.iter().filter(|s| s.1 < 99.0).count(),
                low.join(" ")
            ));
        }
        for (share, frame, block, words, valid) in &self.worst {
            r.line(format!(
                "  worst block: frame {frame} block {block}: {:.1}% of {words} words{}",
                share * 100.0,
                if *valid { "" } else { " (invalid: no vectors)" }
            ));
        }
        if self.frames > 0 {
            r.check(
                pct(self.valid_matched, self.valid_words) >= 99.0,
                format!("pose agreement in valid blocks {:.2}% >= 99%", pct(self.valid_matched, self.valid_words)),
            );
        }
    }
}

/// The previous position at pixel `o`, if it has one: (x, y, W).
fn prev_at(vis: &Visible, o: usize) -> Option<(f32, f32, f32)> {
    let [x, y, w] = vis.prev[o];
    (vis.owner[o] != u32::MAX && w != 0.0).then(|| (x / w, y / w, w))
}

/// Where a vector from pixel `o` lands in the frame before.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Land {
    /// Off the picture (or not a number).
    Off,
    /// On another surface (1/z off by more than 10%) or nothing.
    Occluded,
    /// On the same surface.
    Seen,
}

/// Follow pixel `o`'s previous position `(px, py)` with previous 1/z `e`
/// into `prev`: the 2x2 texels around it, any with 1/z within 10%.
fn land(prev: &Prev, w: usize, h: usize, px: f32, py: f32, e: f32) -> Land {
    if !(px.is_finite() && py.is_finite()) || px < 0.0 || py < 0.0 || px >= w as f32 || py >= h as f32 {
        return Land::Off;
    }
    let (x0, y0) = ((px - 0.5).floor() as isize, (py - 0.5).floor() as isize);
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        let (x, y) = (x0 + dx, y0 + dy);
        if x < 0 || y < 0 || x >= w as isize || y >= h as isize {
            continue;
        }
        let iz = prev.vis.iz[y as usize * w + x as usize];
        if iz > 0.0 && (iz - e).abs() <= 0.1 * iz.max(e) {
            return Land::Seen;
        }
    }
    Land::Occluded
}

/// Bilinear sample of a 3D layer at `(px, py)` (pixel centres at .5) over
/// covered texels only; None if none is.
fn bilinear(layer: &(Vec<u8>, Vec<bool>), w: usize, h: usize, px: f32, py: f32) -> Option<[f32; 3]> {
    let (fx, fy) = (px - 0.5, py - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let (mut sum, mut wsum) = ([0.0f32; 3], 0.0);
    for (dx, dy, wt) in
        [(0, 0, (1.0 - tx) * (1.0 - ty)), (1, 0, tx * (1.0 - ty)), (0, 1, (1.0 - tx) * ty), (1, 1, tx * ty)]
    {
        let (x, y) = ((x0 as isize + dx).clamp(0, w as isize - 1), (y0 as isize + dy).clamp(0, h as isize - 1));
        let o = y as usize * w + x as usize;
        if layer.1[o] && wt > 0.0 {
            for (s, &v) in sum.iter_mut().zip(&layer.0[o * 3..o * 3 + 3]) {
                *s += wt * v as f32;
            }
            wsum += wt;
        }
    }
    (wsum > 0.0).then(|| sum.map(|s| s / wsum))
}

/// Per-pixel motion vectors followed into frame N-1's picture.
#[derive(Default)]
pub struct Vectors {
    frames: u64,
    /// 3D pixels, with motion data, landing off screen / occluded / seen.
    covered: u64,
    with: u64,
    off: u64,
    occluded: u64,
    seen: u64,
    /// Over seen pixels: reprojection error and the error without
    /// reprojection (mean absolute per channel, summed), and those off by
    /// more than 32.
    err: f64,
    base: f64,
    bad: u64,
    /// The same with the layout twin's vectors (over its seen pixels).
    twin_seen: u64,
    twin_err: f64,
    /// Seen pixels and reprojection error of frames that have a twin.
    on_twin: (u64, f64),
    twin_base: f64,
    twin_bad: u64,
    /// Pixels with vectors where the twin has one too, and of those whose
    /// vectors differ by more than 1 px (output).
    compared: u64,
    differ: u64,
    /// Per frame (frame, err, base, bad%, differ%) for frames with motion.
    per_frame: Vec<(u64, f64, f64, f64, f64)>,
    /// Vector lengths (output px) of pixels with motion, sampled.
    lengths: Vec<f32>,
    /// By previous W (z' / z) bucket: pixels, of them seen, and of those bad.
    by_w: [(u64, u64, u64); W_BUCKETS.len() + 1],
    /// By vector length bucket: the same.
    by_len: [(u64, u64, u64); LEN_BUCKETS.len() + 1],
    /// Frames where s0 == s1 doesn't give zero motion.
    identity_fail: Vec<u64>,
}

/// Upper bounds of the W buckets (previous depth over current).
const W_BUCKETS: [f32; 6] = [0.1, 0.25, 0.5, 2.0, 4.0, 10.0];
/// Upper bounds of the length buckets (game pixels).
const LEN_BUCKETS: [f32; 5] = [1.0, 8.0, 32.0, 128.0, 496.0];

fn bucket(b: &[f32], x: f32) -> usize {
    b.iter().position(|&u| x < u).unwrap_or(b.len())
}

impl Vectors {
    /// s0 == s1 must give every point its own position, W = 1.
    fn identity(cx: &Cx) -> bool {
        let s = &cx.f.frame.stream;
        let mi = MotionIn { s0: s, s1: s, tp: 0.0, valid: &[], prev_centres: &[] };
        let (polys, _) = raster::build_motion(s, cx.src, Some(&mi));
        polys.iter().all(|p| {
            p.prev.len() == p.points.len()
                && p.points.iter().zip(&p.prev).all(|(v, q)| {
                    (q[2] - 1.0).abs() <= 1e-6
                        && (q[0] - v.x).abs() <= 1e-3 * (1.0 + v.x.abs())
                        && (q[1] - v.y).abs() <= 1e-3 * (1.0 + v.y.abs())
                })
        })
    }
}

/// A motion vector as a colour: hue for the direction, saturation and
/// brightness for the length (full at 32 output px); white where there is
/// no motion data, black where there is no 3D.
fn mv_colour(dx: f32, dy: f32) -> [u8; 3] {
    let len = (dx * dx + dy * dy).sqrt();
    let hue = (dy.atan2(dx) / std::f32::consts::TAU + 1.0).fract() * 6.0;
    let (s, v) = ((len / 2.0).min(1.0), 0.3 + 0.7 * (len / 32.0).min(1.0));
    let (i, f) = (hue.floor() as u32 % 6, hue.fract());
    let (p, q, t) = (v * (1.0 - s), v * (1.0 - s * f), v * (1.0 - s * (1.0 - f)));
    let c = match i {
        0 => [v, t, p],
        1 => [q, v, p],
        2 => [p, v, t],
        3 => [p, q, v],
        4 => [t, p, v],
        _ => [v, p, q],
    };
    c.map(|x| (x * 255.0).round() as u8)
}

impl Section for Vectors {
    fn header(&self) -> String {
        format!(
            "{:>6} {:>6} {:>6} {:>6} {:>7} {:>7} {:>6} {:>7} {:>6} {:>7}",
            "mv%", "seen%", "occl%", "off%", "err", "base", "bad%", "twinerr", "diff%", "|mv|p50"
        )
    }

    fn frame(&mut self, cx: &Cx) -> Result<String, String> {
        if !Self::identity(cx) {
            self.identity_fail.push(cx.f.frame.frame);
        }
        let empty = format!(
            "{:>6} {:>6} {:>6} {:>6} {:>7} {:>7} {:>6} {:>7} {:>6} {:>7}",
            "-", "-", "-", "-", "-", "-", "-", "-", "-", "-"
        );
        let (Kind::Motion, Some(prev)) = (cx.kind, cx.prev) else { return Ok(empty) };
        let (w, h) = (cx.view.width, cx.view.height);
        let (vis, layer) = (cx.vis, cx.layer);
        let (mut covered, mut with, mut off, mut occl, mut seen) = (0u64, 0u64, 0u64, 0u64, 0u64);
        let (mut err, mut base, mut bad) = (0f64, 0f64, 0u64);
        let (mut tseen, mut terr, mut tbase, mut tbad) = (0u64, 0f64, 0f64, 0u64);
        let (mut compared, mut differ) = (0u64, 0u64);
        let mut lengths = Vec::new();
        let sampled = cx.refs.is_some();
        let mut mv_png = if sampled { vec![0u8; w * h * 3] } else { Vec::new() };
        let mut reproj = if sampled { vec![0u8; w * h * 3] } else { Vec::new() };
        let cur = |o: usize| [0, 1, 2].map(|c| layer.0[o * 3 + c] as f32);
        let diff = |a: [f32; 3], b: [f32; 3]| (0..3).map(|c| (a[c] - b[c]).abs()).sum::<f32>() / 3.0;
        // Follow pixel o's vector into the frame before.
        let follow = |o: usize, (px, py, pw): (f32, f32, f32)| -> (Land, Option<[f32; 3]>) {
            let e = vis.iz[o] / pw;
            match land(prev, w, h, px, py, e) {
                Land::Seen => (Land::Seen, bilinear(&prev.layer, w, h, px, py)),
                l => (l, None),
            }
        };
        for y in 0..h {
            for x in 0..w {
                let o = y * w + x;
                if vis.owner[o] == u32::MAX {
                    continue;
                }
                covered += 1;
                let Some(pp) = prev_at(vis, o) else {
                    if sampled {
                        mv_png[o * 3..o * 3 + 3].copy_from_slice(&[255, 255, 255]);
                        reproj[o * 3..o * 3 + 3].copy_from_slice(&[255, 0, 255]);
                    }
                    continue;
                };
                with += 1;
                let (dx, dy) = (pp.0 - (x as f32 + 0.5), pp.1 - (y as f32 + 0.5));
                let len = (dx * dx + dy * dy).sqrt();
                if o % 7 == 0 {
                    lengths.push(if len.is_finite() { len } else { f32::MAX });
                }
                let wb = bucket(&W_BUCKETS, pp.2);
                let lb = bucket(&LEN_BUCKETS, len / cx.view.sx);
                self.by_w[wb].0 += 1;
                self.by_len[lb].0 += 1;
                let base_here = diff([0, 1, 2].map(|c| prev.layer.0[o * 3 + c] as f32), cur(o));
                match follow(o, pp) {
                    (Land::Seen, Some(c)) => {
                        seen += 1;
                        let e = diff(c, cur(o));
                        err += e as f64;
                        base += base_here as f64;
                        let b = e > 32.0;
                        bad += b as u64;
                        self.by_w[wb].1 += 1;
                        self.by_len[lb].1 += 1;
                        self.by_w[wb].2 += b as u64;
                        self.by_len[lb].2 += b as u64;
                        if sampled {
                            let c = c.map(|v| v.round() as u8);
                            reproj[o * 3..o * 3 + 3].copy_from_slice(&c);
                        }
                    }
                    (Land::Off, _) => {
                        off += 1;
                        if sampled {
                            reproj[o * 3..o * 3 + 3].copy_from_slice(&[255, 255, 0]);
                        }
                    }
                    _ => {
                        occl += 1;
                        if sampled {
                            reproj[o * 3..o * 3 + 3].copy_from_slice(&[0, 64, 255]);
                        }
                    }
                }
                if sampled {
                    mv_png[o * 3..o * 3 + 3].copy_from_slice(&mv_colour(dx, dy));
                }
                if let Some(tp) = cx.twin.and_then(|(_, tv)| prev_at(tv, o)) {
                    compared += 1;
                    let d = ((tp.0 - pp.0).powi(2) + (tp.1 - pp.1).powi(2)).sqrt();
                    differ += (d.is_nan() || d > 1.0) as u64;
                    if let (Land::Seen, Some(c)) = follow(o, tp) {
                        tseen += 1;
                        let e = diff(c, cur(o));
                        terr += e as f64;
                        tbase += base_here as f64;
                        tbad += (e > 32.0) as u64;
                    }
                }
            }
        }
        self.frames += 1;
        self.covered += covered;
        self.with += with;
        self.off += off;
        self.occluded += occl;
        self.seen += seen;
        self.err += err;
        self.base += base;
        self.bad += bad;
        self.twin_seen += tseen;
        self.twin_err += terr;
        if cx.twin.is_some() {
            self.on_twin.0 += seen;
            self.on_twin.1 += err;
        }
        self.twin_base += tbase;
        self.twin_bad += tbad;
        self.compared += compared;
        self.differ += differ;
        lengths.sort_by(f32::total_cmp);
        let mean = |s: f64, n: u64| s / n.max(1) as f64;
        self.per_frame.push((
            cx.f.frame.frame,
            mean(err, seen),
            mean(base, seen),
            pct(bad, seen),
            pct(differ, compared),
        ));
        self.lengths.extend_from_slice(&lengths);
        if sampled {
            let name = |s: &str| cx.out.join(format!("{:06}-{s}.png", cx.f.frame.frame));
            let _ = std::fs::write(name("mv"), png::encode_rgb(w, h, &mv_png));
            // Frame N-1 brought forward by the vectors | frame N |
            // difference x4 (yellow: off screen, blue: occluded, magenta:
            // no motion data).
            let mut tri = vec![0u8; w * 3 * h * 3];
            for y in 0..h {
                for x in 0..w {
                    let o = y * w + x;
                    let row = y * w * 3;
                    tri[(row + x) * 3..(row + x) * 3 + 3].copy_from_slice(&reproj[o * 3..o * 3 + 3]);
                    tri[(row + w + x) * 3..(row + w + x) * 3 + 3].copy_from_slice(&layer.0[o * 3..o * 3 + 3]);
                    let d: [u8; 3] = if vis.owner[o] != u32::MAX && prev_at(vis, o).is_some() {
                        [0, 1, 2].map(|c| reproj[o * 3 + c].abs_diff(layer.0[o * 3 + c]).saturating_mul(4))
                    } else {
                        [0; 3]
                    };
                    tri[(row + 2 * w + x) * 3..(row + 2 * w + x) * 3 + 3].copy_from_slice(&d);
                }
            }
            let _ = std::fs::write(name("reproj"), png::encode_rgb(w * 3, h, &tri));
        }
        Ok(format!(
            "{:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>7.2} {:>7.2} {:>6.2} {:>7} {:>6} {:>7.2}",
            pct(with, covered),
            pct(seen, with),
            pct(occl, with),
            pct(off, with),
            mean(err, seen),
            mean(base, seen),
            pct(bad, seen),
            if cx.twin.is_some() { format!("{:.2}", mean(terr, tseen)) } else { "-".into() },
            if cx.twin.is_some() { format!("{:.2}", pct(differ, compared)) } else { "-".into() },
            quantile(&lengths, 0.5)
        ))
    }

    fn summary(&self, r: &mut Report) {
        let mean = |s: f64, n: u64| s / n.max(1) as f64;
        r.line(format!(
            "vectors       {} frames with motion: {:.1}% of 3D pixels have motion data; of those {:.1}% land on the same surface in frame N-1, {:.1}% occluded, {:.1}% off screen",
            self.frames,
            pct(self.with, self.covered),
            pct(self.seen, self.with),
            pct(self.occluded, self.with),
            pct(self.off, self.with)
        ));
        r.line(format!(
            "  reprojection error {:.2} levels (without reprojection {:.2}), {:.2}% over 32; layout twin's vectors {:.2} (without {:.2}), {:.2}% over 32",
            mean(self.err, self.seen),
            mean(self.base, self.seen),
            pct(self.bad, self.seen),
            mean(self.twin_err, self.twin_seen),
            mean(self.twin_base, self.twin_seen),
            pct(self.twin_bad, self.twin_seen)
        ));
        r.line(format!(
            "  vectors differing from the layout twin's by more than 1 px: {:.2}% of {} pixels",
            pct(self.differ, self.compared),
            self.compared
        ));
        let mut l = self.lengths.clone();
        l.sort_by(f32::total_cmp);
        r.line(format!(
            "  |mv| (output px): p50 {:.2} p90 {:.2} p99 {:.2} p99.9 {:.1} max {:.1}",
            quantile(&l, 0.5),
            quantile(&l, 0.9),
            quantile(&l, 0.99),
            quantile(&l, 0.999),
            quantile(&l, 1.0)
        ));
        let row = |name: String, (n, s, b): (u64, u64, u64)| {
            format!("    {name:<14} {n:>10} px  seen {:>5.1}%  bad {:>5.1}% of seen", pct(s, n), pct(b, s))
        };
        r.line("  by previous W (z'/z):".into());
        let lo = |b: &[f32], k: usize| if k == 0 { 0.0 } else { b[k - 1] };
        for (k, c) in self.by_w.iter().enumerate().filter(|(_, c)| c.0 > 0) {
            let hi = W_BUCKETS.get(k).map_or("inf".into(), |v| v.to_string());
            r.line(row(format!("{}..{hi}", lo(&W_BUCKETS, k)), *c));
        }
        r.line("  by length (game px):".into());
        for (k, c) in self.by_len.iter().enumerate().filter(|(_, c)| c.0 > 0) {
            let hi = LEN_BUCKETS.get(k).map_or("inf".into(), |v| v.to_string());
            r.line(row(format!("{}..{hi}", lo(&LEN_BUCKETS, k)), *c));
        }
        // Information only: the colour error has a floor of its own (the
        // software renderer's point-sampled textures resampled at a
        // fraction of a pixel, lighting that changes, surfaces within 10%
        // in depth), which exact vectors (the layout twin's) show too.
        let moving: Vec<_> = self.per_frame.iter().filter(|f| f.2 >= 2.0).collect();
        let failing: Vec<String> = moving.iter().filter(|f| f.1 > 0.5 * f.2).map(|f| f.0.to_string()).collect();
        r.line(format!(
            "  frames with base >= 2: {}; with err over 0.5 base: {} {}",
            moving.len(),
            failing.len(),
            failing.iter().take(40).cloned().collect::<Vec<_>>().join(" ")
        ));
        r.check(
            self.identity_fail.is_empty(),
            format!("s0 == s1 gives zero motion on every frame (fails: {:?})", self.identity_fail),
        );
        if self.seen > 0 {
            let (e, b) = (mean(self.err, self.seen), mean(self.base, self.seen));
            r.check(e <= 0.5 * b || b < 2.0, format!("reprojection error {e:.2} <= 0.5 x {b:.2} without"));
        }
        if self.compared > 0 {
            // Where the vectors differ from the twin's, one of them is wrong
            // (the twin is when the game reordered its objects): the
            // colours decide. Over the frames that have a twin.
            let (e, t) = (mean(self.on_twin.1, self.on_twin.0), mean(self.twin_err, self.twin_seen));
            r.check(
                e <= t + 0.25,
                format!(
                    "reprojection error {e:.2} <= the layout twin's {t:.2} + 0.25 (vectors differ on {:.2}%)",
                    pct(self.differ, self.compared)
                ),
            );
        }
    }
}

/// Mean absolute difference per channel and the share of pixels off by
/// more than 16 in a channel, over all pixels and outside `skip`.
fn score(a: &[u8], b: &[u8], skip: &[bool]) -> [f64; 4] {
    let (mut s, mut l, mut n, mut s2, mut l2, mut n2) = (0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
    for (i, &k) in skip.iter().enumerate() {
        let d = [0, 1, 2].map(|c| a[i * 3 + c].abs_diff(b[i * 3 + c]) as u64);
        let (sum, large) = (d.iter().sum::<u64>(), d.iter().any(|&x| x > 16) as u64);
        (s, l, n) = (s + sum, l + large, n + 1);
        if !k {
            (s2, l2, n2) = (s2 + sum, l2 + large, n2 + 1);
        }
    }
    [s as f64 / (3 * n).max(1) as f64, pct(l, n), s2 as f64 / (3 * n2).max(1) as f64, pct(l2, n2)]
}

/// `view` drawn at `scale` (percent) of its size, as a temporal renderer's
/// internal target is: the same placement, fewer pixels.
pub fn internal_view(view: &View, scale: u32) -> View {
    let s = scale as f32 / 100.0;
    let (iw, ih) =
        (((view.width as f32 * s).round() as usize).max(1), ((view.height as f32 * s).round() as usize).max(1));
    View {
        width: iw,
        height: ih,
        sx: view.sx * iw as f32 / view.width as f32,
        sy: view.sy * ih as f32 / view.height as f32,
        ..*view
    }
}

/// Compose a frame from the 2D layers and a premultiplied RGBA 3D layer
/// (0 to 1, at the view's size), as the GPU's compose pass does and as
/// `video::compose` does for a layer with whole coverage.
pub fn compose_rgba(layers: &video::Layers2d, view: &View, layer: &[[f32; 4]]) -> Vec<u8> {
    let wide_w = video::WIDTH + 2 * layers.side;
    let has_3d = layer.iter().any(|p| p[3] > 0.0);
    let mut out = vec![0u8; view.width * view.height * 3];
    for oy in 0..view.height {
        let gy = ((oy as f32 + 0.5) / view.sy) as usize;
        if gy >= video::HEIGHT {
            continue;
        }
        for ox in 0..view.width {
            let o = oy * view.width + ox;
            let ex = (ox as f32 + 0.5) / view.sx;
            let bx = (ex - view.pad).floor() as isize + layers.side as isize;
            let c = if (0..wide_w as isize).contains(&bx) {
                let k = (gy * wide_w + bx as usize) * 4;
                [layers.base[k], layers.base[k + 1], layers.base[k + 2]]
            } else if has_3d {
                layers.backdrop
            } else {
                [0, 0, 0]
            };
            let d = layer[o];
            let mut c = [0, 1, 2]
                .map(|k| ((d[k] + (1.0 - d[3]) * c[k] as f32 / 255.0) * 255.0).round().clamp(0.0, 255.0) as u8);
            let hk = (gy * layers.ext_w + ex as usize) * 4;
            if layers.hud.get(hk + 3).is_some_and(|&a| a != 0) {
                c = [layers.hud[hk], layers.hud[hk + 1], layers.hud[hk + 2]];
            }
            out[o * 3..o * 3 + 3].copy_from_slice(&c);
        }
    }
    out
}

/// A software 3D layer as premultiplied RGBA.
fn premultiplied(layer: &(Vec<u8>, Vec<bool>)) -> Vec<[f32; 4]> {
    layer
        .1
        .iter()
        .enumerate()
        .map(|(o, &c)| {
            if c {
                [0, 1, 2, 3].map(|k| if k == 3 { 1.0 } else { layer.0[o * 3 + k] as f32 / 255.0 })
            } else {
                [0.0; 4]
            }
        })
        .collect()
}

/// Bilinear enlargement of an `iw` x `ih` RGBA layer to `w` x `h`.
fn enlarge(src: &[[f32; 4]], iw: usize, ih: usize, w: usize, h: usize) -> Vec<[f32; 4]> {
    let mut out = vec![[0.0; 4]; w * h];
    let (rx, ry) = (iw as f32 / w as f32, ih as f32 / h as f32);
    for y in 0..h {
        let fy = ((y as f32 + 0.5) * ry - 0.5).clamp(0.0, ih as f32 - 1.0);
        let (y0, ty) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(ih - 1);
        for x in 0..w {
            let fx = ((x as f32 + 0.5) * rx - 0.5).clamp(0.0, iw as f32 - 1.0);
            let (x0, tx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(iw - 1);
            let p = |xx: usize, yy: usize| src[yy * iw + xx];
            out[y * w + x] = std::array::from_fn(|k| {
                let a = p(x0, y0)[k] + (p(x1, y0)[k] - p(x0, y0)[k]) * tx;
                let b = p(x0, y1)[k] + (p(x1, y1)[k] - p(x0, y1)[k]) * tx;
                a + (b - a) * ty
            });
        }
    }
    out
}

/// The plain frame and naive upscales against the reference.
#[derive(Default)]
pub struct Baselines {
    frames: u64,
    /// Per column (off, naive at each of SCALES): summed scores.
    sums: Vec<[f64; 4]>,
    stipple: f64,
    /// Pixels where `compose_rgba` differs from `video::compose` (a self-check).
    compose_mismatch: u64,
}

impl Section for Baselines {
    fn header(&self) -> String {
        let mut s = format!("{:>7}", "off");
        for k in SCALES {
            s += &format!(" {:>7}", format!("n{k}"));
        }
        s
    }

    fn frame(&mut self, cx: &Cx) -> Result<String, String> {
        let Some(refs) = cx.refs else {
            return Ok(format!("{:>7}", "") + &" ".repeat(8 * SCALES.len()));
        };
        let f = &cx.f.frame;
        let view = cx.view;
        let name = |s: &str| cx.out.join(format!("{:06}-{s}.png", f.frame));
        let save = |s: &str, rgb: &[u8]| {
            let _ = std::fs::write(name(s), png::encode_rgb(view.width, view.height, rgb));
        };
        let off = video::compose(&f.tiles, &f.palette, Some((&cx.layer.0, &cx.layer.1)), view);
        let layers = video::layers_2d(&f.tiles, &f.palette, view, cx.layer.1.iter().any(|&c| c));
        let again = compose_rgba(&layers, view, &premultiplied(cx.layer));
        self.compose_mismatch +=
            off.as_chunks::<3>().0.iter().zip(again.as_chunks::<3>().0).filter(|(a, b)| a != b).count() as u64;
        save("off", &off);
        let mut cols = vec![score(&off, &refs.reference, &refs.stipple)];
        for s in SCALES {
            let iv = internal_view(view, s);
            let mut p = cx.polys.to_vec();
            let small = raster::draw_view_depth(&mut p, cx.src, &iv, false);
            let big = enlarge(&premultiplied(&small), iv.width, iv.height, view.width, view.height);
            let rgb = compose_rgba(&layers, view, &big);
            save(&format!("naive{s}"), &rgb);
            cols.push(score(&rgb, &refs.reference, &refs.stipple));
        }
        save("ref", &refs.reference);
        if self.sums.is_empty() {
            self.sums = vec![[0.0; 4]; cols.len()];
        }
        for (t, c) in self.sums.iter_mut().zip(&cols) {
            for k in 0..4 {
                t[k] += c[k];
            }
        }
        self.stipple += pct(refs.stipple.iter().filter(|&&s| s).count() as u64, refs.stipple.len() as u64);
        self.frames += 1;
        Ok(cols.iter().map(|c| format!("{:>7.2}", c[0])).collect::<Vec<_>>().join(" "))
    }

    fn summary(&self, r: &mut Report) {
        let n = self.frames.max(1) as f64;
        r.line(format!(
            "baselines     {} sampled frames against the {REF_SS}x{REF_SS} supersampled reference (stipple pixels {:.2}% of the frame): mean |difference| a channel / % over 16; outside stipple",
            self.frames,
            self.stipple / n
        ));
        let mut names = vec!["off".to_string()];
        names.extend(SCALES.iter().map(|s| format!("naive {s}%")));
        for (name, s) in names.iter().zip(&self.sums) {
            r.line(format!(
                "  {name:<10} {:>6.2} / {:>5.2}%   outside stipple {:>6.2} / {:>5.2}%",
                s[0] / n,
                s[1] / n,
                s[2] / n,
                s[3] / n
            ));
        }
        r.check(
            self.compose_mismatch == 0,
            format!("CPU premultiplied compose == video::compose ({} pixels differ)", self.compose_mismatch),
        );
    }
}

impl Refs {
    /// [`Refs::new`], kept in `cache` (if given) under the frame's number
    /// and the view's size.
    fn cached(
        cache: Option<&Path>,
        polys: &[Poly],
        src: &Sources,
        view: &View,
        f: &model2::capture::Frame,
        plain: &(Vec<u8>, Vec<bool>),
    ) -> Refs {
        let file = cache.map(|c| c.join(format!("{:06}-{}x{}.ref", f.frame, view.width, view.height)));
        let n = view.width * view.height;
        if let Some(b) = file.as_ref().and_then(|p| std::fs::read(p).ok()).filter(|b| b.len() == n * 4) {
            return Refs { reference: b[..n * 3].to_vec(), stipple: b[n * 3..].iter().map(|&x| x != 0).collect() };
        }
        let r = Refs::new(polys, src, view, f, plain);
        if let Some(p) = file {
            let mut b = r.reference.clone();
            b.extend(r.stipple.iter().map(|&x| x as u8));
            if let Some(d) = p.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            let _ = std::fs::write(p, b);
        }
        r
    }

    fn new(
        polys: &[Poly],
        src: &Sources,
        view: &View,
        f: &model2::capture::Frame,
        plain: &(Vec<u8>, Vec<bool>),
    ) -> Refs {
        let big = view.supersampled(REF_SS);
        let mut p = polys.to_vec();
        let layer = raster::draw_view_depth(&mut p, src, &big, false);
        let frame = video::compose(&f.tiles, &f.palette, Some((&layer.0, &layer.1)), &big);
        drop(layer);
        let reference = video::downsample(&frame, big.width, big.height, REF_SS);
        // Stipple: what the checkerboard polygons change.
        let mut p: Vec<Poly> = polys.iter().filter(|p| !p.checker).cloned().collect();
        let without = raster::draw_view_depth(&mut p, src, view, false);
        let stipple = (0..plain.1.len())
            .map(|o| {
                plain.1[o] != without.1[o] || (plain.1[o] && plain.0[o * 3..o * 3 + 3] != without.0[o * 3..o * 3 + 3])
            })
            .collect();
        Refs { reference, stipple }
    }
}

/// Check sequence `root` (see the module comment); false if a check failed.
pub fn run(root: &Path, opts: &Options) -> Result<bool, String> {
    let mut seq =
        Seq::open(root).map_err(|e| format!("{}: {e} (make one with `make temporal-capture`)", root.display()))?;
    let texture_rom = std::mem::take(&mut seq.texture_rom);
    let mut view = opts.view;
    if let Ok(g) = romset::games::find(&seq.game) {
        view = view.with_hud_windows(g.hud_windows, g.hud_element_windows);
    }
    std::fs::create_dir_all(&opts.out).map_err(|e| format!("{}: {e}", opts.out.display()))?;
    let mut text = String::new();
    let mut say = |s: String| {
        println!("{s}");
        let _ = writeln!(text, "{s}");
    };
    say(format!("sequence      {} ({}, {} frames)", root.display(), seq.game, seq.len()));
    say(format!(
        "view          {}x{}; sampled every {} frames from frame {WARMUP} of the sequence (pictures in {})",
        view.width,
        view.height,
        opts.every,
        opts.out.display()
    ));
    let mut sections: Vec<Box<dyn Section>> =
        vec![Box::new(Pose::default()), Box::new(Vectors::default()), Box::new(Baselines::default())];
    if opts.gpu_only {
        sections.clear();
    }
    if opts.gpu || opts.gpu_only {
        let g = Gpu::new(opts)?;
        say(format!("gpu           {}", g.adapter()));
        sections.push(Box::new(g));
    }
    let header = sections.iter().map(|s| s.header()).collect::<Vec<_>>().join(" ");
    say(format!("{:<7} {:<6} {header}", "frame", "kind"));
    let (mut prev, mut counts) = (None::<Prev>, [0u64; 4]);
    // The reference of the frame before a sampled one (GPU section).
    let mut held_ref: Option<(u64, Refs)> = None;
    // Frames are numbered from the first directory's.
    let first = std::fs::read_dir(root)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter_map(|e| e.file_name().to_str()?.parse::<u64>().ok())
        .min()
        .unwrap_or(0);
    let sampled = |index: usize| {
        (index >= WARMUP && (index - WARMUP).is_multiple_of(opts.every.max(1)))
            || opts.at.contains(&(first + index as u64))
    };
    let started = std::time::Instant::now();
    for (index, f) in seq.enumerate() {
        let f = f.map_err(|e| e.to_string())?;
        let src = f.frame.sources(&texture_rom);
        let stream = &f.frame.stream;
        let before = prev.as_ref().filter(|p| p.frame + 1 == f.frame.frame);
        let mi = match (&f.motion, before) {
            (Some(m), _) if !m.cut && m.s0.len() == stream.len() => Some(MotionIn {
                s0: &m.s0,
                s1: stream,
                tp: 0.0,
                valid: &m.valid,
                prev_centres: before.map_or(&[][..], |p| &p.centres),
            }),
            _ => None,
        };
        let (mut polys, centres) = raster::build_motion(stream, &src, mi.as_ref());
        let kind = match (&f.motion, before) {
            (_, None) => Kind::First,
            (Some(m), _) if m.cut => Kind::Cut,
            _ if polys.iter().any(|p| !p.prev.is_empty()) => Kind::Motion,
            _ => Kind::NoMotion,
        };
        counts[kind as usize] += 1;
        let hud = video::HudLayout::new(&f.frame.tiles, &f.frame.palette, &view);
        hud.anchor(&mut polys, &view);
        let placed = raster::place(&mut polys, &view);
        let layer = raster::draw_view_depth(&mut polys.clone(), &src, &view, false);
        let vis = raster::visible(&polys, &placed, &src, &view);
        // The layout twin: previous positions from frame N-1's stream.
        let twin =
            before.filter(|p| !opts.gpu_only && p.stream.len() == stream.len() && kind == Kind::Motion).and_then(|p| {
                let mi = MotionIn { s0: &p.stream, s1: stream, tp: 0.0, valid: &[], prev_centres: &p.centres };
                let (mut tp, _) = raster::build_motion(stream, &src, Some(&mi));
                if tp.iter().all(|p| p.prev.is_empty()) {
                    return None;
                }
                hud.anchor(&mut tp, &view);
                let placed = raster::place(&mut tp, &view);
                let vis = raster::visible(&tp, &placed, &src, &view);
                Some((placed, vis))
            });
        let cache = opts.ref_cache.as_deref();
        let refs = sampled(index).then(|| Refs::cached(cache, &polys, &src, &view, &f.frame, &layer));
        let prev_ref = held_ref.take().filter(|(n, _)| *n + 1 == f.frame.frame).map(|(_, r)| r);
        let cx = Cx {
            f: &f,
            src: &src,
            view: &view,
            kind,
            polys: &polys,
            placed: &placed,
            layer: &layer,
            vis: &vis,
            twin: twin.as_ref().map(|(p, v)| (&p[..], v)),
            prev: before,
            refs: refs.as_ref(),
            prev_ref: prev_ref.as_ref().filter(|_| refs.is_some()),
            next_sampled: sampled(index + 1),
            out: &opts.out,
        };
        let mut cells = Vec::new();
        for s in &mut sections {
            cells.push(s.frame(&cx)?);
        }
        let k = match kind {
            Kind::Motion => "motion",
            Kind::Cut => "CUT",
            Kind::NoMotion => "none",
            Kind::First => "first",
        };
        say(format!("{:<7} {k:<6} {}", f.frame.frame, cells.join(" ")));
        if opts.gpu && sampled(index + 1) {
            held_ref = Some((
                f.frame.frame,
                refs.unwrap_or_else(|| Refs::cached(cache, &polys, &src, &view, &f.frame, &layer)),
            ));
        }
        prev = Some(Prev { frame: f.frame.frame, stream: f.frame.stream.clone(), centres, layer, vis });
    }
    let mut r = Report::default();
    r.line(format!(
        "frames        {} with motion, {} cuts, {} without motion data, {} first (or after a gap); {:.1}s",
        counts[Kind::Motion as usize],
        counts[Kind::Cut as usize],
        counts[Kind::NoMotion as usize],
        counts[Kind::First as usize],
        started.elapsed().as_secs_f64()
    ));
    for s in &sections {
        s.summary(&mut r);
    }
    for l in &r.lines {
        say(l.clone());
    }
    let pass = r.fails.is_empty();
    say(format!("result        {}", if pass { "PASS".to_string() } else { format!("FAIL ({})", r.fails.join("; ")) }));
    if opts.gpu {
        say(format!(
            "gpu pictures  {}/FRAME-{{gpu-off,msaa4,taa,up67,up50}}.png, FRAME-taa-change.png (TAA against its frame before brought forward, x4: red still pixels, green moving); FRAME-cropK.png (each crop enlarged {CROP_ZOOM}x: gref off msaa4 taa / up67 up50 naive67 naive50, all GPU; gref = the GPU's 4x4 supersampled frame, FRAME-gref.png)",
            opts.out.display()
        ));
    }
    say(format!(
        "pictures      {}/FRAME-{{ref,off,naive67,naive50}}.png, -mv.png (motion vectors: hue direction, brightness length, white no data), -reproj.png (frame N-1 brought forward | frame N | difference x4; yellow off screen, blue occluded, magenta no data)",
        opts.out.display()
    ));
    drop(say);
    let _ = std::fs::write(opts.out.join("report.txt"), &text);
    Ok(pass)
}

/// Enlargement of the crops.
const CROP_ZOOM: usize = 3;
/// Frames of the still test (one frame repeated), and the frame from which
/// its frame-to-frame change is measured.
const STILL_FRAMES: usize = 40;
const STILL_FROM: usize = 24;

/// One GPU column: a renderer drawing every frame of the sequence.
struct Col {
    name: &'static str,
    quality: gpu::Quality,
    r: gpu::Renderer,
    /// Its last frame (RGB).
    last: Option<Vec<u8>>,
    /// Summed scores against the reference, and against the GPU's own
    /// supersampled frame.
    score: [f64; 4],
    gscore: [f64; 4],
    /// Summed score against gref inside polygons ([`not_interior`]).
    iscore: [f64; 2],
    /// Summed [`detail`] inside polygons.
    detail: f64,
    /// Frame-to-frame change on reprojected pixels, still and moving: over
    /// all frames, and over the sampled frames whose reference has a pair.
    change: [(f64, u64); 2],
    change_paired: [(f64, u64); 2],
}

/// The resolve's rule at a pixel (`fs_inspect`'s output), as a category
/// of [`RULES`].
fn rule_of(d: [f32; 4]) -> usize {
    let r = d[0] as u32;
    let why = d[3] as u32 & 7;
    let (st0, st) = (r & 3, (r >> 2) & 3);
    match () {
        _ if r & 32 != 0 => 0,
        _ if r & 256 != 0 => 1,
        _ if st == 1 => 2,
        _ if st == 0 => 3,
        _ if r & 16 != 0 => 4,
        _ if r & 64 != 0 => 5,
        _ if why == 1 => 6,
        _ if why == 2 => 9,
        _ if why == 3 => 10,
        _ if r & 128 != 0 => 7,
        _ if st0 == 2 => 8,
        _ => 11,
    }
}

/// Names of the [`rule_of`] categories, and their colours in the rules
/// pictures.
const RULES: [(&str, [u8; 3]); 12] = [
    ("after reset", [128, 128, 128]),
    ("not finite", [255, 255, 255]),
    ("hidden: fallback", [255, 0, 0]),
    ("no data", [255, 0, 255]),
    ("dilated", [0, 255, 255]),
    ("blended stipple", [255, 255, 0]),
    ("count (1/(n+1))", [160, 160, 255]),
    ("seen, still (load)", [0, 110, 0]),
    ("seen, moving, base", [0, 230, 0]),
    ("seen, fast motion", [0, 80, 255]),
    ("seen, motion change", [255, 140, 0]),
    ("other", [90, 50, 20]),
];

/// The error budget of a temporal column against the plain frame, by
/// [`rule_of`]: pixels, and summed |difference| a channel against gref
/// of the column and of the plain frame; each split into pixels whose
/// history the clip moved by more than 2 levels or not.
type Budget = [[(u64, f64, f64); 2]; 12];

/// The columns: plain, MSAA 4x, temporal at 100, 67, 50.
fn columns(tuning: Option<gpu::TemporalTuning>, depth: bool) -> Result<Vec<Col>, String> {
    let q = |msaa, temporal| gpu::Quality { msaa, temporal, depth, ..Default::default() };
    [("gpu-off", q(1, 0)), ("msaa4", q(4, 0)), ("taa", q(1, 100)), ("up67", q(1, 67)), ("up50", q(1, 50))]
        .into_iter()
        .map(|(name, quality)| {
            let mut r = gpu::Renderer::new()?;
            let got = r.set_quality(quality);
            if got != quality {
                return Err(format!("{name}: the GPU gives {got:?}"));
            }
            if let Some(t) = tuning {
                r.set_temporal_tuning(t);
            }
            Ok(Col {
                name,
                quality,
                r,
                last: None,
                score: [0.0; 4],
                gscore: [0.0; 4],
                iscore: [0.0; 2],
                detail: 0.0,
                change: [(0.0, 0); 2],
                change_paired: [(0.0, 0); 2],
            })
        })
        .collect()
}

/// The step a temporal renderer takes into a frame of `kind`, with the
/// motion data the tool built (previous frame, tp 0).
fn step_of(kind: Kind) -> gpu::Step {
    match kind {
        Kind::Motion => gpu::Step::Blend { tp: 0.0 },
        Kind::NoMotion => gpu::Step::NoMotion,
        Kind::Cut | Kind::First => gpu::Step::Reset,
    }
}

/// Bilinear sample of an RGB picture at `(px, py)` (pixel centres at .5).
fn bilinear_rgb(img: &[u8], w: usize, h: usize, px: f32, py: f32) -> [f32; 3] {
    let (fx, fy) = (px - 0.5, py - 0.5);
    let (x0, y0) = (fx.floor(), fy.floor());
    let (tx, ty) = (fx - x0, fy - y0);
    let at = |dx: isize, dy: isize, c: usize| {
        let x = (x0 as isize + dx).clamp(0, w as isize - 1) as usize;
        let y = (y0 as isize + dy).clamp(0, h as isize - 1) as usize;
        img[(y * w + x) * 3 + c] as f32
    };
    std::array::from_fn(|c| {
        let a = at(0, 0, c) + (at(1, 0, c) - at(0, 0, c)) * tx;
        let b = at(0, 1, c) + (at(1, 1, c) - at(0, 1, c)) * tx;
        a + (b - a) * ty
    })
}

/// Frame-to-frame change of a picture sequence where the scene is the
/// same: the last picture brought forward by the (CPU) motion vectors
/// against this one, mean |difference| a channel, over 3D pixels whose
/// surface was seen in the frame before; split into still (|mv| < 0.5
/// px) and moving pixels, leaving out `skip` (blended stipple: there a
/// still overlay shows the moving scene through). Aliasing crawl and
/// jitter shimmer show here.
fn change(cx: &Cx, skip: &[bool], last: &[u8], now: &[u8]) -> [(f64, u64); 2] {
    change_map(cx, skip, last, now, None)
}

/// [`change`], also drawn into `map` (RGB, the view's size: red still,
/// green moving, x4).
fn change_map(cx: &Cx, skip: &[bool], last: &[u8], now: &[u8], mut map: Option<&mut [u8]>) -> [(f64, u64); 2] {
    let Some(prev) = cx.prev else { return [(0.0, 0); 2] };
    let (w, h) = (cx.view.width, cx.view.height);
    let mut out = [(0.0, 0u64); 2];
    for o in 0..w * h {
        let Some((px, py, pw)) = prev_at(cx.vis, o) else { continue };
        if skip.get(o).copied().unwrap_or(false) {
            continue;
        }
        if land(prev, w, h, px, py, cx.vis.iz[o] / pw) != Land::Seen {
            continue;
        }
        let (x, y) = ((o % w) as f32 + 0.5, (o / w) as f32 + 0.5);
        let moving = ((px - x).powi(2) + (py - y).powi(2)).sqrt() >= 0.5;
        let c = bilinear_rgb(last, w, h, px, py);
        let d: f32 = (0..3).map(|k| (c[k] - now[o * 3 + k] as f32).abs()).sum::<f32>() / 3.0;
        out[moving as usize].0 += d as f64;
        out[moving as usize].1 += 1;
        if let Some(m) = map.as_deref_mut() {
            m[o * 3 + (moving as usize)] = (d * 4.0).min(255.0) as u8;
        }
    }
    out
}

fn add2(a: &mut [(f64, u64); 2], b: [(f64, u64); 2]) {
    for k in 0..2 {
        a[k].0 += b[k].0;
        a[k].1 += b[k].1;
    }
}

fn mean2(a: (f64, u64)) -> f64 {
    a.0 / a.1.max(1) as f64
}

/// The GPU: velocity target, temporal columns, still frame.
pub struct Gpu {
    cols: Vec<Col>,
    /// Temporal at 100% without jitter: its aux target holds the vectors
    /// at pixel centres.
    mv: gpu::Renderer,
    tuning: Option<gpu::TemporalTuning>,
    crops: Vec<(usize, usize, usize, usize)>,
    /// Per crop: summed mean |difference| a channel against gref inside it,
    /// for off msaa4 taa up67 up50 naive67 naive50, and frames.
    crop_score: Vec<([f64; 7], u64)>,
    /// Sampled frames scored, and the GPU naive upscales' summed scores.
    scored: u64,
    naive: [[f64; 4]; 2],
    gnaive: [[f64; 4]; 2],
    /// The references' own frame-to-frame change (paired frames).
    ref_change: [(f64, u64); 2],
    gref_change: [(f64, u64); 2],
    /// The GPU's supersampled frame (4x4, plain) of the last frame, when
    /// it was drawn.
    gref_last: Option<(u64, Vec<u8>)>,
    /// Summed [`detail`] of gref inside polygons.
    gref_detail: f64,
    /// Velocity target against `raster::visible`: pixels compared (both
    /// have data, the same surface), within 0.05 px (or the half float's
    /// precision), within 0.25 px, summed |difference|; covered pixels
    /// where one has data and the other not, of covered.
    vcmp: u64,
    vfine: u64,
    vnear: u64,
    vsum: f64,
    presence: (u64, u64),
    /// Reprojection error of the software picture with the GPU's vectors
    /// and the CPU's over the same pixels (both land on their surface).
    err: (u64, f64, f64),
    /// Still frame results: (name, score against its reference, frame-to-
    /// frame change after STILL_FROM over 3D pixels).
    still: Vec<(String, [f64; 4], f64)>,
    still_frame: Option<u64>,
    /// This frame's blended-stipple pixels (from the velocity renderer).
    sheer: Vec<bool>,
    depth: bool,
    /// `--inspect`: per temporal column, its error budget by rule.
    inspect: bool,
    budget: Vec<(&'static str, Budget)>,
    /// `--dump`: the rectangle, and the text so far.
    dump: Option<(usize, usize, usize, usize, usize)>,
    dump_text: String,
}

impl Gpu {
    pub fn new(opts: &Options) -> Result<Self, String> {
        let mut mv = gpu::Renderer::new()?;
        mv.set_quality(gpu::Quality { temporal: 100, ..Default::default() });
        mv.set_temporal_jitter(false);
        Ok(Gpu {
            cols: columns(opts.tuning, opts.depth)?,
            depth: opts.depth,
            inspect: opts.inspect,
            budget: Vec::new(),
            dump: opts.dump,
            dump_text: String::new(),
            mv,
            tuning: opts.tuning,
            crops: opts.crops.clone(),
            crop_score: vec![([0.0; 7], 0); opts.crops.len()],
            scored: 0,
            naive: [[0.0; 4]; 2],
            gnaive: [[0.0; 4]; 2],
            ref_change: [(0.0, 0); 2],
            gref_change: [(0.0, 0); 2],
            gref_last: None,
            gref_detail: 0.0,
            vcmp: 0,
            vfine: 0,
            vnear: 0,
            vsum: 0.0,
            presence: (0, 0),
            err: (0, 0.0, 0.0),
            still: Vec::new(),
            still_frame: None,
            sheer: Vec::new(),
        })
    }

    pub fn adapter(&self) -> &str {
        self.mv.adapter()
    }

    /// The velocity target of this frame against the CPU's vectors.
    fn vectors(&mut self, cx: &Cx, layers: &video::Layers2d) {
        let tag = gpu::FrameTag::game(cx.f.frame.frame);
        self.mv.set_temporal_frame(tag, step_of(cx.kind), Vec::new());
        self.mv.frame(cx.polys, cx.placed, cx.src, cx.view, layers);
        let d = self.mv.temporal_debug();
        self.sheer = d.as_ref().map_or(Vec::new(), |d| d.aux.iter().map(|a| a[2] < 0.0).collect());
        let (Kind::Motion, Some(prev), Some(d)) = (cx.kind, cx.prev, d) else { return };
        let (w, h) = (cx.view.width, cx.view.height);
        if (d.width as usize, d.height as usize) != (w, h) {
            return;
        }
        let follow = |o: usize, px: f32, py: f32, e: f32| -> Option<f32> {
            if land(prev, w, h, px, py, e) != Land::Seen {
                return None;
            }
            let c = bilinear(&prev.layer, w, h, px, py)?;
            Some((0..3).map(|k| (c[k] - cx.layer.0[o * 3 + k] as f32).abs()).sum::<f32>() / 3.0)
        };
        for o in 0..w * h {
            let a = d.aux[o];
            // Blended stipple: the GPU's aux holds the checkerboard
            // polygon's, the CPU's picture the cells it leaves.
            if cx.vis.owner[o] == u32::MAX || a[2] <= 0.0 {
                continue;
            }
            let cpu = prev_at(cx.vis, o);
            let (gpu_has, cpu_has) = (a[3] > 0.0, cpu.is_some());
            self.presence.1 += 1;
            self.presence.0 += (gpu_has != cpu_has) as u64;
            let (Some((px, py, pw)), true) = (cpu, gpu_has) else { continue };
            let iz = cx.vis.iz[o];
            if (a[2].abs() - iz).abs() > 2e-3 * iz {
                continue;
            }
            let (x, y) = ((o % w) as f32 + 0.5, (o / w) as f32 + 0.5);
            let (cx_, cy_) = (px - x, py - y);
            let diff = ((a[0] - cx_).powi(2) + (a[1] - cy_).powi(2)).sqrt();
            let len = (cx_ * cx_ + cy_ * cy_).sqrt();
            self.vcmp += 1;
            self.vfine += (diff <= 0.05f32.max(1.5e-3 * len)) as u64;
            self.vnear += (diff <= 0.25) as u64;
            self.vsum += diff.min(1e4) as f64;
            if let (Some(eg), Some(ec)) = (follow(o, x + a[0], y + a[1], a[3]), follow(o, px, py, iz / pw)) {
                self.err.0 += 1;
                self.err.1 += eg as f64;
                self.err.2 += ec as f64;
            }
        }
    }

    /// One frame repeated: each column from a fresh renderer, the frame
    /// drawn STILL_FRAMES times (repeats after the first).
    fn still_test(&mut self, cx: &Cx, layers: &video::Layers2d, refs: &Refs, gref: &[u8]) -> Result<(), String> {
        let n = cx.view.width * cx.view.height;
        for mut c in columns(self.tuning, self.depth)? {
            let mut frames = Vec::new();
            for k in 0..STILL_FRAMES {
                let step = if k == 0 { gpu::Step::Reset } else { gpu::Step::Repeat };
                c.r.set_temporal_frame(gpu::FrameTag::game(cx.f.frame.frame), step, Vec::new());
                frames.push(c.r.frame(cx.polys, cx.placed, cx.src, cx.view, layers));
            }
            let mut ch = 0.0;
            let mut m = 0u64;
            for k in STILL_FROM + 1..STILL_FRAMES {
                for o in (0..n).filter(|&o| cx.vis.owner[o] != u32::MAX) {
                    ch += (0..3).map(|i| frames[k][o * 3 + i].abs_diff(frames[k - 1][o * 3 + i]) as f64).sum::<f64>()
                        / 3.0;
                    m += 1;
                }
            }
            let last = frames.last().unwrap();
            let (a, b) = (score(last, &refs.reference, &refs.stipple), score(last, gref, &refs.stipple));
            self.still.push((c.name.to_string(), [a[0], a[1], b[0], b[1]], ch / m.max(1) as f64));
            let _ = std::fs::write(
                cx.out.join(format!("{:06}-still-{}.png", cx.f.frame.frame, c.name)),
                png::encode_rgb(cx.view.width, cx.view.height, last),
            );
        }
        self.still_frame = Some(cx.f.frame.frame);
        Ok(())
    }
}

/// Pixels not inside a polygon: true unless the same polygon shows at the
/// pixel and its 8 neighbours (and it is not stipple). What is left are
/// texture interiors, where the plain frame is already what the reference
/// is (up to the texture's own sampling), so a temporal picture's error
/// there is its softening.
fn not_interior(cx: &Cx, stipple: &[bool]) -> Vec<bool> {
    let (w, h) = (cx.view.width, cx.view.height);
    let own = &cx.vis.owner;
    (0..w * h)
        .map(|o| {
            let (x, y) = (o % w, o / w);
            if stipple[o] || own[o] == u32::MAX || x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                return true;
            }
            !(0..9).all(|k| own[(y + k / 3 - 1) * w + x + k % 3 - 1] == own[o])
        })
        .collect()
}

/// Mean |Laplacian| of the luma (4 neighbours, levels) over the pixels
/// `skip` leaves: the picture's fine detail. Against the reference's, below
/// 1 is softer, above 1 sharper (or aliased).
fn detail(rgb: &[u8], w: usize, skip: &[bool]) -> f64 {
    let luma = |o: usize| 0.299 * rgb[o * 3] as f64 + 0.587 * rgb[o * 3 + 1] as f64 + 0.114 * rgb[o * 3 + 2] as f64;
    let (mut sum, mut n) = (0.0, 0u64);
    for (o, &k) in skip.iter().enumerate() {
        // Inside polygons only (never on the picture's edge).
        if k {
            continue;
        }
        sum += (4.0 * luma(o) - luma(o - 1) - luma(o + 1) - luma(o - w) - luma(o + w)).abs();
        n += 1;
    }
    sum / n.max(1) as f64
}

/// `--dump`: the resolve's decisions at each pixel of `rect` (TAA), with
/// the pictures there and the internal sample nearest the pixel.
fn dump_rect(
    cx: &Cx,
    (x0, y0, w, h): (usize, usize, usize, usize),
    d: &[[f32; 4]],
    (taa, off): (&[u8], &[u8]),
    dbg: Option<gpu::TemporalDebug>,
    hist: Option<&[[f32; 4]]>,
    gref: Option<&[u8]>,
) -> String {
    let vw = cx.view.width;
    let mut s = format!("frame {} {:?}", cx.f.frame.frame, cx.kind);
    if let Some(t) = &dbg {
        let _ = write!(s, " jitter {:.3},{:.3} step {:?}", t.jitter.0, t.jitter.1, t.step);
    }
    s += "\n  x    y    st0 st dil shr load fin thn | share  clip   trust-by  |mv|  | taa         off         gref        | sample: rgb         mv              1/z        prev 1/z\n";
    let rgb = |img: &[u8], o: usize| format!("{:3},{:3},{:3}", img[o * 3], img[o * 3 + 1], img[o * 3 + 2]);
    for y in y0..y0 + h {
        for x in x0..x0 + w {
            let o = y * vw + x;
            let Some(v) = d.get(o) else { continue };
            let r = v[0] as u32;
            let why = ["base", "reset", "fast", "change", "stipple", "nodata"].get(v[3] as usize & 7).unwrap_or(&"?");
            let _ = write!(
                s,
                "  {x:<4} {y:<4} {:<3} {:<2} {:<3} {:<3} {:<4} {:<3} {:<3} | {:.3}  {:.3}  {:<8} {:>5.2} | {} {} {} |",
                r & 3,
                (r >> 2) & 3,
                (r >> 4) & 1,
                (r >> 6) & 1,
                (r >> 7) & 1,
                (r >> 8) & 1,
                (r >> 10) & 1,
                v[1],
                v[2],
                why,
                (v[3] / 8.0).floor() / 4.0,
                rgb(taa, o),
                rgb(off, o),
                gref.map_or("      -    ".to_string(), |g| rgb(g, o)),
            );
            if let Some(hv) = hist.and_then(|hs| hs.get(o)) {
                let _ = write!(s, " hist {:.6},{:.6},{:.6},{:.6} |", hv[0], hv[1], hv[2], hv[3]);
            }
            if let Some(t) = &dbg {
                // The sample nearest the pixel: floor((P + 0.5) rs + jitter).
                let i = ((((x as f32 + 0.5) * t.rs.0 + t.jitter.0).floor() as usize).min(t.width as usize - 1))
                    + (((y as f32 + 0.5) * t.rs.1 + t.jitter.1).floor() as usize).min(t.height as usize - 1)
                        * t.width as usize;
                let c = &t.colour[i * 4..i * 4 + 4];
                let a = t.aux[i];
                let _ = write!(
                    s,
                    " {:3},{:3},{:3},{:3} {:+7.3},{:+7.3} {:.6} {:.6}{}",
                    c[0],
                    c[1],
                    c[2],
                    c[3],
                    a[0],
                    a[1],
                    a[2],
                    a[3],
                    if t.thin[i] { " thin" } else { "" }
                );
            }
            s += "\n";
        }
    }
    s
}

fn name_of(cx: &Cx, s: &str) -> PathBuf {
    cx.out.join(format!("{:06}-{s}.png", cx.f.frame.frame))
}

/// Mean |difference| a channel between two RGB pictures (width `w`)
/// inside a crop.
fn crop_error(a: &[u8], b: &[u8], w: usize, (x0, y0, cw, ch): (usize, usize, usize, usize)) -> f64 {
    let (mut sum, mut n) = (0u64, 0u64);
    for y in y0..y0 + ch {
        for x in x0..(x0 + cw).min(w) {
            let o = (y * w + x) * 3;
            if o + 3 > a.len() {
                continue;
            }
            sum += (0..3).map(|i| a[o + i].abs_diff(b[o + i]) as u64).sum::<u64>();
            n += 3;
        }
    }
    sum as f64 / n.max(1) as f64
}

/// A crop of an RGB picture enlarged `CROP_ZOOM` times, into a tile grid.
fn paste(
    grid: &mut [u8],
    gw: usize,
    at: (usize, usize),
    img: &[u8],
    w: usize,
    (x0, y0, cw, ch): (usize, usize, usize, usize),
) {
    for y in 0..ch * CROP_ZOOM {
        for x in 0..cw * CROP_ZOOM {
            let (sx, sy) = ((x0 + x / CROP_ZOOM).min(w - 1), y0 + y / CROP_ZOOM);
            let s = (sy * w + sx) * 3;
            let d = ((at.1 + y) * gw + at.0 + x) * 3;
            if s + 3 <= img.len() && d + 3 <= grid.len() {
                grid[d..d + 3].copy_from_slice(&img[s..s + 3]);
            }
        }
    }
}

impl Section for Gpu {
    fn header(&self) -> String {
        let mut s = String::new();
        for c in &self.cols {
            s += &format!(" {:>7}", c.name);
        }
        s + &format!(" {:>7} {:>7} {:>11}", "gn67", "gn50", "taa-change")
    }

    fn frame(&mut self, cx: &Cx) -> Result<String, String> {
        let f = &cx.f.frame;
        let view = cx.view;
        let layers = video::layers_2d(&f.tiles, &f.palette, view, cx.layer.1.iter().any(|&c| c));
        self.vectors(cx, &layers);
        // The GPU's own reference: the plain frame 4x4 supersampled (no
        // GPU-against-software floor in it).
        let gref = (cx.refs.is_some() || cx.next_sampled).then(|| {
            let big = view.supersampled(REF_SS);
            let mut p = cx.polys.to_vec();
            let pl = raster::place(&mut p, &big);
            let bl = video::layers_2d(&f.tiles, &f.palette, &big, !pl.is_empty());
            let rgb = self.cols[0].r.frame(&p, &pl, cx.src, &big, &bl);
            video::downsample(&rgb, big.width, big.height, REF_SS)
        });
        let gref_prev = self.gref_last.take().filter(|(n, _)| n + 1 == f.frame).map(|(_, g)| g);
        if let Some(g) = &gref {
            self.gref_last = Some((f.frame, g.clone()));
        }
        let tag = gpu::FrameTag::game(f.frame);
        let mut outs: Vec<Vec<u8>> = Vec::new();
        let mut taa_change = None;
        let mut inspected = Vec::new();
        for c in &mut self.cols {
            let temporal = c.quality.temporal > 0;
            let dumping = self.dump.is_some_and(|d| c.quality.temporal as usize == d.4);
            let inspect = temporal && ((self.inspect && cx.refs.is_some()) || dumping);
            c.r.set_temporal_inspect(inspect);
            c.r.set_temporal_frame(tag, step_of(cx.kind), Vec::new());
            let rgb = c.r.frame(cx.polys, cx.placed, cx.src, view, &layers);
            let insp = if inspect { c.r.temporal_inspect().map(|t| t.2) } else { None };
            if let (true, Some((x, y, w, h, _)), Some(d)) = (dumping, self.dump, &insp) {
                let rect = (x, y, w, h);
                let hist = c.r.temporal_history().map(|t| t.2);
                self.dump_text +=
                    &dump_rect(cx, rect, d, (&rgb, &outs[0]), c.r.temporal_debug(), hist.as_deref(), gref.as_deref());
                let _ = std::fs::write(cx.out.join("dump.txt"), &self.dump_text);
            }
            inspected.push(insp);
            if let (Some(last), Kind::Motion) = (&c.last, cx.kind) {
                let mut map = (c.name == "taa" && cx.refs.is_some()).then(|| vec![0u8; rgb.len()]);
                let ch = change_map(cx, &self.sheer, last, &rgb, map.as_deref_mut());
                if let Some(m) = map {
                    let _ = std::fs::write(name_of(cx, "taa-change"), png::encode_rgb(view.width, view.height, &m));
                }
                if c.name == "taa" {
                    taa_change = Some(ch);
                }
                add2(&mut c.change, ch);
                if cx.prev_ref.is_some() {
                    add2(&mut c.change_paired, ch);
                }
            }
            c.last = Some(rgb.clone());
            outs.push(rgb);
        }
        let chg = taa_change.map_or("-".to_string(), |c| format!("{:.2}/{:.2}", mean2(c[0]), mean2(c[1])));
        let Some(refs) = cx.refs else {
            return Ok(" ".repeat(8 * (self.cols.len() + 2)) + &format!(" {chg:>11}"));
        };
        let gref = gref.expect("drawn on sampled frames");
        if let (Some(pr), Kind::Motion) = (cx.prev_ref, cx.kind) {
            add2(&mut self.ref_change, change(cx, &self.sheer, &pr.reference, &refs.reference));
            if let Some(gp) = &gref_prev {
                add2(&mut self.gref_change, change(cx, &self.sheer, gp, &gref));
            }
        }
        let _ = std::fs::write(name_of(cx, "gref"), png::encode_rgb(view.width, view.height, &gref));
        let name = |s: &str| cx.out.join(format!("{:06}-{s}.png", f.frame));
        let mut cells = String::new();
        let not_inside = not_interior(cx, &refs.stipple);
        self.gref_detail += detail(&gref, view.width, &not_inside);
        for (c, rgb) in self.cols.iter_mut().zip(&outs) {
            let sc = score(rgb, &refs.reference, &refs.stipple);
            let gs = score(rgb, &gref, &refs.stipple);
            for k in 0..4 {
                c.score[k] += sc[k];
                c.gscore[k] += gs[k];
            }
            let gi = score(rgb, &gref, &not_inside);
            c.iscore[0] += gi[2];
            c.iscore[1] += gi[3];
            c.detail += detail(rgb, view.width, &not_inside);
            cells += &format!(" {:>7.2}", gs[0]);
            let _ = std::fs::write(name(c.name), png::encode_rgb(view.width, view.height, rgb));
        }
        // Naive upscales on the GPU: the 3D layer at the internal size,
        // enlarged bilinearly, composed at the output size.
        let mut naive = Vec::new();
        for (k, s) in SCALES.into_iter().enumerate() {
            let iv = internal_view(view, s);
            let mut p = cx.polys.to_vec();
            let pl = raster::place(&mut p, &iv);
            let small = self.cols[0].r.draw(&p, &pl, cx.src, &iv);
            let big = enlarge(&premultiplied(&small), iv.width, iv.height, view.width, view.height);
            let rgb = compose_rgba(&layers, view, &big);
            let sc = score(&rgb, &refs.reference, &refs.stipple);
            let gs = score(&rgb, &gref, &refs.stipple);
            for i in 0..4 {
                self.naive[k][i] += sc[i];
                self.gnaive[k][i] += gs[i];
            }
            cells += &format!(" {:>7.2}", gs[0]);
            naive.push(rgb);
        }
        self.scored += 1;
        // --inspect: the error above the plain frame's by the rule that
        // made each pixel; pictures of both.
        for (k, insp) in inspected.iter().enumerate() {
            let Some(d) = insp else { continue };
            let name = self.cols[k].name;
            if !self.budget.iter().any(|b| b.0 == name) {
                self.budget.push((name, [[(0, 0.0, 0.0); 2]; 12]));
            }
            let b = &mut self.budget.iter_mut().find(|b| b.0 == name).expect("added").1;
            let (mut rules, mut excess) = (vec![0u8; outs[k].len()], vec![0u8; outs[k].len()]);
            for o in 0..view.width * view.height {
                if cx.layer.1.get(o) != Some(&true) && cx.vis.owner[o] == u32::MAX {
                    continue;
                }
                if refs.stipple[o] {
                    continue;
                }
                let e = |img: &[u8]| (0..3).map(|i| img[o * 3 + i].abs_diff(gref[o * 3 + i]) as f64).sum::<f64>() / 3.0;
                // Against the plain frame at 100%, the naive upscale at 67/50.
                let base = match name {
                    "up67" => &naive[0],
                    "up50" => &naive[1],
                    _ => &outs[0],
                };
                let (et, eo) = (e(&outs[k]), e(base));
                let r = rule_of(d[o]);
                let clipped = (d[o][2] > 2.0 / 255.0) as usize;
                let cell = &mut b[r][clipped];
                cell.0 += 1;
                cell.1 += et;
                cell.2 += eo;
                rules[o * 3..o * 3 + 3].copy_from_slice(&RULES[r].1.map(|v| if clipped == 1 { v } else { v / 2 }));
                let x = ((et - eo) * 8.0).clamp(-255.0, 255.0);
                excess[o * 3 + if x > 0.0 { 0 } else { 1 }] = x.abs() as u8;
            }
            let _ =
                std::fs::write(name_of(cx, &format!("{name}-rules")), png::encode_rgb(view.width, view.height, &rules));
            let _ = std::fs::write(
                name_of(cx, &format!("{name}-excess")),
                png::encode_rgb(view.width, view.height, &excess),
            );
        }
        // Crops: gref off msaa4 taa / up67 up50 naive67 naive50.
        for (k, &crop) in self.crops.iter().enumerate() {
            let (tw, th) = (crop.2 * CROP_ZOOM, crop.3 * CROP_ZOOM);
            let (gw, gh) = (tw * 4 + 3 * 4, th * 2 + 4);
            let mut grid = vec![255u8; gw * gh * 3];
            let tiles: [&[u8]; 8] = [&gref, &outs[0], &outs[1], &outs[2], &outs[3], &outs[4], &naive[0], &naive[1]];
            for (i, img) in tiles.iter().enumerate() {
                paste(&mut grid, gw, ((i % 4) * (tw + 4), (i / 4) * (th + 4)), img, view.width, crop);
                if i > 0 {
                    self.crop_score[k].0[i - 1] += crop_error(img, &gref, view.width, crop);
                }
            }
            self.crop_score[k].1 += 1;
            let _ = std::fs::write(name(&format!("crop{k}")), png::encode_rgb(gw, gh, &grid));
        }
        if self.still_frame.is_none() && cx.kind == Kind::Motion {
            self.still_test(cx, &layers, refs, &gref)?;
        }
        Ok(cells + &format!(" {chg:>11}"))
    }

    fn summary(&self, r: &mut Report) {
        let n = self.scored.max(1) as f64;
        r.line(format!(
            "gpu           {} sampled frames against the reference: mean |difference| a channel / % over 16; outside stipple; frame-to-frame change on reprojected 3D pixels, still / moving (all frames | paired sampled frames)",
            self.scored
        ));
        let rc = (mean2(self.ref_change[0]), mean2(self.ref_change[1]));
        r.line(format!("  {:<10} {:>34}   change {:>16} | {:>5.2} / {:>5.2}", "reference", "", "", rc.0, rc.1));
        let get = |name: &str| self.cols.iter().find(|c| c.name == name).map(|c| c.gscore.map(|v| v / n));
        for c in &self.cols {
            let s = c.score.map(|v| v / n);
            r.line(format!(
                "  {:<10} {:>6.2} / {:>5.2}%   outside stipple {:>6.2} / {:>5.2}%   change {:>5.2} / {:>5.2} | {:>5.2} / {:>5.2}",
                c.name,
                s[0],
                s[1],
                s[2],
                s[3],
                mean2(c.change[0]),
                mean2(c.change[1]),
                mean2(c.change_paired[0]),
                mean2(c.change_paired[1])
            ));
            let _ = c.quality;
        }
        for (k, s) in SCALES.iter().enumerate() {
            let v = self.naive[k].map(|x| x / n);
            r.line(format!(
                "  {:<10} {:>6.2} / {:>5.2}%   outside stipple {:>6.2} / {:>5.2}%",
                format!("naive{s}"),
                v[0],
                v[1],
                v[2],
                v[3]
            ));
        }
        r.line(format!(
            "  against the GPU's own 4x4 supersampled plain frame (gref; its change {:.2} / {:.2}):",
            mean2(self.gref_change[0]),
            mean2(self.gref_change[1])
        ));
        let mut gtab: Vec<(String, [f64; 4])> =
            self.cols.iter().map(|c| (c.name.to_string(), c.gscore.map(|v| v / n))).collect();
        gtab.extend(SCALES.iter().enumerate().map(|(k, s)| (format!("naive{s}"), self.gnaive[k].map(|v| v / n))));
        for (name, v) in &gtab {
            r.line(format!(
                "    {name:<8} {:>6.2} / {:>5.2}%   outside stipple {:>6.2} / {:>5.2}%",
                v[0], v[1], v[2], v[3]
            ));
        }
        for (name, b) in &self.budget {
            let px: u64 = b.iter().flatten().map(|c| c.0).sum();
            let (st, so) = b.iter().flatten().fold((0.0, 0.0), |a, c| (a.0 + c.1, a.1 + c.2));
            r.line(format!(
                "  {name} by rule (3D pixels outside stipple, sampled frames; against {}): share of pixels, mean error {name} / base, excess (sum over the rule / all pixels); unclipped | clipped > 2 levels",
                if name.starts_with("up") { "naive at that scale" } else { "gpu-off" }
            ));
            r.line(format!(
                "    {:<20} {:>6.2}% {:>6.2} / {:>6.2} {:>+7.3}",
                "all",
                100.0,
                st / px.max(1) as f64,
                so / px.max(1) as f64,
                (st - so) / px.max(1) as f64
            ));
            for (i, (rn, _)) in RULES.iter().enumerate() {
                let cells: Vec<String> = b[i]
                    .iter()
                    .map(|c| {
                        format!(
                            "{:>6.2}% {:>6.2} / {:>6.2} {:>+7.3}",
                            pct(c.0, px),
                            c.1 / c.0.max(1) as f64,
                            c.2 / c.0.max(1) as f64,
                            (c.1 - c.2) / px.max(1) as f64
                        )
                    })
                    .collect();
                if b[i].iter().any(|c| c.0 > 0) {
                    r.line(format!("    {rn:<20} {} | {}", cells[0], cells[1]));
                }
            }
        }
        for (k, (s, frames)) in self.crop_score.iter().enumerate() {
            let c = self.crops[k];
            let m = |i: usize| s[i] / (*frames).max(1) as f64;
            r.line(format!(
                "  crop{k} {},{},{},{} against gref ({frames} frames): off {:.2} msaa4 {:.2} taa {:.2} up67 {:.2} up50 {:.2} naive67 {:.2} naive50 {:.2}",
                c.0,
                c.1,
                c.2,
                c.3,
                m(0),
                m(1),
                m(2),
                m(3),
                m(4),
                m(5),
                m(6)
            ));
        }
        r.line(format!(
            "  inside polygons (texture interiors: softening), against gref; detail (mean |Laplacian| of luma) over gref's {:.3}:",
            self.gref_detail / n
        ));
        for c in &self.cols {
            r.line(format!(
                "    {:<8} {:>6.2} / {:>5.2}%   detail {:.3}",
                c.name,
                c.iscore[0] / n,
                c.iscore[1] / n,
                c.detail / self.gref_detail.max(1e-9)
            ));
        }
        if let Some(fr) = self.still_frame {
            r.line(format!(
                "  still: frame {fr} drawn {STILL_FRAMES} times from a fresh renderer: score after, frame-to-frame change over 3D pixels from frame {STILL_FROM}"
            ));
            for (name, s, ch) in &self.still {
                r.line(format!(
                    "    {name:<8} {:>6.2} / {:>5.2}%   against gref {:>6.2} / {:>5.2}%   change {ch:.3}",
                    s[0], s[1], s[2], s[3]
                ));
            }
        }
        r.line(format!(
            "  velocity target vs CPU vectors: {} pixels with both, same surface: {:.3}% within 0.05 px (or half-float precision), {:.3}% within 0.25 px, mean {:.4} px; data presence differs on {:.3}% of {} covered",
            self.vcmp,
            pct(self.vfine, self.vcmp),
            pct(self.vnear, self.vcmp),
            self.vsum / self.vcmp.max(1) as f64,
            pct(self.presence.0, self.presence.1),
            self.presence.1
        ));
        let (eg, ec) = (self.err.1 / self.err.0.max(1) as f64, self.err.2 / self.err.0.max(1) as f64);
        r.line(format!(
            "  reprojection with the GPU's vectors {eg:.3} levels, the CPU's {ec:.3} ({} pixels)",
            self.err.0
        ));
        if self.vcmp > 0 {
            r.check(
                pct(self.vnear, self.vcmp) >= 99.0,
                format!("GPU vectors within 0.25 px of the CPU's on {:.3}% >= 99%", pct(self.vnear, self.vcmp)),
            );
            r.check(
                eg <= ec + 0.25,
                format!("reprojection with the GPU's vectors {eg:.3} <= the CPU's {ec:.3} + 0.25"),
            );
        }
        if self.scored > 0 {
            if let (Some(off), Some(taa), Some(m4)) = (get("gpu-off"), get("taa"), get("msaa4")) {
                r.line(format!(
                    "  taa / off: mean {:.3}, over 16 {:.3}; taa / msaa4: mean {:.3}",
                    taa[0] / off[0],
                    taa[1] / off[1].max(1e-9),
                    taa[0] / m4[0]
                ));
                r.check(
                    taa[0] <= off[0],
                    format!("taa {:.2} <= off {:.2} (mean against gref, moving)", taa[0], off[0]),
                );
            }
            for (k, s) in SCALES.iter().enumerate() {
                if let Some(up) = get(&format!("up{s}")) {
                    let nv = self.gnaive[k][0] / n;
                    r.check(
                        up[0] <= nv,
                        format!("up{s} {:.2} <= naive{s} {:.2} (mean against gref, moving)", up[0], nv),
                    );
                }
            }
        }
        if let Some((_, taa, ch)) = self.still.iter().find(|s| s.0 == "taa") {
            let m4 = self.still.iter().find(|s| s.0 == "msaa4").map_or(f64::MAX, |s| s.1[2]);
            r.check(taa[2] <= 1.1 * m4, format!("still: taa {:.2} <= 1.1 x msaa4 {:.2} (against gref)", taa[2], m4));
            r.check(*ch <= 0.25, format!("still: taa frame-to-frame change {ch:.3} <= 0.25"));
        }
    }
}

/// `--bench`: GPU time a frame, plain / MSAA 4x / temporal 100, 67, 50, and
/// with bloom (50%) on the plain, MSAA 4x and temporal 100 paths, on
/// the first `frames` frames of the sequence with 3D (each drawn `reps`
/// times: once with its motion, then repeated, render RAM uploaded once).
/// Wall time from submit to done, so it includes encoding the frame.
pub fn bench(root: &Path, view: &View, frames: usize, reps: usize) -> Result<(), String> {
    let mut seq = Seq::open(root).map_err(|e| format!("{}: {e}", root.display()))?;
    let texture_rom = std::mem::take(&mut seq.texture_rom);
    let mut built = Vec::new();
    let mut last_stream: Option<(u64, Vec<Centres>)> = None;
    for f in seq {
        let f = f.map_err(|e| e.to_string())?;
        if built.len() >= frames {
            break;
        }
        let src = f.frame.sources(&texture_rom);
        let before = last_stream.as_ref().filter(|(n, _)| n + 1 == f.frame.frame);
        let mi = match (&f.motion, before) {
            (Some(m), Some((_, c))) if !m.cut && m.s0.len() == f.frame.stream.len() => {
                Some(MotionIn { s0: &m.s0, s1: &f.frame.stream, tp: 0.0, valid: &m.valid, prev_centres: c })
            }
            _ => None,
        };
        let (mut polys, centres) = raster::build_motion(&f.frame.stream, &src, mi.as_ref());
        let motion = mi.is_some();
        video::HudLayout::new(&f.frame.tiles, &f.frame.palette, view).anchor(&mut polys, view);
        let placed = raster::place(&mut polys, view);
        last_stream = Some((f.frame.frame, centres));
        if placed.is_empty() {
            continue;
        }
        let layers = video::layers_2d(&f.frame.tiles, &f.frame.palette, view, true);
        built.push((f, polys, placed, layers, motion));
    }
    if built.is_empty() {
        return Err("no frames with 3D".into());
    }
    println!("bench         {}x{}, {} frames x {reps}", view.width, view.height, built.len());
    let q = |msaa, temporal| gpu::Quality { msaa, temporal, ..Default::default() };
    // Bloom (gpu::Quality::bloom) at 50%, on the plain, MSAA and TAA paths.
    let qb = |msaa, temporal| gpu::Quality { bloom: 50, ..q(msaa, temporal) };
    for (name, quality) in [
        ("off", q(1, 0)),
        ("msaa4", q(4, 0)),
        ("taa", q(1, 100)),
        ("up67", q(1, 67)),
        ("up50", q(1, 50)),
        ("bloom", qb(1, 0)),
        ("m4+blm", qb(4, 0)),
        ("taa+bl", qb(1, 100)),
    ] {
        let mut r = gpu::Renderer::new()?;
        r.set_quality(quality);
        let target = r.device().create_texture(&gpu::wgpu::TextureDescriptor {
            label: Some("bench"),
            size: gpu::wgpu::Extent3d {
                width: view.width as u32,
                height: view.height as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: gpu::wgpu::TextureDimension::D2,
            format: gpu::wgpu::TextureFormat::Rgba8Unorm,
            usage: gpu::wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        let tv = target.create_view(&Default::default());
        let mut times = Vec::new();
        for pass in 0..2 {
            for (i, (f, polys, placed, layers, motion)) in built.iter().enumerate() {
                let src = f.frame.sources(&texture_rom);
                for k in 0..reps {
                    let step = match k {
                        0 if *motion => gpu::Step::Blend { tp: 0.0 },
                        0 => gpu::Step::NoMotion,
                        _ => gpu::Step::Repeat,
                    };
                    r.set_temporal_frame(gpu::FrameTag::game(f.frame.frame), step, Vec::new());
                    r.set_ram_versions(i as u64, i as u64);
                    let t = std::time::Instant::now();
                    r.present(polys, placed, &src, view, layers, &tv, gpu::wgpu::TextureFormat::Rgba8Unorm);
                    let _ = r.device().poll(gpu::wgpu::PollType::Wait { submission_index: None, timeout: None });
                    // The first pass warms up (pipelines, uploads).
                    if pass == 1 && k > 0 {
                        times.push(t.elapsed().as_secs_f64() * 1e3);
                    }
                }
            }
        }
        times.sort_by(f64::total_cmp);
        let mean = times.iter().sum::<f64>() / times.len().max(1) as f64;
        println!(
            "{name:<6} {:>7.3} ms mean {:>7.3} median {:>7.3} p90  ({})",
            mean,
            quantile_f64(&times, 0.5),
            quantile_f64(&times, 0.9),
            r.adapter()
        );
    }
    let (w, h) = (view.width as f64, view.height as f64);
    for s in [100u32, 67, 50] {
        let iv = internal_view(view, s);
        let internal = (iv.width * iv.height) as f64;
        // Colour (4 B) + two aux (8 B each) internal; two histories (8 B each) output.
        let mb = (internal * 20.0 + w * h * 16.0) / (1 << 20) as f64;
        println!(
            "memory {s:>3}%  {:.1} MB (internal {}x{}: colour + aux pair 20 B a pixel; histories 16 B an output pixel)",
            mb, iv.width, iv.height
        );
    }
    Ok(())
}

fn quantile_f64(v: &[f64], q: f64) -> f64 {
    v.get(((v.len() as f64 - 1.0) * q).round().max(0.0) as usize).copied().unwrap_or(0.0)
}
