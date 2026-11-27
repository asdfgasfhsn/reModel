//! Photo mode: a frozen game frame seen from another camera, at any point
//! of the last few seconds, without touching the game (docs/photo-mode.md).
//!
//! Everything here works on copies. The game's machine only ever gives:
//! its inputs before each frame and a save state now and then
//! ([`History::before_frame`], read-only apart from waiting for its
//! geometrizer), or a copy of itself ([`Studio::fork`]). A second machine,
//! the [`Studio`], loads those and runs the frames again with smooth
//! motion's recording and the lighting side channel on, so the frame shown
//! comes with what [`crate::smooth::Frame`] needs to run its display list
//! through the native geometrizer again ([`render`]).
//!
//! **Free camera.** The geometrizer receives each object's transform
//! already in the game camera's view space: a rotation and translation
//! from model to view. One more rigid transform `V` (the [`Camera`])
//! applied to every transform the list sets moves everything into another
//! view, generically for every game: `M' = Rᵀ M`, `t' = Rᵀ (t - eye)`.
//! The light vectors the list sets are in the same space and are turned
//! by `Rᵀ` too ([`crate::smooth::Tap::light`]), so the lighting stays put
//! on the scene. Focal lengths scale by the zoom. The geometrizer then
//! culls, clips and projects for the new view as it does for the game's.
//! What it cannot do is show what the game never sent: objects the game
//! culled for its own camera are missing, and direct data (polygons given
//! in view space) stay where the game's camera saw them.

use crate::smooth::{Kind, Xform};
use crate::Machine;
use std::collections::VecDeque;
use std::sync::Arc;

/// A view of the frozen frame relative to the game's camera: the eye's
/// position and turn in the game camera's view space, and a zoom. The
/// default is the game's own view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Where the eye is, in the game camera's view space (x right, y
    /// up, z forward: the screen is at x / z, -y / z from the window's
    /// centre; the geometrizer's units).
    pub eye: [f32; 3],
    /// Turn in radians: about the view's y axis (yaw), then x (pitch),
    /// then z (roll).
    pub yaw: f32,
    pub pitch: f32,
    pub roll: f32,
    /// Focal lengths times this (above 1 zooms in).
    pub zoom: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera { eye: [0.0; 3], yaw: 0.0, pitch: 0.0, roll: 0.0, zoom: 1.0 }
    }
}

type M3 = [[f32; 3]; 3];

/// Columns `a` times `b` (both column by column).
fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..3).map(|k| a[k][r] * b[c][k]).sum()))
}

fn apply(a: &M3, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|r| a[0][r] * v[0] + a[1][r] * v[1] + a[2][r] * v[2])
}

/// `a` transposed, applied to `v`.
fn apply_t(a: &M3, v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|c| a[c][0] * v[0] + a[c][1] * v[1] + a[c][2] * v[2])
}

impl Camera {
    /// The game's own view (nothing to do).
    pub fn is_identity(&self) -> bool {
        *self == Camera::default()
    }

