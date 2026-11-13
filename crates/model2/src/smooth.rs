//! Smooth motion: in-between frames drawn by interpolating the 3D geometry
//! between two game frames, for displays faster than the game's 57.5 Hz
//! (docs/hw/geometrizer.md, "Smooth motion").
//!
//! The game's logic, the display lists and the real geometrizer frames
//! are untouched. While the native geometrizer runs frame N it records
//! the transforms the list sets (the matrix, translation, `topskatr`'s
//! second matrix and the focal lengths), each with the model address of
//! the next object drawn as its key ([`Tap::Record`]). They are matched
//! with frame N-1's by kind, key and order ([`Plan::new`]). For a display
//! refresh at a fraction `t` between the two, a copy of the native
//! geometrizer, in its state from before frame N, runs frame N's list
//! again with each matched transform blended between frame N-1's value
//! and frame N's ([`Tap::Replay`]); everything else (which objects, their
//! lighting, the windows, direct data, the 2D layers) is frame N's. So it
//! shows frame N-1's motion caught up to frame N: one game frame of
//! latency, never a guess at frame N+1.
//!
//! It is render-only: the replay runs on its own copy of the geometrizer's
//! state, reads polygon RAM as it was before frame N and keeps the list's
//! polygon RAM writes in an overlay of its own, and its stream goes only
//! to the renderer.
//!
//! Motion vectors ([`Frame::motion`]) replay frame N's list once more with
//! frame N-1's transforms, and with polygon RAM as frame N-1 left it: a
//! model the list itself rewrites every frame (`lastbrnx`'s fighters'
//! models in polygon RAM, whose vertices move between frames under one
//! transform) is then frame N-1's model too, not frame N's posed with
//! frame N-1's transform.
//!
//! Generic over the programs: any native program that calls
//! [`crate::geo_native::Io::xform`] and [`crate::geo_native::Io::object`]
//! where it takes a transform and an object gets in-betweens.

use std::collections::HashMap;
use std::sync::Arc;

/// What a transform command sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Rotation (9 words, column by column: the images of x, y, z) and
    /// translation (3).
    Matrix,
    /// A second matrix in the same layout (`topskatr`'s fn 0x1B, for its
    /// skinned models).
    Matrix2,
    /// Translation only (3 words).
    Translation,
    /// Focal lengths (2 words).
    Focus,
}

impl Kind {
    /// Words the command takes.
    pub fn words(self) -> usize {
        match self {
            Kind::Matrix | Kind::Matrix2 => 12,
            Kind::Translation => 3,
            Kind::Focus => 2,
        }
    }

    fn rigid(self) -> bool {
        matches!(self, Kind::Matrix | Kind::Matrix2)
    }

    /// The kinds of value a command of this kind sets, as bits for
    /// [`Tap::Replay`]'s `snapped`: a matrix sets the translation too.
    fn sets(self) -> u8 {
        match self {
            Kind::Matrix => 1 | 4,
            Kind::Matrix2 => 2,
            Kind::Translation => 4,
            Kind::Focus => 8,
        }
    }
}

/// A transform as a frame's list set it: its words (IEEE single
/// precision bits, as the list holds them) and the model address of the
/// object drawn next ([`NO_KEY`] if none was).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Xform {
    pub kind: Kind,
    pub words: [u32; 12],
    pub key: u32,
}

/// [`Xform::key`] of a transform no object followed.
pub const NO_KEY: u32 = u32::MAX;

/// A polygon RAM access by a motion replay (word addresses), and each
/// object or direct data block begun, in order ([`unsure_blocks`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Block,
    Read(u32),
    Write(u32),
}

/// The blocks whose model data, read from polygon RAM as frame N-1 left
/// it, may not be what frame N-1's object read: they read a word frame
/// N's list writes more than once, or before the list writes it. (A word
/// the list writes once, before it is read, holds frame N-1's value of it
/// before frame N, if frame N-1's list did the same: only the list writes
/// polygon RAM.)
pub fn unsure_blocks(log: &[Access]) -> Vec<usize> {
    let mut writes: HashMap<u32, u32> = HashMap::new();
    for a in log {
        if let Access::Write(x) = a {
            *writes.entry(*x).or_default() += 1;
        }
    }
    if writes.is_empty() {
        return Vec::new();
    }
    let (mut done, mut block, mut out) = (std::collections::HashSet::new(), None, Vec::new());
    for a in log {
        match *a {
            Access::Block => block = Some(block.map_or(0, |b| b + 1)),
            Access::Write(x) => {
                done.insert(x);
            }
            Access::Read(x) => {
                let sure = writes.get(&x).is_none_or(|&n| n == 1 && done.contains(&x));
                if let (false, Some(b)) = (sure, block) {
                    if out.last() != Some(&b) {
                        out.push(b);
                    }
                }
            }
        }
    }
    out
}

impl Xform {
    pub fn new(kind: Kind, w: &[u32]) -> Self {
        let mut words = [0; 12];
        words[..w.len()].copy_from_slice(w);
        Xform { kind, words, key: NO_KEY }
    }

    fn f(&self, k: usize) -> f32 {
        f32::from_bits(self.words[k])
    }

    /// The translation (Matrix, Matrix2, Translation).
    fn translation(&self) -> [f32; 3] {
        let o = if self.kind.rigid() { 9 } else { 0 };
        [self.f(o), self.f(o + 1), self.f(o + 2)]
    }
}

/// The native geometrizer's hook for transforms and objects
/// ([`crate::geo_native::Io::tap`]).
pub enum Tap {
    /// A real frame: note the transforms and their keys.
    Record { list: Vec<Xform>, keyed: usize },
    /// An in-between: give the transforms these values, in order. Once a
    /// command doesn't match the next value (the list isn't the one
    /// recorded) the rest go through unchanged.
    ///
    /// For motion vectors it also notes, per object block of the output
    /// (each raster `0x1` command, in order), whether every transform in
    /// effect was matched and blended: `snap` says which values are frame
    /// N's whatever `t` ([`Plan::snaps`]; empty: none), `snapped` has a bit
    /// per kind of value now in effect that is ([`Kind::sets`]), and
    /// `blocks` gets one entry per object ([`Tap::object`]) or direct data
    /// block ([`Tap::direct`]).
    ///
    /// `turn`, for photo mode's free camera (`crate::photo`): a rotation
    /// (column by column) given to the light vectors the list sets, which
    /// are in view space as the transforms are, so the scene keeps its
    /// lighting when the camera turns. None: they go through unchanged.
    Replay {
        values: Vec<Xform>,
        next: usize,
        lost: bool,
        snap: Vec<bool>,
        snapped: u8,
        blocks: Vec<bool>,
        turn: Option<[f32; 9]>,
    },
}

impl Tap {
    pub fn record() -> Self {
        Tap::Record { list: Vec::new(), keyed: 0 }
    }

    /// A replay giving the transforms `values`, every one blended.
    pub fn replay(values: Vec<Xform>) -> Self {
        Tap::Replay { values, next: 0, lost: false, snap: Vec::new(), snapped: 0, blocks: Vec::new(), turn: None }
    }

    /// A light vector command's words (three IEEE singles, view space), as
    /// read from the list: turned by a replay's `turn`, else unchanged.
    pub fn light(&mut self, w: &mut [u32]) {
        if let (Tap::Replay { turn: Some(r), .. }, [x, y, z]) = (self, w) {
            let v = [f32::from_bits(*x), f32::from_bits(*y), f32::from_bits(*z)];
            let o: [f32; 3] = std::array::from_fn(|i| r[i] * v[0] + r[3 + i] * v[1] + r[6 + i] * v[2]);
            [*x, *y, *z] = o.map(f32::to_bits);
        }
    }

