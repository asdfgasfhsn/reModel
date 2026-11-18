//! reModel in a browser: the board model compiled to WebAssembly, drawn
//! with WebGPU through the GPU rasteriser, with keyboard and gamepad input.
//! `make web` builds it into out/web/ (see docs/web.md).
//!
//! The page calls [`start`] with a canvas and the URL of the ROM files;
//! everything after that runs from `requestAnimationFrame`. At a display
//! refresh near the game's (60 Hz, or 120 Hz at one frame per two) the
//! machine runs exactly one frame per refresh, counted from the animation
//! frame times (`model2::pacing::Refresh`, which also counts the refreshes
//! a late callback missed); otherwise it follows a steady 57.524 Hz clock.
//! The sound worker resamples the audio to match: the game's speed, steered
//! by the worklet's level reports ([`set_audio_level`]) and followed down
//! when the game persistently can't keep it, through `model2::pacing::Pace`.
//!
//! The sound and music boards run in a Web Worker (sound-worker.js, using
//! [`SoundEngine`] from this same module), on another core: each frame the
//! game posts the sound link bytes it sent, with their CPU cycles, and the
//! worker renders the audio and posts it to the page's AudioWorklet.
//!
//! The geometrizer runs in another worker (geo-worker.js, [`GeoEngine`]):
//! the game posts each vblank's job (`model2::geo::Job`) and draws the
//! newest polygon stream the worker sends back ([`set_geo_frame`]).
//!
//! Smooth motion ([`set_smooth`], `model2::smooth`): on a display faster
//! than the game, refreshes between game frames show the 3D geometry part
//! way from the frame before to the newest. On the page's thread each
//! refresh draws its own in-between; the worker draws a few for each frame
//! (one per refresh the frame lasts) and the page picks the nearest.

#![cfg(target_arch = "wasm32")]

use model2::enhance::{Found, FoundMaterial, Rgba, TexInfo, TexturePack, TileInfo};
use model2::pacing::{FPS, MAX_CATCH_UP};
use model2::video;
use romset::bindings::{Bindings, Capture, Slot, Value};
use romset::{PadAxis, PadButton};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, KeyboardEvent};

/// Audio the worklet should hold (seconds) unless the page says otherwise
/// ([`set_audio_buffer`]): the rate control (`model2::pacing::Pace`) steers
/// towards it.
const AUDIO_TARGET: f64 = 0.07;
/// Following the display, frames a held-back game may run late (at most
/// `MAX_CATCH_UP` in one animation frame): one the geometry worker held
/// back, or the refreshes a stall of up to 100 ms missed, run with the
/// next ones rather than being lost.
const MAX_OWED: u64 = 6;
/// Most frames the geometry worker may have in hand: the game waits for it
/// beyond that (runs slower) rather than letting the polygons fall ever
/// further behind the rest of the picture and the inputs.
const GEO_QUEUE: u32 = 2;
/// Audio level reports older than this (ms) are stale: pace by time.
const AUDIO_STALE_MS: f64 = 250.0;

struct App {
    machine: model2::Machine,
    renderer: gpu::Renderer,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    view: video::View,
    keys: HashSet<String>,
    /// The player's bindings ([`set_bindings`]), and the manifest's, which
    /// the touch controls always go through (they're laid out from them).
    bindings: Bindings,
    defaults: Bindings,
    /// On-screen controls ([`set_touch`]): steering (-1 to 1), the right and
    /// left triggers (0 to 1) and held pad buttons (bits by index).
    touch: (f64, f64, f64, u32),
    /// Light guns as the page reports them ([`set_gun`]): None until a
    /// pointer (mouse, touch, or any other source) has aimed that player.
    guns: [Option<model2::input::GunState>; 2],
    /// A gun the gamepad's sticks aim: player 1's until the page aims it,
    /// then player 2's.
    stick: model2::input::StickCursor,
    /// The pad's sticks as 8-way levers, with their memory.
    stick_dirs: romset::StickDirs,
    stick_player: Option<usize>,
    /// Real time (ms) that emulated frame 0 corresponds to.
    epoch: f64,
    /// FPS counter: window start (ms), frames emulated and drawn since.
    counter: (f64, u32, u32),
    /// Host time (ms) in this window: emulating, preparing frames on the
    /// CPU, and encoding and submitting them to the GPU.
    spent: [f64; 3],
    /// Called with the sound link bytes sent since the last call: (cycles
    /// Float64Array, bytes Uint8Array, render-until cycle, the resampler's
    /// ratio). None: no sound.
    link: Option<js_sys::Function>,
    /// Audio the link calls asked for so far, in output samples.
    samples: u64,
    /// The worklet's last report: stereo frames queued, and when (ms).
    audio_level: Option<(f64, f64)>,
    /// Stereo frames sent since that report (after resampling).
    sent_since: f64,
    /// The display's refreshes, from the animation frames, and the last
    /// frame's time (ms).
    refresh: model2::pacing::Refresh,
    last_t: f64,
    redraws: u64,
    /// Animation frames this status window that showed no new game frame.
    repeats: u32,
    /// Game samples per output sample for the sound worker's resampler,
    /// and its lowest and highest this status window.
    ratio: f64,
    ratio_range: (f64, f64),
    /// The audio rate control, and the audio it holds the worklet at
    /// (seconds).
    pace: model2::pacing::Pace,
    audio_target: f64,
    /// Following the display: frames due and not yet run.
    owed: u64,
    /// Geometrizer in a worker: its newest frame (raster stream, texture
    /// point RAM), once one arrives. None with the local engine.
    geo_frame: Option<Option<(Vec<u32>, Vec<u16>)>>,
    /// Frames posted to the geometry worker and not yet back.
    geo_in_flight: Rc<Cell<u32>>,
    /// A worker frame arrived since the last draw.
    geo_new: bool,
    /// Animation frames this status window that held the game back for the
    /// geometry worker.
    geo_held: u32,
    /// Smooth motion ([`set_smooth`]): on; with the worker, the in-betweens
    /// per game frame it should draw (shared with the job poster), and the
    /// ones it sent with its newest frame (at t = k / their number; empty:
    /// the frame's own stream); when the newest frame's polygons first
    /// showed (ms); in-betweens drawn this status window, and their time
    /// (ms, page thread only).
    smooth: bool,
    smooth_steps: Rc<Cell<u32>>,
    geo_between: Vec<Vec<u32>>,
    /// Smooth shading's side channel (`model2::light::Frame`) for the
    /// worker's newest frame and each of its in-betweens, when the worker
    /// records it (its `light` flag; docs/web.md); None: shaded from the
    /// stream alone.
    geo_light: Option<Arc<model2::light::Frame>>,
    geo_between_light: Vec<Option<Arc<model2::light::Frame>>>,
    smooth_at: f64,
    smooth_drawn: (u32, f64),
    status: web_sys::Element,
    stopped: bool,
    /// Paused by the page ([`set_paused`]): no frames run or drawn.
    paused: bool,
    /// Kept alive: in a browser, dropping the instance loses the device.
    _gpu: (wgpu::Instance, wgpu::Adapter),
}

fn window() -> web_sys::Window {
    web_sys::window().expect("running in a browser window")
}

fn now() -> f64 {
    window().performance().map_or(0.0, |p| p.now())
}

fn set_status(status: &web_sys::Element, text: &str) {
    status.set_text_content(Some(text));
}

async fn fetch(url: &str) -> Result<Vec<u8>, JsValue> {
    let resp: web_sys::Response =
        wasm_bindgen_futures::JsFuture::from(window().fetch_with_str(url)).await?.dyn_into()?;
    if !resp.ok() {
        return Err(JsValue::from_str(&format!("{url}: HTTP {}", resp.status())));
    }
    let buf = wasm_bindgen_futures::JsFuture::from(resp.array_buffer()?).await?;
    Ok(js_sys::Uint8Array::new(&buf).to_vec())
}

/// The first connected gamepad, if any.
fn gamepad() -> Option<web_sys::Gamepad> {
    let pads = window().navigator().get_gamepads().ok()?;
    pads.iter().find_map(|p| p.dyn_into::<web_sys::Gamepad>().ok())
}

/// The browser's own keys besides the players' (fullscreen, the bar):
/// never bound to a game's input.
const PAGE_KEYS: &[&str] = &["KeyF", "KeyH"];

/// Keys the page won't bind: the players' own and the page's.
fn reserved() -> Vec<&'static str> {
    romset::bindings::RESERVED_KEYS.iter().chain(PAGE_KEYS).copied().collect()
}

/// Keyboard and gamepad (W3C standard pad layout), read through the
/// player's bindings.
struct Devices<'a> {
    keys: &'a HashSet<String>,
    pad: Option<web_sys::Gamepad>,
    /// The sticks as 8-way levers, read once a frame ([`Devices::new`]).
    dirs: romset::StickDirs,
}

/// The on-screen controls as a gamepad: steering (-1 to 1, the left
/// stick), the right and left triggers (0 to 1) and pad buttons (bits by
/// index). Read through the manifest's bindings, which they're laid out
/// from.
struct Touch((f64, f64, f64, u32));

impl model2::input::PlayerInput for Touch {
    fn key(&self, _: &str) -> bool {
        false
    }

    fn pad_button(&self, b: romset::PadButton) -> bool {
        self.0 .3 & 1 << b.index() != 0
    }

    fn pad_axis(&self, a: romset::PadAxis) -> f32 {
        let (steer, right, left, _) = self.0;
        match a {
            PadAxis::LeftStickX => (steer as f32).clamp(-1.0, 1.0),
            PadAxis::LeftTrigger => left as f32,
            PadAxis::RightTrigger => right as f32,
            PadAxis::Triggers => (right - left) as f32,
            _ => 0.0,
        }
    }
}

impl<'a> Devices<'a> {
    /// The devices now, moving `sticks` (the memory of the stick levers) by
    /// a frame.
    fn new(keys: &'a HashSet<String>, sticks: &mut romset::StickDirs) -> Self {
        let pad = gamepad();
        let mut d = Devices { keys, pad, dirs: *sticks };
        let raw = |i: u32| d.pad.as_ref().and_then(|p| p.axes().get(i).as_f64()).unwrap_or(0.0) as f32;
        let (l, r) = ((raw(0), raw(1)), (raw(2), raw(3)));
        d.dirs = sticks.update(l, r, 0.15);
        d
    }