    /// The eye's axes in the game camera's view space, column by column
    /// (right, up, forward).
    pub fn axes(&self) -> M3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (sr, cr) = self.roll.sin_cos();
        let yaw = [[cy, 0.0, -sy], [0.0, 1.0, 0.0], [sy, 0.0, cy]];
        let pitch = [[1.0, 0.0, 0.0], [0.0, cp, sp], [0.0, -sp, cp]];
        let roll = [[cr, sr, 0.0], [-sr, cr, 0.0], [0.0, 0.0, 1.0]];
        mul(&mul(&yaw, &pitch), &roll)
    }

    /// A point of the game's view space in this camera's.
    pub fn view(&self, p: [f32; 3]) -> [f32; 3] {
        apply_t(&self.axes(), std::array::from_fn(|k| p[k] - self.eye[k]))
    }

    /// Move the eye by `d` along its own axes (right, up, forward).
    pub fn travel(&mut self, d: [f32; 3]) {
        let m = apply(&self.axes(), d);
        for (e, m) in self.eye.iter_mut().zip(m) {
            *e += m;
        }
    }

    /// The point `distance` ahead of the eye.
    pub fn ahead(&self, distance: f32) -> [f32; 3] {
        let f = self.axes()[2];
        std::array::from_fn(|k| self.eye[k] + f[k] * distance)
    }

    /// Turn about the point `distance` ahead (orbit): yaw and pitch change
    /// by these, the eye moves so that point stays in the middle.
    pub fn orbit(&mut self, distance: f32, yaw: f32, pitch: f32) {
        let pivot = self.ahead(distance);
        self.yaw += yaw;
        self.pitch = (self.pitch + pitch).clamp(-1.55, 1.55);
        let f = self.axes()[2];
        self.eye = std::array::from_fn(|k| pivot[k] - f[k] * distance);
    }

    /// The frame's transforms (`values`, in the list's order) moved into
    /// this view, and the turn for its light vectors (None: the game's
    /// view, nothing changed).
    pub fn pose(&self, mut values: Vec<Xform>) -> (Vec<Xform>, Option<[f32; 9]>) {
        if self.is_identity() {
            return (values, None);
        }
        let r = self.axes();
        let f = |w: u32| f32::from_bits(w);
        for x in &mut values {
            let w = &mut x.words;
            let moved = |w: &mut [u32]| {
                let t = self.view([f(w[0]), f(w[1]), f(w[2])]);
                w.copy_from_slice(&t.map(f32::to_bits));
            };
            match x.kind {
                Kind::Matrix | Kind::Matrix2 => {
                    for c in 0..3 {
                        let col = apply_t(&r, [f(w[3 * c]), f(w[3 * c + 1]), f(w[3 * c + 2])]);
                        w[3 * c..3 * c + 3].copy_from_slice(&col.map(f32::to_bits));
                    }
                    moved(&mut w[9..12]);
                }
                Kind::Translation => moved(&mut w[..3]),
                Kind::Focus => {
                    w[0] = (f(w[0]) * self.zoom).to_bits();
                    w[1] = (f(w[1]) * self.zoom).to_bits();
                }
            }
        }
        // Light vectors: Rᵀ, column by column.
        let turn = std::array::from_fn(|k| r[k % 3][k / 3]);
        (values, Some(turn))
    }
}

/// The words that set a [`Camera`] (`--photo`'s and `scene render
/// --camera`'s): `fov` is `zoom`.
pub const CAMERA_WORDS: &[&str] = &["orbit", "tilt", "yaw", "pitch", "roll", "x", "y", "z", "zoom", "fov"];

impl Camera {
    /// Apply one camera word: `orbit` and `tilt` turn about the point
    /// `pivot` ahead (degrees), `yaw`, `pitch` and `roll` turn in place
    /// (degrees), `x`, `y` and `z` move along the camera's axes in units of
    /// `scale` (the scene's, [`scene_scale`]), `zoom` (`fov`) is the focal
    /// length factor. Ok(false) for any other word, left alone.
    pub fn set_word(&mut self, k: &str, v: &str, scale: f32, pivot: f32) -> Result<bool, String> {
        if !CAMERA_WORDS.contains(&k) {
            return Ok(false);
        }
        let n = v.parse::<f32>().ok().filter(|n| n.is_finite()).ok_or_else(|| format!("{k}={v} wants a number"))?;
        let deg = f32::to_radians;
        match k {
            "orbit" => self.orbit(pivot, deg(n), 0.0),
            "tilt" => self.orbit(pivot, 0.0, deg(n)),
            "yaw" => self.yaw += deg(n),
            "pitch" => self.pitch += deg(n),
            "roll" => self.roll += deg(n),
            "x" => self.travel([n * scale, 0.0, 0.0]),
            "y" => self.travel([0.0, n * scale, 0.0]),
            "z" => self.travel([0.0, 0.0, n * scale]),
            _ => self.zoom = n.clamp(0.2, 20.0),
        }
        Ok(true)
    }
}