    /// A transform command's words, as read from the list: noted, or
    /// replaced by the blended values.
    pub fn xform(&mut self, kind: Kind, w: &mut [u32]) {
        match self {
            Tap::Record { list, .. } => list.push(Xform::new(kind, w)),
            Tap::Replay { values, next, lost, snap, snapped, .. } => {
                match values.get(*next) {
                    Some(v) if !*lost && v.kind == kind => w.copy_from_slice(&v.words[..w.len()]),
                    _ => *lost = true,
                }
                if snap.get(*next).copied().unwrap_or(false) {
                    *snapped |= kind.sets();
                } else {
                    *snapped &= !kind.sets();
                }
                *next += 1;
            }
        }
    }

    /// An object (model at `oba`): the key of the transforms since the last.
    pub fn object(&mut self, oba: u32) {
        match self {
            Tap::Record { list, keyed } => {
                for x in &mut list[*keyed..] {
                    x.key = oba;
                }
                *keyed = list.len();
            }
            Tap::Replay { lost, snapped, blocks, .. } => blocks.push(*snapped == 0 && !*lost),
        }
    }

    /// A block of direct data (polygons given in view space): no motion
    /// known for it.
    pub fn direct(&mut self) {
        if let Tap::Replay { blocks, .. } = self {
            blocks.push(false);
        }
    }

    /// What a recording found.
    pub fn into_list(self) -> Vec<Xform> {
        match self {
            Tap::Record { list, .. } => list,
            Tap::Replay { values, .. } => values,
        }
    }
}

/// How far apart frame N-1's and frame N's transforms may be before an
/// in-between stops blending them.
pub mod limits {
    /// A translation that moves by more than this times its distance from
    /// the eye: not the same object, or a cut. Drawn at frame N's.
    pub const JUMP: f32 = 1.0;
    /// A rotation by more than this (radians) is too fast to tell which way
    /// it turned (a wheel): it steps at the halfway point instead, while
    /// its translation and scale still blend.
    pub const SPIN: f32 = 2.0;
    /// A camera cut: when at least `CUT_SHARE` of a frame's matched
    /// matrices turned by more than `CUT_TURN` (radians) or moved by more
    /// than `CUT_MOVE` times their distance, nothing in it is blended.
    pub const CUT_TURN: f32 = 0.8;
    pub const CUT_MOVE: f32 = 0.5;
    pub const CUT_SHARE: f32 = 0.5;
    /// Focal lengths that change by more than this fraction: a cut too.
    pub const ZOOM: f32 = 0.25;
    /// Matching by key looks this many occurrences either side of a
    /// transform's own place among those with its kind and key (games
    /// reorder identical objects, such as players' shadows, by depth).
    pub const WINDOW: usize = 32;
    /// Matching another key (a part whose model changed with its pose):
    /// moved by at most `NEAR` times its distance, its 3x3 part changed by
    /// at most `RESHAPE` of its size.
    pub const NEAR: f32 = 0.2;
    pub const RESHAPE: f32 = 0.5;
}

/// How one transform of frame N is drawn between frame N-1's and its own.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pair {
    /// Frame N's values whatever `t` (unmatched, or too far apart).
    Snap,
    /// A rotation with a scale per axis, and a translation.
    Rigid(Rigid),
    /// Element by element (translations, focal lengths, and matrices that
    /// aren't a rotation and scale).
    Linear { a: [f32; 12], b: [f32; 12] },
}

/// A matrix as a rotation, a scale per axis and a translation, at frame
/// N-1 and frame N (index 0 and 1): the rotations as unit quaternions (the
/// second on the first's side), the scales, the translations, and what the
/// rebuilt 3x3 part misses of the game's (it is only nearly a rotation
/// times a scale), so the ends are the game's matrices; `flip` for a
/// mirrored matrix (both are), `step` for a rotation too fast to blend.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Rigid {
    q: [[f32; 4]; 2],
    s: [[f32; 3]; 2],
    t: [[f32; 3]; 2],
    rest: [[f32; 9]; 2],
    flip: bool,
    step: bool,
}

/// The 3x3 part for rotation `q` and scales `s` (mirrored with `flip`).
fn rebuild(q: [f32; 4], s: [f32; 3], flip: bool) -> [f32; 9] {
    let c = columns(q);
    let sign = if flip { -1.0 } else { 1.0 };
    std::array::from_fn(|k| c[k / 3][k % 3] * s[k / 3] * sign)
}

/// Frame N's transforms and how each is drawn in between.
#[derive(Clone, Debug, Default)]
pub struct Plan {
    cur: Vec<Xform>,
    pairs: Vec<Pair>,
    /// Per transform (empty: none so far, or none at all): matched with
    /// one of frame N-1's that placed another model (key), so the model
    /// posed with frame N-1's transform is not where frame N-1 drew its
    /// points (a pose model swapped in; vstriker's players). Focal lengths
    /// apply to everything and never count. Only for [`Plan::unknown`]:
    /// the in-betweens blend these as any other.
    rekeyed: Vec<bool>,
    /// A camera cut: nothing is blended.
    pub cut: bool,
    /// Transforms matched with one of frame N-1's, and of those, drawn at
    /// frame N's for being too far apart.
    pub matched: usize,
    pub snapped: usize,
}

fn norm(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
    norm([a[0] - b[0], a[1] - b[1], a[2] - b[2]])
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// How far a translation moved, relative to its distance from the eye.
fn moved(a: [f32; 3], b: [f32; 3]) -> f32 {
    let r = norm(a).max(norm(b));
    if r > 0.0 {
        dist(a, b) / r
    } else {
        0.0
    }
}

/// A matrix's columns (the images of x, y, z) with their lengths, if it is
/// a rotation (possibly mirrored) times a scale per axis.
fn rigid(m: &[f32; 9]) -> Option<([[f32; 3]; 3], [f32; 3], bool)> {
    let col = |c: usize| [m[3 * c], m[3 * c + 1], m[3 * c + 2]];
    let mut cols = [col(0), col(1), col(2)];
    let mut s = [0.0; 3];
    for (c, s) in cols.iter_mut().zip(&mut s) {
        *s = norm(*c);
        if !(*s > 1e-12 && s.is_finite()) {
            return None;
        }
        for x in c.iter_mut() {
            *x /= *s;
        }
    }
    let d = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let tol = 2e-2;
    if d(cols[0], cols[1]).abs() > tol || d(cols[0], cols[2]).abs() > tol || d(cols[1], cols[2]).abs() > tol {
        return None;
    }
    let [a, b, c] = cols;
    let det =
        a[0] * (b[1] * c[2] - b[2] * c[1]) - b[0] * (a[1] * c[2] - a[2] * c[1]) + c[0] * (a[1] * b[2] - a[2] * b[1]);
    if (det.abs() - 1.0).abs() > 5e-2 {
        return None;
    }
    let flip = det < 0.0;
    if flip {
        // A mirrored rotation: -R is a proper one.
        for c in &mut cols {
            for x in c.iter_mut() {
                *x = -*x;
            }
        }
    }
    Some((cols, s, flip))
}

/// A rotation's unit quaternion (w, x, y, z), from its columns.
fn quat(c: &[[f32; 3]; 3]) -> [f32; 4] {
    // r(i, j): row i, column j.
    let r = |i: usize, j: usize| c[j][i];
    let tr = r(0, 0) + r(1, 1) + r(2, 2);
    let q = if tr > 0.0 {
        let s = (tr + 1.0).sqrt() * 2.0;
        [0.25 * s, (r(2, 1) - r(1, 2)) / s, (r(0, 2) - r(2, 0)) / s, (r(1, 0) - r(0, 1)) / s]
    } else if r(0, 0) > r(1, 1) && r(0, 0) > r(2, 2) {
        let s = (1.0 + r(0, 0) - r(1, 1) - r(2, 2)).sqrt() * 2.0;
        [(r(2, 1) - r(1, 2)) / s, 0.25 * s, (r(0, 1) + r(1, 0)) / s, (r(0, 2) + r(2, 0)) / s]
    } else if r(1, 1) > r(2, 2) {
        let s = (1.0 + r(1, 1) - r(0, 0) - r(2, 2)).sqrt() * 2.0;
        [(r(0, 2) - r(2, 0)) / s, (r(0, 1) + r(1, 0)) / s, 0.25 * s, (r(1, 2) + r(2, 1)) / s]
    } else {
        let s = (1.0 + r(2, 2) - r(0, 0) - r(1, 1)).sqrt() * 2.0;
        [(r(1, 0) - r(0, 1)) / s, (r(0, 2) + r(2, 0)) / s, (r(1, 2) + r(2, 1)) / s, 0.25 * s]
    };
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    q.map(|x| x / n)
}

/// A unit quaternion's rotation, as columns.
fn columns(q: [f32; 4]) -> [[f32; 3]; 3] {
    let [w, x, y, z] = q;
    [
        [1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y + w * z), 2.0 * (x * z - w * y)],
        [2.0 * (x * y - w * z), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z + w * x)],
        [2.0 * (x * z + w * y), 2.0 * (y * z - w * x), 1.0 - 2.0 * (x * x + y * y)],
    ]
}

