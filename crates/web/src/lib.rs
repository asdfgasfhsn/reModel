//! Remodel in a browser: the board model compiled to WebAssembly, drawn
//! with WebGPU through the GPU rasteriser, with keyboard and gamepad input.
//! `make web` builds it into out/web/ (see docs/web.md).
//!
//! The page calls [`start`] with a canvas and the URL of the ROM files;
//! everything after that runs from `requestAnimationFrame`. At a display
//! refresh near the game's (60 Hz, or 120 Hz at one frame per two) the
//! machine runs exactly one frame per refresh, measured from the animation
//! frame times; otherwise it follows a steady 57.524 Hz clock. The sound
//! worker resamples the audio to match, and corrects drift against the
//! sound card by stretching it at most 0.5%, from the worklet's level
//! reports ([`set_audio_level`]).
//!
//! The sound and music boards run in a Web Worker (sound-worker.js, using
//! [`SoundEngine`] from this same module), on another core: each frame the
//! game posts the sound link bytes it sent, with their CPU cycles, and the
//! worker renders the audio and posts it to the page's AudioWorklet.
//!
//! The geometrizer runs in another worker (geo-worker.js, [`GeoEngine`]):
//! the game posts each vblank's job (`model2::geo::Job`) and draws the
//! newest polygon stream the worker sends back ([`set_geo_frame`]).

#![cfg(target_arch = "wasm32")]

use model2::video;
use romset::PadButton;
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, KeyboardEvent};

/// The pad buttons `set_touch`'s button bits stand for, by bit: touch
/// works as a gamepad, so it plays every game by its pad bindings (the
/// on-screen labels are topskatr's: jump front, jump tail, start, coin,
/// select left, select right).
const TOUCH_BUTTONS: [romset::PadButton; 6] = [
    romset::PadButton::South,
    romset::PadButton::West,
    romset::PadButton::Start,
    romset::PadButton::Select,
    romset::PadButton::LeftShoulder,
    romset::PadButton::RightShoulder,
];

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per browser frame before dropping time.
const MAX_CATCH_UP: u32 = 3;
/// Audio the worklet should hold (stereo frames, about 70 ms): drift
/// correction steers towards it.
const AUDIO_TARGET: f64 = 0.07 * model2::audio::RATE as f64;
/// Most the audio rate is stretched for drift correction.
const DRIFT: f64 = 0.005;
/// Audio level reports older than this (ms) are stale: pace by time.
const AUDIO_STALE_MS: f64 = 250.0;

struct App {
    machine: model2::Machine,
    renderer: gpu::Renderer,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    view: video::View,
    keys: HashSet<String>,
    /// On-screen controls ([`set_touch`]): steering and slide (-1 to 1, 0 to
    /// 1) and held buttons (bits in `TOUCH_BUTTONS` order).
    touch: (f64, f64, u32),
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
    /// Geometrizer in a worker: its newest frame (raster stream, texture
    /// point RAM), once one arrives. None with the local engine.
    geo_frame: Option<Option<(Vec<u32>, Vec<u16>)>>,
    status: web_sys::Element,
    stopped: bool,
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

/// Keyboard, gamepad and touch as one player (W3C standard pad layout).
struct Devices<'a> {
    keys: &'a HashSet<String>,
    pad: Option<web_sys::Gamepad>,
    /// Touch steering (-1 to 1, the left stick), slide (0 to 1, the right
    /// trigger) and buttons (`TOUCH_BUTTONS` bits).
    touch: (f64, f64, u32),
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
        let touched = TOUCH_BUTTONS.iter().position(|&t| t == b).is_some_and(|i| self.touch.2 & 1 << i != 0);
        touched || self.button(b.index()) > 0.5
    }

    fn pad_axis(&self, a: romset::PadAxis) -> f32 {
        use romset::PadAxis;
        let right_trigger = self.button(PadButton::RightTrigger.index()).max(self.touch.1 as f32);
        let left_trigger = self.button(PadButton::LeftTrigger.index());
        match a {
            PadAxis::None => 0.0,
            PadAxis::LeftStickX => match self.axis(0) {
                0.0 => (self.touch.0 as f32).clamp(-1.0, 1.0),
                v => v,
            },
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
    /// the cabinet's, by the game's bindings (`romset::Controls`; the page
    /// lists them).
    fn apply_inputs(&mut self) {
        let devices = Devices { keys: &self.keys, pad: gamepad(), touch: self.touch };
        model2::input::apply_player(&mut self.machine.bus, &devices, |_| false);
    }

    fn draw(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => return,
        };
        let t0 = now();
        let geo = match &self.geo_frame {
            // The worker's newest frame (nothing to draw before the first).
            Some(Some((s, p))) => Some((&s[..], &p[..])),
            Some(None) => Some((&[][..], &[][..])),
            None => None,
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

    /// Stereo frames the worklet holds now, estimated from its last report,
    /// or None if there's no recent one (no sound, or it stalled).
    fn audio_queued(&self, t: f64) -> Option<f64> {
        let (level, at) = self.audio_level?;
        let age = t - at;
        (self.link.is_some() && age < AUDIO_STALE_MS)
            .then(|| level + self.sent_since - age * model2::audio::RATE as f64 / 1000.0)
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
        // Measure the refresh rate (ignoring gaps such as a hidden tab).
        let dt = t - self.last_t;
        self.last_t = t;
        if dt > 2.0 && dt < 50.0 {
            self.refresh_ms += (dt - self.refresh_ms) * 0.05;
        }
        self.redraws += 1;
        let (due, speed) = match self.per_refresh() {
            Some(k) => {
                let next = (k == 1.0 || self.redraws.is_multiple_of(2)) as u64;
                (self.machine.frames + next, 1000.0 / self.refresh_ms * k / FPS)
            }
            None => (((t - self.epoch) * FPS / 1000.0) as u64, 1.0),
        };
        // Drift correction from the worklet's queue.
        let drift = self.audio_queued(t).map_or(0.0, |q| ((q - AUDIO_TARGET) / AUDIO_TARGET).clamp(-1.0, 1.0));
        self.ratio = speed * (1.0 + drift * DRIFT);
        let mut ran = 0;
        let t0 = now();
        while self.machine.frames < due && ran < MAX_CATCH_UP {
            self.apply_inputs();
            if let Err(e) = self.machine.run_slice(u64::MAX) {
                self.stopped = true;
                set_status(&self.status, &format!("stopped: {e:?}"));
                return;
            }
            self.send_link();
            ran += 1;
        }
        self.spent[0] += now() - t0;
        if self.per_refresh().is_some() || self.machine.frames < due {
            // Following the display, or too far behind: keep the clock current.
            self.epoch = t - self.machine.frames as f64 * 1000.0 / FPS;
        }
        if ran == 0 {
            self.repeats += 1;
        }
        if ran > 0 {
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
                    "{} · {:.0}% speed · {:.0} fps drawn ({} repeats) · {}x{} · i960 {} · ms per frame: emulate {:.1}, draw {:.1}",
                    match self.per_refresh() {
                        Some(_) => format!("synced to {:.0} Hz", 1000.0 / self.refresh_ms),
                        None => "exact speed".into(),
                    },
                    speed,
                    self.counter.2 as f64 / secs,
                    self.repeats,
                    self.view.width,
                    self.view.height,
                    if self.machine.is_native() { "recompiled" } else { "interpreted" },
                    self.spent[0] / emulated,
                    self.spent[1] / drawn,
                ),
            );
            self.counter = (t, 0, 0);
            self.spent = [0.0; 3];
            self.repeats = 0;
        }
    }
}