/// A camera as a still's command line gives it (`scene render --camera
/// SPEC`): photo mode's words in order, `KEY=VALUE,...`.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    pub camera: Camera,
    /// The near plane as a share of the scene's scale (photo mode's
    /// `near`, in percent in the words; 1% by default).
    pub near: f32,
    /// The game's HUD stays (photo mode leaves it out by default).
    pub hud: bool,
    /// Words that mean nothing for a still.
    pub warnings: Vec<String>,
}

impl Spec {
    /// Parse `spec` for a frame whose objects are `scale` away
    /// ([`scene_scale`]). Words: [`CAMERA_WORDS`], `near` (percent), `hud`
    /// (`on` or `off`); `back` (photo mode's time scrub) has no history to
    /// scrub in a scene and is noted and ignored.
    pub fn parse(spec: &str, scale: f32) -> Result<Spec, String> {
        let mut out = Spec { camera: Camera::default(), near: 0.01, hud: false, warnings: Vec::new() };
        for kv in spec.split(',').map(str::trim).filter(|x| !x.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("{kv} wants KEY=VALUE"))?;
            if out.camera.set_word(k, v, scale, scale)? {
                continue;
            }
            match k {
                "near" => {
                    let n = v.parse::<f32>().map_err(|_| format!("{k}={v} wants a number"))?;
                    out.near = n.clamp(0.0, 50.0) / 100.0;
                }
                "hud" => {
                    out.hud = match v {
                        "on" | "1" | "true" => true,
                        "off" | "0" | "false" => false,
                        _ => return Err(format!("{k}={v} wants on or off")),
                    }
                }
                "back" => out.warnings.push("`back` ignored: a scene is one frame, with no history to scrub".into()),
                _ => {
                    let all: Vec<&str> = CAMERA_WORDS.iter().copied().chain(["near", "hud", "back"]).collect();
                    return Err(format!("unknown camera word `{k}` (one of {})", all.join(", ")));
                }
            }
        }
        Ok(out)
    }

    /// The near plane in view units (0: the game's, with the game's view).
    pub fn near_plane(&self, scale: f32) -> f32 {
        if self.camera.is_identity() {
            0.0
        } else {
            self.near * scale
        }
    }
}

/// How far the frame's objects are from the game's eye: the median
/// distance of the matrices' translations (1 if none), the scale photo
/// mode moves the camera by.
pub fn scene_scale(values: &[Xform]) -> f32 {
    let mut d: Vec<f32> = values
        .iter()
        .filter(|x| matches!(x.kind, Kind::Matrix))
        .map(|x| {
            let t = [9, 10, 11].map(|k| f32::from_bits(x.words[k]));
            (t[0] * t[0] + t[1] * t[1] + t[2] * t[2]).sqrt()
        })
        .filter(|d| d.is_finite() && *d > 0.0)
        .collect();
    if d.is_empty() {
        return 1.0;
    }
    d.sort_by(f32::total_cmp);
    d[d.len() / 2]
}

/// `frame`'s display list through the native geometrizer again as `camera`
/// sees it, at `t` from the frame before (1: the frame itself; below,
/// smooth motion's in-between, for motion blur); with `lit`, its lighting
/// side channel. The raster stream is for the renderer, with the frame's
/// texture point RAM ([`crate::smooth::Frame::points`]).
pub fn render(
    frame: &crate::smooth::Frame,
    camera: &Camera,
    t: f32,
    lit: bool,
) -> (Vec<u32>, Option<crate::light::Frame>) {
    let (values, turn) = camera.pose(frame.plan.values(t.min(1.0)));
    frame.render_posed(values, turn, lit)
}

