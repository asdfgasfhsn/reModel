//! remodel-play: a Model 2 game in a native window.
//!
//! The main thread runs the window (winit), input, the board model and
//! drawing (the GPU renderer, presented with vsync). The geometrizer runs on
//! its own thread (the board model's default) and the sound boards on
//! another (`Sound`), which plays through the system's audio API (cpal).
//!
//! Pacing: with a display near the game's 57.524 Hz (60 Hz, or 120 Hz at
//! one game frame per two refreshes) the game runs exactly one frame per
//! refresh, so motion is perfectly even; that's 4.3% fast at 60 Hz, and the
//! audio is resampled to match. With `--exact`, or other refresh rates,
//! frames follow a steady clock at 57.524 per second instead. Either way
//! the audio queue corrects the drift between the clocks by stretching the
//! audio by at most 0.5% (inaudible), never by stalling the game.

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
  --mix SFX:MUSIC    sound board and music board gains in dB (default 8:-8)
  --no-sound         don't run the sound boards
  --hud center|edges wide windows: keep the HUD centred, or move each HUD
                     element to the nearest screen edge (default edges)
  --interp           run the i960 on the interpreter even if this build
                     has recompiled code
  --exact            run at the arcade's exact 57.524 frames per second
                     (default: one frame per display refresh when the
                     refresh is 60 or 120 Hz, which is 4.3% fast but smooth)
  --fullscreen       start fullscreen
  --seconds N        quit after N seconds (for timing runs)
  --input SCRIPT     scripted inputs, as for `remodel run` (for timing runs),
                     e.g. coin1@1100-1104,start@1250-1254

keys: C coin, Enter start, arrows steer (Down slide), Z jump front, X jump
tail, Q/E select, F11 fullscreen, Esc quit. Gamepads work too.
";

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per redraw before dropping time.
const MAX_CATCH_UP: u32 = 3;
/// Audio the sound queue should hold (seconds); output starts once it does.
const AUDIO_TARGET: f64 = 0.05;
/// Most the audio rate is stretched to keep the queue at `AUDIO_TARGET`.
const DRIFT: f64 = 0.005;
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
    fullscreen: bool,
    exact: bool,
    seconds: Option<f64>,
    input: Option<model2::input::InputScript>,
}