thread_local! {
    /// The running app, for calls from the page after `start`.
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

/// On-screen controls: `steer` from -1 (left) to 1 (right), `slide` from 0
/// to 1, and `buttons` held as bits: jump front, jump tail, start, coin,
/// select left, select right. Merged with the keyboard and gamepad.
#[wasm_bindgen]
pub fn set_touch(steer: f64, slide: f64, buttons: u32) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            app.borrow_mut().touch = (steer, slide, buttons);
        }
    });
}

/// The geometrizer worker's newest frame: the raster stream and texture
/// point RAM.
#[wasm_bindgen]
pub fn set_geo_frame(stream: Vec<u32>, points: Vec<u16>) {
    APP.with(|a| {
        if let Some(app) = a.borrow().as_ref() {
            app.borrow_mut().geo_frame = Some(Some((stream, points)));
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
        console_error_panic_hook::set_once();
        let game = romset::games::find(game).map_err(|e| JsValue::from_str(&e.to_string()))?;
        let chips = chips.iter().map(|c| js_sys::Uint8Array::new(&c).to_vec()).collect();
        let roms = romset::RomSet::from_chips(game, chips, false).map_err(|e| JsValue::from_str(&e.to_string()))?;
        Ok(GeoEngine { engine: model2::geo::Engine::new(model2::bus::polygon_rom(&roms)) })
    }

    /// Boot the uploaded program: 64-bit words as (low, high) u32 pairs.
    pub fn boot(&mut self, words: &[u32]) {
        let program: Vec<u64> = words.chunks_exact(2).map(|w| w[0] as u64 | (w[1] as u64) << 32).collect();
        self.engine.run_job(model2::geo::Job::Boot(program));
    }

    /// Run one frame's list from a copy of buffer RAM.
    pub fn frame(&mut self, buffer: Vec<u8>, booted: bool, read_ptr: u32) {
        self.engine.run_job(model2::geo::Job::Frame { buffer, booted, read_ptr });
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
#[wasm_bindgen]
pub fn rom_files(game: &str) -> Vec<String> {
    romset::games::find(game).map_or(Vec::new(), |g| g.files.iter().map(|f| f.name.to_string()).collect())
}

/// The game's controls with their keys and pad bindings, one per line
/// (for the page's help bar).
#[wasm_bindgen]
pub fn controls_help(game: &str) -> String {
    romset::games::find(game).map_or(String::new(), |g| g.controls.describe())
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
    if let Some(post) = geo {
        machine.bus.geo.set_remote(Box::new(move |job| {
            let args = js_sys::Array::new();
            match job {
                model2::geo::Job::Boot(program) => {
                    let words: Vec<u32> = program.iter().flat_map(|&w| [w as u32, (w >> 32) as u32]).collect();
                    args.push(&"boot".into());
                    args.push(&js_sys::Uint32Array::from(&words[..]));
                }
                model2::geo::Job::Frame { buffer, booted, read_ptr } => {
                    args.push(&"frame".into());
                    args.push(&js_sys::Uint8Array::from(&buffer[..]));
                    args.push(&booted.into());
                    args.push(&read_ptr.into());
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
        touch: (0.0, 0.0, 0),
        epoch: now(),
        counter: (now(), 0, 0),
        status,
        stopped: false,
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
        geo_frame: remote_geo.then_some(None),
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