/// What a front end's controls set on the board before a frame: the
/// input ports, the analog channels and the gun counters. Put back before
/// the same frame, they make it run as it did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Inputs {
    inputs: [u8; 4],
    analog: [u8; 8],
    gun: Option<[u8; 16]>,
    /// The cheats on for the frame ([`crate::bus::Hold`]): replayed
    /// frames hold the same values the played ones did.
    holds: Arc<[crate::bus::Hold]>,
}

impl Inputs {
    pub fn of(bus: &crate::Model2Bus) -> Self {
        Inputs {
            inputs: bus.inputs,
            analog: bus.analog,
            gun: bus.gun.as_ref().map(|g| g.counters),
            holds: bus.holds.clone(),
        }
    }

    pub fn put(&self, bus: &mut crate::Model2Bus) {
        bus.inputs = self.inputs;
        bus.analog = self.analog;
        bus.holds = self.holds.clone();
        if let (Some(g), Some(c)) = (bus.gun.as_mut(), self.gun) {
            g.counters = c;
        }
    }
}

/// A save state of the game's machine, compressed on a thread of its own.
enum Key {
    Pending(std::thread::JoinHandle<Vec<u8>>),
    Ready(Arc<Vec<u8>>),
}

/// The game's last few seconds, for photo mode's time scrub: a save
/// state every `every` frames (compressed on another thread) and the
/// inputs of every frame since the oldest. Any frame from the oldest save
/// on can be run again exactly ([`Studio::seek`]).
pub struct History {
    every: u64,
    keep: usize,
    /// (frame, state), oldest first: the machine before running that frame.
    keys: VecDeque<(u64, Key)>,
    /// (frame, inputs) for each frame run: those set before it ran.
    inputs: VecDeque<(u64, Inputs)>,
    /// The machine's loads when recording began: a load starts afresh.
    loads: u64,
    /// Buffers the compressing threads are done with, for the next save
    /// state (about the size of the state each: a fresh one costs more
    /// than the copy into it).
    spare: Arc<std::sync::Mutex<Vec<Vec<u8>>>>,
    /// Save states taken so far, and the host time it took (on the
    /// game's thread; compressing them is on threads of their own).
    pub taken: u64,
    pub cost: std::time::Duration,
}

impl History {
    /// About `seconds` of history (at 57.5 frames a second), a save state
    /// every `every` frames.
    pub fn new(seconds: f64, every: u64) -> Self {
        let every = every.max(2);
        let keep = ((seconds * crate::pacing::FPS / every as f64).ceil() as usize).max(1) + 1;
        History {
            every,
            keep,
            keys: VecDeque::new(),
            inputs: VecDeque::new(),
            loads: 0,
            spare: Arc::default(),
            taken: 0,
            cost: Default::default(),
        }
    }

    /// Key `i`'s state, waiting for its thread if it's still at it.
    fn key_data(&mut self, i: usize) -> Arc<Vec<u8>> {
        let key = &mut self.keys[i].1;
        if let Key::Pending(_) = key {
            let Key::Pending(h) = std::mem::replace(key, Key::Ready(Arc::default())) else { unreachable!() };
            *key = Key::Ready(Arc::new(h.join().unwrap_or_default()));
        }
        match &self.keys[i].1 {
            Key::Ready(d) => d.clone(),
            Key::Pending(_) => unreachable!(),
        }
    }

