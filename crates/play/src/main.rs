//! remodel-play: a Model 2 game in a native window.
//!
//! The main thread runs the window (winit), input, the board model and
//! drawing (the GPU renderer, presented with vsync). The geometrizer runs on
//! its own thread (the board model's default) and the sound boards on
//! another (`Sound`), which plays through the system's audio API (cpal).
//! While sound plays, its queue paces the emulation, as in the browser
//! build: frames run whenever less than `AUDIO_TARGET` is queued, so there's
//! no drift against the sound card's clock. Without sound, frames follow
//! time at 57.524 per second.

use gilrs::{Axis, Button, Gilrs};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId};

const USAGE: &str = "\
usage: remodel-play [--game ID] [--rom DIR] [--mix SFX:MUSIC] [--no-sound] [--hud center|edges] [--interp]

  --game ID          game manifest (default topskatr)
  --rom DIR          directory of unpacked chips (default rom)
  --mix SFX:MUSIC    sound board and music board gains in dB (default 6:-6)
  --no-sound         don't run the sound boards
  --hud center|edges wide windows: keep the HUD centred, or move each HUD
                     element to the nearest screen edge (default edges)
  --interp           run the i960 on the interpreter even if this build
                     has recompiled code

keys: C coin, Enter start, arrows steer (Down slide), Z jump front, X jump
tail, Q/E select, F11 fullscreen, Esc quit. Gamepads work too.
";

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per redraw before dropping time.
const MAX_CATCH_UP: u32 = 3;
/// Audio the sound queue should hold (seconds): frames run below this.
const AUDIO_TARGET: f64 = 0.05;
/// Most audio queued before the oldest is dropped (seconds).
const AUDIO_MAX: f64 = 0.25;
/// Narrowest and widest views; windows beyond are letterboxed.
const ASPECTS: (f32, f32) = (4.0 / 3.0, 32.0 / 9.0);

struct Options {
    game: String,
    rom: PathBuf,
    mix: (f32, f32),
    sound: bool,
    hud: model2::video::Hud,
    interp: bool,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        game: "topskatr".into(),
        rom: PathBuf::from("rom"),
        mix: model2::audio::DEFAULT_MIX,
        sound: true,
        hud: model2::video::Hud::Edges,
        interp: false,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--game" => o.game = value()?,
            "--rom" => o.rom = PathBuf::from(value()?),
            "--mix" => {
                let v = value()?;
                let (s, m) = v.split_once(':').ok_or("--mix wants SFX:MUSIC")?;
                o.mix = (s.parse().map_err(|_| "bad --mix")?, m.parse().map_err(|_| "bad --mix")?);
            }
            "--no-sound" => o.sound = false,
            "--hud" => {
                o.hud = if value()? == "center" { model2::video::Hud::Center } else { model2::video::Hud::Edges }
            }
            "--interp" => o.interp = true,
            _ => return Err(format!("unknown option {a}")),
        }
    }
    Ok(o)
}

/// One frame's sound link bytes (as `Model2Bus::uart_log`) and the cycle
/// to render up to.
type Link = (Vec<(u64, bool, u8)>, u64);

/// Audio queue shared with the output callback: interleaved stereo at the
/// device's rate.
#[derive(Default)]
struct Queue {
    samples: std::collections::VecDeque<f32>,
    underruns: u64,
}

/// The sound boards on their own thread, playing through cpal.
struct Sound {
    _stream: cpal::Stream,
    queue: Arc<Mutex<Queue>>,
    /// Each frame's sound link bytes and the cycle to render up to.
    link: mpsc::Sender<Link>,
    /// Output samples (44.1 kHz) the game asked for, and the thread made.
    asked: u64,
    made: Arc<AtomicU64>,
    rate: u32,
}

