//! Photo mode (docs/photo-mode.md): the game frozen, seen from a free
//! camera, with a look of its own, captured to a PNG larger than the
//! window.
//!
//! The game's machine is never touched: its last seconds are kept as save
//! states and inputs (`model2::photo::History`, while playing), and photo
//! mode runs them again on a machine of its own (`model2::photo::Studio`),
//! which records what the free camera needs. The picture comes from that
//! machine's frame, drawn by the renderer's photo passes
//! (`gpu::Renderer::photo`), on the draw thread while the window shows it
//! and on the main thread for a capture. Here: the controls, the draw job
//! and `--photo`'s scripted sessions; the dock along the bottom edge (egui)
//! is `dock`.

mod dock;

use crate::launcher::Nav;
use dock::{Dock, Tab};
use model2::photo::Camera;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use winit::keyboard::KeyCode;

/// The key that opens and closes photo mode.
pub const KEY: KeyCode = KeyCode::F8;

/// Photo heights offered: the window's, then 1080p to 8K (the width is
/// the window's picture's shape).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Res {
    Window,
    H(u32),
}

pub const RESOLUTIONS: [Res; 5] = [Res::Window, Res::H(1080), Res::H(1440), Res::H(2160), Res::H(4320)];

impl Res {
    pub fn name(self) -> String {
        match self {
            Res::Window => "Window".into(),
            Res::H(2160) => "4K (2160)".into(),
            Res::H(4320) => "8K (4320)".into(),
            Res::H(h) => format!("{h}p"),
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "window" => Some(Res::Window),
            "4k" | "4K" => Some(Res::H(2160)),
            "8k" | "8K" => Some(Res::H(4320)),
            _ => s.trim_end_matches('p').parse().ok().filter(|h| (240..=8640).contains(h)).map(Res::H),
        }
    }
}

/// Letterbox openings offered (width over height; 0: none).
pub const LETTERBOX: [(f32, &str); 6] =
    [(0.0, "Off"), (2.39, "2.39:1"), (1.85, "1.85:1"), (16.0 / 9.0, "16:9"), (4.0 / 3.0, "4:3"), (1.0, "1:1")];

/// What the photo is drawn with beyond its look.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Options {
    /// Lay the game's HUD over the photo.
    pub hud: bool,
    /// Leave the 2D background (tile layers behind the 3D) out.
    pub hide_base: bool,
    /// Shading (None: the game's setting).
    pub shading: Option<gpu::Shading>,
    /// Bloom strength, percent.
    pub bloom: u8,
    pub depth: bool,
    /// Motion blur: images averaged (1: none), over this share of a frame
    /// before the one shown.
    pub blur: u32,
    pub shutter: f32,
    pub res: Res,
}

/// Photo mode while it's open.
pub struct Mode {
    pub camera: Camera,
    /// How far the frame's objects are (`model2::photo::scene_scale`): the
    /// camera moves by it.
    pub scale: f32,
    pub look: gpu::PhotoLook,
    pub opts: Options,
    /// Depth of field on (the look's aperture is kept while it's off).
    pub dof: bool,
    pub aperture: f32,
    /// The frames the time scrub reaches, the one shown, and the one live
    /// when photo mode opened.
    pub range: Option<(u64, u64)>,
    pub frame: u64,
    pub live: u64,
    /// A frame asked for by the scrub, not yet run.
    pub wanted: Option<u64>,
    /// The frame has smooth motion's recording: the free camera and motion
    /// blur work (not with the chip geometrizer).
    pub free: bool,
    /// The dock is shown (else only the picture); the tab it is on; the pad
    /// or keys last moved its focus (the focus ring shows).
    pub ui: bool,
    pub tab: Tab,
    pub dock: Dock,
    pub pad_used: bool,
    /// A mouse drag on the picture: the button and where it was.
    pub drag: Option<(winit::event::MouseButton, (f64, f64))>,
    pub last_click: Option<Instant>,
    /// Photos being written.
    pub saving: Vec<std::thread::JoinHandle<Result<String, String>>>,
    pub last_update: Instant,
    /// Focus picking asked for at this window pixel.
    pub pick: Option<(f64, f64)>,
    /// The near plane with the camera moved, a share of the scene's scale.
    pub near: f32,
}

impl Mode {
    pub fn new(opts: Options) -> Self {
        Mode {
            camera: Camera::default(),
            scale: 1.0,
            look: gpu::PhotoLook::default(),
            opts,
            dof: false,
            aperture: 0.01,
            range: None,
            frame: 0,
            live: 0,
            wanted: None,
            free: false,
            ui: true,
            tab: Tab::Camera,
            dock: Dock::default(),
            pad_used: false,
            drag: None,
            last_click: None,
            saving: Vec::new(),
            last_update: Instant::now(),
            pick: None,
            near: 0.01,
        }
    }

    /// The look as drawn (depth of field off: no aperture).
    pub fn look(&self) -> gpu::PhotoLook {
        gpu::PhotoLook { aperture: if self.dof { self.aperture } else { 0.0 }, ..self.look }
    }

    /// The point the camera orbits: the focus distance with depth of field,
    /// else the scene's scale.
    pub fn pivot(&self) -> f32 {
        if self.dof {
            self.look.focus
        } else {
            self.scale
        }
    }

    pub fn reset_camera(&mut self) {
        self.camera = Camera::default();
    }

    /// The tabs the dock shows: those with something to set as things are.
    pub fn tabs(&self) -> Vec<Tab> {
        dock::TABS.iter().copied().filter(|t| t.rows().iter().any(|r| self.shows(*r))).collect()
    }

    /// The next (`d` 1) or previous (-1) tab, round the ring.
    pub fn step_tab(&mut self, d: i32) {
        let tabs = self.tabs();
        let at = tabs.iter().position(|t| *t == self.tab).unwrap_or(0) as i32;
        self.tab = tabs[(at + d).rem_euclid(tabs.len() as i32) as usize];
        self.pad_used = true;
    }

    /// A navigation press from the pad or keys for the dock.
    pub fn press(&mut self, nav: Nav) {
        self.pad_used = true;
        self.dock.navs.push(nav);
    }

    /// A d-pad press that repeats while held (`repeat_held`).
    pub fn press_held(&mut self, nav: Nav) {
        self.press(nav);
        let now = Instant::now();
        self.dock.held = Some((nav, now, now));
    }

    /// Repeat the held d-pad direction: after a short wait, every 60 ms,
    /// while `down` says it is still held.
    pub fn repeat_held(&mut self, now: Instant, down: impl Fn(Nav) -> bool) {
        let Some((nav, start, last)) = self.dock.held else { return };
        if !down(nav) {
            self.dock.held = None;
        } else if now - start > Duration::from_millis(400) && now - last > Duration::from_millis(60) {
            self.press(nav);
            self.dock.held = Some((nav, start, now));
        }
    }
}

/// The settings the dock's tabs hold (`Tab::rows`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Row {
    Frame,
    Zoom,
    Roll,
    Dof,
    Focus,
    Aperture,
    Bokeh,
    Exposure,
    Contrast,
    Saturation,
    Warmth,
    Tint,
    Vignette,
    Grain,
    Aberration,
    Letterbox,
    Blur,
    Shutter,
    Hud,
    Background,
    Shading,
    Bloom,
    Depth,
    Near,
    Resolution,
    Reset,
}

