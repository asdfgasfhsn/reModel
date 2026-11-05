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
//! audio by at most 0.5% (inaudible), never by stalling the game.
//!
//! The logo shows for two seconds at startup, fading out over the game
//! booting underneath (`splash`). Esc opens the menu (and pauses): save
//! states, sound mix, display, game and ROM folder. Settings persist (`settings`).

mod controls;
mod menu;
mod settings;
mod sound;
mod splash;

use gilrs::Gilrs;
use settings::Settings;
use sound::Sound;
use std::collections::HashSet;
use std::path::PathBuf;
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
                     last one used; else rom in the Remodel source tree,
                     or $XDG_DATA_HOME/remodel/roms anywhere else)
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
  --no-splash        don't show the logo at startup
  --input SCRIPT     scripted inputs, as for `remodel run` (for timing runs),
                     e.g. coin-start (the game's own coin-up and start)

Options given here are remembered (the game once it starts), as are
changes made in the menu.

keys: Esc menu (pause), F5 quick save, F9 quick load, F11 fullscreen. The
game's controls depend on the game (docs/play.md); C coins up and arrows
steer in both games. Gamepads work too.
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
    seconds: Option<f64>,
    /// `--input`, parsed against the game's inputs when it starts.
    input: Option<String>,
    /// `--game`: remembered only once it starts (`Play::start_game`).
    game: Option<String>,
    splash: bool,
}

fn options(s: &mut Settings) -> Result<Options, String> {
    let mut o = Options { sound: true, interp: false, seconds: None, input: None, game: None, splash: true };
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
            "--interp" => o.interp = true,
            "--no-splash" => o.splash = false,
            "--fullscreen" => s.fullscreen = true,
            "--exact" => s.exact = true,
            "--input" => o.input = Some(value()?),
            "--seconds" => o.seconds = Some(value()?.parse().map_err(|_| "bad --seconds")?),
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
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
    redraws: u64,
    stats: Stats,
    started: Instant,
}

/// The refresh rate (Hz) of the monitor `window` is on, if known.
fn refresh_hz(window: &Window) -> Option<f64> {
    window.current_monitor()?.refresh_rate_millihertz().map(|m| m as f64 / 1000.0)
}

impl Play {
    /// Find the playable games in the ROM folder.
    fn scan(&mut self) {
        let sets = romset::sets_in(&self.settings.rom);
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
            Ok(Session { game, machine, sound, script })
        });
        match started {
            Ok(s) => {
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
        let refit = old.hud_edges != new.hud_edges;
        self.settings.save();
        self.applied = self.settings.clone();
        if refit {
            self.refit();
        }
    }

    /// Fit a view into the window: full size between `ASPECTS`, else
    /// letterboxed. Returns it and its top-left corner.
    fn fit(&self, w: u32, h: u32) -> (model2::video::View, (u32, u32)) {
        let aspect = w as f32 / h.max(1) as f32;
        let a = aspect.clamp(ASPECTS.0, ASPECTS.1);
        let vh = if a < aspect { h } else { (w as f32 / a) as u32 };
        let hud = if self.settings.hud_edges { model2::video::Hud::Edges } else { model2::video::Hud::Center };
        let mut view = model2::video::View::new(vh.max(1) as usize, a, hud);
        view.hud_windows = self.session.as_ref().map_or(0, |s| s.game.hud_windows.iter().fold(0, |m, &w| m | 1 << w));
        let origin = ((w - (view.width as u32).min(w)) / 2, (h - (view.height as u32).min(h)) / 2);
        (view, origin)
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
        if self.settings.exact {
            return None;
        }
        [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS)))
    }

    /// Take the gamepads' events: note the one last used, and whether a
    /// button was pressed.
    fn poll_pads(&mut self) -> bool {
        let mut pressed = false;
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                pressed |= matches!(ev.event, gilrs::EventType::ButtonPressed(..));
                if matches!(ev.event, gilrs::EventType::ButtonPressed(..) | gilrs::EventType::AxisChanged(..)) {
                    self.pad = Some(ev.id);
                }
            }
        }
        pressed
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
                model2::input::apply_player(bus, &devices, |_| false);
                if !bus.controls.guns.is_empty() {
                    use model2::input::{apply_gun, GunState, PlayerInput};
                    let key_reload = |p: usize| {
                        bus.controls.guns.get(p).is_some_and(|g| g.keys_reload.iter().any(|k| devices.key(k)))
                    };
                    let (reload0, reload1) = (key_reload(0), key_reload(1));
                    let mut stick = self.stick.update(&devices);
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
        if let Some(snd) = self.session.as_ref().and_then(|s| s.sound.as_ref()) {
            snd.set_speed(speed);
        }
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
        let t0 = Instant::now();
        match self.emulate() {
            Ok(ran) => {
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
        let target = frame.texture.create_view(&Default::default());
        if let Some(s) = &mut self.session {
            g.renderer.present_bus(&mut s.machine.bus, None, &g.view, &target, g.config.format, g.origin);
        }
        let mut actions = Vec::new();
        if self.menu.visible() || self.session.is_none() || self.splash.active() || crosshair.is_some() {
            let view = menu::View {
                playing: self.session.as_ref().map(|s| s.game.title),
                games: &self.games,
                unsupported: self.unsupported,
                slots: &slots,
            };
            let (menu, settings, splash) = (&mut self.menu, &mut self.settings, &mut self.splash);
            let scale = g.window.scale_factor() as f32;
            actions = draw_overlay(g, &target, self.session.is_none(), |ctx| {
                if let Some((x, y)) = crosshair {
                    draw_crosshair(ctx, egui::pos2(x / scale, y / scale));
                }
                let actions = menu.ui(ctx, settings, &view);
                // Over the game or the menu, fading to show it.
                splash.ui(ctx);
                actions
            });
        }
        g.renderer.queue().present(frame);
        self.stats.drawn += 1;
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
        );
        if let Some(g) = &self.gpu {
            let game = self.session.as_ref().map_or("no game", |s| s.game.title);
            g.window.set_title(&format!("Remodel: {game} · {}x{} · {status}", g.view.width, g.view.height));
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

/// Draw egui over `target`, building the UI with `ui`; returns what it
/// returned. `clear` clears to black first (no game under it).
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

fn draw_overlay<T: Default>(
    g: &mut Gpu,
    target: &wgpu::TextureView,
    clear: bool,
    mut ui: impl FnMut(&egui::Context) -> T,
) -> T {
    let o = &mut g.overlay;
    let raw = o.state.take_egui_input(&g.window);
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
    let screen = egui_wgpu::ScreenDescriptor {
        size_in_pixels: [g.config.width, g.config.height],
        pixels_per_point: out.pixels_per_point,
    };
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
            .with_title("Remodel")
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
        gpu: None,
        keys: HashSet::new(),
        gilrs,
        pad: None,
        mouse: None,
        mouse_used: false,
        mouse_buttons: [false; 2],
        stick: model2::input::StickCursor::default(),
        epoch: Instant::now(),
        redraws: 0,
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