fn qdot(a: [f32; 4], b: [f32; 4]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3]
}

/// The angle between two unit quaternions on the same side (radians).
fn angle(a: [f32; 4], b: [f32; 4]) -> f32 {
    2.0 * qdot(a, b).clamp(-1.0, 1.0).acos()
}

/// Spherical interpolation of unit quaternions on the same side.
fn slerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    let d = qdot(a, b).clamp(-1.0, 1.0);
    let (wa, wb) = if d > 0.9995 {
        (1.0 - t, t)
    } else {
        let th = d.acos();
        let s = th.sin();
        (((1.0 - t) * th).sin() / s, (t * th).sin() / s)
    };
    let q: [f32; 4] = std::array::from_fn(|k| a[k] * wa + b[k] * wb);
    let n = qdot(q, q).sqrt();
    q.map(|x| x / n)
}

fn floats(x: &Xform) -> [f32; 12] {
    std::array::from_fn(|k| x.f(k))
}

/// How to draw `b` (frame N's) between `a` (frame N-1's) and itself, and
/// whether it moved like a cut (for [`Plan::new`]'s count).
fn pair(a: &Xform, b: &Xform) -> (Pair, bool) {
    let (fa, fb) = (floats(a), floats(b));
    let n = b.kind.words();
    if fa[..n].iter().chain(&fb[..n]).any(|x| !x.is_finite()) {
        return (Pair::Snap, false);
    }
    match b.kind {
        Kind::Focus => {
            let zoom = (0..2).any(|k| (fb[k] - fa[k]).abs() > limits::ZOOM * fa[k].abs().max(fb[k].abs()));
            (if zoom { Pair::Snap } else { Pair::Linear { a: fa, b: fb } }, zoom)
        }
        Kind::Translation => {
            let m = moved(a.translation(), b.translation());
            (if m > limits::JUMP { Pair::Snap } else { Pair::Linear { a: fa, b: fb } }, m > limits::CUT_MOVE)
        }
        Kind::Matrix | Kind::Matrix2 => {
            let (ta, tb) = (a.translation(), b.translation());
            let m = moved(ta, tb);
            if m > limits::JUMP {
                return (Pair::Snap, true);
            }
            let (ma, mb): ([f32; 9], [f32; 9]) = (std::array::from_fn(|k| fa[k]), std::array::from_fn(|k| fb[k]));
            match (rigid(&ma), rigid(&mb)) {
                (Some((ca, sa, fla)), Some((cb, sb, flb))) if fla == flb => {
                    let qa = quat(&ca);
                    let mut qb = quat(&cb);
                    if qdot(qa, qb) < 0.0 {
                        qb = qb.map(|x| -x);
                    }
                    let turn = angle(qa, qb);
                    let step = turn > limits::SPIN;
                    let cut = m > limits::CUT_MOVE || turn > limits::CUT_TURN;
                    let (ra, rb) = (rebuild(qa, sa, fla), rebuild(qb, sb, fla));
                    let rest = [std::array::from_fn(|k| ma[k] - ra[k]), std::array::from_fn(|k| mb[k] - rb[k])];
                    let r = Rigid { q: [qa, qb], s: [sa, sb], t: [ta, tb], rest, flip: fla, step };
                    (Pair::Rigid(r), cut)
                }
                // A mirror that comes or goes, or not a rotation: frame N's.
                (Some(_), Some(_)) => (Pair::Snap, true),
                _ => (Pair::Linear { a: fa, b: fb }, m > limits::CUT_MOVE),
            }
        }
    }
}

impl Pair {
    /// The words at `t` (0: frame N-1's, 1: frame N's).
    fn at(&self, b: &Xform, t: f32) -> [u32; 12] {
        let mut w = b.words;
        match self {
            Pair::Snap => {}
            Pair::Linear { a, b: fb } => {
                for k in 0..b.kind.words() {
                    w[k] = lerp(a[k], fb[k], t).to_bits();
                }
            }
            Pair::Rigid(r) => {
                let q = match (r.step, t < 0.5) {
                    (true, true) => r.q[0],
                    (true, false) => r.q[1],
                    (false, _) => slerp(r.q[0], r.q[1], t),
                };
                let s = std::array::from_fn(|j| lerp(r.s[0][j], r.s[1][j], t));
                let m = rebuild(q, s, r.flip);
                for k in 0..9 {
                    w[k] = (m[k] + lerp(r.rest[0][k], r.rest[1][k], t)).to_bits();
                }
                for i in 0..3 {
                    w[9 + i] = lerp(r.t[0][i], r.t[1][i], t).to_bits();
                }
            }
        }
        w
    }
}

/// Candidate matches: (distance, places apart, frame N's index, frame
/// N-1's).
type Candidates = Vec<(f32, usize, usize, usize)>;

/// Take the candidates nearest first (then closest in place), each
/// transform once.
fn assign(mut c: Candidates, matched: &mut [Option<usize>], used: &mut [bool]) {
    c.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    for (_, _, ci, pj) in c {
        if matched[ci].is_none() && !used[pj] {
            matched[ci] = Some(pj);
            used[pj] = true;
        }
    }
}

/// The indices of `list`'s transforms grouped by `group`, in order.
fn groups<K: std::hash::Hash + Eq>(list: &[Xform], group: impl Fn(&Xform) -> K) -> HashMap<K, Vec<usize>> {
    let mut out: HashMap<K, Vec<usize>> = HashMap::new();
    for (i, x) in list.iter().enumerate() {
        out.entry(group(x)).or_default().push(i);
    }
    out
}

/// How different two matrices' 3x3 parts are, relative to their size: 0
/// for the same, up to 2 (0 for kinds without one).
fn reshaped(a: &Xform, b: &Xform) -> f32 {
    if !a.kind.rigid() {
        return 0.0;
    }
    let (mut d, mut na, mut nb) = (0.0, 0.0, 0.0);
    for k in 0..9 {
        let (x, y) = (a.f(k), b.f(k));
        d += (x - y) * (x - y);
        na += x * x;
        nb += y * y;
    }
    let n = na.max(nb).sqrt();
    if n > 0.0 && d.is_finite() {
        d.sqrt() / n
    } else {
        2.0
    }
}