    /// Before `m` runs a frame, its inputs set for it: note them, and
    /// every `every` frames a save state.
    pub fn before_frame(&mut self, m: &mut Machine) {
        let f = m.frames;
        // A load, or frames that didn't run here: what was kept doesn't
        // lead up to this one.
        if m.loads() != self.loads || self.inputs.back().is_some_and(|&(g, _)| g + 1 != f) {
            self.keys.clear();
            self.inputs.clear();
            self.loads = m.loads();
        }
        if self.keys.is_empty() || f.is_multiple_of(self.every) {
            let t = std::time::Instant::now();
            let buf = self.spare.lock().map_or_else(|_| Vec::new(), |mut s| s.pop().unwrap_or_default());
            let raw = m.save_state_into(buf);
            self.cost += t.elapsed();
            self.taken += 1;
            let spare = self.spare.clone();
            let h = std::thread::Builder::new()
                .name("photo history".into())
                .spawn(move || {
                    let packed = png::zlib::compress(&raw);
                    if let Ok(mut s) = spare.lock() {
                        // Two are enough: one being filled, one being packed.
                        if s.len() < 2 {
                            s.push(raw);
                        }
                    }
                    packed
                })
                .expect("start a thread");
            self.keys.push_back((f, Key::Pending(h)));
        }
        self.inputs.push_back((f, Inputs::of(&m.bus)));
        while self.keys.len() > self.keep {
            self.keys.pop_front();
        }
        let oldest = self.keys.front().map_or(f, |k| k.0);
        while self.inputs.front().is_some_and(|&(g, _)| g < oldest) {
            self.inputs.pop_front();
        }
    }

    /// The frames [`Studio::seek`] can show (the first two from the oldest
    /// save have no frame before them to blend from), if any: up to the
    /// last frame run.
    pub fn range(&self) -> Option<(u64, u64)> {
        let first = self.keys.front()?.0 + 2;
        let last = self.inputs.back()?.0 + 1;
        (first <= last).then_some((first, last))
    }

    fn inputs_at(&self, frame: u64) -> Option<Inputs> {
        let first = self.inputs.front()?.0;
        let (f, i) = self.inputs.get(frame.checked_sub(first)? as usize)?;
        (*f == frame).then(|| i.clone())
    }

    /// Bytes held (compressed states, inputs).
    pub fn bytes(&self) -> usize {
        let keys: usize = self.keys.iter().map(|(_, k)| if let Key::Ready(d) = k { d.len() } else { 0 }).sum();
        keys + self.inputs.len() * std::mem::size_of::<(u64, Inputs)>()
    }
}

/// Photo mode's own machine: frames of the game run again (from its
/// history) or forked from it, with smooth motion's recording and the
/// lighting side channel on, never seen by the game.
pub struct Studio {
    m: Box<Machine>,
    marks: crate::bus::CopyMarks,
    /// What the machine holds: a frame of the game's real timeline from
    /// history (`seek` may run on from it), or not (forked).
    on_timeline: bool,
}

impl Studio {
    /// A studio for `real`, built from `roms`, running the way it does.
    pub fn new(roms: &romset::RomSet, real: &mut Machine) -> Self {
        let mut m = Box::new(Machine::new(roms));
        m.configure_like(real);
        m.bus.geo.set_threaded(false);
        Studio { m, marks: Default::default(), on_timeline: false }
    }

    /// Start smooth motion's and the side channel's recording afresh (the
    /// machine jumped).
    fn record(&mut self) {
        let g = &mut self.m.bus.geo;
        g.set_smooth(false);
        g.set_smooth(true);
        g.set_light(true);
    }

    /// Run one frame with the inputs on the board.
    fn step(&mut self) -> Result<(), String> {
        self.m.bus.apply_holds();
        let r = self.m.run_slice(u64::MAX);
        // Its sound goes nowhere.
        self.m.bus.uart_log.clear();
        r.map(|_| ()).map_err(|e| format!("the game stopped: {e:?}"))
    }

    /// Take `shown`'s state and run `frames` more (at least 2, so the last
    /// has one before it to blend from) with its inputs as they are.
    pub fn fork(&mut self, shown: &mut Machine, frames: u32) -> Result<(), String> {
        shown.bus.geo.sync();
        self.marks = Default::default();
        self.m.copy_from(shown, &mut self.marks);
        self.record();
        self.on_timeline = false;
        for _ in 0..frames.max(2) {
            self.step()?;
        }
        Ok(())
    }