    fn button(&self, i: u32) -> f32 {
        self.pad
            .as_ref()
            .and_then(|p| p.buttons().get(i).dyn_into::<web_sys::GamepadButton>().ok())
            .map_or(0.0, |b| b.value() as f32)
    }

    fn axis(&self, i: u32) -> f32 {
        let v = self.pad.as_ref().and_then(|p| p.axes().get(i).as_f64()).unwrap_or(0.0) as f32;
        if v.abs() > 0.15 {
            v.clamp(-1.0, 1.0)
        } else {
            0.0
        }
    }
}

impl model2::input::PlayerInput for Devices<'_> {
    fn key(&self, code: &str) -> bool {
        self.keys.contains(code)
    }

    fn pad_button(&self, b: romset::PadButton) -> bool {
        if b.is_stick_dir() {
            return self.dirs.held(b);
        }
        self.button(b.index()) > 0.5
    }

    fn pad_axis(&self, a: romset::PadAxis) -> f32 {
        let right_trigger = self.button(PadButton::RightTrigger.index());
        let left_trigger = self.button(PadButton::LeftTrigger.index());
        match a {
            PadAxis::None => 0.0,
            PadAxis::LeftStickX => self.axis(0),
            PadAxis::LeftStickY => self.axis(1),
            PadAxis::RightStickX => self.axis(2),
            PadAxis::RightStickY => self.axis(3),
            PadAxis::LeftTrigger => left_trigger,
            PadAxis::RightTrigger => right_trigger,
            PadAxis::Triggers => right_trigger - left_trigger,
        }
    }
}

impl App {
    /// Map the keyboard, the first gamepad and the on-screen controls onto
    /// the cabinet's: the keyboard and pad by the player's bindings (the
    /// manifest's, remapped in the settings panel), the touch controls by
    /// the manifest's.
    fn apply_inputs(&mut self) {
        let devices = Devices::new(&self.keys, &mut self.stick_dirs);
        let touch = Touch(self.touch);
        let bus = &mut self.machine.bus;
        let b = &self.bindings;
        model2::input::apply_player(bus, &[(b, &devices), (&self.defaults, &touch)]);
        if bus.controls.guns.is_empty() {
            return;
        }
        let key_reload = |p: usize| model2::input::held(&devices, b.reload(p));
        for (player, g) in self.guns.iter().enumerate() {
            if let Some(mut g) = *g {
                g.reload |= key_reload(player);
                model2::input::apply_gun(bus, player, g);
            }
        }
        // The pad aims the first gun the page doesn't.
        self.stick_player = None;
        if devices.pad.is_some() {
            if let Some(player) = (0..bus.controls.guns.len()).find(|&p| self.guns[p].is_none()) {
                let mut g = self.stick.update(&devices, b);
                g.reload |= key_reload(player);
                model2::input::apply_gun(bus, player, g);
                self.stick_player = Some(player);
            }
        }
    }

    /// A game frame's length (ms): a refresh's (two at about 120 Hz) when
    /// following the display, else the game's.
    fn period(&self) -> f64 {
        match self.per_refresh() {
            Some(k) => self.refresh.ms() / k,
            None => 1000.0 / FPS,
        }
    }