impl Sound {
    fn new(roms: &romset::RomSet, mix: (f32, f32)) -> Result<Sound, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| format!("audio: {e}"))?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let config = supported.config();
        let queue = Arc::new(Mutex::new(Queue::default()));
        let q = queue.clone();
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => device.build_output_stream(
                config,
                move |out: &mut [f32], _: &cpal::OutputCallbackInfo| fill(&q, out, channels, |v| v),
                |e| eprintln!("audio: {e}"),
                None,
            ),
            cpal::SampleFormat::I16 => device.build_output_stream(
                config,
                move |out: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    fill(&q, out, channels, |v| (v * 32767.0).clamp(-32768.0, 32767.0) as i16)
                },
                |e| eprintln!("audio: {e}"),
                None,
            ),
            f => return Err(format!("audio: unsupported sample format {f:?}")),
        }
        .map_err(|e| format!("audio: {e}"))?;
        stream.play().map_err(|e| format!("audio: {e}"))?;

        let (link, rx) = mpsc::channel::<Link>();
        let made = Arc::new(AtomicU64::new(0));
        let mut live = model2::audio::Live::new(roms);
        live.set_mix(mix.0, mix.1);
        let (q, m) = (queue.clone(), made.clone());
        std::thread::Builder::new()
            .name("sound".into())
            .spawn(move || {
                let mut pcm = Vec::new();
                // Linear resampler from 44.1 kHz to the device rate.
                let step = model2::audio::RATE as f64 / rate as f64;
                let (mut pos, mut prev) = (0.0f64, [0.0f32; 2]);
                let max = (AUDIO_MAX * rate as f64) as usize * 2;
                while let Ok((bytes, until)) = rx.recv() {
                    pcm.clear();
                    live.feed(&bytes, until, &mut pcm);
                    m.fetch_add(pcm.len() as u64 / 2, Ordering::Relaxed);
                    let mut out = Vec::with_capacity(pcm.len() * 2);
                    for frame in pcm.as_chunks::<2>().0 {
                        let cur = [frame[0] as f32 / 32768.0, frame[1] as f32 / 32768.0];
                        while pos < 1.0 {
                            let t = pos as f32;
                            out.extend([prev[0] + (cur[0] - prev[0]) * t, prev[1] + (cur[1] - prev[1]) * t]);
                            pos += step;
                        }
                        pos -= 1.0;
                        prev = cur;
                    }
                    let mut q = q.lock().unwrap();
                    q.samples.extend(out);
                    let excess = q.samples.len().saturating_sub(max);
                    q.samples.drain(..excess);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Sound { _stream: stream, queue, link, asked: 0, made, rate })
    }

    /// Seconds of audio queued or still being made.
    fn queued(&self) -> f64 {
        let in_queue = self.queue.lock().unwrap().samples.len() as f64 / 2.0 / self.rate as f64;
        let in_flight = self.asked.saturating_sub(self.made.load(Ordering::Relaxed));
        in_queue + in_flight as f64 / model2::audio::RATE as f64
    }

    /// Send the sound link bytes written since the last call.
    fn send(&mut self, bus: &mut model2::Model2Bus) {
        let target = (bus.now as u128 * model2::audio::RATE as u128 / model2::bus::CPU_HZ as u128) as u64;
        self.asked = target;
        let _ = self.link.send((std::mem::take(&mut bus.uart_log), bus.now));
    }
}

/// Copy queued audio into a device buffer of `channels` channels.
fn fill<T: Copy>(q: &Mutex<Queue>, out: &mut [T], channels: usize, conv: impl Fn(f32) -> T) {
    let mut q = q.lock().unwrap();
    let mut dry = false;
    for frame in out.chunks_mut(channels) {
        let (l, r) = match (q.samples.pop_front(), q.samples.pop_front()) {
            (Some(l), Some(r)) => (l, r),
            _ => {
                dry = true;
                (0.0, 0.0)
            }
        };
        for (c, s) in frame.iter_mut().enumerate() {
            *s = conv(if c % 2 == 0 { l } else { r });
        }
    }
    q.underruns += dry as u64;
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: gpu::Renderer,
    view: model2::video::View,
    origin: (u32, u32),
}

struct Play {
    opts: Options,
    machine: model2::Machine,
    sound: Option<Sound>,
    gpu: Option<Gpu>,
    keys: HashSet<KeyCode>,
    gilrs: Option<Gilrs>,
    /// The gamepad last used (a button or stick moved): keyboards' media-key
    /// interfaces show up as gamepads too, so wait to see which one plays.
    pad: Option<gilrs::GamepadId>,
    hud_windows: u32,
    eeprom: PathBuf,
    /// Real time that emulated frame 0 corresponds to.
    epoch: Instant,
    /// Status window: start, frames emulated and drawn, emulation time.
    counter: (Instant, u32, u32, Duration),
    stopped: bool,
    /// Redraws where the surface gave no texture to draw into.
    surface_misses: u64,
    /// Status reports so far (printed every fifth).
    reports: u64,
}

impl Play {
    /// Fit a view into the window: full size between `ASPECTS`, else
    /// letterboxed. Returns it and its top-left corner.
    fn fit(&self, w: u32, h: u32) -> (model2::video::View, (u32, u32)) {
        let aspect = w as f32 / h.max(1) as f32;
        let a = aspect.clamp(ASPECTS.0, ASPECTS.1);
        let (vw, vh) = if a < aspect { ((h as f32 * a) as u32, h) } else { (w, (w as f32 / a) as u32) };
        let mut view = model2::video::View::new(vh.max(1) as usize, a, self.opts.hud);
        view.hud_windows = self.hud_windows;
        let origin = ((w - (view.width as u32).min(w)) / 2, (h - (view.height as u32).min(h)) / 2);
        let _ = vw;
        (view, origin)
    }

