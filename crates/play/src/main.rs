//! remodel-play: Model 2 games in a native window.
//!
//! The main thread runs the window (winit), input, the board model, drawing
//! (the GPU renderer, presented with vsync) and the menu (egui, over the
//! game). The geometrizer runs on its own thread (the board model's
//! default) and the sound boards on another (`sound`), playing through the
//! system's audio API.
//!
//! Pacing: with a display near the game's 57.524 Hz (60 Hz, or 120 Hz at
//! one game frame per two refreshes) the game runs exactly one frame per
//! refresh, so motion is perfectly even; that's 4.3% fast at 60 Hz, and the
//! audio is resampled to match. With exact speed, or other refresh rates,
//! frames follow a steady clock at 57.524 per second instead. Either way
//! the audio queue corrects the drift between the clocks by stretching the
//! audio by at most 0.5% (inaudible), never by stalling the game. Below
//! full speed the audio follows the frames the game actually runs
//! (`model2::audio::Pace`), so it slows with it instead of running dry.
//!
//! The logo shows for two seconds at startup, fading out over the game
//! booting underneath (`splash`). Esc opens the menu (and pauses): save
//! states, sound mix, display, game and ROM folder. Settings persist (`settings`).
//! Beside a 4:3 picture in a wider window, the game's bezel (`bezel`).

mod bezel;
mod controls;
mod menu;
mod settings;
mod sound;
mod splash;

use gilrs::Gilrs;
use settings::Settings;
use sound::Sound;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Fullscreen, Window, WindowId};