    fn draw(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => return,
        };
        let t0 = now();
        // Smooth motion: how far into the newest frame this refresh is.
        let t = ((t0 - self.smooth_at) / self.period()).max(0.0);
        let mut own = None;
        // Only on a display faster than the game.
        if self.smooth && self.geo_frame.is_none() && 1000.0 / self.refresh.ms() > FPS * 1.2 {
            let engine = self.machine.bus.geo.engine();
            if let Some(f) = engine.smooth_frame() {
                // With smooth shading's side channel on, the in-between's
                // own (shaded at the level of the frames around it).
                own = if engine.light_on()
                    && (self.renderer.light_options().is_some() || self.renderer.quality().materials)
                {
                    f.render_lit(t as f32).map(|(s, side)| {
                        self.renderer.set_light_frame(Some(Arc::new(side)));
                        (s, f)
                    })
                } else {
                    f.render(t as f32).map(|s| (s, f))
                };
                if own.is_some() {
                    self.smooth_drawn.0 += 1;
                    self.smooth_drawn.1 += now() - t0;
                }
            }
        }
        let between = (t * self.geo_between.len() as f64).round() as usize;
        let geo = match &self.geo_frame {
            // The worker's newest frame (nothing to draw before the first),
            // or the in-between it sent nearest this refresh.
            // The matching side channel goes with either.
            Some(Some((s, p))) => match self.geo_between.get(between).filter(|b| !b.is_empty()) {
                Some(b) => {
                    self.smooth_drawn.0 += 1;
                    self.renderer.set_light_frame(self.geo_between_light.get(between).cloned().flatten());
                    Some((&b[..], &p[..]))
                }
                None => {
                    self.renderer.set_light_frame(self.geo_light.clone());
                    Some((&s[..], &p[..]))
                }
            },
            Some(None) => Some((&[][..], &[][..])),
            None => own.as_ref().map(|(s, f)| (&s[..], &f.points[..])),
        };
        let target = frame.texture.create_view(&Default::default());
        self.renderer.present_bus(&mut self.machine.bus, geo, &self.view, &target, self.format, (0, 0));
        let t1 = now();
        self.renderer.queue().present(frame);
        // Preparation and submission are one call now: report them together.
        self.spent[1] += t1 - t0;
    }

    /// Post the sound link bytes sent since the last call to the sound
    /// worker, with the time to render up to. Returns the output samples
    /// that asks for.
    fn send_link(&mut self) -> u64 {
        let bus = &mut self.machine.bus;
        let log = std::mem::take(&mut bus.uart_log);
        let target = (bus.now as u128 * model2::audio::RATE as u128 / model2::bus::CPU_HZ as u128) as u64;
        let produced = target - self.samples;
        self.samples = target;
        if let Some(link) = &self.link {
            let data = log.iter().filter(|e| !e.1);
            let cycles: Vec<f64> = data.clone().map(|e| e.0 as f64).collect();
            let bytes: Vec<u8> = data.map(|e| e.2).collect();
            let args = js_sys::Array::of4(
                &js_sys::Float64Array::from(&cycles[..]),
                &js_sys::Uint8Array::from(&bytes[..]),
                &JsValue::from_f64(bus.now as f64),
                &JsValue::from_f64(self.ratio),
            );
            let _ = link.apply(&JsValue::NULL, &args);
        }
        self.sent_since += produced as f64 / self.ratio;
        produced
    }

    /// Seconds of audio the worklet holds now, estimated from its last
    /// report, or None if there's no recent one (no sound, or it stalled).
    fn audio_queued(&self, t: f64) -> Option<f64> {
        let (level, at) = self.audio_level?;
        let age = t - at;
        let rate = model2::audio::RATE as f64;
        (self.link.is_some() && age < AUDIO_STALE_MS).then(|| (level + self.sent_since) / rate - age / 1000.0)
    }

    /// The status line's texture pack part, with a pack: the last frame's
    /// textured polygons replaced, the images on the GPU, and tile
    /// patterns replaced when the pack has tile images.
    fn pack_status(&self) -> String {
        if !self.renderer.has_pack() {
            return String::new();
        }
        let s = self.renderer.pack_stats();
        let q = self.renderer.quality().tex_filter;
        let tiles =
            if s.tiles_held > 0 { format!(", {} tiles ({} held)", s.tiles, s.tiles_held) } else { String::new() };
        format!(
            " · pack: {}/{} polygons, {} images ({:.0} MB){tiles}, filter {q}",
            s.replaced,
            s.textured,
            s.held,
            s.bytes as f64 / 1048576.0
        )
    }

    /// The status line's smooth shading part, with it on: the level as
    /// drawn (`pixel` needs the native geometrizer's side channel: else
    /// drawn as vertex, and said so), and the last frame's polygons shaded
    /// of those drawn.
    fn light_status(&self) -> String {
        let Some(o) = self.renderer.light_options() else { return String::new() };
        let level = match (o.shading, self.renderer.light_effective()) {
            (gpu::Shading::Pixel, Some(gpu::Shading::Vertex)) => "pixel (as vertex)",
            (s, _) => s.name(),
        };
        let lit = self.renderer.light_stats().map_or(String::new(), |s| format!(", {}/{} polygons", s.lit, s.polys));
        format!(
            " · shading {level}{}{} {}°{lit}",
            if o.fine { " fine" } else { "" },
            if o.balance { "" } else { " unbalanced" },
            o.crease
        )
    }

    /// The status line's material maps part, with them on: what they
    /// changed in the last frame, or why they have no effect (no pack, a
    /// pack without maps, no polygon lit by the exact layer: the chip
    /// geometrizer, a build without the native one).
    fn materials_status(&self) -> String {
        let q = self.renderer.quality();
        if !q.materials {
            return String::new();
        }
        let rule = match q.materials_relief.strength() {
            Some(_) => format!("relief {}", q.materials_relief.name()),
            None => "faithful".to_string(),
        };
        if !self.renderer.has_pack() {
            return " · materials: no effect (no texture pack)".into();
        }
        if !with_pack(|s| s.maps).unwrap_or(false) {
            return " · materials: no effect (the pack has no material maps)".into();
        }
        let s = self.renderer.pack_stats();
        if s.textured > 0 && s.lamped == 0 {
            return " · materials: no effect (no polygon had the game's exact lighting: needs the native geometrizer)"
                .into();
        }
        format!(
            " · materials {rule}: {}/{} lit polygons, {} maps ({:.0} MB)",
            s.material_polys,
            s.lamped,
            s.material_held,
            s.material_bytes as f64 / 1048576.0
        )
    }

    /// The status line's audio part: the resampler's rate (100% is the
    /// game's own speed; following a 60 Hz display, 104.3%), how far it is
    /// from the speed the game is meant to run at and how much it moved in
    /// this status window (cents), and the worklet's queue against its
    /// target.
    fn audio_status(&self, t: f64) -> String {
        if self.link.is_none() || self.redraws < 2 {
            return String::new();
        }
        let cents = |r: f64| 1200.0 * r.log2();
        let off = cents(self.ratio / self.refresh.speed());
        let (lo, hi) = self.ratio_range;
        let moved = if hi >= lo { cents(hi / lo) } else { 0.0 };
        let queued = self.audio_queued(t).map_or("no report".into(), |q| format!("{:.0} ms", q * 1000.0));
        format!(
            " · audio {:.1}% ({off:+.0} ¢, moved {moved:.0} ¢), {queued} queued of {:.0}",
            self.ratio * 100.0,
            self.audio_target * 1000.0
        )
    }

    /// Game frames per refresh when following the display (1 at about
    /// 60 Hz, 1/2 at about 120 Hz), or None to follow the clock.
    fn per_refresh(&self) -> Option<f64> {
        self.refresh.per_refresh()
    }

    fn tick(&mut self, t: f64) {
        if self.stopped {
            return;
        }
        if self.paused {
            // Hold the clocks where they are, so the game resumes from this
            // frame rather than catching up on the time it was paused.
            self.last_t = t;
            self.owed = 0;
            self.epoch = t - self.machine.frames as f64 * 1000.0 / FPS;
            self.counter = (t, 0, 0);
            self.spent = [0.0; 3];
            return;
        }
        // The refreshes since the last animation frame (more than one after
        // missed ones; a gap such as a hidden tab counts one).
        let dt = t - self.last_t;
        self.last_t = t;
        let refreshes = self.refresh.tick(dt);
        self.redraws += 1;
        if self.smooth {
            // In-betweens the worker draws per game frame: one per refresh
            // the frame lasts (none at 60 Hz), at most 4.
            self.smooth_steps.set((self.period() / self.refresh.ms()).round().clamp(1.0, 4.0) as u32);
        }
        // The frames due, the speed they come at, and those of them that
        // count for the audio's pace.
        let (due, speed, mut fell_due) = match self.per_refresh() {
            Some(_) => {
                let next = self.refresh.frames(refreshes);
                // Past what may be owed, after a stall of a game that was
                // keeping up: time dropped, not the game falling behind.
                let over = (self.owed + next).saturating_sub(MAX_OWED);
                let counted = if self.owed == 0 { next - over } else { next };
                self.owed = (self.owed + next).min(MAX_OWED);
                (self.machine.frames + self.owed, self.refresh.speed(), counted as f64)
            }
            None => (((t - self.epoch) * FPS / 1000.0) as u64, 1.0, dt * FPS / 1000.0),
        };
        let mut ran = 0;
        let t0 = now();
        while self.machine.frames < due && ran < MAX_CATCH_UP {
            if self.geo_frame.is_some() && self.geo_in_flight.get() >= GEO_QUEUE {
                self.geo_held += 1;
                break;
            }
            self.apply_inputs();
            if let Err(e) = self.machine.run_slice(u64::MAX) {
                self.stopped = true;
                set_status(&self.status, &format!("stopped: {e:?}"));
                return;
            }
            self.send_link();
            ran += 1;
        }
        self.owed = self.owed.saturating_sub(ran as u64);
        self.spent[0] += now() - t0;
        if ran > 0 && self.geo_frame.is_none() {
            // On the page's thread the new frame's polygons are ready now.
            self.smooth_at = t;
        }
        if self.per_refresh().is_none() && self.machine.frames < due && dt > 2000.0 / FPS {
            // On the clock, frames left due after a long wait are dropped
            // time too.
            fell_due = (fell_due - (due - self.machine.frames) as f64).max(ran as f64);
        }
        // The audio rate for the next frames: the game's speed, steered by
        // the worklet's queue, and slowed with the game once it
        // persistently can't keep up (held back by the geometry worker, or
        // its own cost) instead of running dry.
        if self.redraws > 1 {
            let queued = self.audio_queued(t);
            self.ratio = self.pace.update(dt / 1000.0, speed, ran as f64, fell_due, queued, self.audio_target);
            self.ratio_range = (self.ratio_range.0.min(self.ratio), self.ratio_range.1.max(self.ratio));
        }
        if self.per_refresh().is_some() || self.machine.frames < due {
            // Following the display, or too far behind: keep the clock current.
            self.epoch = t - self.machine.frames as f64 * 1000.0 / FPS;
        }
        if ran == 0 {
            self.repeats += 1;
        }
        // A new frame, or the worker's polygons for one already shown (or,
        // with smooth motion on a display faster than the game, every
        // refresh).
        let smoothing = self.smooth && 1000.0 / self.refresh.ms() > FPS * 1.2;
        if ran > 0 || std::mem::take(&mut self.geo_new) || smoothing {
            self.draw();
            self.counter.2 += 1;
        }
        self.counter.1 += ran;
        if t - self.counter.0 >= 1000.0 {
            let secs = (t - self.counter.0) / 1000.0;
            let speed = self.counter.1 as f64 / secs / FPS * 100.0;
            let (emulated, drawn) = (self.counter.1.max(1) as f64, self.counter.2.max(1) as f64);
            set_status(
                &self.status,
                &format!(
                    "{} · {:.0}% speed{} · {:.0} fps drawn ({} repeats) · {}x{} · i960 {} · ms per frame: emulate {:.1}, draw {:.1}{}{}{}{}{}",
                    match self.per_refresh() {
                        Some(_) => format!("synced to {:.0} Hz", 1000.0 / self.refresh.ms()),
                        None => "exact speed".into(),
                    },
                    speed,
                    if self.geo_held > 0 { format!(" (waiting for geometry: {} times)", self.geo_held) } else { String::new() },
                    self.counter.2 as f64 / secs,
                    self.repeats,
                    self.view.width,
                    self.view.height,
                    if self.machine.is_native() { "recompiled" } else { "interpreted" },
                    self.spent[0] / emulated,
                    self.spent[1] / drawn,
                    match self.smooth_drawn {
                        (0, _) => String::new(),
                        (n, ms) if self.geo_frame.is_none() => format!(" · {n} in-betweens, {:.1} ms each", ms / n as f64),
                        (n, _) => format!(" · {n} in-betweens"),
                    },
                    self.pack_status(),
                    self.light_status(),
                    self.materials_status(),
                    self.audio_status(t),
                ),
            );
            self.ratio_range = (f64::MAX, 0.0);
            self.smooth_drawn = (0, 0.0);
            self.counter = (t, 0, 0);
            self.spent = [0.0; 3];
            self.repeats = 0;
            self.geo_held = 0;
        }
    }
}

thread_local! {
    /// The running app, for calls from the page after `start`.
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
    /// The player's remapped controls ([`set_bindings`]), for the game
    /// `start` starts.
    static OVERRIDES: RefCell<String> = const { RefCell::new(String::new()) };
    /// Smooth motion for the game `start` starts ([`set_smooth`]).
    static SMOOTH: Cell<bool> = const { Cell::new(false) };
    /// Anti-aliasing samples, the depth buffer, the filtering of texture
    /// pack images, and smooth shading with its crease angle
    /// ([`set_quality`]).
    static QUALITY: Cell<Quality> = const { Cell::new(Quality::OFF) };
    /// The texture pack the page is filling ([`set_pack`]), for the game
    /// `start` starts.
    static PACK: RefCell<Option<Arc<Mutex<PackState>>>> = const { RefCell::new(None) };
    /// The audio the game `start` starts holds queued ([`set_audio_buffer`]).
    static AUDIO_BUFFER: Cell<f64> = const { Cell::new(AUDIO_TARGET) };
}

/// The audio to hold queued in the worklet (seconds), for the game `start`
/// starts: more rides out a phone's uneven animation frames without
/// bending the pitch, at the cost of latency (docs/web.md, "Audio rate").
/// The worklet's priming and most should be set to suit (audio-worklet.js).
#[wasm_bindgen]
pub fn set_audio_buffer(seconds: f64) {
    if seconds.is_finite() {
        AUDIO_BUFFER.with(|b| b.set(seconds.clamp(0.02, 1.0)));
    }
}

/// Smooth motion (`model2::smooth`, docs/web.md): on a display faster than
/// the game's 57.5 Hz, draw the refreshes between game frames with the 3D
/// geometry interpolated between the last two (one game frame of extra
/// latency). Native geometrizer only: off with the chip. Call before
/// `start`; the geometry worker needs `smooth: true` in its init message
/// too (geo-worker.js).
#[wasm_bindgen]
pub fn set_smooth(on: bool) {
    SMOOTH.with(|s| s.set(on));
}

/// The page's choice of image quality ([`set_quality`]).
#[derive(Clone, Copy)]
struct Quality {
    aa: u32,
    depth: bool,
    tex_filter: gpu::TexFilter,
    shading: gpu::Shading,
    crease: u8,
    shading_fine: bool,
    shading_balance: bool,
    materials: bool,
    materials_relief: gpu::Relief,
    bloom: u8,
}

impl Quality {
    const OFF: Quality = Quality {
        aa: 0,
        depth: false,
        tex_filter: gpu::TexFilter::Off,
        shading: gpu::Shading::Flat,
        crease: gpu::CREASE_AUTO,
        shading_fine: false,
        shading_balance: true,
        materials: false,
        materials_relief: gpu::Relief::Diffuse { tenths: gpu::Relief::DEFAULT_TENTHS },
        bloom: 0,
    };

    fn gpu(self) -> gpu::Quality {
        let Quality {
            aa,
            depth,
            tex_filter,
            shading,
            crease,
            shading_fine,
            shading_balance,
            materials,
            materials_relief,
            bloom,
        } = self;
        gpu::Quality {
            msaa: aa,
            depth,
            tex_filter,
            shading,
            crease,
            shading_fine,
            shading_balance,
            materials,
            materials_relief,
            bloom,
            ..Default::default()
        }
    }
}