    fn resize(&mut self, w: u32, h: u32) {
        let (view, origin) = self.fit(w, h);
        if let Some(g) = &mut self.gpu {
            g.config.width = w.max(1);
            g.config.height = h.max(1);
            g.surface.configure(g.renderer.device(), &g.config);
            g.view = view;
            g.origin = origin;
        }
    }

    /// Keyboard and the first gamepad onto the cabinet's controls.
    fn apply_inputs(&mut self) {
        let mut pad = None;
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                if matches!(ev.event, gilrs::EventType::ButtonPressed(..) | gilrs::EventType::AxisChanged(..)) {
                    self.pad = Some(ev.id);
                }
            }
            pad = self.pad.map(|id| g.gamepad(id));
        }
        let key = |k: KeyCode| self.keys.contains(&k);
        let pressed = |b: Button| pad.as_ref().is_some_and(|p| p.is_pressed(b));
        let stick = pad.as_ref().map_or(0.0, |p| p.value(Axis::LeftStickX)).clamp(-1.0, 1.0);
        let stick = if stick.abs() > 0.15 { stick } else { 0.0 };
        let trigger = pad.as_ref().and_then(|p| p.button_data(Button::RightTrigger2)).map_or(0.0, |d| d.value());
        let steer = if key(KeyCode::ArrowLeft) || pressed(Button::DPadLeft) {
            -1.0
        } else if key(KeyCode::ArrowRight) || pressed(Button::DPadRight) {
            1.0
        } else {
            stick
        };
        let slide = if key(KeyCode::ArrowDown) || key(KeyCode::Space) || pressed(Button::East) { 1.0 } else { trigger };
        let bus = &mut self.machine.bus;
        // Steering: 0x10 carves right, 0xF0 left, 0x80 centred.
        model2::input::set_analog(bus, "curving", (128.0 - steer * 112.0).round() as u8);
        model2::input::set_analog(bus, "slide", (128.0 + slide * 112.0).round() as u8);
        for (name, down) in [
            ("jumpfront", key(KeyCode::KeyZ) || key(KeyCode::ArrowUp) || pressed(Button::South)),
            ("jumptail", key(KeyCode::KeyX) || pressed(Button::West)),
            ("start", key(KeyCode::Enter) || pressed(Button::Start)),
            ("coin1", key(KeyCode::KeyC) || key(KeyCode::Digit5) || pressed(Button::Select)),
            ("selleft", key(KeyCode::KeyQ) || pressed(Button::LeftTrigger)),
            ("selright", key(KeyCode::KeyE) || pressed(Button::RightTrigger)),
        ] {
            model2::input::set_button(bus, name, down);
        }
    }

    /// Run the frames that are due, then draw.
    fn frame(&mut self) {
        if self.stopped {
            return;
        }
        let now = Instant::now();
        let due = ((now - self.epoch).as_secs_f64() * FPS) as u64;
        let t0 = Instant::now();
        let mut ran = 0;
        loop {
            let more = match &self.sound {
                Some(s) => s.queued() < AUDIO_TARGET,
                None => self.machine.frames < due,
            };
            if !more || ran >= MAX_CATCH_UP {
                break;
            }
            self.apply_inputs();
            if let Err(e) = self.machine.run_slice(u64::MAX) {
                eprintln!("stopped: {e:?}");
                self.stopped = true;
                return;
            }
            match &mut self.sound {
                Some(s) => s.send(&mut self.machine.bus),
                None => self.machine.bus.uart_log.clear(),
            }
            ran += 1;
        }
        if self.sound.is_some() || self.machine.frames < due {
            // Audio paces (or we fell behind): keep the time base current.
            self.epoch = now - Duration::from_secs_f64(self.machine.frames as f64 / FPS);
        }
        self.counter.3 += t0.elapsed();
        self.counter.1 += ran;
        if let Some(g) = &mut self.gpu {
            match g.surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame) | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                    let target = frame.texture.create_view(&Default::default());
                    g.renderer.present_bus(&mut self.machine.bus, None, &g.view, &target, g.config.format, g.origin);
                    g.renderer.queue().present(frame);
                    self.counter.2 += 1;
                }
                // Lost, outdated, hidden or timed out: reconfigure, skip a draw.
                _ => {
                    g.surface.configure(g.renderer.device(), &g.config);
                    self.surface_misses += 1;
                }
            }
        }
        let secs = self.counter.0.elapsed().as_secs_f64();
        if secs >= 1.0 {
            let (emulated, drawn) = (self.counter.1, self.counter.2);
            let audio = self.sound.as_ref().map_or(String::new(), |s| {
                let underruns = s.queue.lock().unwrap().underruns;
                format!(" · audio {:.0} ms queued, {underruns} underruns", s.queued() * 1000.0)
            });
            let status = format!(
                "{:.0}% speed · {:.0} fps · i960 {} · emulate {:.1} ms/frame{audio}{}",
                emulated as f64 / secs / FPS * 100.0,
                drawn as f64 / secs,
                if self.machine.is_native() { "recompiled" } else { "interpreted" },
                self.counter.3.as_secs_f64() * 1000.0 / emulated.max(1) as f64,
                if self.surface_misses > 0 {
                    format!(" · {} draws skipped", self.surface_misses)
                } else {
                    String::new()
                },
            );
            if let Some(g) = &self.gpu {
                g.window
                    .set_title(&format!("Remodel: {} · {}x{} · {status}", self.opts.game, g.view.width, g.view.height));
            }
            self.reports += 1;
            if self.reports.is_multiple_of(5) {
                println!("status        {status}");
            }
            self.counter = (Instant::now(), 0, 0, Duration::ZERO);
        }
    }

    fn save_eeprom(&self) {
        if self.machine.bus.eeprom.dirty {
            if let Some(dir) = self.eeprom.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(&self.eeprom, self.machine.bus.eeprom.save());
        }
    }
}