const USAGE: &str = "\
usage: remodel-play [options]

  --game ID          set to play (default: the last one played, or topskatr)
  --rom DIR          ROM folder: MAME sets as zips or folders (default: the
                     last one used; else rom in the reModel source tree,
                     or $XDG_DATA_HOME/remodel/roms anywhere else)
  --mix SFX:MUSIC    sound board and music board gains in dB (default 8:-8)
  --no-sound         don't run the sound boards
  --hud center|edges wide windows: keep the HUD centred, or move each HUD
                     element to the nearest screen edge (default edges)
  --aspect fit|4:3   fit the window, drawing the game wider than its frame
                     (the default), or keep its 4:3 picture
  --bezel on|off     the game's bezel beside a 4:3 picture in a wider
                     window (default on)
  --interp           run the i960 on the interpreter even if this build
                     has recompiled code
  --geo native|chip  the geometrizer: the native implementation where there
                     is one for the program the game uploads (the default;
                     docs/hw/geometrizer-2b.md, geometrizer-2c.md), or the
                     game's program on its chip, the 2B SHARC or 2C TGP
                     (`sharc` and `tgp` name it too)
  --smooth           smooth motion: on a display faster than the game's
                     57.5 Hz, draw in-between frames, the 3D geometry
                     interpolated between the last two game frames (one
                     game frame, about 17 ms, of extra latency; the native
                     geometrizer only; docs/play.md)
  --exact            run at the arcade's exact 57.524 frames per second
                     (default: one frame per display refresh when the
                     refresh is 60 or 120 Hz, which is 4.3% fast but smooth)
  --fullscreen       start fullscreen
  --seconds N        quit after N seconds (for timing runs)
  --no-splash        don't show the logo at startup
  --input SCRIPT     scripted inputs, as for `remodel run` (for timing runs),
                     e.g. coin-start (the game's own coin-up and start)
  --shot FILE        with --seconds: at the end, draw the last frame as a
                     1920x1080 window would show it (bezel included) to a PNG

Options given here are remembered (the game once it starts), as are
changes made in the menu.

keys: Esc menu (pause), F5 quick save, F9 quick load, F11 fullscreen. The
game's controls depend on the game (docs/play.md); C coins up and arrows
steer. Gamepads work too. The menu's Controls page remaps them per game.
";

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per redraw before dropping time.
const MAX_CATCH_UP: u32 = 3;
/// Narrowest and widest views; windows beyond are letterboxed.
const ASPECTS: (f32, f32) = (4.0 / 3.0, 32.0 / 9.0);

/// Options for this run only (the rest go into the settings).
struct Options {
    sound: bool,
    interp: bool,
    /// `--geo`: how the geometrizer runs.
    geo: model2::geo::GeoMode,
    /// `--smooth`: draw in-between frames (`model2::smooth`).
    smooth: bool,
    seconds: Option<f64>,
    /// `--input`, parsed against the game's inputs when it starts.
    input: Option<String>,
    /// `--game`: remembered only once it starts (`Play::start_game`).
    game: Option<String>,
    splash: bool,
    shot: Option<PathBuf>,
}

fn options(s: &mut Settings) -> Result<Options, String> {
    let mut o = Options {
        sound: true,
        interp: false,
        geo: model2::geo::GeoMode::Native,
        smooth: false,
        seconds: None,
        input: None,
        game: None,
        splash: true,
        shot: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--game" => o.game = Some(value()?),
            "--rom" => s.rom = PathBuf::from(value()?),
            "--mix" => {
                let v = value()?;
                let (x, y) = v.split_once(':').ok_or("--mix wants SFX:MUSIC")?;
                (s.sfx_db, s.music_db) = (x.parse().map_err(|_| "bad --mix")?, y.parse().map_err(|_| "bad --mix")?);
            }
            "--no-sound" => o.sound = false,
            "--hud" => s.hud_edges = value()? != "center",
            "--aspect" => {
                s.wide = match value()?.as_str() {
                    "fit" => true,
                    "4:3" => false,
                    _ => return Err("--aspect wants fit or 4:3".into()),
                }
            }
            "--bezel" => s.bezel = value()? != "off",
            "--shot" => o.shot = Some(value()?.into()),
            "--interp" => o.interp = true,
            "--smooth" => o.smooth = true,
            "--geo" => {
                o.geo = match model2::geo::GeoMode::parse(&value()?) {
                    Some(model2::geo::GeoMode::Check) | None => return Err("--geo wants native or chip".into()),
                    Some(mode) => mode,
                }
            }
            "--no-splash" => o.splash = false,
            "--fullscreen" => s.fullscreen = true,
            "--exact" => s.exact = true,
            "--input" => o.input = Some(value()?),
            "--seconds" => o.seconds = Some(value()?.parse().map_err(|_| "bad --seconds")?),
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if o.smooth && o.geo == model2::geo::GeoMode::Chip {
        eprintln!("--smooth: off with --geo chip (in-betweens come from the native geometrizer)");
        o.smooth = false;
    }
    Ok(o)
}

/// The game being played.
struct Session {
    game: &'static romset::Game,
    machine: model2::Machine,
    sound: Option<Sound>,
    /// `--input`, if given: replaces the player's controls.
    script: Option<model2::input::InputScript>,
    /// The player's bindings: the manifest's, remapped in the menu's
    /// Controls page (`settings::load_bindings`).
    bindings: romset::Bindings,
}

impl Session {
    fn dir(&self) -> PathBuf {
        settings::data_dir().join(self.game.id)
    }

    fn slot_path(&self, slot: u8) -> PathBuf {
        let name = if slot == 0 { "quick.save".to_string() } else { format!("slot{slot}.save") };
        self.dir().join("saves").join(name)
    }

    fn save_eeprom(&self) {
        let bus = &self.machine.bus;
        if bus.eeprom.dirty {
            let _ = std::fs::create_dir_all(self.dir());
            let _ = std::fs::write(self.dir().join("eeprom.bin"), bus.eeprom.save());
        }
    }
}

/// egui over the game: its context, window input and renderer.
struct Overlay {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
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
    overlay: Overlay,
}

/// Status line counters for the current one-second window.
struct Stats {
    start: Instant,
    emulated: u32,
    drawn: u32,
    emulating: Duration,
    /// Redraws that showed no new game frame, and the longest time between
    /// new frames.
    repeats: u32,
    longest_gap: Duration,
    last_new: Instant,
    reports: u64,
    surface_misses: u64,
    /// Frames drawn with the previous frame's polygons (the geometrizer
    /// wasn't done in time).
    geo_late: u32,
    /// `--smooth`: in-betweens drawn, and the host time computing them.
    smooth: u32,
    smoothing: Duration,
}

impl Stats {
    fn new() -> Self {
        Stats {
            start: Instant::now(),
            emulated: 0,
            drawn: 0,
            emulating: Duration::ZERO,
            repeats: 0,
            longest_gap: Duration::ZERO,
            last_new: Instant::now(),
            reports: 0,
            surface_misses: 0,
            geo_late: 0,
            smooth: 0,
            smoothing: Duration::ZERO,
        }
    }
}

struct Play {
    opts: Options,
    settings: Settings,
    /// Settings as last applied (to see what the menu changed).
    applied: Settings,
    session: Option<Session>,
    /// Playable games in the ROM folder, and how many sets aren't supported.
    games: Vec<(String, &'static str)>,
    unsupported: usize,
    menu: menu::Menu,
    splash: splash::Splash,
    /// The game's bezel art: decoded when the game starts, a texture once
    /// it's first drawn.
    bezel: Option<Bezel>,
    gpu: Option<Gpu>,
    keys: HashSet<KeyCode>,
    gilrs: Option<Gilrs>,
    /// The gamepad last used (a button or stick moved): keyboards' media-key
    /// interfaces show up as gamepads too, so wait to see which one plays.
    pad: Option<gilrs::GamepadId>,
    /// Light guns: the mouse (window position, None outside the window;
    /// left and right buttons), once it has moved over the window, aims
    /// player 1; the pad's stick cursor aims player 2 then, else player 1.
    mouse: Option<(f64, f64)>,
    mouse_used: bool,
    mouse_buttons: [bool; 2],
    stick: model2::input::StickCursor,
    /// Real time that emulated frame 0 corresponds to.
    epoch: Instant,
    /// `--smooth`: when the newest game frame was first drawn.
    smooth_at: Instant,
    redraws: u64,
    /// When `emulate` last ran (for the audio rate control).
    last_emulate: Option<Instant>,
    /// Host time per game frame (emulating, waiting for the geometrizer and
    /// drawing, not waiting for vsync), smoothed, in seconds; and whether
    /// that is too slow to follow the display (then the clock paces).
    frame_cost: f64,
    behind: bool,
    stats: Stats,
    started: Instant,
}

struct Bezel {
    size: [usize; 2],
    image: Option<egui::ColorImage>,
    texture: Option<egui::TextureHandle>,
}

impl Bezel {
    /// Paint it at `rect` beside the picture (left and right edges).
    fn paint(&mut self, ctx: &egui::Context, rect: [f32; 4], pic: (f32, f32), window_w: f32, scale: f32) {
        if let Some(image) = self.image.take() {
            self.texture = Some(ctx.load_texture("bezel", image, egui::TextureOptions::LINEAR));
        }
        if let Some(tex) = &self.texture {
            bezel::paint(ctx, tex, rect, pic, window_w, scale);
        }
    }
}

/// Host time one game frame may take: a refresh when following the
/// display (two at about 120 Hz), else the game's frame period.
fn frame_budget(refresh: Option<f64>, exact: bool) -> Duration {
    let per_refresh = refresh
        .filter(|_| !exact)
        .and_then(|hz| [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS))).map(|k| 1.0 / (hz * k)));
    Duration::from_secs_f64(per_refresh.unwrap_or(1.0 / FPS))
}

/// The refresh rate (Hz) of the monitor `window` is on, if known.
fn refresh_hz(window: &Window) -> Option<f64> {
    window.current_monitor()?.refresh_rate_millihertz().map(|m| m as f64 / 1000.0)
}

impl Play {
    /// Find the playable games in the ROM folder.
    fn scan(&mut self) {
        let sets = romset::sets_with_clones(&self.settings.rom);
        self.games = sets.iter().filter_map(|s| romset::games::find(s).ok().map(|g| (s.clone(), g.title))).collect();
        self.unsupported = sets.len() - self.games.len();
        self.menu.rom_text = self.settings.rom.display().to_string();
    }

    /// Start `set` from power-on (saving the current game's EEPROM first).
    fn start_game(&mut self, set: &str) {
        if let Some(s) = self.session.take() {
            s.save_eeprom();
        }
        let started = romset::games::find(set).map_err(|e| e.to_string()).and_then(|game| {
            let script = match &self.opts.input {
                Some(spec) => {
                    Some(model2::input::InputScript::parse(spec, &game.controls).map_err(|e| format!("--input: {e}"))?)
                }
                None => None,
            };
            let roms = romset::RomSet::load(game, &self.settings.rom, false).map_err(|e| e.to_string())?;
            let mut machine = model2::Machine::new(&roms);
            if self.opts.interp {
                machine.disable_native();
            }
            machine.bus.geo.set_mode(self.opts.geo);
            machine.bus.geo.set_smooth(self.opts.smooth);
            // Drawing can go ahead with the last frame's polygons when the
            // geometrizer is late (`frame`).
            machine.bus.geo.set_keep_last(true);
            let dir = settings::data_dir().join(game.id);
            if let Ok(bytes) = std::fs::read(dir.join("eeprom.bin")) {
                machine.bus.eeprom.load(&bytes);
            }
            let sound = if self.opts.sound {
                Sound::new(&roms, (self.settings.sfx_db, self.settings.music_db))
                    .inspect_err(|e| eprintln!("{e}; playing without sound"))
                    .ok()
            } else {
                None
            };
            println!(
                "game          {} ({}), i960 {}{}",
                game.title,
                game.id,
                if machine.is_native() { "recompiled" } else { "interpreted" },
                sound.as_ref().map_or(String::new(), |s| format!(", audio {} Hz", s.rate))
            );
            let bindings = settings::load_bindings(game);
            Ok(Session { game, machine, sound, script, bindings })
        });
        match started {
            Ok(s) => {
                self.bezel =
                    bezel::image(s.game).map(|image| Bezel { size: image.size, image: Some(image), texture: None });
                self.settings.game = set.to_string();
                self.settings.save();
                // A crosshair pointer over gun games.
                if let Some(g) = &self.gpu {
                    let gun = !s.game.controls.guns.is_empty();
                    g.window.set_cursor(if gun { CursorIcon::Crosshair } else { CursorIcon::Default });
                }
                self.session = Some(s);
                self.epoch = Instant::now();
                self.refit();
            }
            Err(e) => {
                eprintln!("{set}: {e}");
                self.menu.notify(format!("Can't start {set}: {e}"));
                self.menu.open = true;
            }
        }
    }

    fn save_slot(&mut self, slot: u8) {
        let Some(s) = &mut self.session else { return };
        let live = s.sound.as_ref().map(|snd| snd.save());
        let data = model2::save_session_parts(&mut s.machine, live);
        let path = s.slot_path(slot);
        let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, data));
        let name = if slot == 0 { "Quick save".to_string() } else { format!("Saved to slot {slot}") };
        self.menu.notify(match result {
            Ok(()) => name,
            Err(e) => format!("Save failed: {e}"),
        });
    }

    fn load_slot(&mut self, slot: u8) {
        let Some(s) = &mut self.session else { return };
        let data = match std::fs::read(s.slot_path(slot)) {
            Ok(d) => d,
            Err(_) => return self.menu.notify(if slot == 0 { "No quick save yet" } else { "That slot is empty" }),
        };
        let result = model2::load_session_parts(&mut s.machine, &data).and_then(|live| match &mut s.sound {
            Some(snd) if !live.is_empty() => snd.load(live, &s.machine.bus),
            _ => Ok(()),
        });
        match result {
            Ok(()) => {
                self.epoch = Instant::now() - Duration::from_secs_f64(s.machine.frames as f64 / FPS);
                self.menu.notify(if slot == 0 { "Quick load".to_string() } else { format!("Loaded slot {slot}") });
            }
            Err(e) => {
                // The machine may be half loaded: start the game afresh.
                let set = s.game.id.to_string();
                self.start_game(&set);
                self.menu.notify(format!("Load failed: {e}"));
            }
        }
    }

    fn slot_times(&self) -> Vec<Option<SystemTime>> {
        let Some(s) = &self.session else { return Vec::new() };
        (0..menu::SLOTS).map(|n| std::fs::metadata(s.slot_path(n)).and_then(|m| m.modified()).ok()).collect()
    }

    /// Apply what the menu changed in the settings, and remember them.
    fn apply_settings(&mut self) {
        if self.settings == self.applied {
            return;
        }
        let (old, new) = (&self.applied, &self.settings);
        if (old.sfx_db, old.music_db) != (new.sfx_db, new.music_db) {
            if let Some(snd) = self.session.as_ref().and_then(|s| s.sound.as_ref()) {
                snd.set_mix(new.sfx_db, new.music_db);
            }
        }
        if old.fullscreen != new.fullscreen {
            if let Some(g) = &self.gpu {
                g.window.set_fullscreen(new.fullscreen.then_some(Fullscreen::Borderless(None)));
            }
        }
        let refit = old.hud_edges != new.hud_edges || old.wide != new.wide;
        self.settings.save();
        self.applied = self.settings.clone();
        if refit {
            self.refit();
        }
    }

    /// Fit a view into the window: full size between `ASPECTS` (just 4:3
    /// unless `wide`), else letterboxed. Returns it and its top-left corner.
    fn fit(&self, w: u32, h: u32) -> (model2::video::View, (u32, u32)) {
        let aspect = w as f32 / h.max(1) as f32;
        let a = if self.settings.wide { aspect.clamp(ASPECTS.0, ASPECTS.1) } else { ASPECTS.0 };
        let vh = if a < aspect { h } else { (w as f32 / a) as u32 };
        let hud = if self.settings.hud_edges { model2::video::Hud::Edges } else { model2::video::Hud::Center };
        let mut view = model2::video::View::new(vh.max(1) as usize, a, hud);
        view.hud_windows = self.session.as_ref().map_or(0, |s| s.game.hud_windows.iter().fold(0, |m, &w| m | 1 << w));
        let origin = ((w - (view.width as u32).min(w)) / 2, (h - (view.height as u32).min(h)) / 2);
        (view, origin)
    }

    /// Where the bezel goes in a window `w` pixels wide showing `view` at
    /// `origin` (None: not shown), and the picture's left and right edges.
    fn bezel_at(&self, view: &model2::video::View, origin: (u32, u32), w: u32) -> Option<([f32; 4], (f32, f32))> {
        let b = self.bezel.as_ref().filter(|_| self.settings.bezel && self.session.is_some())?;
        let pic = (origin.0, origin.1, view.width as u32, view.height as u32);
        let rect = bezel::layout(w, pic, view.pad, b.size)?;
        Some((rect, (origin.0 as f32, (origin.0 as usize + view.width) as f32)))
    }

    /// `--shot`: the game's current frame as a 1920x1080 window shows it,
    /// bezel included, to a PNG.
    fn shot(&mut self, path: &Path) -> Result<(), String> {
        let (w, h) = (1920, 1080);
        let (view, origin) = self.fit(w, h);
        let at = self.bezel_at(&view, origin, w);
        let (Some(g), Some(s)) = (&mut self.gpu, &mut self.session) else { return Err("no game in a window".into()) };
        let format = g.config.format;
        let texture = g.renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("shot"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = texture.create_view(&Default::default());
        g.renderer.present_bus(&mut s.machine.bus, None, &view, &target, format, origin);
        let bezel = &mut self.bezel;
        draw_overlay(g, &target, false, [w, h], |ctx| {
            if let (Some(b), Some((rect, pic))) = (bezel.as_mut(), at) {
                b.paint(ctx, rect, pic, w as f32, ctx.pixels_per_point());
            }
        });
        let mut rgba = g.renderer.read_texture(&texture, w, h);
        if format == wgpu::TextureFormat::Bgra8Unorm {
            rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2));
        }
        std::fs::write(path, png::encode_rgba(w as usize, h as usize, &rgba))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    fn refit(&mut self) {
        if let Some((w, h)) = self.gpu.as_ref().map(|g| (g.config.width, g.config.height)) {
            let (view, origin) = self.fit(w, h);
            let g = self.gpu.as_mut().unwrap();
            (g.view, g.origin) = (view, origin);
        }
    }

    fn resize(&mut self, w: u32, h: u32) {
        if let Some(g) = &mut self.gpu {
            g.config.width = w.max(1);
            g.config.height = h.max(1);
            g.surface.configure(g.renderer.device(), &g.config);
            // A resize can mean the window moved to another monitor.
            g.refresh = refresh_hz(&g.window);
        }
        self.refit();
    }

    /// Game frames per display refresh when the game follows the display
    /// (1 at about 60 Hz, 1/2 at about 120 Hz), or None to follow the clock.
    fn per_refresh(&self) -> Option<f64> {
        let hz = self.gpu.as_ref()?.refresh?;
        if self.settings.exact || self.behind {
            return None;
        }
        [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS)))
    }

    /// Take the gamepads' events: note the one last used, and whether a
    /// button was pressed.
    fn poll_pads(&mut self) -> bool {
        let mut pressed = false;
        let mut events = Vec::new();
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                pressed |= matches!(ev.event, gilrs::EventType::ButtonPressed(..));
                if matches!(ev.event, gilrs::EventType::ButtonPressed(..) | gilrs::EventType::AxisChanged(..)) {
                    self.pad = Some(ev.id);
                }
                if self.menu.capture.is_some() {
                    events.extend(controls::captured(&ev.event));
                }
            }
        }
        for c in events {
            self.capture(c);
        }
        pressed
    }

    /// Bind what the player pressed or moved into the slot the Controls
    /// page is waiting on, or say why it can't be.
    fn capture(&mut self, c: romset::bindings::Capture) {
        let (Some((slot, at)), Some(s)) = (self.menu.capture, &mut self.session) else { return };
        let axis_slot = s.bindings.accepts(slot) == romset::bindings::Accepts::Axis;
        // Sticks resting off centre, or moved on the way to a button.
        if matches!(c, romset::bindings::Capture::Axis(..)) && !axis_slot {
            return;
        }
        match s.bindings.capture(slot, at, c, romset::bindings::RESERVED_KEYS) {
            Ok(()) => {
                settings::save_bindings(s.game, &s.bindings);
                self.menu.capture = None;
            }
            Err(e) => self.menu.notify(e),
        }
    }

    /// Keyboard and the last used gamepad onto the cabinet's controls, by
    /// the game's bindings (`romset::Controls`), or the `--input` script.
    fn apply_inputs(&mut self) {
        self.poll_pads();
        let pad = self.gilrs.as_ref().and_then(|g| self.pad.map(|id| g.gamepad(id)));
        let mouse_aim = self.mouse_aim();
        let Some(s) = &mut self.session else { return };
        match &s.script {
            Some(script) => script.apply(s.machine.frames, &mut s.machine.bus),
            None => {
                let devices = controls::Devices { keys: &self.keys, pad };
                let bus = &mut s.machine.bus;
                let b = &s.bindings;
                model2::input::apply_player(bus, &[(b, &devices)]);
                if !bus.controls.guns.is_empty() {
                    use model2::input::{apply_gun, held, GunState};
                    let (reload0, reload1) = (held(&devices, b.reload(0)), held(&devices, b.reload(1)));
                    let mut stick = self.stick.update(&devices, b);
                    if self.mouse_used {
                        let [trigger, reload] = self.mouse_buttons;
                        apply_gun(bus, 0, GunState { at: mouse_aim, trigger, reload: reload || reload0 });
                        stick.reload |= reload1;
                        apply_gun(bus, 1, stick);
                    } else {
                        stick.reload |= reload0;
                        apply_gun(bus, 0, stick);
                    }
                }
            }
        }
    }

    /// Where the mouse points in the game's view (fractions), None off it.
    fn mouse_aim(&self) -> Option<(f32, f32)> {
        let (g, (x, y)) = (self.gpu.as_ref()?, self.mouse?);
        g.view.to_view_fraction(x as f32 - g.origin.0 as f32, y as f32 - g.origin.1 as f32)
    }

    /// The stick cursor's crosshair, in window pixels, while a gamepad
    /// aims a gun in a gun game (the mouse has the system cursor).
    fn stick_crosshair(&self) -> Option<(f32, f32)> {
        let (g, s) = (self.gpu.as_ref()?, self.session.as_ref()?);
        if s.game.controls.guns.is_empty() || self.pad.is_none() || s.script.is_some() {
            return None;
        }
        let (x, y) = self.stick.at;
        let (ox, oy) = g.view.to_output(x * model2::video::WIDTH as f32, y * model2::video::HEIGHT as f32);
        Some((ox + g.origin.0 as f32, oy + g.origin.1 as f32))
    }

    /// Run the frames that are due (unless paused in the menu).
    fn emulate(&mut self) -> Result<u32, i960::cpu::Stop> {
        let now = Instant::now();
        self.redraws += 1;
        if self.menu.open || self.session.is_none() {
            // Paused: keep the clock with the game for when it resumes.
            if let Some(s) = &self.session {
                self.epoch = now - Duration::from_secs_f64(s.machine.frames as f64 / FPS);
            }
            return Ok(0);
        }
        let frames = self.session.as_ref().map_or(0, |s| s.machine.frames);
        // Follow the display (one frame per refresh, or per two) or the clock.
        let (due, speed) = match self.per_refresh() {
            Some(k) => {
                let hz = self.gpu.as_ref().and_then(|g| g.refresh).unwrap_or(60.0);
                (frames + (k == 1.0 || self.redraws.is_multiple_of(2)) as u64, hz * k / FPS)
            }
            None => (((now - self.epoch).as_secs_f64() * FPS) as u64, 1.0),
        };
        let mut ran = 0;
        while self.session.as_ref().is_some_and(|s| s.machine.frames < due) && ran < MAX_CATCH_UP {
            self.apply_inputs();
            let s = self.session.as_mut().unwrap();
            s.machine.run_slice(u64::MAX)?;
            match &mut s.sound {
                Some(snd) => snd.send(&mut s.machine.bus),
                None => s.machine.bus.uart_log.clear(),
            }
            ran += 1;
        }
        // The audio rate for the next frames: the game's speed, scaled by
        // how well it kept it since the last redraw (a geometrizer slower
        // than real time holds it back), so the audio slows with the game
        // instead of running dry.
        let dt = self.last_emulate.map_or(0.0, |l| (now - l).as_secs_f64());
        self.last_emulate = Some(now);
        if let Some(snd) = self.session.as_mut().and_then(|s| s.sound.as_mut()) {
            if dt > 0.0 {
                snd.set_speed(speed, dt, ran as f64, dt * FPS * speed);
            }
        }
        let frames = self.session.as_ref().map_or(0, |s| s.machine.frames);
        if self.per_refresh().is_some() || frames < due {
            // Following the display (or fell behind): keep the clock current.
            self.epoch = now - Duration::from_secs_f64(frames as f64 / FPS);
        }
        Ok(ran)
    }

    /// One redraw: run the game, draw it and the menu, report status.
    fn frame(&mut self, el: &ActiveEventLoop) {
        if self.splash.active() && self.poll_pads() {
            self.splash.skip();
        }
        // The game isn't reading the pads while the menu is open: the
        // Controls page may be waiting for one.
        if self.menu.open {
            self.poll_pads();
        }
        let t0 = Instant::now();
        let mut ran = 0;
        let result = self.emulate();
        if let (Ok(1..), Some(s)) = (&result, &self.session) {
            // When the newest frame was due: on the clock, or now when
            // following the display (which sets the clock to now).
            self.smooth_at = self.epoch + Duration::from_secs_f64(s.machine.frames as f64 / FPS);
        }
        // A game frame's length in host time: a refresh's (two at about
        // 120 Hz) when following the display, else the game's.
        let period =
            frame_budget(self.gpu.as_ref().and_then(|g| g.refresh), self.settings.exact || self.behind).as_secs_f64();
        let emulate_time = t0.elapsed();
        match result {
            Ok(n) => {
                ran = n;
                let st = &mut self.stats;
                st.emulating += t0.elapsed();
                st.emulated += ran;
                if ran == 0 && !self.menu.open {
                    st.repeats += 1;
                } else if ran > 0 {
                    st.longest_gap = st.longest_gap.max(st.last_new.elapsed());
                    st.last_new = Instant::now();
                }
            }
            Err(e) => {
                eprintln!("stopped: {e:?}");
                self.menu.notify(format!("The game stopped: {e:?}"));
                self.session = None;
                self.menu.open = true;
            }
        }
        let slots = if self.menu.open { self.slot_times() } else { Vec::new() };
        let crosshair = if self.menu.visible() { None } else { self.stick_crosshair() };
        let bezel_at = self.gpu.as_ref().and_then(|g| self.bezel_at(&g.view, g.origin, g.config.width));
        let Some(g) = &mut self.gpu else { return };
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            // Lost, outdated, hidden or timed out: reconfigure, skip a draw.
            _ => {
                g.surface.configure(g.renderer.device(), &g.config);
                self.stats.surface_misses += 1;
                return;
            }
        };
        let t_draw = Instant::now();
        let target = frame.texture.create_view(&Default::default());
        if let Some(s) = &mut self.session {
            // Draw this frame's polygons if the geometrizer has them in time
            // to make this refresh, else the previous frame's: when it's the
            // slowest part, it then runs alongside the next frame instead of
            // after it, at the cost of the polygons lagging a frame.
            let budget = frame_budget(g.refresh, self.settings.exact);
            let bus = &mut s.machine.bus;
            if bus.geo.wait_until(t0 + budget.mul_f64(0.8)) {
                // `--smooth`: the polygons part way from the frame before
                // to this one, by the time since this one was first shown.
                // Only on a display faster than the game, and not when it's
                // already behind.
                let t_smooth = Instant::now();
                let fast = g.refresh.is_none_or(|hz| hz > FPS * 1.2) && !self.behind && !self.menu.open;
                let between = bus.geo.engine().smooth_frame().filter(|_| fast).and_then(|f| {
                    let t = t_smooth.saturating_duration_since(self.smooth_at).as_secs_f64() / period;
                    f.render(t as f32).map(|stream| (stream, f))
                });
                match &between {
                    Some((stream, f)) => {
                        self.stats.smooth += 1;
                        self.stats.smoothing += t_smooth.elapsed();
                        g.renderer.present_bus(
                            bus,
                            Some((stream, &f.points)),
                            &g.view,
                            &target,
                            g.config.format,
                            g.origin,
                        )
                    }
                    None => g.renderer.present_bus(bus, None, &g.view, &target, g.config.format, g.origin),
                }
            } else {
                g.renderer.present_bus_last(bus, &g.view, &target, g.config.format, g.origin);
                self.stats.geo_late += 1;
            }
        }
        let mut actions = Vec::new();
        if self.menu.visible()
            || self.session.is_none()
            || self.splash.active()
            || crosshair.is_some()
            || bezel_at.is_some()
        {
            let view = menu::View {
                playing: self.session.as_ref().map(|s| s.game.title),
                games: &self.games,
                unsupported: self.unsupported,
                slots: &slots,
            };
            let (menu, settings, splash, bezel) =
                (&mut self.menu, &mut self.settings, &mut self.splash, &mut self.bezel);
            let no_game = self.session.is_none();
            let mut bindings = self.session.as_mut().map(|s| &mut s.bindings);
            let scale = g.window.scale_factor() as f32;
            let size = [g.config.width, g.config.height];
            actions = draw_overlay(g, &target, no_game, size, |ctx| {
                if let (Some(b), Some((rect, pic))) = (bezel.as_mut(), bezel_at) {
                    b.paint(ctx, rect, pic, size[0] as f32, scale);
                }
                if let Some((x, y)) = crosshair {
                    draw_crosshair(ctx, egui::pos2(x / scale, y / scale));
                }
                let actions = menu.ui(ctx, settings, &view, bindings.as_deref_mut());
                // Over the game or the menu, fading to show it.
                splash.ui(ctx);
                actions
            });
        }
        // Not counting `present`, which can wait for vsync.
        let draw_time = t_draw.elapsed();
        g.renderer.queue().present(frame);
        self.stats.drawn += 1;
        if ran > 0 {
            // Below full speed when following the display, pace by the clock
            // instead: frames are then run as fast as they can be (dropping
            // to half the refresh rate otherwise).
            let cost = (emulate_time + draw_time).as_secs_f64() / ran as f64;
            self.frame_cost += (cost - self.frame_cost) * 0.05;
            let budget = frame_budget(g.refresh, false).as_secs_f64();
            if self.frame_cost > budget * 0.95 {
                self.behind = true;
            } else if self.frame_cost < budget * 0.8 {
                self.behind = false;
            }
        }
        for action in actions {
            match action {
                menu::Action::Resume => self.set_menu(false),
                menu::Action::Save(n) => self.save_slot(n),
                menu::Action::Load(n) => {
                    self.load_slot(n);
                    self.set_menu(false);
                }
                menu::Action::Game(set) => {
                    self.start_game(&set);
                    self.set_menu(self.session.is_none());
                }
                menu::Action::Rescan => self.scan(),
                menu::Action::Bindings => {
                    if let Some(s) = &self.session {
                        settings::save_bindings(s.game, &s.bindings);
                    }
                }
                menu::Action::Quit => self.quit(el),
            }
        }
        self.apply_settings();
        self.status();
    }

    fn status(&mut self) {
        let secs = self.stats.start.elapsed().as_secs_f64();
        if secs < 1.0 {
            return;
        }
        let st = &self.stats;
        let snd = self.session.as_ref().and_then(|s| s.sound.as_ref());
        let audio = snd.map_or(String::new(), |s| {
            let underruns = s.queue.lock().unwrap().underruns;
            format!(" · audio {:.0} ms queued, {underruns} underruns", s.queued() * 1000.0)
        });
        let mode = match (self.per_refresh(), self.gpu.as_ref().and_then(|g| g.refresh)) {
            (Some(_), Some(hz)) => format!("synced to {hz:.0} Hz"),
            _ if self.behind => "too slow to sync, paced by the clock".into(),
            _ => "exact speed".into(),
        };
        let status = format!(
            "{mode} · {:.0}% speed · {:.0} fps ({} repeats, longest gap {:.0} ms) · emulate {:.1} ms/frame{audio}{}",
            st.emulated as f64 / secs / FPS * 100.0,
            st.drawn as f64 / secs,
            st.repeats,
            st.longest_gap.as_secs_f64() * 1000.0,
            st.emulating.as_secs_f64() * 1000.0 / st.emulated.max(1) as f64,
            if st.surface_misses > 0 { format!(" · {} draws skipped", st.surface_misses) } else { String::new() },
        ) + &if st.geo_late > 0 {
            format!(" · {} drawn with the last frame's polygons", st.geo_late)
        } else {
            String::new()
        } + &if st.smooth > 0 {
            format!(
                " · {} in-betweens, {:.2} ms each",
                st.smooth,
                st.smoothing.as_secs_f64() * 1000.0 / st.smooth as f64
            )
        } else {
            String::new()
        };
        if let Some(g) = &self.gpu {
            let game = self.session.as_ref().map_or("no game", |s| s.game.title);
            g.window.set_title(&format!("reModel: {game} · {}x{} · {status}", g.view.width, g.view.height));
        }
        let reports = self.stats.reports + 1;
        if reports.is_multiple_of(5) && !self.menu.open {
            println!("status        {status}");
        }
        let misses = self.stats.surface_misses;
        self.stats = Stats { reports, surface_misses: misses, ..Stats::new() };
    }

    /// Open or close the menu; the game pauses while it's open.
    fn set_menu(&mut self, open: bool) {
        self.menu.open = open || self.session.is_none();
        // Keys held when the menu opened would stay pressed otherwise.
        self.keys.clear();
    }

    /// Quit if `--seconds` has run out, reporting the run; true if so.
    fn time_up(&mut self, el: &ActiveEventLoop) -> bool {
        if !self.opts.seconds.is_some_and(|s| self.started.elapsed().as_secs_f64() >= s) {
            return false;
        }
        if let Some(path) = self.opts.shot.clone() {
            match self.shot(&path) {
                Ok(()) => println!("shot          {}", path.display()),
                Err(e) => eprintln!("--shot: {e}"),
            }
        }
        if let Some(s) = &mut self.session {
            let hash = s.machine.bus.state_hash();
            println!("frames        {}, state hash {hash:016x}", s.machine.frames);
            if let Some(d) = &s.machine.bus.drive {
                println!("drive board   {}", d.describe());
            }
        }
        self.quit(el);
        true
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        if let Some(s) = &self.session {
            s.save_eeprom();
        }
        self.settings.save();
        el.exit();
    }
}