/// What a row asks of the player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Take,
    Leave,
}

const SHADINGS: [Option<gpu::Shading>; 4] =
    [None, Some(gpu::Shading::Flat), Some(gpu::Shading::Vertex), Some(gpu::Shading::Pixel)];
const BLOOMS: [u8; 4] = [0, 25, 50, 100];
const BLURS: [u32; 5] = [1, 4, 8, 16, 32];
const NEARS: [f32; 7] = [0.0, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1];

fn step_in<T: PartialEq + Copy>(list: &[T], v: T, d: i32) -> T {
    let i = list.iter().position(|x| *x == v).unwrap_or(0) as i32;
    list[(i + d).clamp(0, list.len() as i32 - 1) as usize]
}

impl Mode {
    /// Whether `row` does anything as things are (else it's left out).
    pub fn shows(&self, row: Row) -> bool {
        match row {
            Row::Frame => self.range.is_some(),
            Row::Zoom | Row::Roll | Row::Reset | Row::Near => self.free,
            Row::Focus | Row::Aperture | Row::Bokeh => self.dof,
            Row::Blur => self.free,
            Row::Shutter => self.free && self.opts.blur > 1,
            _ => true,
        }
    }

    /// Step `row` by `d` (left -1, right +1; `fine` for a smaller step).
    pub fn adjust(&mut self, row: Row, d: i32) {
        let f = d as f32;
        let l = &mut self.look;
        match row {
            Row::Frame => {
                if let Some((a, b)) = self.range {
                    let at = self.wanted.unwrap_or(self.frame) as i64 + d as i64;
                    self.wanted = Some(at.clamp(a as i64, b as i64) as u64);
                }
            }
            Row::Zoom => self.camera.zoom = (self.camera.zoom * 1.1f32.powf(f)).clamp(0.2, 20.0),
            Row::Roll => self.camera.roll += f * 2f32.to_radians(),
            Row::Dof => self.dof = d > 0,
            Row::Focus => l.focus = (l.focus * 1.05f32.powf(f)).max(1e-3),
            Row::Aperture => self.aperture = (self.aperture + f * 0.002).clamp(0.001, 0.03),
            Row::Bokeh => l.bokeh = (l.bokeh + f * 0.1).clamp(0.0, 1.0),
            Row::Exposure => l.exposure = (l.exposure + f * 0.1).clamp(-3.0, 3.0),
            Row::Contrast => l.contrast = (l.contrast + f * 0.05).clamp(0.5, 1.5),
            Row::Saturation => l.saturation = (l.saturation + f * 0.05).clamp(0.0, 2.0),
            Row::Warmth => l.warmth = (l.warmth + f * 0.05).clamp(-1.0, 1.0),
            Row::Tint => l.tint = (l.tint + f * 0.05).clamp(-1.0, 1.0),
            Row::Vignette => l.vignette = (l.vignette + f * 0.05).clamp(0.0, 1.0),
            Row::Grain => l.grain = (l.grain + f * 0.05).clamp(0.0, 1.0),
            Row::Aberration => l.aberration = (l.aberration + f * 0.05).clamp(0.0, 1.0),
            Row::Letterbox => {
                let all: Vec<f32> = LETTERBOX.iter().map(|x| x.0).collect();
                l.letterbox = step_in(&all, l.letterbox, d);
            }
            Row::Blur => self.opts.blur = step_in(&BLURS, self.opts.blur, d),
            Row::Shutter => self.opts.shutter = (self.opts.shutter + f * 0.05).clamp(0.05, 1.0),
            Row::Hud => self.opts.hud = d > 0,
            Row::Background => self.opts.hide_base = d < 0,
            Row::Shading => self.opts.shading = step_in(&SHADINGS, self.opts.shading, d),
            Row::Bloom => self.opts.bloom = step_in(&BLOOMS, self.opts.bloom, d),
            Row::Depth => self.opts.depth = d > 0,
            Row::Near => self.near = step_in(&NEARS, self.near, d),
            Row::Resolution => self.opts.res = step_in(&RESOLUTIONS, self.opts.res, d),
            Row::Reset => self.reset_camera(),
        }
    }

    /// The row's name and value, as shown.
    pub fn text(&self, row: Row) -> (&'static str, String) {
        let l = &self.look;
        let pct = |v: f32| format!("{:.0}%", v * 100.0);
        let signed = |v: f32| format!("{v:+.2}");
        let on = |b: bool| if b { "On" } else { "Off" }.to_string();
        match row {
            Row::Frame => (
                "Time",
                match self.range {
                    Some((_, b)) => {
                        let at = self.wanted.unwrap_or(self.frame);
                        if at == self.live {
                            "Live".into()
                        } else {
                            format!("{:.2} s before", (b.max(self.live) - at) as f64 / model2::pacing::FPS)
                        }
                    }
                    None => "Live".into(),
                },
            ),
            Row::Zoom => ("Field of view", format!("{:.2}x zoom", self.camera.zoom)),
            Row::Roll => ("Roll", format!("{:.0}°", self.camera.roll.to_degrees())),
            Row::Dof => ("Depth of field", on(self.dof)),
            Row::Focus => ("Focus distance", format!("{:.2}", l.focus / self.scale.max(1e-6))),
            Row::Aperture => ("Aperture (blur)", format!("{:.1}%", self.aperture * 100.0)),
            Row::Bokeh => ("Bokeh highlights", pct(l.bokeh)),
            Row::Exposure => ("Exposure", format!("{:+.1} EV", l.exposure)),
            Row::Contrast => ("Contrast", pct(l.contrast)),
            Row::Saturation => ("Saturation", pct(l.saturation)),
            Row::Warmth => ("Warmth", signed(l.warmth)),
            Row::Tint => ("Tint", signed(l.tint)),
            Row::Vignette => ("Vignette", pct(l.vignette)),
            Row::Grain => ("Film grain", pct(l.grain)),
            Row::Aberration => ("Chromatic aberration", pct(l.aberration)),
            Row::Letterbox => {
                ("Letterbox", LETTERBOX.iter().find(|x| x.0 == l.letterbox).map_or("Off", |x| x.1).to_string())
            }
            Row::Blur => {
                ("Motion blur", if self.opts.blur > 1 { format!("{} images", self.opts.blur) } else { "Off".into() })
            }
            Row::Shutter => ("Shutter", format!("{:.0}% of a frame", self.opts.shutter * 100.0)),
            Row::Hud => ("HUD", if self.opts.hud { "Shown" } else { "Hidden" }.into()),
            Row::Background => ("2D background", if self.opts.hide_base { "Hidden" } else { "Shown" }.into()),
            Row::Shading => (
                "Shading",
                match self.opts.shading {
                    None => "The game's setting".into(),
                    Some(s) => s.name().to_string(),
                },
            ),
            Row::Bloom => ("Bloom", if self.opts.bloom == 0 { "Off".into() } else { format!("{}%", self.opts.bloom) }),
            Row::Depth => ("Depth buffer", on(self.opts.depth)),
            Row::Near => (
                "Near clip",
                if self.near == 0.0 { "The game's".into() } else { format!("{:.1}% of the scene", self.near * 100.0) },
            ),
            Row::Resolution => ("Photo size", self.opts.res.name()),
            Row::Reset => ("Reset camera", String::new()),
        }
    }