impl Plan {
    /// Match frame N's transforms (`cur`) with frame N-1's (`prev`) of the
    /// same kind, cheapest first. The cost is the distance between their
    /// translations, doubled between different keys (the same model drawn
    /// again is the likelier match), plus a tenth of the distance from the
    /// eye times how much the 3x3 part differs ([`reshaped`]: of two parts
    /// drawn at one place, the one turned the same way). Candidates:
    /// - the same key, at most [`limits::WINDOW`] apart in place among the
    ///   key's transforms and moved by at most [`limits::JUMP`] times its
    ///   distance;
    /// - another key, moved by at most [`limits::NEAR`] times its distance
    ///   and reshaped by at most [`limits::RESHAPE`] (a part of a model drawn
    ///   in a new pose is often a model of its own, games share pose models
    ///   between characters, and reorder their lists: found by place, from
    ///   frame N-1's sorted by depth).
    pub fn new(prev: &[Xform], cur: Vec<Xform>) -> Self {
        let mut matched = vec![None; cur.len()];
        let mut used = vec![false; prev.len()];
        let cost = |c: &Xform, p: &Xform, other: bool| {
            if c.kind == Kind::Focus {
                return 0.0;
            }
            let (tc, tp) = (c.translation(), p.translation());
            dist(tc, tp) * if other { 2.0 } else { 1.0 } + 0.1 * norm(tc) * reshaped(c, p)
        };
        let m =
            |c: &Xform, p: &Xform| if c.kind == Kind::Focus { 0.0 } else { moved(p.translation(), c.translation()) };
        let mut c = Candidates::new();
        let key = |x: &Xform| (x.kind, x.key);
        let (pk, ck) = (groups(prev, key), groups(&cur, key));
        for (k, cs) in &ck {
            let Some(ps) = pk.get(k) else { continue };
            for (oc, &ci) in cs.iter().enumerate() {
                let (lo, hi) = (oc.saturating_sub(limits::WINDOW), (oc + limits::WINDOW + 1).min(ps.len()));
                let near = (lo..hi).filter(|&op| m(&cur[ci], &prev[ps[op]]) <= limits::JUMP);
                c.extend(near.map(|op| (cost(&cur[ci], &prev[ps[op]], false), oc.abs_diff(op), ci, ps[op])));
            }
        }
        // Another key: frame N-1's of the kind by depth; one that moved by
        // at most NEAR of its distance is within a quarter of frame N's
        // distance in depth.
        let mut by_depth: HashMap<Kind, Vec<(f32, usize)>> = HashMap::new();
        for (j, p) in prev.iter().enumerate().filter(|(_, p)| p.kind != Kind::Focus) {
            by_depth.entry(p.kind).or_default().push((p.translation()[2], j));
        }
        for v in by_depth.values_mut() {
            v.sort_by(|a, b| a.0.total_cmp(&b.0));
        }
        for (ci, x) in cur.iter().enumerate() {
            let Some(ps) = by_depth.get(&x.kind) else { continue };
            let (t, band) = (x.translation(), limits::NEAR / (1.0 - limits::NEAR) * norm(x.translation()));
            let from = ps.partition_point(|&(z, _)| z < t[2] - band);
            for &(_, j) in ps[from..].iter().take_while(|&&(z, _)| z <= t[2] + band) {
                let p = &prev[j];
                if p.key != x.key && m(x, p) <= limits::NEAR && reshaped(x, p) <= limits::RESHAPE {
                    c.push((cost(x, p, true), 0, ci, j));
                }
            }
        }
        // Focal lengths under another key: by order, after any of the key.
        let focus = |l: &[Xform]| l.iter().enumerate().filter(|(_, x)| x.kind == Kind::Focus).map(|(i, _)| i).collect();
        let (pf, cf): (Vec<usize>, Vec<usize>) = (focus(prev), focus(&cur));
        for (oc, &ci) in cf.iter().enumerate() {
            let other = pf.iter().enumerate().filter(|(_, &j)| prev[j].key != cur[ci].key);
            c.extend(other.map(|(op, &j)| (1.0, oc.abs_diff(op), ci, j)));
        }
        assign(c, &mut matched, &mut used);
        let prev_keys: std::collections::HashSet<(Kind, u32)> = prev.iter().map(key).collect();

        let mut plan = Plan { pairs: Vec::with_capacity(cur.len()), ..Default::default() };
        // Matrices matched, and those that turned or moved like a cut; a
        // matrix whose model frame N-1 drew too but that matched nothing
        // (it jumped) counts as both.
        let (mut cuts, mut rigid_matched) = (0, 0);
        for (x, m) in cur.iter().zip(&matched) {
            let Some(j) = *m else {
                if x.kind.rigid() && prev_keys.contains(&key(x)) {
                    rigid_matched += 1;
                    cuts += 1;
                }
                plan.pairs.push(Pair::Snap);
                continue;
            };
            let (p, cut) = pair(&prev[j], x);
            plan.matched += 1;
            if p == Pair::Snap {
                plan.snapped += 1;
            }
            if x.kind.rigid() {
                rigid_matched += 1;
                cuts += cut as usize;
            } else if x.kind == Kind::Focus && cut {
                // A zoom that jumps is a cut whatever the objects do.
                plan.cut = true;
            }
            plan.pairs.push(p);
        }
        if rigid_matched > 0 && cuts as f32 >= limits::CUT_SHARE * rigid_matched as f32 {
            plan.cut = true;
        }
        plan.rekeyed = cur
            .iter()
            .zip(&matched)
            .map(|(x, m)| m.is_some_and(|j| prev[j].key != x.key && x.kind != Kind::Focus))
            .collect();
        plan.cur = cur;
        plan
    }

    /// Frame N's transforms, as recorded.
    pub fn transforms(&self) -> &[Xform] {
        &self.cur
    }

    /// Per transform, in frame N's order: drawn at frame N's values
    /// whatever `t` (unmatched, or too far apart to blend).
    pub fn snaps(&self) -> Vec<bool> {
        self.pairs.iter().map(|p| *p == Pair::Snap).collect()
    }

    /// Per transform, in frame N's order: where the objects it places
    /// were in frame N-1 is unknown, so they get no motion vectors
    /// ([`Motion::valid`]): snapped ([`Plan::snaps`]), or matched with a
    /// transform that placed another model.
    pub fn unknown(&self) -> Vec<bool> {
        let mut s = self.snaps();
        for (s, r) in s.iter_mut().zip(&self.rekeyed) {
            *s |= r;
        }
        s
    }

    /// Nothing to blend: draw frame N as it is.
    pub fn still(&self) -> bool {
        self.cut || self.pairs.iter().all(|p| *p == Pair::Snap)
    }

    /// The transforms for an in-between at `t`, in frame N's order.
    pub fn values(&self, t: f32) -> Vec<Xform> {
        let t = t.clamp(0.0, 1.0);
        self.cur
            .iter()
            .zip(&self.pairs)
            .map(|(x, p)| {
                let words = if self.cut || t >= 1.0 { x.words } else { p.at(x, t) };
                Xform { words, ..*x }
            })
            .collect()
    }
}

/// The native geometrizer as it was before a frame, to run the frame
/// again.
#[derive(Clone)]
pub enum Native {
    B(Box<crate::geo_native::Native>),
    C(Box<crate::geo_native_2c::Native>),
}

/// Everything an in-between of one game frame needs, apart from the
/// machine: the geometrizer's state before frame N, frame N's list (a copy
/// of buffer RAM), polygon memory as it was then, the matched transforms.
/// Shared (`Arc`) so a front end can draw in-betweens while the engine
/// runs the next frame.
pub struct Frame {
    native: Native,
    buffer: Vec<u8>,
    jumps: Vec<u32>,
    read_ptr: u32,
    latch: u32,
    poly: Arc<Vec<u32>>,
    rom: Arc<Vec<u32>>,
    /// Texture point RAM after frame N, to draw the in-betweens with.
    pub points: Vec<u16>,
    pub plan: Plan,
    /// The game frame (the engine's count) this is frame N of.
    pub frame: u64,
    /// The window the rasteriser had from the frame before, which a
    /// replay's stream is given as frame N's was (`geo::carry_window`).
    window: Option<[u32; 7]>,
    /// [`Frame::motion`], made on first use.
    motion: std::sync::OnceLock<Motion>,
}

/// What motion vectors need from a frame's plan: frame N's list posed
/// with frame N-1's transforms, which has frame N's stream's layout word
/// for word (the native geometrizers emit every polygon of every object,
/// whatever the transforms), so a vertex's previous view-space position is
/// at the same place in `s0` as its current one in the real stream.
#[derive(Clone, Debug, Default)]
pub struct Motion {
    /// The frame's list posed with the frame before's transforms (and
    /// polygon RAM as the frame before left it); empty on a cut.
    pub s0: Vec<u32>,
    /// Per object block of the stream (each raster `0x1` command, in
    /// order): every transform in effect was matched and blended (not
    /// snapped to frame N's) with one that placed the same model
    /// ([`Plan::unknown`]), and its model data is frame N-1's
    /// ([`unsure_blocks`]), so its motion is known. Direct data: false.
    pub valid: Vec<bool>,
    /// A camera cut ([`Plan::cut`]): nothing to take motion from.
    pub cut: bool,
}