/// A crosshair at `at` (points), over everything.
fn draw_crosshair(ctx: &egui::Context, at: egui::Pos2) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("crosshair")));
    for (w, c) in [(4.0, egui::Color32::BLACK), (2.0, egui::Color32::WHITE)] {
        let stroke = egui::Stroke::new(w, c);
        painter.circle_stroke(at, 10.0, stroke);
        for d in [egui::vec2(1.0, 0.0), egui::vec2(0.0, 1.0)] {
            painter.line_segment([at - d * 16.0, at - d * 5.0], stroke);
            painter.line_segment([at + d * 5.0, at + d * 16.0], stroke);
        }
    }
}

/// Draw egui over `target` (`size` pixels), building the UI with `ui`;
/// returns what it returned. `clear` clears to black first (no game under
/// it).
fn draw_overlay<T: Default>(
    g: &mut Gpu,
    target: &wgpu::TextureView,
    clear: bool,
    size: [u32; 2],
    mut ui: impl FnMut(&egui::Context) -> T,
) -> T {
    let o = &mut g.overlay;
    let mut raw = o.state.take_egui_input(&g.window);
    let ppp = g.window.scale_factor() as f32;
    raw.screen_rect =
        Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32) / ppp));
    let mut result = T::default();
    let out = o.ctx.run_ui(raw, |ui_root| result = ui(ui_root.ctx()));
    o.state.handle_platform_output(&g.window, out.platform_output);
    let jobs = o.ctx.tessellate(out.shapes, out.pixels_per_point);
    let (device, queue) = (g.renderer.device(), g.renderer.queue());
    for (id, deltas) in &out.textures_delta.set {
        for delta in deltas {
            o.renderer.update_texture(device, queue, *id, delta);
        }
    }
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: out.pixels_per_point };
    let mut enc = device.create_command_encoder(&Default::default());
    let extra = o.renderer.update_buffers(device, queue, &mut enc, &jobs, &screen);
    {
        let load = if clear { wgpu::LoadOp::Clear(wgpu::Color::BLACK) } else { wgpu::LoadOp::Load };
        let mut pass = enc
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("menu"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load, store: wgpu::StoreOp::Store },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        o.renderer.render(&mut pass, &jobs, &screen);
    }
    queue.submit(extra.into_iter().chain([enc.finish()]));
    for id in &out.textures_delta.free {
        o.renderer.free_texture(id);
    }
    result
}