    /// Move the camera for `dt` seconds of held controls: `mv` along its
    /// axes (right, up, forward; -1 to 1), `look` yaw and pitch, `roll`,
    /// `zoom` (in or out), each -1 to 1.
    pub fn fly(&mut self, dt: f32, mv: [f32; 3], look: [f32; 2], roll: f32, zoom: f32) {
        if !self.free {
            return;
        }
        let speed = self.scale * 0.5 * dt;
        if mv != [0.0; 3] {
            self.camera.travel(mv.map(|v| v * speed));
        }
        self.camera.yaw += look[0] * 1.2 * dt;
        self.camera.pitch = (self.camera.pitch + look[1] * 1.2 * dt).clamp(-1.55, 1.55);
        self.camera.roll += roll * 0.8 * dt;
        if zoom != 0.0 {
            self.camera.zoom = (self.camera.zoom * (1.0 + zoom * dt)).clamp(0.2, 20.0);
        }
    }

    /// A mouse drag by (`dx`, `dy`) pixels with `button`.
    pub fn drag_by(&mut self, button: winit::event::MouseButton, dx: f32, dy: f32) {
        if !self.free {
            return;
        }
        use winit::event::MouseButton::*;
        match button {
            Left => self.camera.orbit(self.pivot(), dx * 0.005, dy * 0.005),
            Right => {
                self.camera.yaw += dx * 0.003;
                self.camera.pitch = (self.camera.pitch + dy * 0.003).clamp(-1.55, 1.55);
            }
            _ => self.camera.travel([-dx * self.scale * 0.002, dy * self.scale * 0.002, 0.0]),
        }
    }

    /// The mouse wheel by `lines` (up: in).
    pub fn wheel(&mut self, lines: f32) {
        if self.free {
            self.camera.travel([0.0, 0.0, lines * self.scale * 0.08]);
        }
    }
}

/// The photo's size: the window picture's shape at `res`'s height.
pub fn size(res: Res, window: (u32, u32)) -> (u32, u32) {
    let h = match res {
        Res::Window => window.1,
        Res::H(h) => h,
    };
    let w = (h as f64 * window.0 as f64 / window.1.max(1) as f64).round() as u32;
    (w.max(1), h.max(1))
}

/// What a photo may take on the GPU, bytes: about 1.4 GB, so that a 4K
/// photo draws with a desktop and its compositor using most of an 8 GB
/// card.
const GPU_BYTES: f64 = 1.4e9;

/// The most pixels worth drawing a photo at, whatever the memory.
const DRAW_PIXELS: f64 = 36e6;

/// What the GPU holds for each pixel the photo is drawn at, bytes
/// (measured: docs/photo-mode.md): the picture, its depth and depth test,
/// the 3D layer and its depth buffer, the look's 16-bit float picture and
/// the bloom chain, 28 in all; the HUD's two pictures and the motion
/// blur's sum take 8 more each.
fn bytes_per_pixel(o: &Options) -> f64 {
    28.0 + if o.hud { 8.0 } else { 0.0 } + if o.blur > 1 { 8.0 } else { 0.0 }
}

/// What 4x multisampling adds to a pixel: colour and depth, 4 samples each.
const MSAA_BYTES: f64 = 32.0;

/// Supersampling for a photo of `w` x `h` at `cost` bytes a pixel: as many
/// times the size a side (up to 4) as keeps the picture drawn within the
/// budget and `max_side`.
fn supersampling(w: u32, h: u32, max_side: u32, cost: f64) -> u32 {
    let px = w as f64 * h as f64;
    let mut k = (((DRAW_PIXELS).min(GPU_BYTES / cost) / px).sqrt().floor() as u32).clamp(1, 4);
    while k > 1 && (w * k > max_side || h * k > max_side) {
        k -= 1;
    }
    k
}

/// Multisampling to go with supersampling `k`: 4x only where the picture
/// is not supersampled (supersampling already antialiases, and the
/// multisampled targets would cost more than the whole picture) and fits
/// the budget.
fn multisampling(w: u32, h: u32, k: u32, cost: f64) -> u32 {
    if k == 1 && w as f64 * h as f64 * (cost + MSAA_BYTES) <= GPU_BYTES {
        4
    } else {
        1
    }
}

/// What to try, in order, for a photo of `w` x `h` at `cost` bytes a
/// pixel: the first step is 1080p at 4x (7680x4320), 1440p at 3x, 4K at 2x,
/// then each smaller supersampling (no multisampling), then the picture at
/// its own size with 4x multisampling where the budget allows, and last
/// with none. The GPU running out of memory moves down a step.
fn ladder(w: u32, h: u32, max_side: u32, cost: f64) -> Vec<(u32, u32)> {
    let k = supersampling(w, h, max_side, cost);
    let mut steps: Vec<(u32, u32)> = (1..=k).rev().map(|k| (k, multisampling(w, h, k, cost))).collect();
    if !steps.contains(&(1, 1)) {
        steps.push((1, 1));
    }
    steps
}

/// "7680x4320 with 4x MSAA" for a photo of `w` x `h` at `k` and `msaa`.
fn describe_sampling(w: u32, h: u32, k: u32, msaa: u32) -> String {
    if msaa > 1 {
        format!("{}x{} with {msaa}x MSAA", w * k, h * k)
    } else {
        format!("{}x{}", w * k, h * k)
    }
}

/// Where photos go: `reModel/<set>/` in the user's pictures folder (the
/// XDG pictures directory on Linux, Pictures on macOS and Windows), or
/// `--photo-dir`. Without a pictures folder, `<data dir>/<set>/photos/`.
pub fn dir(set: &str, over: Option<&Path>) -> PathBuf {
    if let Some(d) = over {
        return d.to_path_buf();
    }
    match dirs::picture_dir() {
        Some(p) => p.join("reModel").join(set),
        None => crate::settings::data_dir().join(set).join("photos"),
    }
}

/// A file name for a photo of `set` at `frame`, not yet taken in `dir`.
pub fn file_name(dir: &Path, set: &str, frame: u64, name: Option<&str>) -> PathBuf {
    if let Some(n) = name {
        return dir.join(format!("{n}.png"));
    }
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    (0..)
        .map(|k| dir.join(format!("{set}-{secs}-{frame}{}.png", if k == 0 { String::new() } else { format!("-{k}") })))
        .find(|p| !p.exists())
        .unwrap()
}

/// `--photo`'s session: at game frame `at`, open photo mode, set these,
/// take a photo, close it.
#[derive(Clone, Debug, Default)]
pub struct Script {
    pub at: u64,
    pub sets: Vec<(String, String)>,
}

/// `--photo FRAME[:KEY=VALUE,...]`, sessions separated by `;`.
pub fn parse_scripts(s: &str) -> Result<Vec<Script>, String> {
    let mut out = Vec::new();
    for item in s.split(';').filter(|x| !x.is_empty()) {
        let (at, rest) = item.split_once(':').unwrap_or((item, ""));
        let at = at.trim().parse().map_err(|_| format!("--photo: bad frame {at}"))?;
        let mut sets = Vec::new();
        for kv in rest.split(',').filter(|x| !x.is_empty()) {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("--photo: {kv} wants KEY=VALUE"))?;
            if !SCRIPT_KEYS.contains(&k) {
                return Err(format!("--photo: unknown {k} (one of {})", SCRIPT_KEYS.join(", ")));
            }
            sets.push((k.to_string(), v.to_string()));
        }
        out.push(Script { at, sets });
    }
    Ok(out)
}