    /// Run frame `target` of the game's timeline again from `history`
    /// ([`History::range`]), with the inputs it had, going on from the
    /// frame held where that is on the way.
    pub fn seek(&mut self, history: &mut History, target: u64) -> Result<(), String> {
        let (first, last) = history.range().ok_or("no history yet")?;
        if !(first..=last).contains(&target) {
            return Err(format!("frame {target} is outside the history ({first} to {last})"));
        }
        // From the frame held, if it is on the way and close; else from the
        // newest save at least two frames before.
        let from_here = self.on_timeline && self.m.frames + 2 <= target && self.m.frames >= first - 2;
        let key = history.keys.iter().rposition(|(f, _)| f + 2 <= target).ok_or("no save state early enough")?;
        let key_frame = history.keys[key].0;
        if !from_here || key_frame > self.m.frames {
            let data = history.key_data(key);
            let raw = png::zlib::decompress(&data).map_err(|e| format!("history: {e}"))?;
            self.m.load_state(&raw)?;
            self.record();
        }
        self.on_timeline = true;
        while self.m.frames < target {
            let inputs = history.inputs_at(self.m.frames).ok_or("history has no inputs for a frame")?;
            inputs.put(&mut self.m.bus);
            if let Err(e) = self.step() {
                self.on_timeline = false;
                return Err(e);
            }
        }
        Ok(())
    }

    /// Run `frames` more with the inputs on the board (run-ahead's frames
    /// beyond the real one, as its machine runs them).
    pub fn extend(&mut self, frames: u32, inputs: Inputs) -> Result<(), String> {
        self.on_timeline = false;
        inputs.put(&mut self.m.bus);
        for _ in 0..frames {
            self.step()?;
        }
        Ok(())
    }

    /// The frame held, ready to draw again: its smooth motion data.
    pub fn frame(&mut self) -> Option<Arc<crate::smooth::Frame>> {
        self.m.bus.geo.sync();
        self.m.bus.geo.engine().smooth_frame()
    }

    /// The machine (to copy its video memory for drawing, or check its
    /// state hash).
    pub fn machine(&mut self) -> &mut Machine {
        &mut self.m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matrix(m: [f32; 9], t: [f32; 3]) -> Xform {
        let w: Vec<u32> = m.iter().chain(&t).map(|x| x.to_bits()).collect();
        Xform::new(Kind::Matrix, &w)
    }

    fn floats(x: &Xform) -> [f32; 12] {
        x.words.map(f32::from_bits)
    }

    fn close(a: &[f32], b: &[f32]) -> bool {
        a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-4)
    }

    /// A model point through a posed matrix lands where the camera sees the
    /// point the game's matrix put it at.
    #[test]
    fn posed_transforms_are_the_camera_view_of_the_game_view() {
        let c = 0.6f32.cos();
        let s = 0.6f32.sin();
        let game = matrix([c, 0.0, -s, 0.0, 1.0, 0.0, s, 0.0, c], [1.0, -2.0, 30.0]);
        let cam = Camera { eye: [3.0, 1.0, 5.0], yaw: 0.4, pitch: -0.2, roll: 0.1, zoom: 2.0 };
        let focus = Xform::new(Kind::Focus, &[100.0f32.to_bits(), 120.0f32.to_bits()]);
        let tr = Xform::new(Kind::Translation, &[4.0f32, 5.0, 6.0].map(f32::to_bits));
        let (posed, turn) = cam.pose(vec![game, focus, tr]);
        let (g, p) = (floats(&game), floats(&posed[0]));
        let model = [0.5, -1.5, 2.0];
        let at = |m: &[f32; 12], v: [f32; 3]| -> [f32; 3] {
            std::array::from_fn(|r| m[9 + r] + m[r] * v[0] + m[3 + r] * v[1] + m[6 + r] * v[2])
        };
        assert!(close(&at(&p, model), &cam.view(at(&g, model))));
        assert_eq!(floats(&posed[1])[..2], [200.0, 240.0]);
        assert!(close(&floats(&posed[2])[..3], &cam.view([4.0, 5.0, 6.0])));
        // The light turns as the normals do: Rᵀ.
        let turn = turn.unwrap();
        let mut l = [0.0f32, 0.0, 1.0].map(f32::to_bits);
        let mut tap = crate::smooth::Tap::Replay {
            values: Vec::new(),
            next: 0,
            lost: false,
            snap: Vec::new(),
            snapped: 0,
            blocks: Vec::new(),
            turn: Some(turn),
        };
        tap.light(&mut l);
        let r = cam.axes();
        assert!(close(&l.map(f32::from_bits), &apply_t(&r, [0.0, 0.0, 1.0])));
    }

