//! reModel in a browser: the board model compiled to WebAssembly, drawn
//! with WebGPU through the GPU rasteriser, with keyboard and gamepad input.
//! `make web` builds it into out/web/ (see docs/web.md).
//!
//! The page calls [`start`] with a canvas and the URL of the ROM files;
//! everything after that runs from `requestAnimationFrame`. At a display
//! refresh near the game's (60 Hz, or 120 Hz at one frame per two) the
//! machine runs exactly one frame per refresh, measured from the animation
//! frame times; otherwise it follows a steady 57.524 Hz clock. The sound
//! worker resamples the audio to match: the game's speed, scaled by the
//! pace it actually keeps and steered by the worklet's level reports
//! ([`set_audio_level`]) through `model2::audio::Pace`, so the audio slows
//! with the game when it can't keep up instead of running dry.
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

use model2::video;
use romset::bindings::{Bindings, Capture, Slot, Value};
use romset::{PadAxis, PadButton};
use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, KeyboardEvent};

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per browser frame before dropping time.
const MAX_CATCH_UP: u32 = 3;
/// Audio the worklet should hold (seconds): the rate control
/// (`model2::audio::Pace`) steers towards it.
const AUDIO_TARGET: f64 = 0.07;
/// Following the display, frames a held-back game may run late (at most
/// `MAX_CATCH_UP` in one animation frame): one the geometry worker held
/// back is run with the next, not lost.
const MAX_OWED: u64 = 2;
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
    stick_player: Option<usize>,
    /// Real time (ms) that emulated frame 0 corresponds to.
    epoch: f64,
    /// FPS counter: window start (ms), frames emulated and drawn since.
    counter: (f64, u32, u32),
    /// Host time (ms) in this window: emulating, preparing frames on the
    /// CPU, and encoding and submitting them to the GPU.
    spent: [f64; 3],
    /// Called with the sound link bytes sent since the last call: (cycles
    /// Float64Array, bytes Uint8Array, render-until cycle). None: no sound.
    link: Option<js_sys::Function>,
    /// Audio the link calls asked for so far, in output samples.
    samples: u64,
    /// The worklet's last report: stereo frames queued, and when (ms).
    audio_level: Option<(f64, f64)>,
    /// Stereo frames sent since that report (after resampling).
    sent_since: f64,
    /// Average time between animation frames (ms), and the last frame time.
    refresh_ms: f64,
    last_t: f64,
    redraws: u64,
    /// Animation frames this status window that showed no new game frame.
    repeats: u32,
    /// Game samples per output sample for the sound worker's resampler.
    ratio: f64,
    /// The audio rate control: follows the game's actual pace.
    pace: model2::audio::Pace,
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