impl ApplicationHandler for Play {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("Remodel")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0));
        let window = Arc::new(el.create_window(attrs).expect("create window"));
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
            el.owned_display_handle(),
        )));
        let surface = instance.create_surface(window.clone()).expect("create surface");
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .expect("no GPU adapter");
        let name = gpu::adapter_name(&adapter);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("remodel"),
            required_limits: gpu::limits(&adapter),
            ..Default::default()
        }))
        .expect("create device");
        let size = window.inner_size();
        let mut config =
            surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        // Colours are already display values: present without sRGB encoding.
        let formats = surface.get_capabilities(&adapter).formats;
        config.format = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
            .into_iter()
            .find(|f| formats.contains(f))
            .unwrap_or(config.format.remove_srgb_suffix());
        config.present_mode = wgpu::PresentMode::AutoVsync;
        surface.configure(&device, &config);
        println!("gpu           {name}");
        let renderer = gpu::Renderer::from_device(device, queue, name);
        let (view, origin) = self.fit(config.width, config.height);
        // Redraws drive everything; Wayland sends none until asked.
        window.request_redraw();
        self.gpu = Some(Gpu { window, surface, config, renderer, view, origin });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.save_eeprom();
                el.exit();
            }
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                match event.state {
                    ElementState::Pressed => {
                        if code == KeyCode::Escape {
                            self.save_eeprom();
                            el.exit();
                        } else if code == KeyCode::F11 {
                            if let Some(g) = &self.gpu {
                                let full = g.window.fullscreen().is_some();
                                g.window.set_fullscreen((!full).then_some(Fullscreen::Borderless(None)));
                            }
                        }
                        self.keys.insert(code);
                    }
                    ElementState::Released => {
                        self.keys.remove(&code);
                    }
                }
            }
            WindowEvent::RedrawRequested => {
                self.frame();
                if let Some(g) = &self.gpu {
                    g.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    let opts = match options() {
        Ok(o) => o,
        Err(e) => {
            eprint!("error: {e}\n\n{USAGE}");
            std::process::exit(2);
        }
    };
    let game = match romset::games::find(&opts.game) {
        Ok(g) => g,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let roms = match romset::RomSet::load(game, &opts.rom, false) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };
    let mut machine = model2::Machine::new(&roms);
    if opts.interp {
        machine.disable_native();
    }
    println!("i960          {}", if machine.is_native() { "recompiled" } else { "interpreter" });
    let eeprom = PathBuf::from("out").join(game.id).join("eeprom.bin");
    if let Ok(bytes) = std::fs::read(&eeprom) {
        machine.bus.eeprom.load(&bytes);
    }
    let sound = if opts.sound {
        match Sound::new(&roms, opts.mix) {
            Ok(s) => {
                println!("audio         {} Hz, mix sound {:+} dB, music {:+} dB", s.rate, opts.mix.0, opts.mix.1);
                Some(s)
            }
            Err(e) => {
                eprintln!("{e}; playing without sound");
                None
            }
        }
    } else {
        None
    };
    let gilrs = Gilrs::new().ok();
    if let Some(g) = &gilrs {
        for (_, p) in g.gamepads() {
            println!("controller    {} (used once a button or stick moves)", p.name());
        }
    }
    let mut play = Play {
        hud_windows: game.hud_windows.iter().fold(0, |m, &w| m | 1 << w),
        opts,
        machine,
        sound,
        gpu: None,
        keys: HashSet::new(),
        gilrs,
        pad: None,
        eeprom,
        epoch: Instant::now(),
        counter: (Instant::now(), 0, 0, Duration::ZERO),
        stopped: false,
        surface_misses: 0,
        reports: 0,
    };
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut play).expect("event loop");
}