fn options() -> Result<Options, String> {
    let mut o = Options {
        game: "topskatr".into(),
        rom: PathBuf::from("rom"),
        mix: model2::audio::DEFAULT_MIX,
        sound: true,
        hud: model2::video::Hud::Edges,
        interp: false,
        fullscreen: false,
        exact: false,
        seconds: None,
        input: None,
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
            "--fullscreen" => o.fullscreen = true,
            "--exact" => o.exact = true,
            "--input" => o.input = Some(model2::input::InputScript::parse(&value()?)?),
            "--seconds" => o.seconds = Some(value()?.parse().map_err(|_| "bad --seconds")?),
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
    /// Playing (else filling up to `AUDIO_TARGET` first).
    primed: bool,
    /// Samples per second (per channel) at the device.
    rate: f64,
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
    /// Game samples per real-time sample (f64 bits): the game's speed
    /// relative to real time, times the drift correction.
    ratio: Arc<AtomicU64>,
}

impl Sound {
    fn new(roms: &romset::RomSet, mix: (f32, f32)) -> Result<Sound, String> {
        use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
        let device = cpal::default_host().default_output_device().ok_or("no audio output device")?;
        let supported = device.default_output_config().map_err(|e| format!("audio: {e}"))?;
        let rate = supported.sample_rate();
        let channels = supported.channels() as usize;
        let config = supported.config();
        let queue = Arc::new(Mutex::new(Queue { rate: rate as f64, ..Queue::default() }));
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
        let ratio = Arc::new(AtomicU64::new(1f64.to_bits()));
        let (q, m, r) = (queue.clone(), made.clone(), ratio.clone());
        std::thread::Builder::new()
            .name("sound".into())
            .spawn(move || {
                let mut pcm = Vec::new();
                // Game time at 44.1 kHz to the device rate.
                let base = model2::audio::RATE as f64 / rate as f64;
                let mut resampler = model2::audio::Resampler::default();
                let max = (AUDIO_MAX * rate as f64) as usize * 2;
                while let Ok((bytes, until)) = rx.recv() {
                    pcm.clear();
                    live.feed(&bytes, until, &mut pcm);
                    m.fetch_add(pcm.len() as u64 / 2, Ordering::Relaxed);
                    let step = base * f64::from_bits(r.load(Ordering::Relaxed));
                    let mut out = Vec::with_capacity(pcm.len() * 2);
                    resampler.run(&pcm, step, &mut out);
                    let mut q = q.lock().unwrap();
                    q.samples.extend(out);
                    let excess = q.samples.len().saturating_sub(max);
                    q.samples.drain(..excess);
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Sound { _stream: stream, queue, link, asked: 0, made, rate, ratio })
    }

    /// Set the game's speed relative to real time (1 = exact), corrected
    /// for drift to hold the queue at `AUDIO_TARGET`.
    fn set_speed(&self, speed: f64) {
        let error = ((self.queued() - AUDIO_TARGET) / AUDIO_TARGET).clamp(-1.0, 1.0);
        self.ratio.store((speed * (1.0 + error * DRIFT)).to_bits(), Ordering::Relaxed);
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
    if !q.primed && q.samples.len() as f64 >= AUDIO_TARGET * q.rate * 2.0 {
        q.primed = true;
    }
    if !q.primed {
        out.iter_mut().for_each(|s| *s = conv(0.0));
        return;
    }
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
    if dry {
        q.underruns += 1;
        q.primed = false;
    }
}

struct Gpu {
    window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    renderer: gpu::Renderer,
    view: model2::video::View,
    origin: (u32, u32),
    /// The display's refresh rate (Hz), if known.
    refresh: Option<f64>,
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
    started: Instant,
    repeats: u32,
    longest_gap: Duration,
    last_new: Instant,
    redraws: u64,
}

/// The refresh rate (Hz) of the monitor `window` is on, if known.
fn refresh_hz(window: &Window) -> Option<f64> {
    window.current_monitor()?.refresh_rate_millihertz().map(|m| m as f64 / 1000.0)
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
            // A resize can mean the window moved to another monitor.
            g.refresh = refresh_hz(&g.window);
        }
    }

    /// Game frames per display refresh when the game follows the display
    /// (1 at about 60 Hz, 1/2 at about 120 Hz), or None to follow the clock.
    fn per_refresh(&self) -> Option<f64> {
        let hz = self.gpu.as_ref()?.refresh?;
        if self.opts.exact {
            return None;
        }
        [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS)))
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
        self.redraws += 1;
        // Follow the display (one frame per refresh, or per two) or the clock.
        let (due, speed) = match self.per_refresh() {
            Some(k) => {
                let hz = self.gpu.as_ref().and_then(|g| g.refresh).unwrap_or(60.0);
                let due = self.machine.frames + (k == 1.0 || self.redraws.is_multiple_of(2)) as u64;
                (due, hz * k / FPS)
            }
            None => (((now - self.epoch).as_secs_f64() * FPS) as u64, 1.0),
        };
        if let Some(s) = &self.sound {
            s.set_speed(speed);
        }
        let t0 = Instant::now();
        let mut ran = 0;
        while self.machine.frames < due && ran < MAX_CATCH_UP {
            match &self.opts.input {
                Some(script) => script.apply(self.machine.frames, &mut self.machine.bus),
                None => self.apply_inputs(),
            }
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
        if self.per_refresh().is_some() || self.machine.frames < due {
            // Following the display (or fell behind): keep the clock current.
            self.epoch = now - Duration::from_secs_f64(self.machine.frames as f64 / FPS);
        }
        self.counter.3 += t0.elapsed();
        self.counter.1 += ran;
        // Pacing: redraws that showed no new game frame (a repeat on
        // screen), and the longest time between new frames.
        if ran == 0 {
            self.repeats += 1;
        } else {
            self.longest_gap = self.longest_gap.max(self.last_new.elapsed());
            self.last_new = Instant::now();
        }
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
            let mode = match (self.per_refresh(), self.gpu.as_ref().and_then(|g| g.refresh)) {
                (Some(_), Some(hz)) => format!("synced to {hz:.0} Hz"),
                _ => "exact speed".into(),
            };
            let status = format!(
                "{mode} · {:.0}% speed · {:.0} fps ({} repeats, longest gap {:.0} ms) · i960 {} · emulate {:.1} ms/frame{audio}{}",
                emulated as f64 / secs / FPS * 100.0,
                drawn as f64 / secs,
                self.repeats,
                self.longest_gap.as_secs_f64() * 1000.0,
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
            self.repeats = 0;
            self.longest_gap = Duration::ZERO;
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
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_fullscreen(self.opts.fullscreen.then_some(Fullscreen::Borderless(None)));
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
        let refresh = refresh_hz(&window);
        self.gpu = Some(Gpu { window, surface, config, renderer, view, origin, refresh });
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
                if self.opts.seconds.is_some_and(|s| self.started.elapsed().as_secs_f64() >= s) {
                    self.save_eeprom();
                    el.exit();
                    return;
                }
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
        started: Instant::now(),
        repeats: 0,
        longest_gap: Duration::ZERO,
        last_new: Instant::now(),
        redraws: 0,
    };
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut play).expect("event loop");
}