impl ApplicationHandler for Play {
    /// `--seconds` is checked here as well as on redraws, and the loop set
    /// to wake at the limit: a hidden window may get almost no redraws
    /// (about one a second on Wayland). Without `--seconds` the loop keeps
    /// winit's default (wait for events), so pacing is unchanged.
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        let Some(s) = self.opts.seconds else { return };
        if !self.time_up(el) {
            el.set_control_flow(ControlFlow::WaitUntil(self.started + Duration::from_secs_f64(s)));
        }
    }

    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.gpu.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title("reModel")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_fullscreen(self.settings.fullscreen.then_some(Fullscreen::Borderless(None)));
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
        let ctx = egui::Context::default();
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, Default::default());
        let renderer = gpu::Renderer::from_device(device, queue, name);
        let (view, origin) = self.fit(config.width, config.height);
        // Redraws drive everything; Wayland sends none until asked.
        window.request_redraw();
        let refresh = refresh_hz(&window);
        let overlay = Overlay { ctx, state, renderer: egui_renderer };
        self.gpu = Some(Gpu { window, surface, config, renderer, view, origin, refresh, overlay });
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        // Any key, click or touch skips the splash, and does nothing else.
        if self.splash.active() {
            let press = match &event {
                WindowEvent::KeyboardInput { event, .. } => event.state == ElementState::Pressed,
                WindowEvent::MouseInput { state, .. } => *state == ElementState::Pressed,
                WindowEvent::Touch(t) => t.phase == winit::event::TouchPhase::Started,
                _ => false,
            };
            if press {
                return self.splash.skip();
            }
        }
        // Remapping: the next key binds (Esc cancels), and doesn't reach the
        // menu.
        if self.menu.capture.is_some() {
            if let WindowEvent::KeyboardInput { event, .. } = &event {
                if event.state == ElementState::Pressed && !event.repeat {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Escape) => self.menu.capture = None,
                        PhysicalKey::Code(k) => match controls::code_name(k) {
                            Some(name) => self.capture(romset::bindings::Capture::Key(name.to_string())),
                            None => self.menu.notify("That key can't be bound: pick another"),
                        },
                        _ => self.menu.notify("That key can't be bound: pick another"),
                    }
                }
                return;
            }
        }
        let to_menu = match &mut self.gpu {
            Some(g) => {
                let r = g.overlay.state.on_window_event(&g.window, &event);
                self.menu.open && r.consumed
            }
            None => false,
        };
        match event {
            WindowEvent::CloseRequested => self.quit(el),
            WindowEvent::Resized(size) => self.resize(size.width, size.height),
            WindowEvent::KeyboardInput { event, .. } => {
                let PhysicalKey::Code(code) = event.physical_key else { return };
                let pressed = event.state == ElementState::Pressed;
                if pressed && !event.repeat {
                    match code {
                        KeyCode::Escape => return self.set_menu(!self.menu.open),
                        KeyCode::F11 => {
                            self.settings.fullscreen = !self.settings.fullscreen;
                            return;
                        }
                        KeyCode::F5 => return self.save_slot(0),
                        KeyCode::F9 => return self.load_slot(0),
                        _ => {}
                    }
                }
                if self.menu.open || to_menu {
                    return;
                }
                if pressed {
                    self.keys.insert(code);
                } else {
                    self.keys.remove(&code);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = Some((position.x, position.y));
                self.mouse_used = true;
            }
            WindowEvent::CursorLeft { .. } => self.mouse = None,
            WindowEvent::MouseInput { state, button, .. } if !self.menu.open && !to_menu => {
                let i = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    _ => return,
                };
                self.mouse_buttons[i] = state == ElementState::Pressed;
            }
            WindowEvent::RedrawRequested => {
                if self.time_up(el) {
                    return;
                }
                self.frame(el);
                if let Some(g) = &self.gpu {
                    g.window.request_redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    recomp::register();
    let mut settings = Settings::load();
    let opts = match options(&mut settings) {
        Ok(o) => o,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("error: {e}\n");
            }
            eprint!("{USAGE}");
            std::process::exit(2);
        }
    };
    let gilrs = Gilrs::new().ok();
    if let Some(g) = &gilrs {
        for (_, p) in g.gamepads() {
            println!("controller    {} (used once a button or stick moves)", p.name());
        }
    }
    let splash = splash::Splash::new(opts.splash);
    let mut play = Play {
        opts,
        applied: settings.clone(),
        settings,
        session: None,
        games: Vec::new(),
        unsupported: 0,
        menu: menu::Menu::default(),
        splash,
        bezel: None,
        gpu: None,
        keys: HashSet::new(),
        gilrs,
        pad: None,
        mouse: None,
        mouse_used: false,
        mouse_buttons: [false; 2],
        stick: model2::input::StickCursor::default(),
        epoch: Instant::now(),
        smooth_at: Instant::now(),
        redraws: 0,
        last_emulate: None,
        frame_cost: 0.0,
        behind: false,
        stats: Stats::new(),
        started: Instant::now(),
    };
    play.scan();
    // The last game played (or --game), if it's here; else the menu.
    let set = play.opts.game.clone().unwrap_or_else(|| play.settings.game.clone());
    if play.settings.rom == Settings::default().rom {
        // So there's an obvious place to put sets.
        let _ = std::fs::create_dir_all(&play.settings.rom);
    }
    if play.games.iter().any(|g| g.0 == set) {
        play.start_game(&set);
    } else {
        println!("no {set} in {}: choose a game in the menu", play.settings.rom.display());
        play.menu.open = true;
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut play).expect("event loop");
}