/// Image quality of the 3D layer (docs/enhancements.md): `aa` samples a
/// pixel (0 for none; WebGPU has 4), the depth buffer, `tex_filter`, the
/// filtering of texture pack images (`off`, `mip`, `2`, `4`, `8`, `16`:
/// `gpu::TexFilter::parse`; anything else is off), and smooth shading
/// (`model2::light`): `shading` `flat` (or left out: the original),
/// `vertex` or `pixel` (per pixel needs the native geometrizer's side
/// channel; with the chip it draws as `vertex`), with faces meeting at
/// more than `crease` degrees (0 to 180) kept hard; left out (or 255) is
/// the game's own angle (`romset::Shading`), and `fine` true for finer
/// levels (`gpu::Quality::shading_fine`: lit pixels blend two colour
/// columns instead of the game's 64 levels; left out is off), and
/// `balance` false to drop the brightness balance
/// (`gpu::Quality::shading_balance`: one offset per smooth surface keeps
/// its mean the game's; left out is on), and `materials` true for the
/// texture pack's material maps (`gpu::Quality::materials`: normal maps
/// change the brightness of surfaces the game lit with the exact lighting
/// layer, the native geometrizer's side channel; left out is off) with
/// `relief` their rule (`gpu::Relief::parse`: a strength from 0.1 to 8, or
/// `off` for the faithful rule; left out or unreadable is 2). The shading
/// is done on this page, from the polygon stream and the side channel the
/// geometrizer recorded (`geo=local`: turned on here; with the worker the
/// page asks it for it, geo-worker.js `light`), and `bloom` the strength
/// of bloom in percent (`gpu::Quality::bloom`: 0 or left out is off, 100 the
/// strongest; the 3D layer's bright parts glow, the HUD stays sharp). Before `start`, for the
/// game it starts; after, from the next frame.
#[wasm_bindgen]
pub fn set_quality(
    aa: u32,
    depth: bool,
    tex_filter: &str,
    shading: Option<String>,
    crease: Option<u32>,
    fine: Option<bool>,
    balance: Option<bool>,
    materials: Option<bool>,
    relief: Option<String>,
    bloom: Option<u32>,
) {
    let q = Quality {
        aa,
        depth,
        tex_filter: gpu::TexFilter::parse(tex_filter).unwrap_or_default(),
        shading: shading.as_deref().and_then(gpu::Shading::parse).unwrap_or_default(),
        crease: match crease {
            Some(n) if n <= 180 => n as u8,
            _ => gpu::CREASE_AUTO,
        },
        shading_fine: fine.unwrap_or(false),
        shading_balance: balance.unwrap_or(true),
        materials: materials.unwrap_or(false),
        materials_relief: relief.as_deref().and_then(gpu::Relief::parse).unwrap_or_default(),
        bloom: bloom.unwrap_or(0).min(100) as u8,
    };
    QUALITY.with(|c| c.set(q));
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let mut app = app.borrow_mut();
            app.renderer.set_quality(q.gpu());
            // The local engine's side channel (the worker's is the page's
            // `set` message).
            if app.geo_frame.is_none() {
                app.machine.bus.geo.set_light(q.gpu().side_channel_on());
            }
        }
    });
}

/// The crease angle (degrees) smooth shading uses for `game` by default
/// (`romset::Shading::crease`), for the settings panel's "Auto".
#[wasm_bindgen]
pub fn game_crease(game: &str) -> u32 {
    romset::games::find(game).map_or(romset::Shading::DEFAULT.crease, |g| g.shading.crease) as u32
}

/// What the settings panel's Enhanced preset turns on for `game`
/// (`romset::Recommended`; the browser has no temporal anti-aliasing or
/// run-ahead): JSON `{"depth": bool, "shading": bool}`.
#[wasm_bindgen]
pub fn game_recommended(game: &str) -> String {
    let rec = romset::games::find(game).map_or(romset::Recommended::DEFAULT, |g| g.recommended);
    format!("{{\"depth\":{},\"shading\":{}}}", rec.depth, rec.shading)
}

/// Time smooth shading's grouping pass (`model2::light::shade`) on this
/// page: `runs` passes over the newest polygon stream with the running
/// game's options (for a test driver; docs/web.md, "Smooth shading").
/// Returns `[median ms, worst ms, polygons, lit polygons]`, or empty with
/// shading off or before a frame.
#[wasm_bindgen]
pub fn shading_cost(runs: u32) -> Vec<f64> {
    APP.with(|a| {
        let app = a.borrow();
        let Some(app) = app.as_ref() else { return Vec::new() };
        let mut app = app.borrow_mut();
        let app = &mut *app;
        let Some(opts) = app.renderer.light_options() else { return Vec::new() };
        let stream = match &app.geo_frame {
            Some(Some((s, _))) => s.clone(),
            Some(None) => return Vec::new(),
            None => {
                app.machine.bus.geo.sync();
                app.machine.bus.geo.engine().stream.clone()
            }
        };
        let mut times = Vec::new();
        let mut stats = model2::light::Stats::default();
        for _ in 0..runs.max(1) {
            let t = now();
            stats = model2::light::shade(&stream, &opts).stats;
            times.push(now() - t);
        }
        times.sort_by(f64::total_cmp);
        vec![times[times.len() / 2], times[times.len() - 1], stats.polys as f64, stats.lit as f64]
    })
}

/// The folders under `/texpack/` to look for `game`'s pack in, in order
/// (`romset::pack_sets`): its own, then for a clone its parent's. The page
/// uses the first whose `index.txt` it can fetch.
#[wasm_bindgen]
pub fn pack_sets(game: &str) -> Vec<String> {
    match romset::games::find(game) {
        Ok(g) => romset::pack_sets(g).map(str::to_string).collect(),
        Err(_) => vec![game.to_string()],
    }
}

/// A texture pack served beside the page (docs/web.md, "Texture packs"):
/// the names its `index.txt` lists, and the images the page has fetched
/// and decoded so far. The renderer asks for a texture by name; one in the
/// index is fetched the first time it is drawn (`Pending` until then),
/// anything else is `Missing`, so the page never asks the server for a
/// file that isn't there.
#[derive(Default)]
struct PackState {
    index: HashSet<String>,
    /// Names asked for and not yet handed to the renderer.
    asked: HashSet<String>,
    /// Names for the page to fetch ([`pack_wanted`]).
    wanted: Vec<String>,
    /// Decoded images waiting for the renderer ([`pack_image`]).
    ready: HashMap<String, Arc<Rgba>>,
    /// Names that couldn't be fetched or decoded ([`pack_missing`]).
    failed: HashSet<String>,
    /// The index names tile images (`tile_<key>`).
    tiles: bool,
    /// The pack's manifest, and whether the index names material maps it
    /// reads (`Manifest::reads_maps`).
    manifest: Arc<model2::pack::Manifest>,
    maps: bool,
}

/// The pack, and its manifest (also in the state) for `TexturePack::manifest`.
struct WebPack(Arc<Mutex<PackState>>, Arc<model2::pack::Manifest>);

impl WebPack {
    fn new(state: Arc<Mutex<PackState>>) -> WebPack {
        let manifest = state.lock().unwrap().manifest.clone();
        WebPack(state, manifest)
    }

    /// The image named `name` (a file stem): see `TexturePack::find`.
    fn get(&mut self, name: String) -> Found {
        let mut s = self.0.lock().unwrap();
        if let Some(img) = s.ready.remove(&name) {
            // The renderer keeps it now; if it drops it (over its memory
            // budget), asking again fetches it again (from the HTTP cache).
            s.asked.remove(&name);
            return Found::Image(img);
        }
        if !s.index.contains(&name) || s.failed.contains(&name) {
            return Found::Missing;
        }
        if s.asked.insert(name.clone()) {
            s.wanted.push(name);
        }
        Found::Pending
    }
}

impl TexturePack for WebPack {
    fn find(&mut self, info: &TexInfo) -> Found {
        self.get(info.name())
    }

    fn has_tiles(&self) -> bool {
        self.0.lock().unwrap().tiles
    }

    fn find_tile(&mut self, info: &TileInfo) -> Found {
        self.get(info.name())
    }

    fn has_materials(&self) -> bool {
        self.0.lock().unwrap().maps
    }

    /// Each map file is fetched like an image (`pack_wanted`); once all of a
    /// material's have arrived (or failed), they are assembled here and
    /// handed over.
    fn material(&mut self, info: &TexInfo) -> FoundMaterial {
        let mut s = self.0.lock().unwrap();
        if !s.maps {
            return FoundMaterial::Missing;
        }
        model2::enhance::gather_material(&mut *s, info, &self.1)
    }

    fn manifest(&self) -> &model2::pack::Manifest {
        &self.1
    }
}

/// Map files come as the page fetches them: in the index and not failed,
/// fetched once asked for (`pack_wanted`), handed over by `pack_image`.
impl model2::enhance::MapSource for PackState {
    fn has(&self, stem: &str) -> bool {
        self.index.contains(stem) && !self.failed.contains(stem)
    }

    fn arrived(&self, stem: &str) -> bool {
        self.ready.contains_key(stem)
    }

    fn ask(&mut self, stem: &str) {
        if self.asked.insert(stem.to_string()) {
            self.wanted.push(stem.to_string());
        }
    }

    fn take(&mut self, stem: &str) -> Option<Arc<Rgba>> {
        self.asked.remove(stem);
        self.ready.remove(stem)
    }
}

fn with_pack<T>(f: impl FnOnce(&mut PackState) -> T) -> Option<T> {
    PACK.with(|p| p.borrow().as_ref().map(|p| f(&mut p.lock().unwrap())))
}