pub const SCRIPT_KEYS: &[&str] = &[
    "back",
    "orbit",
    "tilt",
    "yaw",
    "pitch",
    "roll",
    "x",
    "y",
    "z",
    "zoom",
    "focus",
    "dof",
    "bokeh",
    "exposure",
    "contrast",
    "saturation",
    "warmth",
    "tint",
    "vignette",
    "grain",
    "aberration",
    "letterbox",
    "blur",
    "shutter",
    "hud",
    "background",
    "shading",
    "bloom",
    "depth",
    "near",
    "res",
    "name",
];

impl Mode {
    /// Apply one of `--photo`'s settings (all but `back`, the scrub, and
    /// `name`, which the player reads). Distances are in the scene's
    /// scale, angles in degrees.
    pub fn script(&mut self, k: &str, v: &str) -> Result<(), String> {
        let num = || v.parse::<f32>().map_err(|_| format!("--photo: {k}={v} wants a number"));
        let on = || match v {
            "on" | "1" | "true" => Ok(true),
            "off" | "0" | "false" => Ok(false),
            _ => Err(format!("--photo: {k}={v} wants on or off")),
        };
        let (s, deg) = (self.scale, |x: f32| x.to_radians());
        match k {
            "orbit" => self.camera.orbit(self.pivot(), deg(num()?), 0.0),
            "tilt" => self.camera.orbit(self.pivot(), 0.0, deg(num()?)),
            "yaw" => self.camera.yaw += deg(num()?),
            "pitch" => self.camera.pitch += deg(num()?),
            "roll" => self.camera.roll += deg(num()?),
            "x" => self.camera.travel([num()? * s, 0.0, 0.0]),
            "y" => self.camera.travel([0.0, num()? * s, 0.0]),
            "z" => self.camera.travel([0.0, 0.0, num()? * s]),
            "zoom" => self.camera.zoom = num()?.clamp(0.2, 20.0),
            "focus" => self.look.focus = num()? * s,
            "dof" => {
                let a = num()?;
                self.dof = a > 0.0;
                if a > 0.0 {
                    self.aperture = (a / 100.0).clamp(0.001, 0.03);
                }
            }
            "bokeh" => self.look.bokeh = num()?,
            "exposure" => self.look.exposure = num()?,
            "contrast" => self.look.contrast = num()?,
            "saturation" => self.look.saturation = num()?,
            "warmth" => self.look.warmth = num()?,
            "tint" => self.look.tint = num()?,
            "vignette" => self.look.vignette = num()?,
            "grain" => self.look.grain = num()?,
            "aberration" => self.look.aberration = num()?,
            "letterbox" => {
                self.look.letterbox = match v.split_once(':') {
                    Some((a, b)) => {
                        let (a, b): (f32, f32) = (a.parse().unwrap_or(0.0), b.parse().unwrap_or(1.0));
                        a / b.max(1e-3)
                    }
                    None => num()?,
                }
            }
            "blur" => self.opts.blur = (num()? as u32).clamp(1, 64),
            "shutter" => self.opts.shutter = num()?.clamp(0.01, 1.0),
            "hud" => self.opts.hud = on()?,
            "background" => self.opts.hide_base = !on()?,
            "shading" => {
                self.opts.shading = Some(gpu::Shading::parse(v).ok_or("--photo: shading wants flat, vertex or pixel")?)
            }
            "bloom" => self.opts.bloom = num()?.clamp(0.0, 100.0) as u8,
            "depth" => self.opts.depth = on()?,
            "near" => self.near = num()?.clamp(0.0, 50.0) / 100.0,
            "res" => {
                self.opts.res = Res::parse(v).ok_or("--photo: res wants window, 1080, 1440, 4k, 8k or a height")?
            }
            _ => {}
        }
        Ok(())
    }
}

/// The toast for a photo written, from `take_photo`'s line: "Saved
/// NAME.png (3840x2160)" (None: the line is a failure's).
pub fn saved_notice(line: &str) -> Option<String> {
    line.contains("; written in")
        .then(|| {
            let (path, rest) = line.split_once(" (")?;
            let name = Path::new(path).file_name()?.to_string_lossy();
            let size = rest.split(',').next()?;
            Some(format!("Saved {name} ({size})"))
        })
        .flatten()
}

/// Wait for photos being written; say what became of them.
pub fn reap(saving: &mut Vec<std::thread::JoinHandle<Result<String, String>>>, wait: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut keep = Vec::new();
    for h in saving.drain(..) {
        if wait || h.is_finished() {
            out.push(match h.join() {
                Ok(Ok(s)) => s,
                Ok(Err(e)) => e,
                Err(_) => "the photo's writer stopped".into(),
            });
        } else {
            keep.push(h);
        }
    }
    *saving = keep;
    out
}