    /// The game's own view changes no word; orbiting keeps the pivot ahead.
    #[test]
    fn identity_and_orbit() {
        let m = matrix([1.0, -0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0], [0.0, 0.0, 10.0]);
        let (v, turn) = Camera::default().pose(vec![m]);
        assert_eq!(v[0].words, m.words);
        assert!(turn.is_none());
        let mut cam = Camera::default();
        cam.orbit(10.0, 0.5, 0.3);
        assert!(close(&cam.ahead(10.0), &[0.0, 0.0, 10.0]));
        assert!(close(&cam.view([0.0, 0.0, 10.0]), &[0.0, 0.0, 10.0]));
        cam.travel([0.0, 0.0, 4.0]);
        assert!(close(&cam.view([0.0, 0.0, 10.0]), &[0.0, 0.0, 6.0]));
        assert_eq!(scene_scale(&[m]), 10.0);
    }

    #[test]
    fn inputs_round_trip() {
        let game = romset::games::find("topskatr").unwrap();
        let roms = romset::RomSet { game, program: vec![0; 0x1000], chips: vec![Vec::new(); game.files.len()] };
        let mut m = Machine::new(&roms);
        m.bus.inputs = [1, 2, 3, 4];
        m.bus.analog[3] = 77;
        let i = Inputs::of(&m.bus);
        m.bus.inputs = [0; 4];
        m.bus.analog = [0; 8];
        i.put(&mut m.bus);
        assert_eq!((m.bus.inputs, m.bus.analog[3]), ([1, 2, 3, 4], 77));
    }

    /// The cheats on are part of what a frame ran with: kept with the
    /// inputs, put back with them, written into RAM before a frame, and
    /// copied with the machine (run-ahead's and the studio's forks).
    #[test]
    fn holds_travel_with_the_inputs_and_hold_their_value() {
        let game = romset::games::find("topskatr").unwrap();
        let roms = romset::RomSet { game, program: vec![0; 0x1000], chips: vec![Vec::new(); game.files.len()] };
        let mut m = Machine::new(&roms);
        let ram = |m: &mut Machine| u16::from_le_bytes([m.bus.work_ram()[1].1[0x30b0], m.bus.work_ram()[1].1[0x30b1]]);
        m.bus.apply_holds();
        assert_eq!(ram(&mut m), 0, "no cheat on: nothing written");
        m.bus.holds =
            vec![crate::bus::Hold { name: "hold-clock".into(), addr: 0x5030b0, value: 0x0e10, width: 2 }].into();
        let on = Inputs::of(&m.bus);
        for _ in 0..3 {
            // The game ran over it, as its clock counts down.
            m.bus.apply_holds();
            assert_eq!(ram(&mut m), 0x0e10);
            m.bus.poke(0x5030b0, 0x0e0f, 2);
        }
        let mut copy = Machine::new(&roms);
        copy.copy_from(&mut m, &mut Default::default());
        assert_eq!(copy.bus.holds, m.bus.holds, "a fork holds what the real machine does");
        m.bus.holds = Default::default();
        on.put(&mut m.bus);
        assert_eq!(m.bus.holds.len(), 1, "put back with the inputs of the frame");
        let off = Inputs::of(&Machine::new(&roms).bus);
        off.put(&mut m.bus);
        assert!(m.bus.holds.is_empty(), "and a frame played with none holds none");
    }
}