/// Use a texture pack whose `index.txt` is `index` (one file name a line,
/// `.png` optional; blank lines and `#` comments ignored) and whose
/// manifest `pack.txt` is `manifest` (`undefined`: it has none), or no pack
/// (`index` `undefined`, the default: the original textures). Before
/// `start`, for the game it starts; after, from the next frame. Returns the
/// number of images (textures and tiles) in the index; throws, with no pack
/// set, when the manifest says this build can't show the pack
/// (docs/texture-packs.md, "Pack format"). The page then fetches what
/// [`pack_wanted`] names and hands it over with [`pack_image`] or
/// [`pack_missing`].
#[wasm_bindgen]
pub fn set_pack(index: Option<String>, manifest: Option<String>) -> Result<u32, JsValue> {
    use model2::pack::{parse_name, FileKind, Manifest};
    // A pack this build can't show: none, and the page says why.
    let (manifest, refused) =
        match index.is_some().then(|| Manifest::accept_for(manifest.as_deref(), &model2::pack::SHOWN_WEB)) {
            Some(Ok(m)) => {
                if let Some(n) = m.notice() {
                    web_sys::console::log_1(&format!("texture pack: {n}").into());
                }
                (m, None)
            }
            Some(Err(e)) => (Manifest::NONE, Some(e)),
            None => (Manifest::NONE, None),
        };
    let state = index.filter(|_| refused.is_none()).map(|text| {
        let index: HashSet<String> = text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| l.strip_suffix(".png").unwrap_or(l).to_string())
            .collect();
        let tiles = index.iter().any(|n| matches!(parse_name(n), Ok(FileKind::Tile { .. })));
        let maps = manifest.reads_maps() && index.iter().any(|n| matches!(parse_name(n), Ok(FileKind::Map { .. })));
        let manifest = Arc::new(manifest);
        Arc::new(Mutex::new(PackState { index, tiles, manifest, maps, ..Default::default() }))
    });
    let names = state.as_ref().map_or(0, |s| {
        let image = |n: &&String| matches!(parse_name(n), Ok(FileKind::Image { .. } | FileKind::Tile { .. }));
        s.lock().unwrap().index.iter().filter(image).count() as u32
    });
    PACK.with(|p| *p.borrow_mut() = state.clone());
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            app.borrow_mut().renderer.set_pack(state.map(|s| Box::new(WebPack::new(s)) as Box<dyn TexturePack + Send>));
        }
    });
    match refused {
        Some(e) => Err(JsValue::from_str(&e)),
        None => Ok(names),
    }
}

/// The pack images drawn since the last call that the page should fetch
/// (file stems: `<name>.png` in the pack's folder).
#[wasm_bindgen]
pub fn pack_wanted() -> Vec<String> {
    with_pack(|s| std::mem::take(&mut s.wanted)).unwrap_or_default()
}

/// A pack image, decoded (straight alpha RGBA, `width` x `height`): the
/// renderer uploads it the next time it draws its texture.
#[wasm_bindgen]
pub fn pack_image(name: String, width: u32, height: u32, rgba: Vec<u8>) {
    let (w, h) = (width as usize, height as usize);
    if rgba.len() != w * h * 4 {
        return pack_missing(name);
    }
    with_pack(|s| {
        if s.asked.contains(&name) {
            s.ready.insert(name, Arc::new(Rgba { width: w, height: h, data: rgba }));
        }
    });
}

/// A pack image that couldn't be fetched or decoded: its texture keeps its
/// original from now on.
#[wasm_bindgen]
pub fn pack_missing(name: String) {
    with_pack(|s| {
        s.asked.remove(&name);
        s.failed.insert(name);
    });
}

/// A decoded PNG file ([`decode_png`]).
#[wasm_bindgen]
pub struct Png {
    pub width: u32,
    pub height: u32,
    rgba: Vec<u8>,
}

#[wasm_bindgen]
impl Png {
    /// Its pixels, 8-bit RGBA with straight alpha, rows top to bottom.
    pub fn rgba(self) -> Vec<u8> {
        self.rgba
    }
}

/// Decode a PNG file with the repo's own decoder (`png::decode`, as the
/// native player and `remodel` read packs), for the pack worker
/// (pack-worker.js).
#[wasm_bindgen]
pub fn decode_png(file: &[u8]) -> Result<Png, JsValue> {
    let img = png::decode(file).map_err(|e| JsValue::from_str(&e))?;
    Ok(Png { width: img.width as u32, height: img.height as u32, rgba: img.rgba })
}

/// Use the player's remapped controls: `overrides` as the settings panel
/// keeps them (`romset::bindings`; only what differs from the manifest).
/// Before `start`, for the game it starts; after, from the next frame.
#[wasm_bindgen]
pub fn set_bindings(overrides: &str) {
    OVERRIDES.with(|o| *o.borrow_mut() = overrides.to_string());
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let mut app = app.borrow_mut();
            app.bindings = Bindings::with_overrides(app.defaults.controls(), overrides, &reserved());
        }
    });
}

/// On-screen controls, which act as a gamepad: `steer` (the left stick,
/// -1 left to 1 right), the right and left triggers (0 to 1), and the pad
/// buttons held, as bits by W3C standard index ([`touch_layout`] says which
/// to offer). Merged with the keyboard and gamepad.
#[wasm_bindgen]
pub fn set_touch(steer: f64, right: f64, left: f64, buttons: u32) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            app.borrow_mut().touch = (steer, right, left, buttons);
        }
    });
}

/// Pause or resume the game (the settings panel pauses it). While paused
/// no frames run, nothing is drawn (the canvas keeps the last picture) and
/// nothing goes to the workers; resuming continues from the same frame.
#[wasm_bindgen]
pub fn set_paused(paused: bool) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let mut app = app.borrow_mut();
            app.paused = paused;
            if paused {
                set_status(&app.status, "paused");
            }
        }
    });
}

/// HUD placement in wide views: at the screen's edges, or centred as on
/// the original 4:3 screen. Takes effect from the next frame drawn.
#[wasm_bindgen]
pub fn set_hud(edges: bool) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            app.borrow_mut().view.hud = if edges { video::Hud::Edges } else { video::Hud::Center };
        }
    });
}

/// Aim light gun `player` (0 for player 1) at `(x, y)` in the game's view
/// (fractions, 0 to 1 across and down the 496x384 picture), or off the
/// screen when `onscreen` is false; `buttons`: bit 0 the trigger, bit 1
/// reload. Every pointer source on the page (mouse, touch, and any other,
/// such as a tracked hand) reports through here; [`view_point`] turns
/// canvas pixels into view fractions. Takes effect from the next frame and
/// holds until the next call.
#[wasm_bindgen]
pub fn set_gun(player: u32, x: f64, y: f64, onscreen: bool, buttons: u32) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            if let Some(g) = app.borrow_mut().guns.get_mut(player as usize) {
                *g = Some(model2::input::GunState {
                    at: onscreen.then_some((x as f32, y as f32)),
                    trigger: buttons & 1 != 0,
                    reload: buttons & 2 != 0,
                });
            }
        }
    });
}

/// Where canvas pixel `(x, y)` (in the canvas's own pixels, not CSS
/// pixels) falls in the game's view: `[x, y]` as fractions for
/// [`set_gun`], or empty off the picture (including the sides a wide view
/// adds).
#[wasm_bindgen]
pub fn view_point(x: f64, y: f64) -> Vec<f64> {
    APP.with(|a| {
        let app = a.borrow();
        let Some(app) = app.as_ref() else { return Vec::new() };
        let view = app.borrow().view;
        view.to_view_fraction(x as f32, y as f32).map_or(Vec::new(), |(fx, fy)| vec![fx as f64, fy as f64])
    })
}

/// The gamepad's gun cursor, for the page to draw: `[x, y]` in canvas
/// pixels, or empty when no pad aims a gun.
#[wasm_bindgen]
pub fn stick_cursor() -> Vec<f64> {
    APP.with(|a| {
        let app = a.borrow();
        let Some(app) = app.as_ref() else { return Vec::new() };
        let app = app.borrow();
        if app.stick_player.is_none() {
            return Vec::new();
        }
        let (x, y) = app.stick.at;
        let (ox, oy) = app.view.to_output(x * video::WIDTH as f32, y * video::HEIGHT as f32);
        vec![ox as f64, oy as f64]
    })
}

/// The on-screen controls for `game`, from its pad bindings, as JSON:
/// `steer` (an analog follows the left stick), `top` (coin, start and the
/// shoulder buttons: small, along the top) and `actions` (the face buttons
/// and the triggers: round, at the right), each `{label, bit}` with `bit`
/// the pad button's index, or `{label, trigger: "right" | "left"}`. Each of
/// the game's inputs appears once, at its first binding in that order.
/// Gun games (`gun: true`) get no `actions` (taps on the picture shoot)
/// and a reload pill, `{label, gun: "reload"}`, in `top`.
#[wasm_bindgen]
pub fn touch_layout(game: &str) -> String {
    let Ok(game) = romset::games::find(game) else { return "{}".into() };
    let c = &game.controls;
    let mut shown = HashSet::new();
    let mut place = |pads: &[PadButton]| -> Vec<String> {
        let mut out = Vec::new();
        for &p in pads {
            if let Some(b) = c.buttons.iter().find(|b| b.pad.contains(&p) && !shown.contains(b.name)) {
                shown.insert(b.name);
                out.push(format!("{{\"label\": {}, \"bit\": {}}}", json_str(&b.label()), p.index()));
            }
        }
        out
    };
    let mut actions = place(&[PadButton::North, PadButton::East, PadButton::West]);
    let south = place(&[PadButton::South]);
    let top = place(&[PadButton::Select, PadButton::Start, PadButton::LeftShoulder, PadButton::RightShoulder]);
    for (side, axis, sign) in [("left", PadAxis::LeftTrigger, " −"), ("right", PadAxis::RightTrigger, " +")] {
        let label = c.analogs.iter().find_map(|a| match a.axis {
            x if x == axis => Some(a.label()),
            PadAxis::Triggers => Some(a.label() + sign),
            _ => None,
        });
        if let Some(l) = label {
            actions.push(format!("{{\"label\": {}, \"trigger\": \"{side}\"}}", json_str(&l)));
        }
    }
    actions.extend(south);
    let steer = c.analogs.iter().any(|a| a.axis == PadAxis::LeftStickX);
    // Gun games: taps on the picture shoot, so the right side stays clear;
    // a reload pill joins coin and start.
    let gun = !c.guns.is_empty();
    let mut top = top;
    if gun {
        actions.clear();
        let label = c.guns[0].reload_label(c);
        top.push(format!("{{\"label\": {}, \"gun\": \"reload\"}}", json_str(&label)));
    }
    format!(
        "{{\"steer\": {steer}, \"gun\": {gun}, \"top\": [{}], \"actions\": [{}]}}",
        top.join(", "),
        actions.join(", ")
    )
}

