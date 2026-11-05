//! Remodel in a browser: the board model compiled to WebAssembly, drawn
//! with WebGPU through the GPU rasteriser, with keyboard and gamepad input.
//! `make web` builds it into out/web/ (see docs/web.md).
//!
//! The page calls [`start`] with a canvas and the URL of the ROM files;
//! everything after that runs from `requestAnimationFrame`. The machine
//! runs one emulated frame per 1/57.524 s of real time, catching up a few
//! frames at most when the browser falls behind. No sound yet.

#![cfg(target_arch = "wasm32")]

use model2::{raster, video};
use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{HtmlCanvasElement, KeyboardEvent};

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per browser frame before dropping time.
const MAX_CATCH_UP: u32 = 3;

struct App {
    machine: model2::Machine,
    renderer: gpu::Renderer,
    surface: wgpu::Surface<'static>,
    format: wgpu::TextureFormat,
    view: video::View,
    keys: HashSet<String>,
    /// Real time (ms) that emulated frame 0 corresponds to.
    epoch: f64,
    /// FPS counter: window start (ms), frames emulated and drawn since.
    counter: (f64, u32, u32),
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

impl App {
    /// Map the keyboard and first gamepad onto the cabinet's controls.
    ///
    /// Keyboard: arrows steer (Down slides), Z jump (front), X jump (tail),
    /// Enter start, C or 5 coin, Q / E select left / right.
    /// Gamepad (standard layout): left stick or d-pad steers, right
    /// trigger or B slides, A jump (front), X jump (tail), Start, Back
    /// coin, shoulder buttons select.
    fn apply_inputs(&mut self) {
        let key = |c: &str| self.keys.contains(c);
        let pad = gamepad();
        let button = |i: u32| {
            pad.as_ref()
                .and_then(|p| p.buttons().get(i).dyn_into::<web_sys::GamepadButton>().ok())
                .map_or(0.0, |b| b.value())
        };
        let pressed = |i: u32| button(i) > 0.5;
        let stick = pad.as_ref().and_then(|p| p.axes().get(0).as_f64()).filter(|x| x.abs() > 0.15).unwrap_or(0.0);
        // Steering: 0x10 carves right, 0xF0 left, 0x80 centred.
        let steer = if key("ArrowLeft") || pressed(14) {
            -1.0
        } else if key("ArrowRight") || pressed(15) {
            1.0
        } else {
            stick
        };
        let slide = if key("ArrowDown") || key("Space") || pressed(1) { 1.0 } else { button(7) };
        let bus = &mut self.machine.bus;
        model2::input::set_analog(bus, "curving", (128.0 - steer * 112.0).round() as u8);
        model2::input::set_analog(bus, "slide", (128.0 + slide * 112.0).round() as u8);
        for (name, down) in [
            ("jumpfront", key("KeyZ") || key("ArrowUp") || pressed(0)),
            ("jumptail", key("KeyX") || pressed(2)),
            ("start", key("Enter") || pressed(9)),
            ("coin1", key("KeyC") || key("Digit5") || pressed(8)),
            ("selleft", key("KeyQ") || pressed(4)),
            ("selright", key("KeyE") || pressed(5)),
        ] {
            model2::input::set_button(bus, name, down);
        }
    }

    fn draw(&mut self) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => return,
        };
        let m = &mut self.machine;
        m.bus.geo.sync();
        let b = &m.bus;
        let geo = b.geo.engine();
        let (xoff, yoff) = b.sync_offsets();
        let src = raster::Sources {
            texture_rom: &b.texture_rom,
            texture_point_ram: &geo.texture_point_ram,
            render_ram: b.render_ram(),
            palette: b.palette_ram(),
            xoff,
            yoff,
        };
        let mut polys = raster::build(&geo.stream, &src);
        let placed = raster::place(&mut polys, &self.view);
        let layers = video::layers_2d(b.tile_ram(), b.palette_ram(), &self.view, !placed.is_empty());
        let target = frame.texture.create_view(&Default::default());
        self.renderer.present(&polys, &placed, &src, &self.view, &layers, &target, self.format);
        self.renderer.queue().present(frame);
    }

    fn tick(&mut self, t: f64) {
        if self.stopped {
            return;
        }
        let due = ((t - self.epoch) * FPS / 1000.0) as u64;
        let mut ran = 0;
        while self.machine.frames < due && ran < MAX_CATCH_UP {
            self.apply_inputs();
            if let Err(e) = self.machine.run_slice(u64::MAX) {
                self.stopped = true;
                set_status(&self.status, &format!("stopped: {e:?}"));
                return;
            }
            ran += 1;
        }
        if self.machine.frames < due {
            // Too far behind: drop the time rather than spiral.
            self.epoch = t - self.machine.frames as f64 * 1000.0 / FPS;
        }
        if ran > 0 {
            self.draw();
            self.counter.2 += 1;
        }
        self.counter.1 += ran;
        if t - self.counter.0 >= 1000.0 {
            let secs = (t - self.counter.0) / 1000.0;
            let speed = self.counter.1 as f64 / secs / FPS * 100.0;
            set_status(
                &self.status,
                &format!(
                    "{:.0}% speed · {:.0} fps drawn · {}x{} · i960 {}",
                    speed,
                    self.counter.2 as f64 / secs,
                    self.view.width,
                    self.view.height,
                    if self.machine.is_native() { "recompiled" } else { "interpreted" }
                ),
            );
            self.counter = (t, 0, 0);
        }
    }
}

/// Load the ROMs from `rom_url` (one file per chip, as in `rom/`), set up
/// WebGPU on `canvas` and start the game. `aspect` is width / height,
/// `hud_edges` moves HUD elements to the screen edges in wide views.
#[wasm_bindgen]
pub async fn start(
    canvas: HtmlCanvasElement,
    status: web_sys::Element,
    rom_url: String,
    height: u32,
    aspect: f32,
    hud_edges: bool,
) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let err = |e: &dyn std::fmt::Display| JsValue::from_str(&e.to_string());
    let game = romset::games::find("topskatr").map_err(|e| err(&e))?;
    let mut chips = Vec::new();
    for (i, f) in game.files.iter().enumerate() {
        set_status(&status, &format!("loading ROMs: {} ({}/{})", f.name, i + 1, game.files.len()));
        chips.push(fetch(&format!("{rom_url}/{}", f.name)).await?);
    }
    let roms = romset::RomSet::from_chips(game, chips, false).map_err(|e| err(&e))?;
    let machine = model2::Machine::new(&roms);

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
        epoch: now(),
        counter: (now(), 0, 0),
        status,
        stopped: false,
        _gpu: (instance, adapter),
    }));

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