/// The draw thread's job in photo mode: the camera and look to draw the
/// captured frame (`draw::Core::frame`, the studio's) with.
#[derive(Clone)]
pub struct Job {
    pub camera: Camera,
    pub look: gpu::PhotoLook,
    pub opts: Options,
    pub quality: gpu::Quality,
    pub rules: (&'static romset::Shading, u64),
    /// Times the output's size a side to draw at.
    pub ss: u32,
    /// The near plane (view space z; 0: the game's).
    pub near: f32,
}

/// The photo's images: the geometry for each (raster stream, lighting
/// side channel), kept in `cache` for the same frame, camera and blur.
pub type Shots = Vec<(Vec<u32>, Option<std::sync::Arc<model2::light::Frame>>)>;

/// What `shots` were made for.
#[derive(Clone, PartialEq)]
pub struct ShotKey {
    frame: usize,
    camera: [f32; 7],
    blur: u32,
    shutter: f32,
    lit: bool,
}

/// Draw `job` of the frame in `core` into `target`, `view` at `origin`.
pub fn draw(
    core: &mut crate::draw::Core,
    job: &Job,
    target: &wgpu::TextureView,
    view: &model2::video::View,
    origin: (u32, u32),
    format: wgpu::TextureFormat,
) -> Duration {
    let t = Instant::now();
    let f = &core.frame;
    if f.is_empty() {
        return Duration::ZERO;
    }
    let lit = job.quality.shading != gpu::Shading::Flat || job.quality.materials;
    core.renderer.set_light_rules(job.rules.0, job.rules.1);
    core.renderer.set_quality(job.quality);
    // The geometry: the camera's, at each motion blur image's place
    // between the frame before and this one; or the frame's own.
    let c = &job.camera;
    let key = ShotKey {
        frame: f.smooth().map_or(0, |sf| std::sync::Arc::as_ptr(sf) as usize),
        camera: [c.eye[0], c.eye[1], c.eye[2], c.yaw, c.pitch, c.roll, c.zoom],
        blur: job.opts.blur,
        shutter: job.opts.shutter,
        lit,
    };
    let fresh = !matches!(&core.photo_shots, Some((k, _)) if *k == key);
    if fresh {
        let shots: Shots = match f.smooth() {
            Some(sf) => {
                let n = job.opts.blur.max(1);
                (0..n)
                    .map(|i| {
                        let t = if n == 1 { 1.0 } else { 1.0 - job.opts.shutter * (n - 1 - i) as f32 / (n - 1) as f32 };
                        let (stream, side) = model2::photo::render(sf, c, t, lit);
                        (stream, side.map(std::sync::Arc::new))
                    })
                    .collect()
            }
            None => vec![(f.stream().to_vec(), f.light_frame().cloned())],
        };
        core.photo_shots = Some((key, shots));
    }
    let crate::draw::Core { frame, renderer, photo_shots, .. } = core;
    let shots = &photo_shots.as_ref().unwrap().1;
    let points = frame.points();
    let list: Vec<gpu::PhotoShot> =
        shots.iter().map(|(s, l)| gpu::PhotoShot { stream: s, points, light: l.clone() }).collect();
    let k = job.ss.max(1);
    let big = model2::video::View {
        width: view.width * k as usize,
        height: view.height * k as usize,
        sx: view.sx * k as f32,
        sy: view.sy * k as f32,
        ..*view
    };
    let req = gpu::PhotoRequest {
        view: big,
        shots: &list,
        hud: job.opts.hud,
        hide_base: job.opts.hide_base,
        look: job.look,
        bloom_levels: k.ilog2(),
        near: job.near,
    };
    renderer.photo(frame, &req, target, format, origin, (view.width as u32, view.height as u32));
    t.elapsed()
}

/// The dock's readout: the photo's size and, with the free camera, its
/// zoom and roll; else why the camera is fixed.
pub fn status(m: &Mode, window: (u32, u32)) -> String {
    let (w, h) = size(m.opts.res, window);
    if m.free {
        format!("{w}×{h} · {:.2}x zoom · roll {:.0}°", m.camera.zoom, m.camera.roll.to_degrees())
    } else {
        format!("{w}×{h} · fixed camera (the free camera needs the native geometrizer)")
    }
}

/// Where the picture is in the window, in points: the thirds grid and the
/// readout line up with it.
pub fn picture_rect(origin: (u32, u32), view: &model2::video::View, ppp: f32) -> egui::Rect {
    egui::Rect::from_min_size(
        egui::pos2(origin.0 as f32, origin.1 as f32) / ppp,
        egui::vec2(view.width as f32, view.height as f32) / ppp,
    )
}

// ---- The player's side: opening and closing, controls, captures. ----

impl crate::Play {
    /// A redraw in photo mode: the controls, then the photo drawn into the
    /// window with the dock over it (the game stays paused).
    pub(crate) fn photo_frame(&mut self) {
        self.photo_update();
        if self.photo.is_none() {
            return;
        }
        // `--perf-log`: the photo's redraws aren't the game's.
        if let Some(p) = self.session.as_mut().and_then(|s| s.perf.as_mut()) {
            p.pause();
        }
        let Some(g) = &mut self.gpu else { return };
        g.drawer.finish();
        self.take_report();
        // The window's preview: at most 8 motion blur images, to keep it
        // moving (the photo takes them all).
        let job = self.photo_job(1, 4).map(|mut j| {
            j.opts.blur = j.opts.blur.min(8);
            j
        });
        let g = self.gpu.as_mut().unwrap();
        let window = (g.view.width as u32, g.view.height as u32);
        let size = [g.config.width, g.config.height];
        let picture = picture_rect(g.origin, &g.view, g.window.scale_factor() as f32);
        let (menu, m) = (&mut self.menu, self.photo.as_mut().unwrap());
        let line = status(m, window);
        let (paint, actions) = crate::run_overlay(g, false, size, None, |ctx| {
            menu.photo_notices(ctx);
            m.ui(ctx, picture, &line)
        });
        let job = crate::draw::Job {
            game: None,
            photo: job,
            view: g.view,
            origin: g.origin,
            config: g.config.clone(),
            overlay: Some(paint),
            hash: self.frame_log.is_some(),
        };
        g.drawer.submit(job);
        if !g.drawer.overlaps() {
            self.take_report();
        }
        for a in actions {
            self.photo_action(a);
        }
        self.status();
    }

    /// Open photo mode on the frame shown (the game pauses).
    pub(crate) fn enter_photo(&mut self) -> Result<(), String> {
        let rom = self.settings.rom.clone();
        let opts = Options {
            hud: false,
            hide_base: false,
            shading: None,
            bloom: if self.temporal_supported { self.values.bloom } else { 0 },
            depth: self.values.depth,
            blur: 1,
            shutter: 0.5,
            res: Res::H(2160),
        };
        let s = self.session.as_mut().filter(|s| !s.preview).ok_or("no game")?;
        let mut studio = match s.studio.take() {
            Some(st) => st,
            None => {
                let roms = romset::RomSet::load(s.game, &rom, false).map_err(|e| e.to_string())?;
                model2::photo::Studio::new(&roms, &mut s.machine)
            }
        };
        // The frame shown: the game's, or run-ahead's that many on.
        let real = s.machine.frames;
        let ahead = s.run_ahead.as_ref().filter(|ra| ra.is_ahead_of(&s.machine)).map_or(0, |ra| ra.frames());
        let range = s.history.as_ref().and_then(|h| h.range()).filter(|r| r.1 == real);
        let mut m = Mode::new(opts);
        m.live = real + ahead as u64;
        let went = match (&mut s.history, range) {
            (Some(h), Some(r)) => {
                m.range = Some((r.0, m.live));
                studio.seek(h, real).and_then(|_| {
                    if ahead > 0 {
                        studio.extend(ahead, model2::photo::Inputs::of(&s.machine.bus))
                    } else {
                        Ok(())
                    }
                })
            }
            _ => {
                // No history: two frames on from the one shown.
                m.live = s.shown().frames + 2;
                studio.fork(s.shown(), 2)
            }
        };
        s.studio = Some(studio);
        went?;
        m.frame = m.live;
        self.keys.clear();
        self.photo = Some(m);
        if let Some(g) = &mut self.gpu {
            // The game's temporal history and the like sit unused while
            // photos are drawn, at the window's size or a photo's.
            g.drawer.core().renderer.photo_release();
        }
        self.photo_capture();
        self.autofocus();
        Ok(())
    }