/// `s` as a JSON string.
fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 => out += &format!("\\u{:04x}", c as u32),
            c => out.push(c),
        }
    }
    out + "\""
}

/// The geometrizer worker finished a frame: its raster stream and texture
/// point RAM, or neither for a frame it skipped posting (overtaken by
/// newer ones while it caught up). Called once per frame job. `between`:
/// with smooth motion, the frame's in-betweens ([`GeoEngine::between`]).
/// `light`, `between_light`: with smooth shading's side channel on in the
/// worker, the frame's and each in-between's (`model2::light::Frame`
/// words, [`GeoEngine::light`], [`GeoEngine::between_light`]; empty or
/// left out: none).
#[wasm_bindgen]
pub fn set_geo_frame(
    stream: Option<Vec<u32>>,
    points: Option<Vec<u16>>,
    between: Option<js_sys::Array>,
    light: Option<Vec<u32>>,
    between_light: Option<js_sys::Array>,
) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let mut app = app.borrow_mut();
            app.geo_in_flight.set(app.geo_in_flight.get().saturating_sub(1));
            if let (Some(stream), Some(points)) = (stream, points) {
                app.geo_frame = Some(Some((stream, points)));
                app.geo_new = true;
                app.geo_between =
                    between.map_or(Vec::new(), |b| b.iter().map(|s| js_sys::Uint32Array::new(&s).to_vec()).collect());
                let side = |w: &[u32]| model2::light::Frame::from_words(w).map(Arc::new);
                app.geo_light = light.as_deref().and_then(side);
                app.geo_between_light = between_light
                    .map_or(Vec::new(), |b| b.iter().map(|w| side(&js_sys::Uint32Array::new(&w).to_vec())).collect());
                app.smooth_at = now();
            }
        }
    });
}

/// The geometrizer on its own, for a Web Worker: it runs the jobs the
/// game's geometrizer front end posts (boot, then one per frame).
#[wasm_bindgen]
pub struct GeoEngine {
    engine: model2::geo::Engine,
    /// The side channels of the in-betweens the last `between` drew.
    between_light: Vec<Vec<u32>>,
}

#[wasm_bindgen]
impl GeoEngine {
    /// `chips`: Uint8Arrays of the files [`rom_files`] lists for `game`,
    /// in order.
    #[wasm_bindgen(constructor)]
    pub fn new(game: &str, chips: js_sys::Array) -> Result<GeoEngine, JsValue> {
        // This worker is a module instance of its own.
        recomp::register();
        console_error_panic_hook::set_once();
        let game = romset::games::find(game).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let chips = chips.iter().map(|c| js_sys::Uint8Array::new(&c).to_vec()).collect();
        let roms = romset::RomSet::from_chips(game, chips, false).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let core = model2::geo::Core::of(game.board);
        Ok(GeoEngine {
            engine: model2::geo::Engine::new(model2::bus::polygon_rom(&roms), core),
            between_light: Vec::new(),
        })
    }

    /// Run frames on the native geometrizer when it implements the
    /// uploaded program (docs/hw/geometrizer-2b.md, geometrizer-2c.md),
    /// else the chip (the 2B SHARC, the 2C TGP). Call before the boot job.
    pub fn set_native(&mut self, on: bool) {
        self.engine.set_mode(if on { model2::geo::GeoMode::Native } else { model2::geo::GeoMode::Chip });
    }

    /// Smooth motion: keep what in-betweens of each frame need
    /// (`model2::smooth`; native geometrizer only).
    pub fn set_smooth(&mut self, on: bool) {
        self.engine.set_smooth(on);
    }

    /// Smooth shading's side channel (`model2::light`): record what the
    /// geometrizer knew about each frame's lighting, for the page to shade
    /// with ([`GeoEngine::light`]). From the next frame.
    pub fn set_light(&mut self, on: bool) {
        self.engine.set_light(on);
    }

    /// The last frame's side channel as words (`model2::light::Frame::
    /// to_words`); empty when it is off or the frame had none.
    pub fn light(&self) -> Vec<u32> {
        self.engine.light_frame().map_or(Vec::new(), |f| f.to_words())
    }

    /// The last frame's in-betweens at t = k / `steps` for k from 0 to
    /// `steps` - 1, from the frame before towards it: Uint32Arrays, empty
    /// where the frame's own stream shows (a cut, nothing to blend). Empty
    /// without smooth motion, or for a frame the native geometrizer didn't
    /// run. With the side channel on, each one's is kept for
    /// [`GeoEngine::between_light`].
    pub fn between(&mut self, steps: u32) -> js_sys::Array {
        let out = js_sys::Array::new();
        self.between_light.clear();
        let lit = self.engine.light_on();
        if let Some(f) = self.engine.smooth_frame() {
            for k in 0..steps {
                let t = k as f32 / steps as f32;
                let (s, l) = if lit {
                    f.render_lit(t).map_or((Vec::new(), Vec::new()), |(s, side)| (s, side.to_words()))
                } else {
                    (f.render(t).unwrap_or_default(), Vec::new())
                };
                out.push(&js_sys::Uint32Array::from(&s[..]));
                self.between_light.push(l);
            }
        }
        out
    }

    /// The side channels of the in-betweens the last [`GeoEngine::between`]
    /// drew, in order: Uint32Arrays (`model2::light::Frame::to_words`),
    /// empty where there is none.
    pub fn between_light(&self) -> js_sys::Array {
        self.between_light.iter().map(|w| js_sys::Uint32Array::from(&w[..])).collect()
    }

    /// Boot the uploaded program: 64-bit words as (low, high) u32 pairs.
    pub fn boot(&mut self, words: &[u32]) {
        let program: Vec<u64> = words.chunks_exact(2).map(|w| w[0] as u64 | (w[1] as u64) << 32).collect();
        self.engine.run_job(model2::geo::Job::Boot(program));
    }

    /// Run one frame's list from a copy of buffer RAM.
    pub fn frame(&mut self, buffer: Vec<u8>, booted: bool, read_ptr: u32, jumps: Vec<u32>) {
        self.engine.run_job(model2::geo::Job::Frame { buffer, booted, read_ptr, jumps });
    }

    /// The last frame's raster stream.
    pub fn stream(&self) -> Vec<u32> {
        self.engine.stream.clone()
    }

    /// Texture point RAM after the last frame.
    pub fn points(&self) -> Vec<u16> {
        self.engine.texture_point_ram.clone()
    }
}

/// The ROM file names the game needs, in manifest order (for a worker that
/// fetches them itself).
/// The machine without a page (no canvas, sound or workers), for timing
/// the emulation core in any WebAssembly host, such as Node: scripted
/// inputs (`remodel` input scripts), the i960 side and the geometrizer
/// run by separate calls so a host can time each (docs/web.md, "Timing
/// without a browser").
#[wasm_bindgen]
pub struct Headless {
    machine: model2::Machine,
    script: model2::input::InputScript,
    jobs: Rc<RefCell<Vec<model2::geo::Job>>>,
    geo: model2::geo::Engine,
    /// Sound link data bytes since the last `link_bytes`: (cycle, byte).
    link: Vec<(f64, u8)>,
}

#[wasm_bindgen]
impl Headless {
    #[wasm_bindgen(constructor)]
    pub fn new(game: &str, chips: js_sys::Array, script: &str) -> Result<Headless, JsValue> {
        recomp::register();
        console_error_panic_hook::set_once();
        let game = romset::games::find(game).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let chips = chips.iter().map(|c| js_sys::Uint8Array::new(&c).to_vec()).collect();
        let roms = romset::RomSet::from_chips(game, chips, false).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let script = model2::input::InputScript::parse(script, &game.controls).map_err(|e| JsValue::from_str(&e))?;
        let mut machine = model2::Machine::new(&roms);
        let jobs = Rc::new(RefCell::new(Vec::new()));
        let sink = jobs.clone();
        machine.bus.geo.set_remote(Box::new(move |job| sink.borrow_mut().push(job)));
        let geo = model2::geo::Engine::new(model2::bus::polygon_rom(&roms), model2::geo::Core::of(game.board));
        script.apply(0, &mut machine.bus);
        Ok(Headless { machine, script, jobs, geo, link: Vec::new() })
    }

    /// Start from a save file (`remodel run --save-state`), sound boards
    /// left out. The geometrizer starts from its state in the file.
    pub fn load(&mut self, data: &[u8]) -> Result<(), JsValue> {
        model2::load_session(&mut self.machine, None, data).map_err(|e| JsValue::from_str(&e))?;
        self.geo = self.machine.bus.geo.take_engine();
        self.jobs.borrow_mut().clear();
        self.script.apply(self.machine.frames, &mut self.machine.bus);
        Ok(())
    }

    /// Run the i960 side (CPU, copro, I/O) to the end of the frame.
    pub fn run_frame(&mut self) -> Result<(), JsValue> {
        self.machine.run_slice(u64::MAX).map_err(|e| JsValue::from_str(&format!("stopped: {e:?}")))?;
        let log = std::mem::take(&mut self.machine.bus.uart_log);
        self.link.extend(log.iter().filter(|e| !e.1).map(|e| (e.0 as f64, e.2)));
        self.script.apply(self.machine.frames, &mut self.machine.bus);
        Ok(())
    }

    /// The sound link bytes sent since the last call, as the page posts
    /// them to the sound worker: their CPU cycles (then the bus time, last).
    /// Feed `SoundEngine::feed` with these and `link_bytes` to time the
    /// sound boards on the same frames.
    pub fn link_cycles(&mut self) -> Vec<f64> {
        let mut v: Vec<f64> = self.link.iter().map(|e| e.0).collect();
        v.push(self.machine.bus.now as f64);
        v
    }