impl Devices<'_> {
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
        let devices = Devices { keys: &self.keys, pad: gamepad() };
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
            Some(k) => self.refresh_ms / k,
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
        if self.smooth && self.geo_frame.is_none() && 1000.0 / self.refresh_ms > FPS * 1.2 {
            if let Some(f) = self.machine.bus.geo.engine().smooth_frame() {
                own = f.render(t as f32).map(|s| (s, f));
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
            Some(Some((s, p))) => match self.geo_between.get(between).filter(|b| !b.is_empty()) {
                Some(b) => {
                    self.smooth_drawn.0 += 1;
                    Some((&b[..], &p[..]))
                }
                None => Some((&s[..], &p[..])),
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

    /// Game frames per animation frame when following the display (1 at
    /// about 60 Hz, 1/2 at about 120 Hz), or None to follow the clock.
    fn per_refresh(&self) -> Option<f64> {
        let hz = 1000.0 / self.refresh_ms;
        [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS)))
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
        // Measure the refresh rate (ignoring gaps such as a hidden tab).
        let dt = t - self.last_t;
        self.last_t = t;
        if dt > 2.0 && dt < 50.0 {
            self.refresh_ms += (dt - self.refresh_ms) * 0.05;
        }
        self.redraws += 1;
        if self.smooth {
            // In-betweens the worker draws per game frame: one per refresh
            // the frame lasts (none at 60 Hz), at most 4.
            self.smooth_steps.set((self.period() / self.refresh_ms).round().clamp(1.0, 4.0) as u32);
        }
        let (due, speed) = match self.per_refresh() {
            Some(k) => {
                let next = (k == 1.0 || self.redraws.is_multiple_of(2)) as u64;
                self.owed = (self.owed + next).min(MAX_OWED);
                (self.machine.frames + self.owed, 1000.0 / self.refresh_ms * k / FPS)
            }
            None => (((t - self.epoch) * FPS / 1000.0) as u64, 1.0),
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
        // The audio rate for the next frames: the game's speed, scaled by
        // how well it's keeping it (held back by the geometry worker, or
        // its own cost) and steered by the worklet's queue, so the audio
        // slows with the game instead of running dry.
        if self.redraws > 1 {
            let queued = self.audio_queued(t);
            let pace = self.pace.update(dt / 1000.0, ran as f64, dt * FPS * speed / 1000.0, queued, AUDIO_TARGET);
            self.ratio = speed * pace;
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
        let smoothing = self.smooth && 1000.0 / self.refresh_ms > FPS * 1.2;
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
                    "{} · {:.0}% speed{} · {:.0} fps drawn ({} repeats) · {}x{} · i960 {} · ms per frame: emulate {:.1}, draw {:.1}{}",
                    match self.per_refresh() {
                        Some(_) => format!("synced to {:.0} Hz", 1000.0 / self.refresh_ms),
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
                ),
            );
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
#[wasm_bindgen]
pub fn set_geo_frame(stream: Option<Vec<u32>>, points: Option<Vec<u16>>, between: Option<js_sys::Array>) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            let mut app = app.borrow_mut();
            app.geo_in_flight.set(app.geo_in_flight.get().saturating_sub(1));
            if let (Some(stream), Some(points)) = (stream, points) {
                app.geo_frame = Some(Some((stream, points)));
                app.geo_new = true;
                app.geo_between =
                    between.map_or(Vec::new(), |b| b.iter().map(|s| js_sys::Uint32Array::new(&s).to_vec()).collect());
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
        Ok(GeoEngine { engine: model2::geo::Engine::new(model2::bus::polygon_rom(&roms), core) })
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

    /// The last frame's in-betweens at t = k / `steps` for k from 0 to
    /// `steps` - 1, from the frame before towards it: Uint32Arrays, empty
    /// where the frame's own stream shows (a cut, nothing to blend). Empty
    /// without smooth motion, or for a frame the native geometrizer didn't
    /// run.
    pub fn between(&self, steps: u32) -> js_sys::Array {
        let out = js_sys::Array::new();
        if let Some(f) = self.engine.smooth_frame() {
            for k in 0..steps {
                let s = f.render(k as f32 / steps as f32).unwrap_or_default();
                out.push(&js_sys::Uint32Array::from(&s[..]));
            }
        }
        out
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

/// The keys the panel won't bind (the players' own and the page's), as
/// `KeyboardEvent.code`s, one per line.
#[wasm_bindgen]
pub fn reserved_keys() -> String {
    reserved().join("\n")
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
/// word pairs) or ("frame", Uint8Array buffer RAM, booted, read pointer).
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
    let adapter = instance
        .request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        })
        .await
        .map_err(|e| err(&format!("no WebGPU adapter ({e}); this browser may not support WebGPU")))?;
    let name = gpu::adapter_name(&adapter);
    let (device, queue) = adapter
        .request_device(&wgpu::DeviceDescriptor {
            label: Some("remodel"),
            required_limits: gpu::limits(&adapter),
            ..Default::default()
        })
        .await
        .map_err(|e| err(&e))?;

    device
        .on_uncaptured_error(std::sync::Arc::new(|e| web_sys::console::error_1(&format!("WebGPU error: {e}").into())));
    device.set_device_lost_callback(|reason, msg| {
        web_sys::console::error_1(&format!("WebGPU device lost ({reason:?}): {msg}").into())
    });

    let hud = if hud_edges { video::Hud::Edges } else { video::Hud::Center };
    let mut view = video::View::new(height as usize, aspect, hud);
    view.hud_windows = game.hud_windows.iter().fold(0, |m, &w| m | 1 << w);
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
    let renderer = gpu::Renderer::from_device(device, queue, name.clone());
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
        refresh_ms: 1000.0 / 60.0,
        last_t: 0.0,
        redraws: 0,
        repeats: 0,
        ratio: 1.0,
        pace: Default::default(),
        owed: 0,
        geo_frame: remote_geo.then_some(None),
        geo_in_flight,
        geo_new: false,
        geo_held: 0,
        smooth,
        smooth_steps,
        geo_between: Vec::new(),
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