    /// Focus on what is in the middle of the picture, a little below the
    /// centre (where third-person games put the player): a small draw of
    /// the photo, its depth read at a few points there, the nearest kept.
    fn autofocus(&mut self) {
        let Some(job) = self.photo_job(1, 1) else { return };
        let (Some(g), Some(m)) = (&mut self.gpu, &mut self.photo) else { return };
        let view = g.view;
        let (w, h) = (view.width as u32, view.height as u32);
        let core = g.drawer.core();
        let texture = core.renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("photo focus"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        draw(core, &job, &texture.create_view(&Default::default()), &view, (0, 0), wgpu::TextureFormat::Rgba8Unorm);
        let at = [(0.5, 0.55), (0.5, 0.65), (0.45, 0.6), (0.55, 0.6), (0.5, 0.5)];
        let nearest = at
            .iter()
            .filter_map(|&(x, y)| core.renderer.photo_iz_at((w as f32 * x) as u32, (h as f32 * y) as u32))
            .filter(|iz| *iz > 0.0)
            .fold(0.0f32, f32::max);
        if nearest > 0.0 {
            m.look.focus = 1.0 / nearest;
        }
    }

    /// Close photo mode: the game goes on where it was.
    pub(crate) fn leave_photo(&mut self) {
        let Some(mut m) = self.photo.take() else { return };
        for line in reap(&mut m.saving, true) {
            println!("photo         {line}");
        }
        self.keys.clear();
        if let Some(g) = &mut self.gpu {
            let core = g.drawer.core();
            core.forget_frame();
            core.photo_shots = None;
            // The photo pass's targets go; the game's are made again.
            core.renderer.photo_release();
        }
        if let Some(s) = &mut self.session {
            // What the renderer holds is the studio's: start afresh.
            s.drew_ahead = !s.drew_ahead;
        }
    }

    /// Run the studio to `target` (a frame of the scrub's range).
    fn photo_go(&mut self, target: u64) -> Result<(), String> {
        let (Some(m), Some(s)) = (&mut self.photo, &mut self.session) else { return Ok(()) };
        let (Some(h), Some(st)) = (&mut s.history, &mut s.studio) else { return Ok(()) };
        let real = s.machine.frames;
        st.seek(h, target.min(real))?;
        if target > real {
            st.extend((target - real) as u32, model2::photo::Inputs::of(&s.machine.bus))?;
        }
        m.frame = target;
        self.photo_capture();
        Ok(())
    }

    /// The studio's frame into the drawer's copy, and what photo mode
    /// learns from it.
    fn photo_capture(&mut self) {
        let (Some(g), Some(s), Some(m)) = (&mut self.gpu, &mut self.session, &mut self.photo) else { return };
        let Some(st) = &mut s.studio else { return };
        let core = g.drawer.core();
        core.forget_frame();
        core.photo_shots = None;
        core.frame.capture(&mut st.machine().bus, false);
        let first = m.scale == 1.0 && m.camera.is_identity();
        match core.frame.smooth() {
            Some(sf) => {
                m.free = true;
                m.scale = model2::photo::scene_scale(sf.plan.transforms());
            }
            None => m.free = false,
        }
        if first {
            m.look.focus = m.scale;
        }
    }

    /// The renderer's quality for photos: the game's settings with the
    /// photo's picture options, no temporal (supersampling instead), `msaa`.
    fn photo_quality(&self, m: &Mode, msaa: u32) -> gpu::Quality {
        let mut q = self.values.quality(self.temporal_supported);
        q.temporal = 0;
        // Photos are the game's picture, not the monitor's.
        q.crt = 0;
        q.msaa = msaa;
        q.depth = m.opts.depth;
        q.bloom = if self.temporal_supported { m.opts.bloom } else { 0 };
        if let Some(sh) = m.opts.shading {
            q.shading = sh;
        }
        if q.shading == gpu::Shading::Pixel {
            q.shading_fine = true;
        }
        if q.tex_filter != model2::enhance::TexFilter::Off {
            q.tex_filter = model2::enhance::TexFilter::Aniso(16);
        }
        q
    }

    /// The draw job for the photo as set (`ss` times the size a side).
    pub(crate) fn photo_job(&self, ss: u32, msaa: u32) -> Option<Job> {
        let (m, s) = (self.photo.as_ref()?, self.session.as_ref()?);
        Some(Job {
            camera: m.camera,
            look: m.look(),
            opts: m.opts,
            quality: self.photo_quality(m, msaa),
            rules: (&s.game.shading, crate::hud_mask(s.game)),
            ss,
            near: if m.camera.is_identity() { 0.0 } else { m.near * m.scale },
        })
    }

    /// Take the photo: drawn at its size, read back, written to a PNG on
    /// another thread (waited for with `wait`). Says what it did.
    pub(crate) fn take_photo(&mut self, name: Option<&str>, wait: bool) -> Result<String, String> {
        let g = self.gpu.as_ref().ok_or("no window")?;
        let window = (g.view.width as u32, g.view.height as u32);
        let hud_mode = g.view.hud;
        let (m, s) = (self.photo.as_ref().ok_or("not in photo mode")?, self.session.as_ref().ok_or("no game")?);
        let (w, h) = size(m.opts.res, window);
        let cost = bytes_per_pixel(&m.opts);
        let set = s.game.id;
        let frame = m.frame;
        let (hw, hew) = (s.game.hud_windows, s.game.hud_element_windows);
        let g = self.gpu.as_mut().unwrap();
        let max_side = g.drawer.core().renderer.device().limits().max_texture_dimension_2d;
        let steps = ladder(w, h, max_side, cost);
        let view = model2::video::View::new(h as usize, w as f32 / h as f32, hud_mode).with_hud_windows(hw, hew);
        let (w, h) = (view.width as u32, view.height as u32);
        let t = Instant::now();
        // Drawn at the first step that fits in the GPU's memory.
        let mut short = Vec::new();
        let mut done = None;
        for &(k, msaa) in &steps {
            let job = self.photo_job(k, msaa).ok_or("no game")?;
            let core = self.gpu.as_mut().unwrap().drawer.core();
            let device = core.renderer.device().clone();
            let drawn = gpu::catch(&device, || {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("photo"),
                    size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let target = texture.create_view(&Default::default());
                draw(core, &job, &target, &view, (0, 0), wgpu::TextureFormat::Rgba8Unorm);
                texture
            });
            match drawn {
                Ok(texture) => {
                    done = Some((k, msaa, core.renderer.read_texture(&texture, w, h)));
                    break;
                }
                Err(gpu::GpuFault::OutOfMemory) => short.push(describe_sampling(w, h, k, msaa)),
                Err(gpu::GpuFault::Other(e)) => {
                    core.renderer.photo_release();
                    return Err(format!("A draw failed: {e}"));
                }
            }
            core.renderer.photo_release();
        }
        let core = self.gpu.as_mut().unwrap().drawer.core();
        // What the picture held is the photo's: the preview starts afresh
        // at its own size.
        core.renderer.photo_release();
        let Some((k, msaa, rgba)) = done else {
            return Err(format!("The GPU ran out of memory drawing {w}x{h} ({})", short.join(", ")));
        };
        let drawn = t.elapsed();
        // The window's preview is drawn at its own size again.
        core.photo_shots = None;
        let dir = dir(set, self.opts.photo_dir.as_deref());
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = file_name(&dir, set, frame, name);
        let fell = if short.is_empty() {
            String::new()
        } else {
            format!(", after the GPU ran out of memory at {}", short.join(" and "))
        };
        let what = format!(
            "{} ({w}x{h}, drawn at {} in {:.0} ms{fell}, frame {frame})",
            path.display(),
            describe_sampling(w, h, k, msaa),
            drawn.as_secs_f64() * 1000.0
        );
        let writer = std::thread::spawn(move || {
            let t = Instant::now();
            let png = png::encode_rgba(w as usize, h as usize, &rgba);
            std::fs::write(&path, png).map_err(|e| format!("{}: {e}", path.display()))?;
            Ok(format!("{what}; written in {:.0} ms", t.elapsed().as_secs_f64() * 1000.0))
        });
        let m = self.photo.as_mut().unwrap();
        m.saving.push(writer);
        if wait {
            let lines = reap(&mut m.saving, true);
            return lines.into_iter().next().ok_or_else(|| "no photo".into());
        }
        Ok(format!("Taking {w}x{h}..."))
    }

    /// One redraw's photo mode: held keys and the pad's sticks move the
    /// camera, the scrub runs the studio, a pick sets the focus, photos
    /// written are reported.
    pub(crate) fn photo_update(&mut self) {
        let now = Instant::now();
        let pad = self.gilrs.as_ref().and_then(|g| self.pad.map(|id| g.gamepad(id)));
        let Some(m) = &mut self.photo else { return };
        let dt = (now - m.last_update).as_secs_f32().min(0.1);
        m.last_update = now;
        let key = |k: KeyCode| self.keys.contains(&k) as i32 as f32;
        let fast = if key(KeyCode::ShiftLeft) + key(KeyCode::ShiftRight) > 0.0 {
            4.0
        } else if key(KeyCode::ControlLeft) > 0.0 {
            0.25
        } else {
            1.0
        };
        let mut mv = [
            key(KeyCode::KeyD) - key(KeyCode::KeyA),
            key(KeyCode::KeyR) - key(KeyCode::KeyF),
            key(KeyCode::KeyW) - key(KeyCode::KeyS),
        ];
        let mut look =
            [key(KeyCode::ArrowRight) - key(KeyCode::ArrowLeft), key(KeyCode::ArrowDown) - key(KeyCode::ArrowUp)];
        let mut roll = key(KeyCode::KeyE) - key(KeyCode::KeyQ);
        let mut zoom = key(KeyCode::KeyX) - key(KeyCode::KeyZ);
        if let Some(p) = pad {
            let axis = |a: gilrs::Axis| {
                let v = p.value(a);
                if v.abs() < 0.15 {
                    0.0
                } else {
                    v
                }
            };
            let button = |b: gilrs::Button| p.button_data(b).map_or(0.0, |d| d.value());
            mv[0] += axis(gilrs::Axis::LeftStickX);
            mv[2] += axis(gilrs::Axis::LeftStickY);
            look[0] += axis(gilrs::Axis::RightStickX);
            look[1] -= axis(gilrs::Axis::RightStickY);
            // LB and RB switch the dock's tabs: the stick clicks roll.
            roll += p.is_pressed(gilrs::Button::RightThumb) as i32 as f32
                - p.is_pressed(gilrs::Button::LeftThumb) as i32 as f32;
            let held = |n: Nav| match n {
                Nav::Up => p.is_pressed(gilrs::Button::DPadUp),
                Nav::Down => p.is_pressed(gilrs::Button::DPadDown),
                Nav::Left => p.is_pressed(gilrs::Button::DPadLeft),
                _ => p.is_pressed(gilrs::Button::DPadRight),
            };
            m.repeat_held(now, held);
            zoom += button(gilrs::Button::RightTrigger2) - button(gilrs::Button::LeftTrigger2);
        }
        m.fly(dt, mv.map(|v| v * fast), look, roll, zoom);
        let saved = reap(&mut m.saving, false);
        let wanted = m.wanted.take().filter(|t| *t != m.frame);
        for line in saved {
            println!("photo         {line}");
            match saved_notice(&line) {
                Some(text) => self.menu.notify(text),
                None => self.menu.warn(format!("Photo failed: {line}")),
            }
        }
        if let Some(target) = wanted {
            if let Err(e) = self.photo_go(target) {
                self.menu.warn(format!("Time scrub: {e}"));
            }
        }
        let Some(m) = self.photo.as_mut() else { return };
        if let (Some((x, y)), Some(g)) = (m.pick.take(), &mut self.gpu) {
            let (ox, oy) = (x - g.origin.0 as f64, y - g.origin.1 as f64);
            if ox >= 0.0 && oy >= 0.0 {
                let iz = g.drawer.core().renderer.photo_iz_at(ox as u32, oy as u32);
                match iz.filter(|z| *z > 0.0) {
                    Some(iz) => {
                        m.look.focus = 1.0 / iz;
                        m.dof = true;
                        self.menu.notify(format!("Focus at {:.2}", m.look.focus / m.scale.max(1e-6)));
                    }
                    None => self.menu.warn("Nothing to focus on there"),
                }
            }
        }
    }

    /// A key in photo mode: held ones move the camera (`photo_update`),
    /// presses do what they say.
    pub(crate) fn photo_key(&mut self, code: KeyCode, pressed: bool, repeat: bool) {
        if pressed {
            self.keys.insert(code);
        } else {
            self.keys.remove(&code);
        }
        if !pressed {
            return;
        }
        let Some(m) = &mut self.photo else { return };
        // The dock's keys repeat while held; the rest act once.
        let nav = match code {
            KeyCode::PageUp => Some(Nav::Up),
            KeyCode::PageDown => Some(Nav::Down),
            KeyCode::Comma => Some(Nav::Left),
            KeyCode::Period => Some(Nav::Right),
            _ => None,
        };
        if let Some(n) = nav {
            m.press(n);
            return;
        }
        if repeat {
            return;
        }
        let action = match code {
            KeyCode::Escape | KEY => Some(Action::Leave),
            KeyCode::Enter | KeyCode::Space | KeyCode::NumpadEnter => Some(Action::Take),
            KeyCode::KeyH => {
                m.ui = !m.ui;
                None
            }
            KeyCode::Tab => {
                m.opts.hud = !m.opts.hud;
                None
            }
            KeyCode::Backspace => {
                m.reset_camera();
                None
            }
            KeyCode::BracketLeft => {
                m.step_tab(-1);
                None
            }
            KeyCode::BracketRight => {
                m.step_tab(1);
                None
            }
            _ => None,
        };
        if let Some(a) = action {
            self.photo_action(a);
        }
    }

    /// A gamepad button pressed in photo mode.
    pub(crate) fn photo_button(&mut self, b: gilrs::Button) {
        use gilrs::Button::*;
        let Some(m) = &mut self.photo else { return };
        m.pad_used = true;
        let action = match b {
            // With the dock hidden there is only the shutter for A.
            South if !m.ui => Some(Action::Take),
            South => {
                m.dock.navs.push(Nav::Accept);
                None
            }
            East | Start | Mode => Some(Action::Leave),
            North => {
                m.ui = !m.ui;
                None
            }
            West => {
                m.opts.hud = !m.opts.hud;
                None
            }
            Select => {
                m.reset_camera();
                None
            }
            LeftTrigger => {
                m.step_tab(-1);
                None
            }
            RightTrigger => {
                m.step_tab(1);
                None
            }
            DPadUp => {
                m.press_held(Nav::Up);
                None
            }
            DPadDown => {
                m.press_held(Nav::Down);
                None
            }
            DPadLeft => {
                m.press_held(Nav::Left);
                None
            }
            DPadRight => {
                m.press_held(Nav::Right);
                None
            }
            _ => None,
        };
        if let Some(a) = action {
            self.photo_action(a);
        }
    }

    /// Mouse input in photo mode (not over the dock): drags move the
    /// camera, a double-click picks the focus, the wheel moves in and out.
    pub(crate) fn photo_mouse(&mut self, ev: &winit::event::WindowEvent) {
        use winit::event::{ElementState, MouseScrollDelta, WindowEvent};
        let at = self.mouse;
        let Some(m) = &mut self.photo else { return };
        match ev {
            WindowEvent::MouseInput { state, button, .. } => {
                if *state == ElementState::Pressed {
                    if let Some(at) = at {
                        m.drag = Some((*button, at));
                        let double = m.last_click.is_some_and(|t| t.elapsed() < Duration::from_millis(350));
                        if *button == winit::event::MouseButton::Left {
                            if double {
                                m.pick = Some(at);
                                m.last_click = None;
                            } else {
                                m.last_click = Some(Instant::now());
                            }
                        }
                    }
                } else {
                    m.drag = None;
                }
                m.pad_used = false;
            }
            WindowEvent::CursorMoved { position, .. } => {
                if let Some((b, from)) = m.drag {
                    m.drag_by(b, (position.x - from.0) as f32, (position.y - from.1) as f32);
                    m.drag = Some((b, (position.x, position.y)));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let lines = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                m.wheel(lines);
            }
            _ => {}
        }
    }

    /// Do what the dock, a key or a button asked.
    pub(crate) fn photo_action(&mut self, a: Action) {
        match a {
            Action::Leave => self.leave_photo(),
            Action::Take => match self.take_photo(None, false) {
                Ok(t) => self.menu.notify(t),
                Err(e) => self.menu.error(format!("Photo failed: {e}")),
            },
        }
    }

    /// `--photo`'s next session, when the game has reached its frame:
    /// open photo mode, set it up, take a photo, check the studio against
    /// the game, close it.
    pub(crate) fn photo_script(&mut self) {
        let due = self.session.as_ref().filter(|s| !s.preview).map(|s| s.machine.frames);
        let Some(frames) = due else { return };
        if self.photo.is_some() || self.opts.photo.front().is_none_or(|p| p.at > frames) {
            return;
        }
        let script = self.opts.photo.pop_front().unwrap();
        let t = Instant::now();
        if let Err(e) = self.enter_photo() {
            eprintln!("--photo: {e}");
            return;
        }
        let entered = t.elapsed();
        // The studio's frame against the game's: the same machine state,
        // when it is the game's own frame (no run-ahead).
        if let (Some(s), Some(m)) = (&mut self.session, &self.photo) {
            let (real, real_hash) = (s.machine.frames, s.machine.bus.state_hash());
            if let Some(st) = s.studio.as_mut().filter(|_| m.frame == real && m.range.is_some()) {
                let h = st.machine().bus.state_hash();
                println!(
                    "photo         frame {real}: studio state hash {h:016x} {} the game's",
                    if h == real_hash { "is" } else { "DIFFERS from" }
                );
            }
        }
        let mut name = None;
        for (k, v) in &script.sets {
            match k.as_str() {
                "back" => {
                    let m = self.photo.as_mut().unwrap();
                    let back: u64 = v.parse().unwrap_or(0);
                    let to = m.range.map_or(m.frame, |r| m.live.saturating_sub(back).max(r.0));
                    if let Err(e) = self.photo_go(to) {
                        eprintln!("--photo: back={v}: {e}");
                    }
                }
                "name" => name = Some(v.clone()),
                _ => {
                    if let Err(e) = self.photo.as_mut().unwrap().script(k, v) {
                        eprintln!("{e}");
                    }
                }
            }
        }
        match self.take_photo(name.as_deref(), true) {
            Ok(line) => println!("photo         {line}"),
            Err(e) => eprintln!("--photo: {e}"),
        }
        println!("photo         opened in {:.0} ms", entered.as_secs_f64() * 1000.0);
        self.leave_photo();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAX: u32 = 16384;
    const PLAIN: f64 = 28.0;
    /// The HUD and motion blur both on.
    const FULL: f64 = 44.0;

    fn first(w: u32, h: u32, max_side: u32, cost: f64) -> (u32, u32) {
        ladder(w, h, max_side, cost)[0]
    }

    #[test]
    fn photos_supersample_or_multisample_never_both() {
        // 1080p, 1440p and 4K supersample (4x, 3x, 2x); 5K and up are not.
        assert_eq!(first(1920, 1080, MAX, PLAIN), (4, 1));
        assert_eq!(first(2560, 1440, MAX, PLAIN), (3, 1));
        assert_eq!(first(3840, 2160, MAX, PLAIN), (2, 1));
        // 4x multisampling where the budget has room for its targets.
        assert_eq!(first(5120, 2880, MAX, PLAIN), (1, 4));
        assert_eq!(first(7680, 4320, MAX, PLAIN), (1, 1));
        for cost in [PLAIN, FULL] {
            for (w, h) in [(640, 480), (1280, 720), (1920, 1080), (3840, 2160), (5120, 2880), (7680, 4320)] {
                for (k, msaa) in ladder(w, h, MAX, cost) {
                    assert!(k == 1 || msaa == 1, "{w}x{h}: {k}x with {msaa}x MSAA");
                }
            }
        }
    }

    #[test]
    fn a_4k_photo_fits_the_budget() {
        for cost in [PLAIN, FULL] {
            let (k, msaa) = first(3840, 2160, MAX, cost);
            let px = 3840.0 * 2160.0 * (k * k) as f64;
            assert!(px * (cost + if msaa > 1 { MSAA_BYTES } else { 0.0 }) <= GPU_BYTES, "{k}x {msaa}");
        }
        // The HUD and motion blur's extra targets cost the supersampling.
        assert_eq!(first(3840, 2160, MAX, FULL), (1, 4));
        assert_eq!(first(1920, 1080, MAX, FULL).0, 3);
    }

    #[test]
    fn supersampling_stays_within_the_gpus_largest_texture() {
        assert_eq!(first(1920, 1080, 4096, PLAIN), (2, 1));
        assert_eq!(first(1920, 1080, 1920, PLAIN), (1, 4));
    }

    #[test]
    fn the_ladder_steps_down_to_the_picture_alone() {
        assert_eq!(ladder(3840, 2160, MAX, PLAIN), [(2, 1), (1, 4), (1, 1)]);
        assert_eq!(ladder(1920, 1080, MAX, PLAIN), [(4, 1), (3, 1), (2, 1), (1, 4), (1, 1)]);
        assert_eq!(ladder(5120, 2880, MAX, PLAIN), [(1, 4), (1, 1)]);
        // Where multisampling is out of budget already, the picture alone.
        assert_eq!(ladder(15360, 8640, MAX, PLAIN), [(1, 1)]);
        for (w, h) in [(1280, 720), (3840, 2160), (7680, 4320), (15360, 8640)] {
            let l = ladder(w, h, MAX, FULL);
            assert_eq!(l.last(), Some(&(1, 1)));
            assert!(l.windows(2).all(|p| p[0].0 >= p[1].0), "{l:?} gets no cheaper");
        }
    }

    #[test]
    fn what_was_drawn_is_described() {
        assert_eq!(describe_sampling(3840, 2160, 2, 1), "7680x4320");
        assert_eq!(describe_sampling(3840, 2160, 1, 4), "3840x2160 with 4x MSAA");
    }
}