    /// The bytes for [`Headless::link_cycles`]; clears them.
    pub fn link_bytes(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.link).into_iter().map(|e| e.1).collect()
    }

    /// The geometrizer as [`GeoEngine::set_native`] says.
    pub fn set_geo_native(&mut self, on: bool) {
        self.geo.set_mode(if on { model2::geo::GeoMode::Native } else { model2::geo::GeoMode::Chip });
    }

    /// Smooth motion as [`GeoEngine::set_smooth`] says.
    pub fn set_geo_smooth(&mut self, on: bool) {
        self.geo.set_smooth(on);
    }

    /// Draw the last geometrizer frame's in-between at `t` (to time it):
    /// its words, 0 where the frame's own stream shows.
    pub fn between_len(&self, t: f32) -> u32 {
        self.geo.smooth_frame().and_then(|f| f.render(t)).map_or(0, |s| s.len() as u32)
    }

    /// Run the geometrizer jobs the frames so far posted.
    pub fn run_geo(&mut self) -> u32 {
        let jobs = std::mem::take(&mut *self.jobs.borrow_mut());
        let n = jobs.len() as u32;
        for j in jobs {
            self.geo.run_job(j);
        }
        n
    }

    pub fn frames(&self) -> f64 {
        self.machine.frames as f64
    }

    /// Words in the last raster stream (to check it's drawing).
    pub fn stream_len(&self) -> u32 {
        self.geo.stream.len() as u32
    }
}

#[wasm_bindgen]
pub fn rom_files(game: &str) -> Vec<String> {
    romset::games::find(game).map_or(Vec::new(), |g| g.files.iter().map(|f| f.name.to_string()).collect())
}

/// The game's title (for the page's).
#[wasm_bindgen]
pub fn game_title(game: &str) -> String {
    romset::games::find(game).map_or(String::new(), |g| g.title.to_string())
}

/// The game's controls with their keys and pad bindings, one per line
/// (for the page's help bar): the manifest's with the player's
/// `overrides` ([`set_bindings`]).
#[wasm_bindgen]
pub fn controls_help(game: &str, overrides: &str) -> String {
    romset::games::find(game).map_or(String::new(), |g| bindings(g, overrides).describe())
}

fn bindings(game: &'static romset::Game, overrides: &str) -> Bindings {
    Bindings::with_overrides(&game.controls, overrides, &reserved())
}

/// The game's controls for the settings panel, as JSON, with the player's
/// `overrides`: `{rows, conflicts}`. Each row is a bindable slot
/// (`romset::bindings`): `{slot, label, input (the control's own label),
/// accepts: "inputs" | "pad" | "axis", items: [{text, token}] (keys and
/// pad buttons, Xbox names) or axis: {text, invert}, default (as the
/// manifest has it), unbound (the manifest binds nothing and nor does the
/// player), fixed (what else drives it and can't be remapped: the mouse's
/// buttons on a gun), conflicts: [text]}`. Gun games start with their aim,
/// `{label, fixed, accepts: "none"}`. `conflicts` counts the keys and
/// buttons bound twice (allowed, but both inputs get them).
#[wasm_bindgen]
pub fn bindings_view(game: &str, overrides: &str) -> String {
    let Ok(game) = romset::games::find(game) else { return "{\"rows\": [], \"conflicts\": 0}".into() };
    let c = &game.controls;
    let b = bindings(game, overrides);
    let mut rows = Vec::new();
    for (i, _) in c.guns.iter().enumerate() {
        let who = if c.guns.len() > 1 { format!(" (player {})", i + 1) } else { String::new() };
        let aim = if i == 0 { "mouse or touch, else the pad's sticks" } else { "the pad's sticks" };
        rows.push(format!(
            "{{\"label\": {}, \"fixed\": {}, \"accepts\": \"none\"}}",
            json_str(&format!("Aim{who}")),
            json_str(aim)
        ));
    }
    let gun0 = c.guns.first();
    for (slot, value) in b.slots() {
        let input = match *slot {
            Slot::Button(i) => c.buttons[i].label(),
            Slot::Axis(i) | Slot::Neg(i) | Slot::Pos(i) => c.analogs[i].label(),
            _ => String::new(),
        };
        let fixed = match (*slot, gun0) {
            (Slot::Button(i), Some(g)) if c.buttons[i].name == g.trigger => "left click",
            (Slot::Reload(0), _) => "right click",
            (Slot::Button(i), Some(g)) if g.reload == romset::Reload::Button(c.buttons[i].name) => "right click",
            _ => "",
        };
        let what = match value {
            Value::Inputs(list) => {
                let items: Vec<String> = list
                    .iter()
                    .map(|x| format!("{{\"text\": {}, \"token\": {}}}", json_str(&x.text()), json_str(&x.token())))
                    .collect();
                format!("\"items\": [{}]", items.join(", "))
            }
            Value::Axis(a) => {
                format!("\"axis\": {{\"text\": {}, \"invert\": {}}}", json_str(a.axis.label()), a.invert)
            }
        };
        let conflicts: Vec<String> = b
            .conflicts_of(*slot)
            .iter()
            .map(|x| {
                let with: Vec<String> = x.slots.iter().filter(|s| *s != slot).map(|&s| b.label(s)).collect();
                json_str(&format!("{} is also {}", x.input.text(), with.join(", ")))
            })
            .collect();
        let empty = Value::Inputs(Vec::new());
        let unbound = *value == empty && b.default_of(*slot) == Some(&empty);
        rows.push(format!(
            "{{\"slot\": {}, \"label\": {}, \"input\": {}, \"accepts\": \"{}\", {what}, \"default\": {}, \"unbound\": {unbound}, \"fixed\": {}, \"conflicts\": [{}]}}",
            json_str(&b.slot_name(*slot)),
            json_str(&b.label(*slot)),
            json_str(&input),
            b.accepts(*slot).name(),
            b.is_default(*slot),
            json_str(fixed),
            conflicts.join(", ")
        ));
    }
    format!("{{\"rows\": [{}], \"conflicts\": {}}}", rows.join(", "), b.conflicts().len())
}

/// Bind what the player pressed in the settings panel: in `slot` (its
/// stored name), in place of binding `index` (-1: added). `kind` and
/// `value`: "key" and a `KeyboardEvent.code`, "button" and a standard
/// gamepad button's index, or "axis" and a stick axis's index (0 to 3) with
/// `+` or `-`, the way it moved. Returns JSON `{overrides, error}`: the new
/// overrides, or the old ones and why it couldn't be bound (a key the
/// player or page keeps, the wrong kind for the slot).
#[wasm_bindgen]
pub fn bindings_capture(game: &str, overrides: &str, slot: &str, index: i32, kind: &str, value: &str) -> String {
    let result = (|| {
        let game = romset::games::find(game).map_err(|e| e.to_string())?;
        let mut b = bindings(game, overrides);
        let slot = b.slot_named(slot).ok_or("No such input")?;
        let capture = match kind {
            "key" => Capture::Key(value.to_string()),
            "button" => Capture::Button(value.parse().ok().and_then(PadButton::from_index).ok_or("Unknown button")?),
            "axis" => {
                let (i, sign) = value.split_at(value.len().saturating_sub(1));
                let axis = match i {
                    "0" => PadAxis::LeftStickX,
                    "1" => PadAxis::LeftStickY,
                    "2" => PadAxis::RightStickX,
                    "3" => PadAxis::RightStickY,
                    _ => return Err("Unknown axis".to_string()),
                };
                Capture::Axis(axis, sign == "-")
            }
            _ => return Err("Unknown input".to_string()),
        };
        b.capture(slot, usize::try_from(index).ok(), capture, &reserved())?;
        Ok(b.overrides())
    })();
    match result {
        Ok(o) => format!("{{\"overrides\": {}, \"error\": null}}", json_str(&o)),
        Err(e) => format!("{{\"overrides\": {}, \"error\": {}}}", json_str(overrides), json_str(&e)),
    }
}

/// Change the player's bindings from the settings panel: `op` "remove"
/// (binding `index` of `slot`, or its axis), "invert" (an axis), "reset"
/// (`slot` to the manifest's) or "reset-all". Returns the new overrides.
#[wasm_bindgen]
pub fn bindings_edit(game: &str, overrides: &str, op: &str, slot: &str, index: u32) -> String {
    let Ok(game) = romset::games::find(game) else { return overrides.to_string() };
    let mut b = bindings(game, overrides);
    match (op, b.slot_named(slot)) {
        ("reset-all", _) => b.reset_all(),
        ("remove", Some(s)) => b.remove(s, index as usize),
        ("invert", Some(s)) => b.invert(s),
        ("reset", Some(s)) => b.reset(s),
        _ => {}
    }
    b.overrides()
}

/// The sound and music boards on their own, for a Web Worker: fed the sound
/// link bytes the game sends, it renders the audio.
#[wasm_bindgen]
pub struct SoundEngine {
    live: model2::audio::Live,
    link: Vec<(u64, bool, u8)>,
    pcm: Vec<i16>,
    resampler: model2::audio::Resampler,
}

#[wasm_bindgen]
impl SoundEngine {
    /// `chips`: Uint8Arrays of the files [`rom_files`] lists for `game`,
    /// in order.
    #[wasm_bindgen(constructor)]
    pub fn new(game: &str, chips: js_sys::Array) -> Result<SoundEngine, JsValue> {
        console_error_panic_hook::set_once();
        let game = romset::games::find(game).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let chips = chips.iter().map(|c| js_sys::Uint8Array::new(&c).to_vec()).collect();
        let roms = romset::RomSet::from_chips(game, chips, false).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(SoundEngine {
            live: model2::audio::Live::new(&roms),
            link: Vec::new(),
            pcm: Vec::new(),
            resampler: Default::default(),
        })
    }

    /// Set the mix: sound board (effects, announcer) and music in dB.
    pub fn set_mix(&mut self, sound_db: f32, music_db: f32) {
        self.live.set_mix(sound_db, music_db);
    }