impl Frame {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        native: Native,
        buffer: Vec<u8>,
        jumps: Vec<u32>,
        read_ptr: u32,
        latch: u32,
        poly: Arc<Vec<u32>>,
        rom: Arc<Vec<u32>>,
        points: Vec<u16>,
        plan: Plan,
        frame: u64,
        window: Option<[u32; 7]>,
    ) -> Self {
        Frame {
            native,
            buffer,
            jumps,
            read_ptr,
            latch,
            poly,
            rom,
            points,
            plan,
            frame,
            window,
            motion: Default::default(),
        }
    }

    /// The raster stream between frame N-1 and frame N at `t` (0 to 1),
    /// or None where it is frame N's own (`t` of 1 or more, a cut, nothing
    /// matched): draw the engine's stream then.
    pub fn render(&self, t: f32) -> Option<Vec<u32>> {
        if t >= 1.0 || self.plan.still() {
            return None;
        }
        Some(self.render_values(self.plan.values(t)))
    }

    /// [`Frame::render`] with the in-between's lighting side channel
    /// (`crate::light::Frame`, numbered as frame N): the replay records it
    /// as the engine records a game frame's with `Engine::set_light` on,
    /// so smooth shading draws in-betweens at the level of the game
    /// frames around them, not from the stream alone.
    pub fn render_lit(&self, t: f32) -> Option<(Vec<u32>, crate::light::Frame)> {
        if t >= 1.0 || self.plan.still() {
            return None;
        }
        let mut tap = crate::light::Tap::new();
        let (stream, _) = self.replay(self.plan.values(t), Vec::new(), None, Some(&mut tap), None);
        Some((stream, tap.finish(self.frame)))
    }

    /// Motion data for this frame ([`Motion`]): one replay at `t` = 0,
    /// with polygon RAM as frame N-1 left it, made on first use and kept.
    /// On a cut there is no replay (`s0` empty); when nothing matched the
    /// replay still runs, every block invalid.
    pub fn motion(&self) -> &Motion {
        self.motion.get_or_init(|| {
            if self.plan.cut {
                return Motion { cut: true, ..Default::default() };
            }
            let mut log = Vec::new();
            let (s0, mut valid) = self.replay(self.plan.values(0.0), self.plan.unknown(), Some(&mut log), None, None);
            for b in unsure_blocks(&log) {
                if let Some(v) = valid.get_mut(b) {
                    *v = false;
                }
            }
            Motion { s0, valid, cut: false }
        })
    }

    /// Frame N's list again with these transforms.
    pub(crate) fn render_values(&self, values: Vec<Xform>) -> Vec<u32> {
        self.replay(values, Vec::new(), None, None, None).0
    }

    /// Frame N's list again with these transforms (photo mode's free
    /// camera, `crate::photo`: frame N's or an in-between's, moved into
    /// another view), its light vectors turned by `turn` (column by
    /// column); with `lit`, its lighting side channel too, numbered as
    /// frame N.
    pub fn render_posed(
        &self,
        values: Vec<Xform>,
        turn: Option<[f32; 9]>,
        lit: bool,
    ) -> (Vec<u32>, Option<crate::light::Frame>) {
        let mut tap = lit.then(crate::light::Tap::new);
        let (stream, _) = self.replay(values, Vec::new(), None, tap.as_mut(), turn);
        (stream, tap.map(|t| t.finish(self.frame)))
    }

    /// Frame N's list again with these transforms (`snap`: those that are
    /// frame N's whatever `t`), and per object block whether all its
    /// transforms were blended. With `log`, polygon RAM is read as it was
    /// before frame N whatever the list writes, the accesses noted there;
    /// without, the list's writes are read back, as in frame N. `light`:
    /// record its lighting side channel. `turn`: turn the light vectors
    /// ([`Tap::light`]).
    fn replay(
        &self,
        values: Vec<Xform>,
        snap: Vec<bool>,
        log: Option<&mut Vec<Access>>,
        light: Option<&mut crate::light::Tap>,
        turn: Option<[f32; 9]>,
    ) -> (Vec<u32>, Vec<bool>) {
        let mut native = self.native.clone();
        let (mut words, mut fed, mut out, mut latch) = (0, Vec::new(), Vec::new(), self.latch);
        let mut overlay = HashMap::new();
        let mut tap = Tap::Replay { values, next: 0, lost: false, snap, snapped: 0, blocks: Vec::new(), turn };
        let mut list = crate::geo::ListDma::new(&self.buffer, &self.jumps, self.read_ptr, &mut words, &mut fed);
        let poly = match log {
            Some(log) => crate::geo::GeoExt::before(&self.poly, log, &self.rom),
            None => crate::geo::GeoExt::overlay(&self.poly, &mut overlay, &self.rom),
        };
        let mut io = crate::geo_native::Io {
            list: &mut list,
            poly,
            latch: &mut latch,
            out: &mut out,
            tap: Some(&mut tap),
            light,
        };
        match &mut native {
            Native::B(n) => n.run_frame(&mut io),
            Native::C(n) => n.run_frame(&mut io),
        }
        out.retain(|&w| w != crate::geo::RASTER_NOP);
        crate::geo::carry_window(&mut out, self.window);
        let Tap::Replay { blocks, .. } = tap else { unreachable!() };
        (out, blocks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(m: [f32; 9], t: [f32; 3], key: u32) -> Xform {
        let w: Vec<u32> = m.iter().chain(&t).map(|x| x.to_bits()).collect();
        Xform { key, ..Xform::new(Kind::Matrix, &w) }
    }

    /// Rotation about z by `a` radians, scaled by `s`, as columns.
    fn rot_z(a: f32, s: f32) -> [f32; 9] {
        let (c, d) = (a.cos() * s, a.sin() * s);
        [c, d, 0.0, -d, c, 0.0, 0.0, 0.0, s]
    }

    fn f(w: &[u32; 12]) -> [f32; 12] {
        w.map(f32::from_bits)
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    /// A transform matched with one that placed another model blends like
    /// any other, but its motion is unknown; focal lengths never are.
    #[test]
    fn another_model_in_its_place_has_unknown_motion() {
        let focus = |f: f32, key| Xform { key, ..Xform::new(Kind::Focus, &[f.to_bits(), f.to_bits()]) };
        let prev = [
            focus(400.0, 1),
            matrix(rot_z(0.1, 1.0), [0.0, 0.0, 100.0], 7),
            matrix(rot_z(0.0, 1.0), [5.0, 0.0, 100.0], 8),
        ];
        // The first object is drawn again; the second's place is taken by
        // another model (a new pose), the focal lengths keyed differently.
        let cur = vec![
            focus(400.0, 2),
            matrix(rot_z(0.12, 1.0), [0.5, 0.0, 100.0], 7),
            matrix(rot_z(0.02, 1.0), [5.5, 0.0, 100.0], 9),
        ];
        let plan = Plan::new(&prev, cur);
        assert_eq!(plan.matched, 3);
        assert_eq!(plan.snaps(), [false, false, false]);
        assert_eq!(plan.unknown(), [false, false, true]);
        // The in-between still blends it.
        assert!(!close(&f(&plan.values(0.5)[2].words), &f(&plan.values(1.0)[2].words)));
    }

    #[test]
    fn rotation_blends_on_the_arc_and_keeps_scale() {
        let a = matrix(rot_z(0.0, 2.0), [0.0, 0.0, 10.0], 1);
        let b = matrix(rot_z(0.5, 4.0), [2.0, 0.0, 10.0], 1);
        let plan = Plan::new(&[a], vec![b]);
        assert!(!plan.cut);
        let mid = f(&plan.values(0.5)[0].words);
        // Halfway round, halfway in scale (3) and translation.
        assert!(close(&mid[..9], &rot_z(0.25, 3.0)), "{mid:?}");
        assert!(close(&mid[9..], &[1.0, 0.0, 10.0]));
        // The ends are the frames' own words exactly.
        assert_eq!(plan.values(1.0)[0].words, b.words);
        assert!(close(&f(&plan.values(0.0)[0].words), &f(&a.words)));
        // A matrix that is only nearly a rotation (as the games' are) is
        // still its own at t = 0, not the nearest rotation.
        let mut m = rot_z(0.0, 2.0);
        m[3] = 0.03;
        let a = matrix(m, [0.0, 0.0, 10.0], 1);
        let start = f(&Plan::new(&[a], vec![b]).values(0.0)[0].words);
        assert!(start.iter().zip(f(&a.words)).all(|(x, y)| (x - y).abs() < 1e-6), "{start:?}");
    }

    #[test]
    fn mirrored_matrices_stay_mirrored() {
        let mut m = rot_z(0.0, 1.0);
        let mut n = rot_z(0.4, 1.0);
        m[8] = -1.0;
        n[8] = -1.0;
        let plan = Plan::new(&[matrix(m, [0.0, 0.0, 5.0], 7)], vec![matrix(n, [0.0, 0.0, 5.0], 7)]);
        let mid = f(&plan.values(0.5)[0].words);
        let mut want = rot_z(0.2, 1.0);
        want[8] = -1.0;
        assert!(close(&mid[..9], &want), "{mid:?}");
    }

    #[test]
    fn non_rigid_matrices_blend_linearly() {
        // A shear.
        let a = matrix([1.0, 0.0, 0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 5.0], 3);
        let b = matrix([1.0, 0.0, 0.0, 1.5, 1.0, 0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 5.0], 3);
        let mid = f(&Plan::new(&[a], vec![b]).values(0.5)[0].words);
        assert!(close(&mid[..9], &[1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 0.0, 1.0]));
    }

    #[test]
    fn big_jumps_snap_and_cuts_stop_everything() {
        let a = matrix(rot_z(0.0, 1.0), [0.0, 0.0, 10.0], 1);
        // Moved by more than its distance: not matched, frame N's, and
        // (all there is) a cut.
        let far = matrix(rot_z(0.0, 1.0), [25.0, 0.0, -5.0], 1);
        let plan = Plan::new(&[a], vec![far]);
        assert_eq!(plan.matched, 0);
        assert!(plan.cut);
        assert_eq!(plan.values(0.3)[0].words, far.words);
        // A mirror that comes or goes is matched, but snaps.
        let mut m = rot_z(0.0, 1.0);
        m[8] = -1.0;
        let plan = Plan::new(&[a], vec![matrix(m, [0.0, 0.0, 10.0], 1)]);
        assert_eq!((plan.matched, plan.snapped), (1, 1));
        // Every object turned by 90 degrees at once: a cut.
        let prev: Vec<Xform> = (0..4).map(|k| matrix(rot_z(0.0, 1.0), [k as f32, 0.0, 10.0], k)).collect();
        let cur: Vec<Xform> = (0..4).map(|k| matrix(rot_z(1.6, 1.0), [k as f32, 0.0, 10.0], k)).collect();
        let plan = Plan::new(&prev, cur.clone());
        assert!(plan.cut && plan.still());
        assert_eq!(plan.values(0.5)[2].words, cur[2].words);
        // One fast spinner among still objects is not a cut: it steps at
        // the halfway point while its translation blends.
        let mut cur: Vec<Xform> = prev.clone();
        cur[0] = matrix(rot_z(3.0, 1.0), [1.0, 0.0, 10.0], 0);
        let plan = Plan::new(&prev, cur);
        assert!(!plan.cut);
        let early = f(&plan.values(0.25)[0].words);
        assert!(close(&early[..9], &rot_z(0.0, 1.0)));
        assert!(close(&early[9..], &[0.25, 0.0, 10.0]));
    }

    #[test]
    fn matching_by_key_and_place() {
        let at = |x: f32, key| matrix(rot_z(0.0, 1.0), [x, 0.0, 20.0], key);
        // Frame N draws a new object (key 9) first and one of three of
        // model 5 is gone: the others still find their own.
        let prev = vec![at(0.0, 5), at(4.0, 5), at(8.0, 5), at(1.0, 6)];
        let cur = vec![at(9.0, 9), at(4.2, 5), at(8.2, 5), at(1.2, 6)];
        let plan = Plan::new(&prev, cur.clone());
        assert_eq!(plan.matched, 3);
        let v = plan.values(0.5);
        assert_eq!(v[0].words, cur[0].words, "unmatched: frame N's");
        assert!(close(&f(&v[1].words)[9..], &[4.1, 0.0, 20.0]), "nearest of its model, not the first");
        assert!(close(&f(&v[2].words)[9..], &[8.1, 0.0, 20.0]));
        assert!(close(&f(&v[3].words)[9..], &[1.1, 0.0, 20.0]));
        // A part whose model changed (a new pose) at the same place still
        // matches; one that moved far doesn't.
        let plan = Plan::new(&[at(1.0, 6)], vec![at(1.1, 7)]);
        assert_eq!(plan.matched, 1);
        assert_eq!(Plan::new(&[at(1.0, 6)], vec![at(9.0, 7)]).matched, 0);
        // Identical objects drawn in another order: each finds its own.
        let prev: Vec<Xform> = (0..6).map(|k| at(k as f32 * 3.0, 5)).collect();
        let cur: Vec<Xform> = (0..6).rev().map(|k| at(k as f32 * 3.0 + 0.1, 5)).collect();
        let v = Plan::new(&prev, cur).values(0.5);
        for (k, x) in v.iter().enumerate() {
            let want = (5 - k) as f32 * 3.0 + 0.05;
            assert!(close(&f(&x.words)[9..], &[want, 0.0, 20.0]), "{k}");
        }
        // Two characters sharing pose models: A, now in the pose B was in,
        // keeps its own previous transform rather than taking B's, even with
        // the list reordered.
        let prev = vec![at(0.0, 0x10), at(6.0, 0x11)];
        let cur = vec![at(6.1, 0x12), at(0.1, 0x11)];
        let v = Plan::new(&prev, cur).values(0.5);
        assert!(close(&f(&v[0].words)[9..], &[6.05, 0.0, 20.0]));
        assert!(close(&f(&v[1].words)[9..], &[0.05, 0.0, 20.0]));
        // Kinds don't mix.
        let t = Xform { key: 5, ..Xform::new(Kind::Translation, &[4f32.to_bits(), 0, 20f32.to_bits()]) };
        assert_eq!(Plan::new(&[t], vec![at(4.0, 5)]).matched, 0);
    }

    #[test]
    fn recording_keys_transforms_by_the_next_object() {
        let mut tap = Tap::record();
        tap.xform(Kind::Focus, &mut [1, 2]);
        tap.xform(Kind::Matrix, &mut [0; 12]);
        tap.object(0x80_1234);
        tap.object(0x80_5678);
        tap.xform(Kind::Translation, &mut [0; 3]);
        let list = tap.into_list();
        assert_eq!(list.iter().map(|x| x.key).collect::<Vec<_>>(), [0x80_1234, 0x80_1234, NO_KEY]);
        // A replay that meets a different command passes the rest through.
        let mut tap = Tap::replay(list);
        let mut w = [9, 9];
        tap.xform(Kind::Focus, &mut w);
        assert_eq!(w, [1, 2]);
        let mut w = [7; 3];
        tap.xform(Kind::Translation, &mut w);
        assert_eq!(w, [7; 3]);
    }

    /// Through the 2B native geometrizer: recording changes nothing, an
    /// in-between at t = 1 is the frame's own stream word for word, one at
    /// t = 0.5 has the object halfway, and polygon RAM writes in the list
    /// stay in the in-between's overlay.
    #[test]
    fn in_betweens_through_the_native_geometrizer() {
        use crate::geo_native::{Native as N2b, Program};
        let f = |v: f32| v.to_bits();
        let cmd = |fnc: u32| fnc << 23 | fnc << 8 | fnc;
        // P0, P1, a quad facing +z, the terminator.
        let mut rom = vec![f(0.0), f(0.0), f(0.0), f(1.0), f(0.0), f(0.0)];
        rom.extend([1 | 1 << 18 | 1 << 27, f(0.0), f(0.0), f(1.0)]);
        rom.extend([f(1.0), f(1.0), f(0.0), f(0.0), f(1.0), f(0.0)]);
        rom.push(0);
        let list = |x: f32| {
            let mut l = vec![cmd(0x0b)];
            l.extend([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, 0.0, 10.0].map(f));
            // A polygon RAM write, then the object.
            l.extend([cmd(0x09), f(2.0), f(3.0), cmd(0x05), 0x10, 1, 0xabcd]);
            l.extend([cmd(0x01), 0, 0, 0x80_0000, 0, cmd(0x0f)]);
            l.iter().flat_map(|w| w.to_le_bytes()).chain(std::iter::repeat(0)).take(0x2_0000).collect::<Vec<u8>>()
        };
        let rom = Arc::new(rom);
        let run = |n: &mut N2b, buffer: &[u8], tap: Option<&mut Tap>, ram: &mut Vec<u32>| {
            let (mut words, mut fed, mut out, mut latch) = (0, Vec::new(), Vec::new(), 0);
            let mut list = crate::geo::ListDma::new(buffer, &[], 0, &mut words, &mut fed);
            let mut io = crate::geo_native::Io {
                list: &mut list,
                poly: crate::geo::GeoExt::new(ram, &rom),
                latch: &mut latch,
                out: &mut out,
                tap,
                light: None,
            };
            n.run_frame(&mut io);
            out.retain(|&w| w != crate::geo::RASTER_NOP);
            out
        };
        let mut ram = vec![0; 0x1_0000];
        let mut n = N2b::new(Program::Shared2B, 0x7378_842e);
        run(&mut n, &list(0.0), None, &mut ram);
        let mut tap = Tap::record();
        run(&mut n, &list(0.0), Some(&mut tap), &mut ram);
        let prev = tap.into_list();
        assert_eq!(prev.len(), 2);
        assert_eq!((prev[1].kind, prev[1].key), (Kind::Focus, 0x80_0000), "keyed by the next object");
        let (before, poly) = (n.clone(), Arc::new(vec![0; 0x1_0000]));
        let mut plain = n.clone();
        let mut tap = Tap::record();
        let real = run(&mut n, &list(2.0), Some(&mut tap), &mut ram);
        assert_eq!(real, run(&mut plain, &list(2.0), None, &mut vec![0; 0x1_0000]), "recording changes nothing");
        let plan = Plan::new(&prev, tap.into_list());
        assert_eq!(plan.matched, 2);
        let native = Native::B(Box::new(before));
        let frame =
            Frame::new(native, list(2.0), Vec::new(), 0, 0, poly.clone(), rom.clone(), Vec::new(), plan, 3, None);
        assert_eq!(frame.render_values(frame.plan.values(1.0)), real);
        assert!(frame.render(1.0).is_none());
        let mid = frame.render(0.5).unwrap();
        assert_eq!(mid.len(), real.len());
        // P0 at the translation, x times fx: 2 · 1 halfway, 2 · 2 in frame N.
        assert_eq!(mid[3], f(2.0) >> 8);
        assert_eq!(real[3], f(4.0) >> 8);
        // With its lighting side channel: the same stream, one block with
        // its face (the stream's one object block; no window here, so
        // nothing to check its lumas against).
        let (lit, side) = frame.render_lit(0.5).unwrap();
        assert_eq!(lit, mid);
        assert_eq!((side.frame, side.blocks.len(), side.faces.len()), (3, 1, 1));
        assert_eq!(side.faces[0].n, [0.0, 0.0, 1.0]);
        assert_eq!(crate::light::self_check(&lit, &side).stats.misaligned_frames, 0);
        assert!(frame.render_lit(1.0).is_none());
        assert!(poly.iter().all(|&w| w == 0), "polygon RAM untouched");
        assert_eq!(ram[0x10], 0xabcd, "the real frame's write");
    }

    #[test]
    fn unsure_blocks_read_words_written_twice_or_late() {
        use Access::*;
        // Block 0 reads a word written once before (sure) and one never
        // written; block 1 reads a word written again later; block 2 one
        // written after it is read; block 3 reads after the second write.
        let log = [
            Write(1),
            Write(2),
            Block,
            Read(1),
            Read(9),
            Block,
            Read(2),
            Read(3),
            Write(3),
            Write(2),
            Block,
            Read(1),
            Block,
            Read(2),
        ];
        assert_eq!(unsure_blocks(&log), [1, 3]);
        assert!(unsure_blocks(&[Block, Read(1), Block, Read(1)]).is_empty(), "no writes: nothing to doubt");
    }

    /// A model the list writes into polygon RAM every frame (`lastbrnx`'s
    /// fighters), its vertices moved under the same transform: the motion
    /// replay draws frame N-1's model with frame N-1's transform, word for
    /// word frame N-1's block, while the in-between at t = 0 still draws
    /// frame N's model. Drawn before the list writes it, or written
    /// twice, it gets no motion data.
    #[test]
    fn motion_of_a_model_the_list_rewrites() {
        use crate::geo_native::{Native as N2b, Program};
        let f = |v: f32| v.to_bits();
        let cmd = |fnc: u32| fnc << 23 | fnc << 8 | fnc;
        // P0, P1 (its x given), a double-sided quad facing +z, the
        // terminator: 17 words at polygon RAM 0x100.
        let model = |x: f32| {
            let mut m = vec![f(0.0), f(0.0), f(0.0), f(x), f(0.0), f(0.0)];
            m.extend([1 | 1 << 8 | 1 << 17 | 1 << 18 | 1 << 27, f(0.0), f(0.0), f(1.0)]);
            m.extend([f(1.0), f(1.0), f(0.0), f(0.0), f(1.0), f(0.0)]);
            m.push(0);
            m
        };
        let write = |x: f32| {
            let mut l = vec![cmd(0x05), 0x100, 17];
            l.extend(model(x));
            l
        };
        // The model written with each x of `before`, drawn at translation
        // `tx`, then written with each of `after`.
        let list = |before: &[f32], tx: f32, after: &[f32]| {
            let mut l = vec![cmd(0x09), f(2.0), f(3.0)];
            l.extend(before.iter().flat_map(|&x| write(x)));
            l.push(cmd(0x0b));
            l.extend([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, tx, 0.0, 10.0].map(f));
            l.extend([cmd(0x01), 0, 0, 0x100, 0]);
            l.extend(after.iter().flat_map(|&x| write(x)));
            l.push(cmd(0x0f));
            l.iter().flat_map(|w| w.to_le_bytes()).chain(std::iter::repeat(0)).take(0x2_0000).collect::<Vec<u8>>()
        };
        let rom = Arc::new(vec![0]);
        let run = |n: &mut N2b, buffer: &[u8], tap: &mut Tap, ram: &mut Vec<u32>| {
            let (mut words, mut fed, mut out, mut latch) = (0, Vec::new(), Vec::new(), 0);
            let mut list = crate::geo::ListDma::new(buffer, &[], 0, &mut words, &mut fed);
            let mut io = crate::geo_native::Io {
                list: &mut list,
                poly: crate::geo::GeoExt::new(ram, &rom),
                latch: &mut latch,
                out: &mut out,
                tap: Some(tap),
                light: None,
            };
            n.run_frame(&mut io);
            out.retain(|&w| w != crate::geo::RASTER_NOP);
            out
        };
        // Frames N-1 and N with these lists (before them, the model holds
        // P1 at x = 9): frame N-1's stream, frame N's, its smooth frame.
        let frames = |prev: &[u8], now: &[u8]| {
            let (mut n, mut ram) = (N2b::new(Program::Shared2B, 0x7378_842e), vec![0; 0x1_0000]);
            ram[0x100..0x111].copy_from_slice(&model(9.0));
            // A frame first, as in `motion_through_the_native_geometrizer`.
            let empty: Vec<u8> =
                cmd(0x0f).to_le_bytes().into_iter().chain(std::iter::repeat(0)).take(0x2_0000).collect();
            run(&mut n, &empty, &mut Tap::record(), &mut ram);
            let mut tap = Tap::record();
            let prev_stream = run(&mut n, prev, &mut tap, &mut ram);
            let prev = tap.into_list();
            let (before, poly) = (n.clone(), Arc::new(ram.clone()));
            let mut tap = Tap::record();
            let real = run(&mut n, now, &mut tap, &mut ram);
            let plan = Plan::new(&prev, tap.into_list());
            let native = Native::B(Box::new(before));
            let frame = Frame::new(native, now.to_vec(), Vec::new(), 0, 0, poly, rom.clone(), vec![], plan, 3, None);
            (prev_stream, real, frame)
        };
        // P1 moves from x = 1 to x = 2 in the model, the object from 0 to
        // 0.5: frame N-1's block exactly, valid.
        let (prev_stream, real, frame) = frames(&list(&[1.0], 0.0, &[]), &list(&[2.0], 0.5, &[]));
        let m = frame.motion();
        assert_eq!(m.valid, [true]);
        assert_eq!(m.s0, prev_stream);
        assert_ne!(real, prev_stream);
        // The in-between at t = 0 keeps frame N's model (P1 at 2 + 0 = 2,
        // times fx 2), as before.
        let t0 = frame.render_values(frame.plan.values(0.0));
        assert_eq!(t0[6], f(4.0) >> 8);
        assert_eq!(m.s0[6], f(2.0) >> 8);
        // Written after it is drawn (the next frame's model), or twice:
        // no motion data.
        let (_, _, frame) = frames(&list(&[], 0.0, &[1.0]), &list(&[], 0.5, &[2.0]));
        assert_eq!(frame.motion().valid, [false]);
        let (_, _, frame) = frames(&list(&[5.0, 1.0], 0.0, &[]), &list(&[5.0, 2.0], 0.5, &[]));
        assert_eq!(frame.motion().valid, [false]);
    }

    /// Motion data through the 2B native geometrizer: the replay at t = 0
    /// has the real stream's layout, frame N-1's pose, and a validity entry
    /// per object block of the stream (objects whose transforms all
    /// blended, not new ones or direct data); a cut has none.
    #[test]
    fn motion_through_the_native_geometrizer() {
        use crate::geo_native::{Native as N2b, Program};
        let f = |v: f32| v.to_bits();
        let cmd = |fnc: u32| fnc << 23 | fnc << 8 | fnc;
        // Two copies of a model: P0, P1, a double-sided quad facing +z, the
        // terminator.
        let mut rom = Vec::new();
        for _ in 0..2 {
            rom.extend([f(0.0), f(0.0), f(0.0), f(1.0), f(0.0), f(0.0)]);
            rom.extend([1 | 1 << 8 | 1 << 17 | 1 << 18 | 1 << 27, f(0.0), f(0.0), f(1.0)]);
            rom.extend([f(1.0), f(1.0), f(0.0), f(0.0), f(1.0), f(0.0)]);
            rom.push(0);
        }
        let (a, b) = (0x80_0000, 0x80_0000 + 17);
        // Objects as (x, model), then direct data (a triangle).
        let list = |fx: f32, objects: &[(f32, u32)]| {
            let mut l = vec![cmd(0x09), f(fx), f(3.0)];
            for &(x, oba) in objects {
                l.push(cmd(0x0b));
                l.extend([1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, x, 0.0, 10.0].map(f));
                l.extend([cmd(0x01), 0, 0, oba, 0]);
            }
            l.extend([cmd(0x02), 0, 0]);
            l.extend([0.0, 0.0, 10.0, 1.0, 0.0, 10.0].map(f));
            l.extend([2 | 1 << 8, 0, 0, f(1.0), f(1.0), f(10.0), 0, cmd(0x0f)]);
            l.iter().flat_map(|w| w.to_le_bytes()).chain(std::iter::repeat(0)).take(0x2_0000).collect::<Vec<u8>>()
        };
        let rom = Arc::new(rom);
        let run = |n: &mut N2b, buffer: &[u8], tap: &mut Tap| {
            let (mut words, mut fed, mut out, mut latch) = (0, Vec::new(), Vec::new(), 0);
            let mut ram = vec![0; 0x1_0000];
            let mut list = crate::geo::ListDma::new(buffer, &[], 0, &mut words, &mut fed);
            let mut io = crate::geo_native::Io {
                list: &mut list,
                poly: crate::geo::GeoExt::new(&mut ram, &rom),
                latch: &mut latch,
                out: &mut out,
                tap: Some(tap),
                light: None,
            };
            n.run_frame(&mut io);
            out.retain(|&w| w != crate::geo::RASTER_NOP);
            out
        };
        // Frame N-1 draws model a at x = 0; frame N at x = 2, and model b
        // (new: nothing to match) far away.
        let frames = |fx_now: f32| {
            let mut n = N2b::new(Program::Shared2B, 0x7378_842e);
            run(&mut n, &list(2.0, &[]), &mut Tap::record());
            let mut tap = Tap::record();
            let prev_stream = run(&mut n, &list(2.0, &[(0.0, a)]), &mut tap);
            let prev = tap.into_list();
            let before = n.clone();
            let mut tap = Tap::record();
            let now = list(fx_now, &[(2.0, a), (500.0, b)]);
            let real = run(&mut n, &now, &mut tap);
            let plan = Plan::new(&prev, tap.into_list());
            let poly = Arc::new(vec![0; 0x1_0000]);
            let frame = Frame::new(
                Native::B(Box::new(before)),
                now,
                Vec::new(),
                0,
                0,
                poly,
                rom.clone(),
                vec![],
                plan,
                3,
                None,
            );
            (prev_stream, real, frame)
        };
        let (prev_stream, real, frame) = frames(2.0);
        assert!(!frame.plan.cut);
        let m = frame.motion();
        assert!(!m.cut);
        assert_eq!(m.s0.len(), real.len(), "the real stream's layout");
        let blocks = real.iter().filter(|&&w| w >> 24 == 0xff && w & 0xf == 1).count();
        assert_eq!(blocks, 3);
        assert_eq!(m.valid, [true, false, false]);
        assert!(std::ptr::eq(m, frame.motion()), "made once");
        // Model a's block is frame N-1's, word for word.
        let block = |s: &[u32]| s.iter().position(|&w| w >> 24 == 0xff && w & 0xf == 1).unwrap();
        let (i, j) = (block(&m.s0), block(&prev_stream));
        assert_eq!(m.s0[i..i + 16], prev_stream[j..j + 16]);
        assert_ne!(real[i..i + 16], prev_stream[j..j + 16]);
        // Through raster::build_motion: polygon for polygon.
        let palette = vec![0u8; 0x1_c000];
        let src = crate::raster::Sources {
            texture_rom: &[],
            texture_point_ram: &[],
            render_ram: &[],
            palette: &palette,
            xoff: 0.0,
            yoff: 0.0,
        };
        // A window first, as the games send.
        let window = [0xff00_0003, 0, 0x1ef17f, 0xf80c0, 0xf80c0, 0xf80c0, 0xf80c0];
        let with_window = |s: &[u32]| window.iter().chain(s).copied().collect::<Vec<u32>>();
        let (s0, s1) = (with_window(&m.s0), with_window(&real));
        let mi = crate::raster::MotionIn { s0: &s0, s1: &s1, tp: 0.0, valid: &m.valid, prev_centres: &[] };
        let (polys, _) = crate::raster::build_motion(&s1, &src, Some(&mi));
        assert_eq!(polys.iter().map(|p| !p.prev.is_empty()).collect::<Vec<_>>(), [true, false, false]);
        // A zoom: a cut, no replay.
        let (_, _, frame) = frames(4.0);
        assert!(frame.plan.cut);
        let m = frame.motion();
        assert!(m.cut && m.s0.is_empty() && m.valid.is_empty());
    }
}