    /// Deliver link `bytes` written at CPU `cycles` and render up to cycle
    /// `until`: interleaved stereo at 44.1 kHz, resampled by `ratio` (game
    /// samples per output sample: the game's speed, with drift correction).
    pub fn feed(&mut self, cycles: &[f64], bytes: &[u8], until: f64, ratio: f64) -> js_sys::Float32Array {
        self.link.clear();
        self.link.extend(cycles.iter().zip(bytes).map(|(&c, &b)| (c as u64, false, b)));
        self.pcm.clear();
        self.live.feed(&self.link, until as u64, &mut self.pcm);
        let mut out = Vec::with_capacity(self.pcm.len());
        self.resampler.run(&self.pcm, ratio, &mut out);
        js_sys::Float32Array::from(&out[..])
    }
}

/// The audio worklet's level report: `queued` stereo frames waiting to play.
/// Lets the audio clock pace the emulation.
#[wasm_bindgen]
pub fn set_audio_level(queued: f64) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let mut app = app.borrow_mut();
            app.audio_level = Some((queued, now()));
            app.sent_since = 0.0;
        }
    });
}

/// Load `game`'s ROMs from `rom_url` (one file per chip, as in `rom/`), set up
/// WebGPU on `canvas` and start the game. `aspect` is width / height,
/// `hud_edges` moves HUD elements to the screen edges in wide views. `link`
/// receives the sound link bytes each frame (see `App::send_link`) for the
/// sound worker; leave it out for no sound. `geo`, if given, receives the
/// geometrizer's jobs for a worker: ("boot", Uint32Array of (low, high)
/// word pairs) or ("frame", Uint8Array buffer RAM, booted, read pointer,
/// Uint32Array jump word offsets, smooth motion's steps).
#[wasm_bindgen]
pub async fn start(
    game: String,
    canvas: HtmlCanvasElement,
    status: web_sys::Element,
    rom_url: String,
    height: u32,
    aspect: f32,
    hud_edges: bool,
    link: Option<js_sys::Function>,
    geo: Option<js_sys::Function>,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    recomp::register();
    let err = |e: &dyn std::fmt::Display| JsValue::from_str(&e.to_string());
    let game = romset::games::find(&game).map_err(|e| err(&e))?;
    let mut chips = Vec::new();
    for (i, f) in game.files.iter().enumerate() {
        set_status(&status, &format!("loading ROMs: {} ({}/{})", f.name, i + 1, game.files.len()));
        chips.push(fetch(&format!("{rom_url}/{}", f.name)).await?);
    }
    let roms = romset::RomSet::from_chips(game, chips, false).map_err(|e| err(&e))?;
    let mut machine = model2::Machine::new(&roms);
    let remote_geo = geo.is_some();
    let smooth = SMOOTH.with(|s| s.get());
    if !remote_geo {
        // On the page's thread, the native geometrizer where it implements
        // the program (`?geo=chip` is for the worker).
        machine.bus.geo.set_mode(model2::geo::GeoMode::Native);
        machine.bus.geo.set_smooth(smooth);
        // Smooth shading lights from what the geometrizer knew.
        machine.bus.geo.set_light(QUALITY.with(|q| q.get()).gpu().side_channel_on());
    }
    let geo_in_flight = Rc::new(Cell::new(0));
    let smooth_steps = Rc::new(Cell::new(0));
    if let Some(post) = geo {
        let in_flight = geo_in_flight.clone();
        let steps = smooth_steps.clone();
        machine.bus.geo.set_remote(Box::new(move |job| {
            let args = js_sys::Array::new();
            match job {
                model2::geo::Job::Boot(program) => {
                    let words: Vec<u32> = program.iter().flat_map(|&w| [w as u32, (w >> 32) as u32]).collect();
                    args.push(&"boot".into());
                    args.push(&js_sys::Uint32Array::from(&words[..]));
                }
                model2::geo::Job::Frame { buffer, booted, read_ptr, jumps } => {
                    in_flight.set(in_flight.get() + 1);
                    args.push(&"frame".into());
                    args.push(&js_sys::Uint8Array::from(&buffer[..]));
                    args.push(&booted.into());
                    args.push(&read_ptr.into());
                    args.push(&js_sys::Uint32Array::from(&jumps[..]));
                    args.push(&steps.get().into());
                }
            }
            let _ = post.apply(&JsValue::NULL, &args);
        }));
    }

    set_status(&status, "starting WebGPU");
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
    desc.backends = wgpu::Backends::BROWSER_WEBGPU;
    let instance = wgpu::Instance::new(desc);
    let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone())).map_err(|e| err(&e))?;
    set_status(&status, "starting WebGPU: requesting an adapter");
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(|e| err(&format!("no WebGPU adapter ({e}); this browser may not support WebGPU")))?;
    let name = gpu::adapter_name(&adapter);
    set_status(&status, &format!("starting WebGPU: requesting a device from {name}"));
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("remodel"),
            required_limits: gpu::limits(&adapter),
            ..Default::default()
        })
        .await
        .map_err(|e| err(&e))?;

    // A browser without a console (a phone) still shows what the GPU
    // refused: shown in the status line while the game starts.
    // (Looked up by id: the handler must be Send, as on native targets.)
    device.on_uncaptured_error(std::sync::Arc::new(|e| {
        web_sys::console::error_1(&format!("WebGPU error: {e}").into());
        let shown = window().document().and_then(|d| d.get_element_by_id("status"));
        if let Some(shown) = shown
            .filter(|s| s.text_content().is_some_and(|t| t.starts_with("starting") || t.starts_with("WebGPU error")))
        {
            set_status(&shown, &format!("WebGPU error: {e}"));
        }
    }));
    device.set_device_lost_callback(|reason, msg| {
        web_sys::console::error_1(&format!("WebGPU device lost ({reason:?}): {msg}").into())
    });

    let hud = if hud_edges { video::Hud::Edges } else { video::Hud::Center };
    let mut view = video::View::new(height as usize, aspect, hud);
    view = view.with_hud_windows(game.hud_windows, game.hud_element_windows);
    canvas.set_width(view.width as u32);
    canvas.set_height(view.height as u32);
    let mut config = surface
        .get_default_config(&adapter, view.width as u32, view.height as u32)
        .ok_or_else(|| JsValue::from_str("canvas surface not supported by this adapter"))?;
    // Colours are already display values: present without sRGB encoding.
    let caps = surface.get_capabilities(&adapter);
    // Prefer BGRA, the browsers' usual canvas format (Chrome can lack
    // swap chain support for RGBA).
    let formats = &caps.formats;
    config.format = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
        .into_iter()
        .find(|f| formats.contains(f))
        .or_else(|| formats.iter().copied().find(|f| !f.is_srgb()))
        .unwrap_or(config.format.remove_srgb_suffix());
    surface.configure(&device, &config);
    set_status(&status, "starting WebGPU: building the renderer (shaders)");
    let mut renderer = gpu::Renderer::from_device(device, queue, name.clone());
    renderer.set_quality(QUALITY.with(|q| q.get()).gpu());
    // Smooth shading's per-game rules (crease angle, windows left flat).
    let hud = model2::light::window_mask(game.hud_windows);
    renderer.set_light_rules(&game.shading, hud);
    if let Some(pack) = PACK.with(|p| p.borrow().clone()) {
        renderer.set_pack(Some(Box::new(WebPack::new(pack))));
    }
    web_sys::console::log_1(&format!("remodel: {name}, {}x{}, {:?}", view.width, view.height, config.format).into());

    let app = Rc::new(RefCell::new(App {
        machine,
        renderer,
        surface,
        format: config.format,
        view,
        keys: HashSet::new(),
        bindings: Bindings::with_overrides(&game.controls, &OVERRIDES.with(|o| o.borrow().clone()), &reserved()),
        defaults: Bindings::defaults(&game.controls),
        touch: (0.0, 0.0, 0.0, 0),
        guns: [None; 2],
        stick: model2::input::StickCursor::default(),
        stick_dirs: romset::StickDirs::default(),
        stick_player: None,
        epoch: now(),
        counter: (now(), 0, 0),
        status,
        stopped: false,
        paused: false,
        spent: [0.0; 3],
        link,
        samples: 0,
        audio_level: None,
        sent_since: 0.0,
        refresh: model2::pacing::Refresh::new(FPS),
        last_t: 0.0,
        redraws: 0,
        repeats: 0,
        ratio: 1.0,
        ratio_range: (f64::MAX, 0.0),
        pace: Default::default(),
        audio_target: AUDIO_BUFFER.with(|b| b.get()),
        owed: 0,
        geo_frame: remote_geo.then_some(None),
        geo_in_flight,
        geo_new: false,
        geo_held: 0,
        smooth,
        smooth_steps,
        geo_between: Vec::new(),
        geo_light: None,
        geo_between_light: Vec::new(),
        smooth_at: 0.0,
        smooth_drawn: (0, 0.0),
        _gpu: (instance, adapter),
    }));

    APP.with(|a| *a.borrow_mut() = Some(app.clone()));

    // Keyboard: remember held keys by code; keep the page from scrolling.
    for (event, down) in [("keydown", true), ("keyup", false)] {
        let app = app.clone();
        let cb = Closure::<dyn FnMut(KeyboardEvent)>::new(move |e: KeyboardEvent| {
            let code = e.code();
            if code.starts_with("Arrow") || code == "Space" {
                e.prevent_default();
            }
            let keys = &mut app.borrow_mut().keys;
            if down {
                keys.insert(code);
            } else {
                keys.remove(&code);
            }
        });
        window().add_event_listener_with_callback(event, cb.as_ref().unchecked_ref())?;
        cb.forget();
    }

    // requestAnimationFrame loop.
    let frame: Rc<RefCell<Option<Closure<dyn FnMut(f64)>>>> = Rc::new(RefCell::new(None));
    let next = frame.clone();
    *frame.borrow_mut() = Some(Closure::new(move |t: f64| {
        app.borrow_mut().tick(t);
        if let Some(cb) = next.borrow().as_ref() {
            let _ = window().request_animation_frame(cb.as_ref().unchecked_ref());
        }
    }));
    if let Some(cb) = frame.borrow().as_ref() {
        window().request_animation_frame(cb.as_ref().unchecked_ref())?;
    }
    Ok(())
}
