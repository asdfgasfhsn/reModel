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
//! audio by at most 1% (inaudible), never by stalling the game. When the
//! game persistently runs below full speed the audio follows the frames it
//! actually runs (`model2::audio::Pace`), so it slows with it instead of
//! running dry.
//!
//! The logo shows for two seconds at startup, fading out over the game
//! booting underneath (`splash`). With no game given, the launcher opens
//! (`launcher`): the games as cards, the selected one's attract mode live
//! (`preview`) over a wireframe backdrop (`backdrop`), its settings, and
//! Continue where it was left. Esc opens the menu (and pauses): save
//! states, sound mix, display, back to the games. Settings persist
//! (`settings`). Beside a 4:3 picture in a wider window, the game's bezel
//! (`bezel`). With the texture pack setting on, replacement textures come
//! from the game's pack directory, decoded in the background (`pack`).

mod art;
mod backdrop;
mod bezel;
mod controls;
mod focus;
mod launcher;
mod menu;
mod pack;
mod pad;
mod preview;
mod settings;
mod sound;
mod splash;

use gilrs::Gilrs;
use settings::{GameSettings, Settings, Source, KEYS};
use sound::Sound;
use std::collections::{HashMap, HashSet};
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

  --game ID          start this set straight away (default: the launcher,
                     to choose one)
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
  --smooth on|off    smooth motion: on a display faster than the game's
                     57.5 Hz, draw in-between frames, the 3D geometry
                     interpolated between the last two game frames (one
                     game frame, about 17 ms, of extra latency; the native
                     geometrizer only; docs/play.md). Default off
  --run-ahead N      run-ahead: show the game N frames (0 to 3) ahead of
                     where it is, which takes up to N frames of the game's
                     own input lag away (each game's is in docs/games/),
                     for N more emulated frames each frame; dropped if
                     the machine can't keep up (docs/play.md). Default 0
  --aa off|4|8       anti-aliasing: multisample the 3D layer (4 or 8 samples
                     a pixel, 8 where the GPU has it; docs/enhancements.md).
                     Default off, the original look
  --temporal off|taa|75|67|50
                     temporal anti-aliasing: each frame's 3D layer drawn
                     at a slightly different sub-pixel offset and blended
                     with the frames before along the motion; taa at full
                     size, a percentage to draw it smaller and upscale (50
                     to 100). Turns multisampling (--aa) off and smooth
                     motion's recording on (not its in-betweens); the
                     native geometrizer only (docs/play.md). Default off
  --stipple blend|keep
                     with --temporal: draw the games' checkerboard (screen
                     door) translucency as an even 50% blend, or keep the
                     pattern (where the picture is big enough to hold it).
                     Default blend
  --depth on|off     a depth buffer for the 3D layer, from each vertex's
                     1/z, instead of the game's draw order: intersecting
                     and mis-sorted polygons come out right (HUD windows
                     stay on top; docs/enhancements.md). Default off
  --shading flat|vertex|pixel
                     smooth shading: flat is the game's own, one
                     brightness a polygon; vertex blends brightness across
                     curved surfaces (the game's own lighting worked out
                     at each corner, faces meeting at more than the
                     crease angle keep their edge); pixel works the
                     game's lighting out for every pixel (with --geo
                     chip: drawn as vertex; docs/play.md). Default flat
  --crease auto|N    smooth shading's crease angle in degrees, 0 to 180
                     (0: nothing smoothed); auto is the game's own
                     (40 unless its rules say otherwise). Default auto
  --shading-fine on|off
                     with --shading: blend the two colour levels around
                     each lit pixel's brightness instead of the game's 64
                     levels, so large smooth surfaces don't band (never
                     outside the polygon's colour). Default off
  --pack on|off      the game's texture pack: replacement textures from
                     texpack/ in the game's folder beside its saves (a
                     version of a game uses its parent's if it has none;
                     docs/texture-packs.md). Default off
  --texture-pack DIR use the texture pack in DIR, for every game this run
                     (turns --pack on)
  --tex-filter off|mip|2|4|8|16
                     filtering of texture pack images: off (as they are),
                     mipmaps, or mipmaps and anisotropic filtering up to 2
                     to 16 times where the GPU has it. Default off
  --exact            run at the arcade's exact 57.524 frames per second
                     (default: one frame per display refresh when the
                     refresh is 60 or 120 Hz, which is 4.3% fast but smooth)
  --fullscreen       start fullscreen
  --seconds N        quit after N seconds (for timing runs)
  --no-splash        don't show the logo at startup
  --input SCRIPT     scripted inputs, as for `remodel run` (for timing runs),
                     e.g. coin-start (the game's own coin-up and start)
  --shot FILE        with --seconds: at the end, draw the last frame (the
                     game, bezel included, or the launcher) as a 1920x1080
                     window would show it to a PNG
  --shot-size WxH[@S] the --shot window's size in pixels, and its display
                     scaling (e.g. 3840x2160@2)
  --nav LIST         scripted gamepad input, for checking with --shot:
                     NAME@SECONDS, comma separated; names left, right, up,
                     down (the d-pad), accept (A), back (B), settings (Y),
                     prev, next (shoulders), start, select, guide; a+b
                     presses both, @S:H holds them H seconds

The game's settings (mix, HUD, aspect, bezel, speed, smooth motion,
run-ahead, anti-aliasing, temporal anti-aliasing and stipple, depth buffer,
shading, crease and finer levels, texture pack and filtering, geometrizer) given here are for this run only, over the
defaults for all games and the game's own, which the launcher's settings
and the menu keep.
--rom and --fullscreen are remembered.

keys: Esc menu (pause), F5 quick save, F9 quick load, F11 fullscreen. The
game's controls depend on the game (docs/play.md); C coins up and arrows
steer. Gamepads work too: Guide, or Start and Select together, opens the
menu. The menu's Controls page remaps them per game.
";

/// The game's HUD windows, a bit each: smooth shading leaves them flat
/// (`gpu::Renderer::set_light_rules`).
fn hud_mask(game: &romset::Game) -> u64 {
    game.hud_windows.iter().filter(|&&w| w < 64).fold(0, |m, &w| m | 1 << w)
}

/// Model 2 video frame rate.
const FPS: f64 = 57.524;
/// Most emulated frames run per redraw before dropping time.
const MAX_CATCH_UP: u32 = 3;
/// Narrowest and widest views; windows beyond are letterboxed.
const ASPECTS: (f32, f32) = (4.0 / 3.0, 32.0 / 9.0);
/// The launcher's preview plays this much quieter than the game.
const PREVIEW_DB: f32 = -9.0;
/// The preview's sound fades in over this, its picture out over `FADE_OUT`.
const SOUND_FADE: Duration = Duration::from_millis(1200);
const FADE_OUT: Duration = Duration::from_millis(200);
/// Seconds of game the player may run behind before run-ahead is turned
/// off.
const RUN_AHEAD_GRACE: f64 = 3.0;
/// A game started from the launcher fades in from black.
const GAME_FADE: Duration = Duration::from_millis(400);

/// Options for this run only (the game settings go into the settings'
/// command-line layer).
struct Options {
    sound: bool,
    interp: bool,
    seconds: Option<f64>,
    /// `--input`, parsed against the game's inputs when it starts.
    input: Option<String>,
    /// `--game`: start it straight away.
    game: Option<String>,
    splash: bool,
    shot: Option<PathBuf>,
    /// `--shot-size`: pixels and display scaling.
    shot_size: ([u32; 2], f32),
    /// `--nav`: gamepad presses and releases at times since the start
    /// (seconds), in order.
    nav: std::collections::VecDeque<(f64, pad::Event)>,
    /// `--texture-pack`: the pack for every game this run, in place of
    /// each game's own.
    texture_pack: Option<PathBuf>,
}

/// `--nav`'s list: NAME[+NAME]@SECONDS[:HELD], pad buttons pressed together
/// at SECONDS and released HELD seconds later (default 0.08).
fn parse_nav(list: &str) -> Result<std::collections::VecDeque<(f64, pad::Event)>, String> {
    use gilrs::Button;
    let mut out: Vec<(f64, pad::Event)> = Vec::new();
    for item in list.split(',').filter(|s| !s.is_empty()) {
        let (names, at) = item.split_once('@').ok_or("--nav wants NAME@SECONDS")?;
        let (at, held) = at.split_once(':').unwrap_or((at, "0.08"));
        let at: f64 = at.parse().map_err(|_| format!("--nav: bad time {at}"))?;
        let held: f64 = held.parse().map_err(|_| format!("--nav: bad hold {held}"))?;
        for name in names.split('+') {
            let b = match name {
                "left" => Button::DPadLeft,
                "right" => Button::DPadRight,
                "up" => Button::DPadUp,
                "down" => Button::DPadDown,
                "accept" => Button::South,
                "back" => Button::East,
                "settings" => Button::North,
                "prev" => Button::LeftTrigger,
                "next" => Button::RightTrigger,
                "start" => Button::Start,
                "select" => Button::Select,
                "guide" => Button::Mode,
                _ => return Err(format!("--nav: unknown {name}")),
            };
            out.push((at, pad::Event::Pressed(b)));
            out.push((at + held, pad::Event::Released(b)));
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    Ok(out.into())
}

fn options(s: &mut Settings) -> Result<Options, String> {
    let mut o = Options {
        sound: true,
        interp: false,
        seconds: None,
        input: None,
        game: None,
        splash: true,
        shot: None,
        shot_size: ([1920, 1080], 1.0),
        nav: Default::default(),
        texture_pack: None,
    };
    let mut args = std::env::args().skip(1).peekable();
    let mut cli = |k: &str, v: &str| s.cli.insert(k.to_string(), v.to_string());
    let mut rom = None;
    let mut fullscreen = false;
    while let Some(a) = args.next() {
        let mut value = || args.next().ok_or_else(|| format!("{a} needs a value"));
        match a.as_str() {
            "--game" => o.game = Some(value()?),
            "--rom" => rom = Some(PathBuf::from(value()?)),
            "--mix" => {
                let v = value()?;
                let (x, y) = v.split_once(':').ok_or("--mix wants SFX:MUSIC")?;
                let (x, y): (f32, f32) = (x.parse().map_err(|_| "bad --mix")?, y.parse().map_err(|_| "bad --mix")?);
                cli("sfx_db", &x.to_string());
                cli("music_db", &y.to_string());
            }
            "--no-sound" => o.sound = false,
            "--hud" => {
                let edges = match value()?.as_str() {
                    "center" => false,
                    "edges" => true,
                    v => return Err(format!("--hud wants center or edges, not {v}")),
                };
                cli("hud_edges", &edges.to_string());
            }
            "--aspect" => {
                let wide = match value()?.as_str() {
                    "fit" => true,
                    "4:3" => false,
                    _ => return Err("--aspect wants fit or 4:3".into()),
                };
                cli("wide", &wide.to_string());
            }
            "--bezel" => {
                let on = match value()?.as_str() {
                    "on" => true,
                    "off" => false,
                    v => return Err(format!("--bezel wants on or off, not {v}")),
                };
                cli("bezel", &on.to_string());
            }
            "--shot" => o.shot = Some(value()?.into()),
            "--shot-size" => {
                let v = value()?;
                let (size, scale) = v.split_once('@').unwrap_or((&v, "1"));
                let (w, h) = size.split_once('x').ok_or("--shot-size wants WxH[@S]")?;
                let px = |n: &str| n.parse::<u32>().ok().filter(|n| (64..=8192).contains(n)).ok_or("bad --shot-size");
                let scale: f32 =
                    scale.parse().ok().filter(|s: &f32| (0.5..=4.0).contains(s)).ok_or("bad --shot-size scale")?;
                o.shot_size = ([px(w)?, px(h)?], scale);
            }
            "--nav" => o.nav = parse_nav(&value()?)?,
            "--interp" => o.interp = true,
            "--smooth" => {
                // Bare `--smooth` (as before it took a value) turns it on.
                let on = match args.peek().map(String::as_str) {
                    Some("on") => {
                        args.next();
                        true
                    }
                    Some("off") => {
                        args.next();
                        false
                    }
                    _ => true,
                };
                cli("smooth", &on.to_string());
            }
            "--geo" => match model2::geo::GeoMode::parse(&value()?) {
                Some(model2::geo::GeoMode::Check) | None => return Err("--geo wants native or chip".into()),
                Some(mode) => {
                    cli("geo", if mode == model2::geo::GeoMode::Chip { "chip" } else { "native" });
                }
            },
            "--run-ahead" => {
                let n = value()?;
                match n.parse::<u32>() {
                    Ok(n) if n <= settings::MAX_RUN_AHEAD => cli("run_ahead", &n.to_string()),
                    _ => return Err(format!("--run-ahead wants 0 to {}", settings::MAX_RUN_AHEAD)),
                };
            }
            "--aa" => {
                let v = value()?;
                let n = match v.as_str() {
                    "off" => Some(0),
                    "4x" | "4" => Some(4),
                    "8x" | "8" => Some(8),
                    _ => None,
                };
                let Some(n) = n else { return Err("--aa wants off, 4 or 8".into()) };
                cli("aa", &n.to_string());
            }
            "--temporal" => {
                let v = value()?;
                if gpu::Quality::parse_temporal(&v).is_none() {
                    return Err("--temporal wants off, taa or a percentage from 50 to 100 (75, 67, 50)".into());
                }
                cli("temporal", &v);
            }
            "--stipple" => {
                let v = value()?;
                if gpu::Stipple::parse(&v).is_none() {
                    return Err("--stipple wants blend or keep".into());
                }
                cli("stipple", &v);
            }
            "--depth" => {
                let on = match args.peek().map(String::as_str) {
                    Some("on") => {
                        args.next();
                        true
                    }
                    Some("off") => {
                        args.next();
                        false
                    }
                    _ => true,
                };
                cli("depth", &on.to_string());
            }
            "--shading" => {
                let Some(shading) = gpu::Shading::parse(&value()?) else {
                    return Err("--shading wants flat, vertex or pixel".into());
                };
                cli("shading", shading.name());
            }
            "--crease" => {
                let v = value()?;
                if model2::light::parse_crease(&v).is_none() {
                    return Err("--crease wants auto or degrees from 0 to 180".into());
                }
                cli("crease", &v);
            }
            "--shading-fine" => {
                let on = match args.peek().map(String::as_str) {
                    Some("on") => {
                        args.next();
                        true
                    }
                    Some("off") => {
                        args.next();
                        false
                    }
                    _ => true,
                };
                cli("shading_fine", &on.to_string());
            }
            "--pack" => {
                let on = match args.peek().map(String::as_str) {
                    Some("on") => {
                        args.next();
                        true
                    }
                    Some("off") => {
                        args.next();
                        false
                    }
                    _ => true,
                };
                cli("pack", &on.to_string());
            }
            "--texture-pack" => {
                o.texture_pack = Some(value()?.into());
                cli("pack", "true");
            }
            "--tex-filter" => {
                let v = value()?;
                let Some(f) = model2::enhance::TexFilter::parse(&v) else {
                    return Err("--tex-filter wants off, mip, 2, 4, 8 or 16".into());
                };
                cli("tex_filter", &f.to_string());
            }
            "--no-splash" => o.splash = false,
            "--fullscreen" => fullscreen = true,
            "--exact" => {
                cli("exact", "true");
            }
            "--input" => o.input = Some(value()?),
            "--seconds" => o.seconds = Some(value()?.parse().map_err(|_| "bad --seconds")?),
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("unknown option {a}")),
        }
    }
    if let Some(rom) = rom {
        s.rom = rom;
    }
    s.fullscreen |= fullscreen;
    if s.cli.get("smooth").is_some_and(|v| v == "true") && s.cli.get("geo").is_some_and(|v| v == "chip") {
        eprintln!("--smooth: off with --geo chip (in-betweens come from the native geometrizer)");
    }
    if s.cli.get("temporal").is_some_and(|v| gpu::Quality::parse_temporal(v) != Some(0))
        && s.cli.get("geo").is_some_and(|v| v == "chip")
    {
        eprintln!("--temporal: off with --geo chip (its motion data comes from the native geometrizer)");
    }
    Ok(o)
}

/// The game being played, or previewed in the launcher.
struct Session {
    game: &'static romset::Game,
    machine: model2::Machine,
    sound: Option<Sound>,
    /// `--input`, if given: replaces the player's controls.
    script: Option<model2::input::InputScript>,
    /// The player's bindings: the manifest's, remapped in the menu's
    /// Controls page (`settings::load_bindings`).
    bindings: romset::Bindings,
    /// The launcher's attract mode preview: no input, quieter, never saved
    /// (its EEPROM isn't the player's).
    preview: bool,
    /// Run-ahead, when it's on: what's drawn is its frames ahead of
    /// `machine`, which stays on the game's real timeline (sound, saves).
    run_ahead: Option<model2::runahead::RunAhead>,
    /// Run-ahead was turned off because the machine couldn't keep up.
    run_ahead_dropped: bool,
    /// The last frame drawn was run-ahead's (so the renderer's copy of
    /// video RAM is that machine's).
    drew_ahead: bool,
}

/// `set`'s save state folder.
fn saves_dir(set: &str) -> PathBuf {
    settings::data_dir().join(set).join("saves")
}

/// Where a game's resume state goes: written on leaving it, offered by
/// the launcher as Continue.
fn resume_path(set: &str) -> PathBuf {
    saves_dir(set).join("resume.save")
}

impl Session {
    fn dir(&self) -> PathBuf {
        settings::data_dir().join(self.game.id)
    }

    fn slot_path(&self, slot: u8) -> PathBuf {
        let name = if slot == 0 { "quick.save".to_string() } else { format!("slot{slot}.save") };
        saves_dir(self.game.id).join(name)
    }

    fn save_eeprom(&self) {
        let bus = &self.machine.bus;
        if bus.eeprom.dirty && !self.preview {
            let _ = std::fs::create_dir_all(self.dir());
            let _ = std::fs::write(self.dir().join("eeprom.bin"), bus.eeprom.save());
        }
    }

    /// Run-ahead of `frames` (0: off; never for the preview) and smooth
    /// motion's recording `smooth` (for in-betweens or temporal
    /// anti-aliasing), where it goes: the machine drawn, the frames run
    /// ahead while they are. `roms` if at hand, else loaded from `rom`.
    fn set_run_ahead(&mut self, frames: u32, smooth: bool, roms: Option<&romset::RomSet>, rom: &Path) {
        if frames == 0 || self.preview {
            self.run_ahead = None;
        } else if let Some(ra) = &mut self.run_ahead {
            ra.set_frames(frames);
        } else {
            let loaded = match roms {
                Some(_) => None,
                None => match romset::RomSet::load(self.game, rom, false) {
                    Ok(r) => Some(r),
                    Err(e) => {
                        eprintln!("run-ahead: {e}");
                        return;
                    }
                },
            };
            let roms = roms.or(loaded.as_ref()).unwrap();
            self.run_ahead = Some(model2::runahead::RunAhead::new(roms, &mut self.machine, frames));
        }
        self.machine.bus.geo.set_smooth(smooth && self.run_ahead.is_none());
        if let Some(ra) = &mut self.run_ahead {
            ra.set_smooth(smooth);
        }
    }

    /// Smooth shading's side channel on or off, for the game and
    /// run-ahead's machine alike (the frames drawn from either are shaded
    /// at the same level).
    fn set_light(&mut self, on: bool) {
        self.machine.bus.geo.set_light(on);
        if let Some(ra) = &mut self.run_ahead {
            ra.shadow().bus.geo.set_light(on);
        }
    }

    /// The machine to draw: run-ahead's frames ahead, while they're of the
    /// game as it is now, else the game.
    fn shown(&mut self) -> &mut model2::Machine {
        match &mut self.run_ahead {
            Some(ra) => ra.shown(&mut self.machine),
            None => &mut self.machine,
        }
    }

    /// The whole machine, sound boards included, as a save file.
    fn save_state(&mut self) -> Vec<u8> {
        let live = self.sound.as_ref().map(|snd| snd.save());
        model2::save_session_parts(&mut self.machine, live)
    }
}

/// egui over the game: its context, window input and renderer.
struct Overlay {
    ctx: egui::Context,
    state: egui_winit::State,
    renderer: egui_wgpu::Renderer,
}

/// The launcher's preview: the game drawn into a texture egui shows.
struct PreviewTarget {
    _texture: wgpu::Texture,
    view: wgpu::TextureView,
    id: egui::TextureId,
    size: [u32; 2],
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
    backdrop: backdrop::Backdrop,
    preview: Option<PreviewTarget>,
}

/// Status line counters for the current one-second window.
struct Stats {
    start: Instant,
    emulated: u32,
    drawn: u32,
    emulating: Duration,
    /// Host time drawing (not waiting for vsync).
    drawing: Duration,
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
    /// Smooth motion: in-betweens drawn, and the host time computing them.
    smooth: u32,
    smoothing: Duration,
    /// Run-ahead: times it ran (once a redraw with new frames), and the
    /// host time copying the game and running the frames ahead.
    ahead: u32,
    run_ahead: Duration,
    /// Smooth shading: draws shaded, the host time it took (repeats
    /// none), and the last one's level as drawn, lit and drawn polygons.
    shaded: u32,
    shading: Duration,
    shading_as: Option<(gpu::Shading, u32, u32)>,
}

impl Stats {
    fn new() -> Self {
        Stats {
            start: Instant::now(),
            emulated: 0,
            drawn: 0,
            emulating: Duration::ZERO,
            drawing: Duration::ZERO,
            repeats: 0,
            longest_gap: Duration::ZERO,
            last_new: Instant::now(),
            reports: 0,
            surface_misses: 0,
            geo_late: 0,
            smooth: 0,
            smoothing: Duration::ZERO,
            ahead: 0,
            run_ahead: Duration::ZERO,
            shaded: 0,
            shading: Duration::ZERO,
            shading_as: None,
        }
    }
}

/// The launcher's preview as it runs: when it first showed, and since when
/// it's fading out (the selection moved).
#[derive(Default)]
struct PreviewState {
    shown: Option<Instant>,
    leaving: Option<Instant>,
    /// Games that can't be previewed, and why.
    failed: HashMap<String, String>,
    /// The game whose preview is wanted (the selected card's).
    wanted: Option<String>,
}

struct Play {
    opts: Options,
    settings: Settings,
    /// The settings of the game playing (or selected in the launcher),
    /// where each came from, and as last applied to its machine.
    values: GameSettings,
    sources: [Source; KEYS.len()],
    applied: GameSettings,
    values_for: String,
    session: Option<Session>,
    /// Playable games in the ROM folder, and how many sets aren't supported.
    games: Vec<&'static romset::Game>,
    unsupported: usize,
    menu: menu::Menu,
    launcher: launcher::Launcher,
    previews: preview::Previews,
    preview: PreviewState,
    /// The bindings the launcher's Controls page edits (its game's).
    launcher_bindings: Option<romset::Bindings>,
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
    /// The gamepad driving the launcher or menu, and what it asked for
    /// this frame.
    pad_nav: pad::PadNav,
    navs: Vec<launcher::Nav>,
    /// Between the pad and the game: the menu's buttons, and buttons kept
    /// from the game.
    gate: pad::Gate,
    /// Light guns: the mouse (window position, None outside the window;
    /// left and right buttons), once it has moved over the window, aims
    /// player 1; the pad's stick cursor aims player 2 then, else player 1.
    mouse: Option<(f64, f64)>,
    mouse_used: bool,
    mouse_buttons: [bool; 2],
    stick: model2::input::StickCursor,
    /// Real time that emulated frame 0 corresponds to.
    epoch: Instant,
    /// Smooth motion: when the newest game frame was first drawn.
    smooth_at: Instant,
    /// A game started from the launcher: since when (fading in).
    game_fade: Option<Instant>,
    redraws: u64,
    /// When `emulate` last ran (for the audio rate control).
    last_emulate: Option<Instant>,
    /// Host time per game frame (emulating, waiting for the geometrizer and
    /// drawing, not waiting for vsync), smoothed, in seconds; and whether
    /// that is too slow to follow the display (then the clock paces).
    frame_cost: f64,
    behind: bool,
    /// Frames run since `behind` was last clear.
    behind_frames: u32,
    stats: Stats,
    started: Instant,
    /// The texture pack set on the renderer.
    pack: PackState,
    /// Anisotropic filtering the GPU has (`gpu::max_anisotropy`).
    max_anisotropy: u8,
    /// The GPU can do temporal anti-aliasing (`gpu::temporal_supported`).
    temporal_supported: bool,
}

/// The texture pack the renderer draws with (`Play::sync_pack`).
#[derive(Default)]
struct PackState {
    /// The directory wanted (None: no pack).
    dir: Option<PathBuf>,
    /// Images in it, or why it can't be used.
    loaded: Option<Result<usize, String>>,
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

/// A gain in dB for a linear fade `f` (0..1).
fn fade_db(f: f32) -> f32 {
    20.0 * f.clamp(0.001, 1.0).log10()
}

impl Play {
    /// Find the playable games in the ROM folder.
    fn scan(&mut self) {
        let sets = romset::sets_with_clones(&self.settings.rom);
        self.games = sets.iter().filter_map(|s| romset::games::playable(s)).collect();
        self.unsupported = sets.len() - self.games.len();
        self.launcher.rom_text = self.settings.rom.display().to_string();
        self.launcher.set_games(&self.games, &self.settings.recent);
    }

    /// Use `set`'s settings (resolved from the layers) from now on.
    fn use_values(&mut self, set: &str) {
        let (v, src) = self.settings.resolve(set);
        self.values = v;
        self.sources = src;
        if self.values_for != set {
            self.values_for = set.to_string();
            self.applied = self.values.clone();
        }
        self.apply_values();
    }

    /// Keep what the launcher or menu changed in `set`'s settings, and
    /// apply it.
    fn edit_values(&mut self, set: &str, v: GameSettings) {
        if v != self.values {
            self.settings.edit(set, &v);
        }
        self.use_values(set);
    }

    /// Open the launcher, leaving the game (if any) where it was.
    fn open_launcher(&mut self) {
        let set = self.session.as_ref().map(|s| s.game.id);
        self.leave_game();
        self.launcher.set_games(&self.games, &self.settings.recent);
        if let Some(set) = set {
            self.launcher.select_set(set);
        }
        self.launcher.open = true;
        // Left from the pad: the launcher's hints say its buttons.
        self.launcher.pad_used |= self.menu.pad_used;
        self.set_menu(false);
        if let Some(g) = &self.gpu {
            g.window.set_cursor(CursorIcon::Default);
        }
    }

    /// Stop the game: keep its EEPROM and a resume state (for Continue),
    /// unless it's the launcher's preview or a scripted run.
    fn leave_game(&mut self) {
        let Some(mut s) = self.session.take() else { return };
        s.save_eeprom();
        if s.preview || s.script.is_some() {
            return;
        }
        let data = s.save_state();
        let path = resume_path(s.game.id);
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, data));
        if let Err(e) = written {
            eprintln!("{}: can't keep the resume state: {e}", path.display());
        }
    }

    /// Start `set` from power-on, or from its resume state with `resume`
    /// (leaving the current game first).
    fn start_game(&mut self, set: &str, resume: bool) {
        self.leave_game();
        self.previews.cancel_all();
        self.use_values(set);
        let values = self.values.clone();
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
            machine.bus.geo.set_mode(values.geo_mode());
            machine.bus.geo.set_smooth(values.motion_on());
            // Smooth shading lights from what the geometrizer knew (run-ahead's
            // machine is made like this one).
            machine.bus.geo.set_light(values.light_on());
            // Drawing can go ahead with the last frame's polygons when the
            // geometrizer is late (`frame`).
            machine.bus.geo.set_keep_last(true);
            let dir = settings::data_dir().join(game.id);
            if let Ok(bytes) = std::fs::read(dir.join("eeprom.bin")) {
                machine.bus.eeprom.load(&bytes);
            }
            let sound = if self.opts.sound {
                Sound::new(&roms, (values.sfx_db, values.music_db))
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
            let mut s = Session {
                game,
                machine,
                sound,
                script,
                bindings,
                preview: false,
                run_ahead: None,
                run_ahead_dropped: false,
                drew_ahead: false,
            };
            s.set_run_ahead(values.run_ahead, values.motion_on(), Some(&roms), &self.settings.rom);
            Ok(s)
        });
        match started {
            Ok(s) => {
                self.bezel =
                    bezel::image(s.game).map(|image| Bezel { size: image.size, image: Some(image), texture: None });
                self.settings.played(set);
                self.settings.save();
                // A crosshair pointer over gun games.
                if let Some(g) = &mut self.gpu {
                    let gun = !s.game.controls.guns.is_empty();
                    g.window.set_cursor(if gun { CursorIcon::Crosshair } else { CursorIcon::Default });
                    g.renderer.forget_ram();
                    g.renderer.reset_history();
                }
                self.session = Some(s);
                self.epoch = Instant::now();
                self.refit();
                if resume {
                    self.load_file(&resume_path(set), "Continued");
                }
            }
            Err(e) => {
                eprintln!("{set}: {e}");
                self.menu.notify(format!("Can't start {set}: {e}"));
                self.launcher.open = true;
            }
        }
    }

    /// Start the launcher's preview from what a preview job got ready.
    fn start_preview(&mut self, r: preview::Ready) {
        let t = Instant::now();
        let game = r.game;
        let mut machine = model2::Machine::new(&r.roms);
        let t_new = t.elapsed();
        if self.opts.interp {
            machine.disable_native();
        }
        let geo = self.values.geo_mode();
        machine.bus.geo.set_mode(geo);
        // Previews draw without temporal anti-aliasing: in-betweens only.
        machine.bus.geo.set_smooth(self.values.smooth_on());
        machine.bus.geo.set_keep_last(true);
        let live = match model2::load_session_parts(&mut machine, &r.state) {
            Ok(live) => live,
            Err(e) => {
                // Made by another version, say: make it again.
                let _ = std::fs::remove_file(preview::cache_path(game.id, geo));
                if r.made {
                    self.preview.failed.insert(game.id.to_string(), e);
                }
                return;
            }
        };
        let t_machine = t.elapsed();
        let sound = if self.opts.sound {
            let mix = (self.values.sfx_db + PREVIEW_DB - 60.0, self.values.music_db + PREVIEW_DB - 60.0);
            Sound::new(&r.roms, mix).inspect_err(|e| eprintln!("{e}; previews without sound")).ok().and_then(
                |mut snd| {
                    snd.load(live, &machine.bus).ok()?;
                    Some(snd)
                },
            )
        } else {
            None
        };
        println!(
            "preview       {} ({}): {} in {:.0} ms, started in {:.0} ms (machine {:.0} ms, state {:.0} ms)",
            game.title,
            game.id,
            if r.made { "made the attract state" } else { "loaded" },
            r.took.as_secs_f64() * 1000.0,
            t.elapsed().as_secs_f64() * 1000.0,
            t_new.as_secs_f64() * 1000.0,
            (t_machine - t_new).as_secs_f64() * 1000.0,
        );
        if let Some(g) = &mut self.gpu {
            g.renderer.forget_ram();
            g.renderer.reset_history();
        }
        let bindings = settings::load_bindings(game);
        self.epoch = Instant::now() - Duration::from_secs_f64(machine.frames as f64 / FPS);
        self.session = Some(Session {
            game,
            machine,
            sound,
            script: None,
            bindings,
            preview: true,
            run_ahead: None,
            run_ahead_dropped: false,
            drew_ahead: false,
        });
        self.preview.shown = None;
        self.preview.leaving = None;
        self.behind = false;
        self.frame_cost = 0.0;
    }

    fn save_slot(&mut self, slot: u8) {
        let Some(s) = self.session.as_mut().filter(|s| !s.preview) else { return };
        let data = s.save_state();
        let path = s.slot_path(slot);
        let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, data));
        let name = if slot == 0 { "Quick save".to_string() } else { format!("Saved to slot {slot}") };
        self.menu.notify(match result {
            Ok(()) => name,
            Err(e) => format!("Save failed: {e}"),
        });
    }

    fn load_slot(&mut self, slot: u8) {
        let Some(path) = self.session.as_ref().filter(|s| !s.preview).map(|s| s.slot_path(slot)) else { return };
        if !path.is_file() {
            return self.menu.notify(if slot == 0 { "No quick save yet" } else { "That slot is empty" });
        }
        let done = if slot == 0 { "Quick load".to_string() } else { format!("Loaded slot {slot}") };
        self.load_file(&path, &done);
    }

    /// Load the save file at `path` into the game, saying `done` if it did.
    fn load_file(&mut self, path: &Path, done: &str) {
        let Some(s) = &mut self.session else { return };
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => return self.menu.notify(format!("Can't read {}: {e}", path.display())),
        };
        let result = model2::load_session_parts(&mut s.machine, &data).and_then(|live| match &mut s.sound {
            Some(snd) if !live.is_empty() => snd.load(live, &s.machine.bus),
            _ => Ok(()),
        });
        match result {
            Ok(()) => {
                self.epoch = Instant::now() - Duration::from_secs_f64(s.machine.frames as f64 / FPS);
                // A jump the motion data doesn't know of.
                if let Some(g) = &mut self.gpu {
                    g.renderer.reset_history();
                }
                self.menu.notify(done);
            }
            Err(e) => {
                // The machine may be half loaded: start the game afresh.
                let set = s.game.id.to_string();
                self.session = None;
                self.start_game(&set, false);
                self.menu.notify(format!("Load failed: {e}"));
            }
        }
    }

    fn slot_times(&self) -> Vec<Option<SystemTime>> {
        let Some(s) = &self.session else { return Vec::new() };
        (0..menu::SLOTS).map(|n| std::fs::metadata(s.slot_path(n)).and_then(|m| m.modified()).ok()).collect()
    }

    /// Apply what changed in the game's settings to its machine.
    fn apply_values(&mut self) {
        if self.values == self.applied {
            return;
        }
        let (old, new) = (self.applied.clone(), self.values.clone());
        self.applied = new.clone();
        let preview = self.session.as_ref().is_some_and(|s| s.preview);
        if (old.sfx_db, old.music_db) != (new.sfx_db, new.music_db) && !preview {
            if let Some(snd) = self.session.as_ref().and_then(|s| s.sound.as_ref()) {
                snd.set_mix(new.sfx_db, new.music_db);
            }
        }
        // Smooth motion's recording: previews draw without temporal.
        let motion = |v: &GameSettings| if preview { v.smooth_on() } else { v.motion_on() };
        if (motion(&old), old.run_ahead) != (motion(&new), new.run_ahead) {
            let rom = self.settings.rom.clone();
            if let Some(s) = &mut self.session {
                // Setting it again tries again after it was dropped.
                s.run_ahead_dropped &= old.run_ahead == new.run_ahead;
                let frames = if s.run_ahead_dropped { 0 } else { new.run_ahead };
                s.set_run_ahead(frames, motion(&new), None, &rom);
            }
        }
        if old.light_on() != new.light_on() && !preview {
            if let Some(s) = &mut self.session {
                s.set_light(new.light_on());
            }
        }
        if old.geo_chip != new.geo_chip {
            if preview {
                // The preview starts again on the other geometrizer.
                self.session = None;
                self.preview.wanted = None;
            } else if self.session.is_some() {
                self.menu.notify("The geometrizer changes when the game next starts");
            }
        }
        if old.hud_edges != new.hud_edges || old.wide != new.wide {
            self.refit();
        }
    }

    /// Fit a view into `w` x `h` pixels: full size between `ASPECTS`
    /// (just 4:3 unless wide), else letterboxed. Returns it and its
    /// top-left corner.
    fn fit(&self, w: u32, h: u32) -> (model2::video::View, (u32, u32)) {
        let aspect = w as f32 / h.max(1) as f32;
        let a = if self.values.wide { aspect.clamp(ASPECTS.0, ASPECTS.1) } else { ASPECTS.0 };
        let vh = if a < aspect { h } else { (w as f32 / a) as u32 };
        let hud = if self.values.hud_edges { model2::video::Hud::Edges } else { model2::video::Hud::Center };
        let mut view = model2::video::View::new(vh.max(1) as usize, a, hud);
        if let Some(s) = &self.session {
            view = view.with_hud_windows(s.game.hud_windows, s.game.hud_element_windows);
        }
        let origin = ((w - (view.width as u32).min(w)) / 2, (h - (view.height as u32).min(h)) / 2);
        (view, origin)
    }

    /// Where the bezel goes in a window `w` pixels wide showing `view` at
    /// `origin` (None: not shown), and the picture's left and right edges.
    fn bezel_at(&self, view: &model2::video::View, origin: (u32, u32), w: u32) -> Option<([f32; 4], (f32, f32))> {
        let b =
            self.bezel.as_ref().filter(|_| self.values.bezel && self.session.as_ref().is_some_and(|s| !s.preview))?;
        let pic = (origin.0, origin.1, view.width as u32, view.height as u32);
        let rect = bezel::layout(w, pic, view.pad, b.size)?;
        Some((rect, (origin.0 as f32, (origin.0 as usize + view.width) as f32)))
    }

    /// `--shot`: what the window shows now (the game's current frame,
    /// bezel included, or the launcher) as a window of `--shot-size` would,
    /// to a PNG.
    fn shot(&mut self, path: &Path) -> Result<(), String> {
        let ([w, h], ppp) = self.opts.shot_size;
        let Some(g) = &mut self.gpu else { return Err("no window".into()) };
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
        if self.launcher.open {
            self.draw_launcher(&target, [w, h], ppp, Instant::now());
        } else {
            self.sync_pack();
            let (view, origin) = self.fit(w, h);
            let at = self.bezel_at(&view, origin, w);
            let slots = self.slot_times();
            let pack_text = self.pack_text();
            let (Some(g), Some(s)) = (&mut self.gpu, &mut self.session) else { return Err("no game".into()) };
            g.renderer.forget_ram();
            // A one-off frame: with temporal on, the plain (unjittered) one.
            g.renderer.reset_history();
            g.renderer.set_light_rules(&s.game.shading, hud_mask(s.game));
            g.renderer.set_quality(self.values.quality(self.temporal_supported));
            g.renderer.present_bus(&mut s.shown().bus, None, &view, &target, format, origin);
            s.drew_ahead = s.run_ahead.as_ref().is_some_and(|ra| ra.is_ahead_of(&s.machine));
            let (bezel, menu) = (&mut self.bezel, &mut self.menu);
            let mut values = self.values.clone();
            let (mut fullscreen, mut for_all) = (self.settings.fullscreen, self.settings.for_all);
            let menu_view = menu::View {
                playing: Some(s.game.title),
                slots: &slots,
                input_lag: Some(s.game.input_lag),
                max_anisotropy: self.max_anisotropy,
                temporal_supported: self.temporal_supported,
                pack: pack_text.as_deref(),
            };
            draw_overlay(g, &target, false, [w, h], Some(ppp), |ctx| {
                if let (Some(b), Some((rect, pic))) = (bezel.as_mut(), at) {
                    b.paint(ctx, rect, pic, w as f32, ctx.pixels_per_point());
                }
                // The menu as it is (drawing it changes nothing).
                if menu.open {
                    let prefs = menu::Prefs { fullscreen: &mut fullscreen, for_all: &mut for_all };
                    menu.ui(ctx, &mut values, prefs, &menu_view, Some(&mut s.bindings), &[]);
                }
            });
        }
        let g = self.gpu.as_ref().unwrap();
        let mut rgba = g.renderer.read_texture(&texture, w, h);
        if format == wgpu::TextureFormat::Bgra8Unorm {
            rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2));
        }
        std::fs::write(path, png::encode_rgba(w as usize, h as usize, &rgba))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Set the texture pack the settings ask for on the renderer, when
    /// that changed: the game's (or `--texture-pack`'s) with the setting
    /// on, none for the launcher's preview.
    fn sync_pack(&mut self) {
        let wanted = match &self.session {
            Some(s) if !s.preview && self.values.pack => {
                Some(self.opts.texture_pack.clone().unwrap_or_else(|| pack::game_dir(s.game)))
            }
            _ => None,
        };
        if wanted == self.pack.dir {
            return;
        }
        let Some(g) = &mut self.gpu else { return };
        self.pack = PackState { dir: wanted.clone(), loaded: None };
        let Some(dir) = wanted else {
            g.renderer.set_pack(None);
            return;
        };
        let opened = if dir.is_dir() {
            pack::DirPack::open(&dir)
        } else {
            Err(format!("no texture pack: {} not found", dir.display()))
        };
        match opened {
            Ok(p) => {
                println!("texture pack  {}: {} images", dir.display(), p.len());
                self.pack.loaded = Some(Ok(p.len()));
                g.renderer.set_pack(Some(Box::new(p)));
            }
            Err(e) => {
                eprintln!("{e}");
                self.menu.notify(format!("{}{}", e[..1].to_uppercase(), &e[1..]));
                self.pack.loaded = Some(Err(e));
                g.renderer.set_pack(None);
            }
        }
    }

    /// The menu's line on the texture pack: where it is and what it
    /// replaced in the last frame, or why there is none.
    fn pack_text(&self) -> Option<String> {
        let g = self.gpu.as_ref()?;
        let dir = self.pack.dir.as_ref()?;
        Some(match self.pack.loaded.as_ref()? {
            Ok(images) => {
                let st = g.renderer.pack_stats();
                let tiles = if st.tiles_held > 0 {
                    format!("; {} tile patterns ({} held)", st.tiles, st.tiles_held)
                } else {
                    String::new()
                };
                format!(
                    "{}: {images} images; replacing {} of {} textured polygons ({} images, {} MB held){tiles}",
                    dir.display(),
                    st.replaced,
                    st.textured,
                    st.held,
                    st.bytes >> 20
                )
            }
            Err(e) => e.clone(),
        })
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
            // The temporal targets are made again at the new size.
            g.renderer.reset_history();
        }
        self.refit();
    }

    /// Game frames per display refresh when the game follows the display
    /// (1 at about 60 Hz, 1/2 at about 120 Hz), or None to follow the clock.
    fn per_refresh(&self) -> Option<f64> {
        let hz = self.gpu.as_ref()?.refresh?;
        if self.values.exact || self.behind {
            return None;
        }
        [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS)))
    }

    /// Take the gamepads' events (the last used one's) and `--nav`'s due
    /// now: see `pad_event`. Then the held direction's repeat, and Start
    /// and Select held long enough to open the menu.
    fn poll_pads(&mut self) {
        let now = Instant::now();
        let mut events = Vec::new();
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                if matches!(ev.event, gilrs::EventType::ButtonPressed(..) | gilrs::EventType::AxisChanged(..))
                    && self.pad != Some(ev.id)
                {
                    self.pad = Some(ev.id);
                    self.gate = pad::Gate::default();
                }
                if Some(ev.id) == self.pad {
                    events.extend(pad::Event::from_gilrs(&ev.event).map(|e| (e, false)));
                }
            }
        }
        let secs = (now - self.started).as_secs_f64();
        while self.opts.nav.front().is_some_and(|n| n.0 <= secs) {
            events.push((self.opts.nav.pop_front().unwrap().1, true));
        }
        for (ev, scripted) in events {
            self.pad_event(ev, scripted, now);
        }
        if (self.launcher.open || self.menu.open) && self.menu.capture.is_none() {
            self.navs.extend(self.pad_nav.repeat(now));
        }
        if !self.launcher.open && !self.menu.open && self.gate.tick(now) {
            self.set_menu(true);
            self.menu.pad_used = true;
        }
    }

    /// A gamepad event: a press skips the splash (and does nothing else,
    /// unless `scripted`); else it's for the Controls page waiting for a
    /// binding (Guide cancels), the launcher, or the game: through the gate
    /// (`pad::Gate`), which opens and closes the menu, to the menu while
    /// it's open.
    fn pad_event(&mut self, ev: pad::Event, scripted: bool, now: Instant) {
        use pad::Event;
        if let Event::Released(b) = ev {
            self.gate.release(b, now);
        }
        if let (Event::Pressed(b), true, false) = (ev, self.splash.active(), scripted) {
            self.gate.keep(b);
            return self.splash.skip();
        }
        if self.menu.capture.is_some() {
            if let Event::Pressed(b) = ev {
                self.gate.keep(b);
                if b == gilrs::Button::Mode {
                    self.menu.capture = None;
                    return;
                }
            }
            if let Some(c) = controls::captured(ev) {
                self.capture(c);
            }
            return;
        }
        if self.launcher.open {
            self.navs.extend(self.pad_nav.event(ev, now));
            return;
        }
        if let Event::Pressed(b) = ev {
            if self.gate.press(b, now, self.menu.open) {
                self.set_menu(!self.menu.open);
                self.menu.pad_used = true;
                return;
            }
        }
        if self.menu.open {
            self.navs.extend(self.pad_nav.event(ev, now));
        }
    }

    /// Bind what the player pressed or moved into the slot the Controls
    /// page is waiting on, or say why it can't be.
    fn capture(&mut self, c: romset::bindings::Capture) {
        let Some((slot, at)) = self.menu.capture else { return };
        let (bindings, game) = if self.launcher.open {
            (self.launcher_bindings.as_mut(), self.launcher.selected())
        } else {
            match &mut self.session {
                Some(s) => (Some(&mut s.bindings), Some(s.game)),
                None => (None, None),
            }
        };
        let (Some(b), Some(game)) = (bindings, game) else { return };
        let axis_slot = b.accepts(slot) == romset::bindings::Accepts::Axis;
        // Sticks resting off centre, or moved on the way to a button.
        if matches!(c, romset::bindings::Capture::Axis(..)) && !axis_slot {
            return;
        }
        match b.capture(slot, at, c, romset::bindings::RESERVED_KEYS) {
            Ok(()) => {
                settings::save_bindings(game, b);
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
                let devices = controls::Devices { keys: &self.keys, pad, gate: &self.gate };
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
        if s.game.controls.guns.is_empty() || self.pad.is_none() || s.script.is_some() || s.preview {
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
            if self.session.as_ref().is_some_and(|s| !s.preview) {
                self.apply_inputs();
                // The pad opened the menu: pause now.
                if self.menu.open {
                    break;
                }
            }
            let s = self.session.as_mut().unwrap();
            s.machine.run_slice(u64::MAX)?;
            match &mut s.sound {
                Some(snd) => snd.send(&mut s.machine.bus),
                None => s.machine.bus.uart_log.clear(),
            }
            ran += 1;
        }
        // Run-ahead: from the newest real frame, the frames shown.
        if let Some(s) = self.session.as_mut().filter(|_| ran > 0) {
            if let Some(ra) = &mut s.run_ahead {
                let t = Instant::now();
                let r = ra.run(&mut s.machine);
                self.stats.run_ahead += t.elapsed();
                self.stats.ahead += 1;
                if let Err(e) = r {
                    // The game will stop there itself, that many frames on.
                    eprintln!("run-ahead stopped: {e:?}");
                    s.run_ahead = None;
                    s.machine.bus.geo.set_smooth(self.values.motion_on());
                }
            }
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

    /// Emulate what's due and keep the stats; false if the game stopped.
    fn run_due(&mut self) -> (u32, Duration) {
        let t0 = Instant::now();
        let result = self.emulate();
        if let (Ok(1..), Some(s)) = (&result, &self.session) {
            // When the newest frame was due: on the clock, or now when
            // following the display (which sets the clock to now).
            self.smooth_at = self.epoch + Duration::from_secs_f64(s.machine.frames as f64 / FPS);
        }
        let emulate_time = t0.elapsed();
        match result {
            Ok(ran) => {
                let st = &mut self.stats;
                st.emulating += emulate_time;
                st.emulated += ran;
                if ran == 0 && !self.menu.open {
                    st.repeats += 1;
                } else if ran > 0 {
                    st.longest_gap = st.longest_gap.max(st.last_new.elapsed());
                    st.last_new = Instant::now();
                }
                (ran, emulate_time)
            }
            Err(e) => {
                eprintln!("stopped: {e:?}");
                let preview = self.session.as_ref().filter(|s| s.preview).map(|s| s.game.id.to_string());
                self.session = None;
                match preview {
                    Some(set) => {
                        self.preview.failed.insert(set, format!("the game stopped ({e:?})"));
                    }
                    None => {
                        self.menu.notify(format!("The game stopped: {e:?}"));
                        self.open_launcher();
                    }
                }
                (0, emulate_time)
            }
        }
    }

    /// Draw the session's current frame into `target` as `view` at
    /// `origin`: this frame's polygons if the geometrizer has them in time
    /// for this refresh (`t0`, when the redraw started), else the previous
    /// frame's; with smooth motion, an in-between.
    ///
    /// With temporal anti-aliasing each image is tagged for the renderer
    /// (`gpu::FrameTag`): the game's frame N as (N, 1) by `present_bus`
    /// itself (drawn again: a repeat), an in-between at t as (N, t)
    /// through `present_bus_motion`, the previous frame's polygons as
    /// that frame by `present_bus_last`. A switch of the machine shown
    /// (run-ahead) starts the accumulation again.
    fn draw_session(
        &mut self,
        target: &wgpu::TextureView,
        view: &model2::video::View,
        origin: (u32, u32),
        format: wgpu::TextureFormat,
        t0: Instant,
    ) {
        self.sync_pack();
        let (Some(g), Some(s)) = (&mut self.gpu, &mut self.session) else { return };
        // When the geometrizer is the slowest part, drawing the previous
        // frame's polygons lets it run alongside the next frame instead of
        // after it, at the cost of the polygons lagging a frame.
        let budget = frame_budget(g.refresh, self.values.exact);
        let period = frame_budget(g.refresh, self.values.exact || self.behind).as_secs_f64();
        // Switching between the game and run-ahead's machine: video RAM
        // versions are each machine's own.
        let ahead = s.run_ahead.as_ref().is_some_and(|ra| ra.is_ahead_of(&s.machine));
        if ahead != s.drew_ahead {
            s.drew_ahead = ahead;
            g.renderer.forget_ram();
            // The other machine's frames: their tags don't follow on.
            g.renderer.reset_history();
        }
        // Anti-aliasing, the depth buffer and smooth shading (by the game's
        // rules): the game's settings, not the launcher preview's (flat).
        // Every image below (game frame, in-between, late, run-ahead) is
        // shaded by the renderer from the polygons it draws.
        let q = if s.preview { gpu::Quality::default() } else { self.values.quality(self.temporal_supported) };
        g.renderer.set_light_rules(&s.game.shading, hud_mask(s.game));
        let temporal = g.renderer.set_quality(q).temporal > 0;
        let bus = &mut s.shown().bus;
        if bus.geo.wait_until(t0 + budget.mul_f64(0.8)) {
            // Smooth motion: the polygons part way from the frame before to
            // this one, by the time since this one was first shown. Only on
            // a display faster than the game, and not when it's already
            // behind.
            let t_smooth = Instant::now();
            // Smooth motion's setting, not just its recording (which
            // temporal anti-aliasing turns on too).
            let fast =
                g.refresh.is_none_or(|hz| hz > FPS * 1.2) && !self.behind && !self.menu.open && self.values.smooth_on();
            // With smooth shading, the in-between's own lighting side
            // channel (its replay records it as the game frame's was), so
            // it is shaded at the level of the frames around it.
            let lit = q.shading != gpu::Shading::Flat && bus.geo.engine().light_on();
            let between = bus.geo.engine().smooth_frame().filter(|_| fast).and_then(|f| {
                let t = (t_smooth.saturating_duration_since(self.smooth_at).as_secs_f64() / period) as f32;
                if lit {
                    f.render_lit(t).map(|(stream, side)| (stream, Some(std::sync::Arc::new(side)), f, t))
                } else {
                    f.render(t).map(|stream| (stream, None, f, t))
                }
            });
            if let Some((_, side, _, _)) = &between {
                g.renderer.set_light_frame(side.clone());
            }
            match &between {
                Some((stream, _, f, t)) if temporal => {
                    // Tagged (N, t): the renderer works out the motion
                    // since the image before. t = 0 is the frame before's
                    // pose, which follows that frame's own image (N-1, 1)
                    // as a step forward, not a repeat.
                    let tag = gpu::FrameTag { frame: f.frame, t: t.max(1e-4) };
                    let m = gpu::BusMotion::of(f, tag, None);
                    g.renderer.present_bus_motion(
                        bus,
                        Some((stream, &f.points)),
                        Some(&m),
                        view,
                        target,
                        format,
                        origin,
                    );
                    self.stats.smooth += 1;
                    self.stats.smoothing += t_smooth.elapsed();
                }
                Some((stream, _, f, _)) => {
                    self.stats.smooth += 1;
                    self.stats.smoothing += t_smooth.elapsed();
                    g.renderer.present_bus(bus, Some((stream, &f.points)), view, target, format, origin)
                }
                // With temporal on, tagged (N, 1) from the engine's smooth
                // frame (a repeat when drawn again).
                None => g.renderer.present_bus(bus, None, view, target, format, origin),
            }
        } else {
            // With temporal on, tagged as the frame these polygons are
            // (`Geometrizer::last_smooth`, kept with `set_keep_last`).
            g.renderer.present_bus_last(bus, view, target, format, origin);
            self.stats.geo_late += 1;
        }
        // Smooth shading: what the frame was drawn as, and its cost (a
        // repeat of a frame costs nothing: the renderer keeps its shading).
        if let (Some(st), Some(eff)) = (g.renderer.light_stats(), g.renderer.light_effective()) {
            self.stats.shaded += 1;
            self.stats.shading += Duration::from_secs_f32(st.ms.max(0.0) / 1000.0);
            self.stats.shading_as = Some((eff, st.lit, st.polys - st.hidden));
        }
    }

    /// Smooth the frame cost (`ran` frames emulated in `emulate`, drawn in
    /// `draw`): below full speed when following the display, pace by the
    /// clock instead, running frames as fast as they can be (dropping to
    /// half the refresh rate otherwise).
    fn note_cost(&mut self, ran: u32, emulate: Duration, draw: Duration) {
        let Some(g) = &self.gpu else { return };
        if ran > 0 {
            let cost = (emulate + draw).as_secs_f64() / ran as f64;
            self.frame_cost += (cost - self.frame_cost) * 0.05;
            let budget = frame_budget(g.refresh, false).as_secs_f64();
            if self.frame_cost > budget * 0.95 {
                self.behind = true;
            } else if self.frame_cost < budget * 0.8 {
                self.behind = false;
            }
        }
        // Run-ahead the machine can't keep up with: off (until it's set
        // again), rather than the game running slow. Counted in frames
        // run, so time in the menu doesn't count.
        self.behind_frames = if self.behind { self.behind_frames + ran } else { 0 };
        if self.behind_frames as f64 > RUN_AHEAD_GRACE * FPS {
            if let Some(s) = self.session.as_mut().filter(|s| s.run_ahead.is_some()) {
                s.run_ahead = None;
                s.run_ahead_dropped = true;
                s.machine.bus.geo.set_smooth(self.values.motion_on());
                self.frame_cost = 0.0;
                self.behind = false;
                self.behind_frames = 0;
                self.menu.notify("Run-ahead is off: this computer can't keep up with it");
                println!("run-ahead     off: too slow to keep up");
            }
        }
    }

    /// One redraw: run the game, draw it and the menu, report status.
    fn frame(&mut self, el: &ActiveEventLoop) {
        // Also while the game isn't reading them (the menu's open, or the
        // launcher), and before it runs (the pad may open the menu).
        self.poll_pads();
        if self.launcher.open {
            return self.launcher_frame(el);
        }
        let t0 = Instant::now();
        let (ran, emulate_time) = self.run_due();
        if self.launcher.open {
            return;
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
        let (view, origin, format) = (g.view, g.origin, g.config.format);
        self.draw_session(&target, &view, origin, format, t0);
        let mut actions = Vec::new();
        let fade =
            self.game_fade.map(|t| 1.0 - t.elapsed().as_secs_f32() / GAME_FADE.as_secs_f32()).filter(|f| *f > 0.0);
        if fade.is_none() {
            self.game_fade = None;
        }
        if self.menu.visible()
            || self.session.is_none()
            || self.splash.active()
            || crosshair.is_some()
            || bezel_at.is_some()
            || fade.is_some()
        {
            let pack_text = self.pack_text();
            let g = self.gpu.as_mut().unwrap();
            let view = menu::View {
                playing: self.session.as_ref().map(|s| s.game.title),
                slots: &slots,
                input_lag: self.session.as_ref().map(|s| s.game.input_lag),
                max_anisotropy: self.max_anisotropy,
                temporal_supported: self.temporal_supported,
                pack: pack_text.as_deref(),
            };
            let navs = std::mem::take(&mut self.navs);
            let (menu, splash, bezel) = (&mut self.menu, &mut self.splash, &mut self.bezel);
            let mut values = self.values.clone();
            let (mut fullscreen, mut for_all) = (self.settings.fullscreen, self.settings.for_all);
            let no_game = self.session.is_none();
            let mut bindings = self.session.as_mut().map(|s| &mut s.bindings);
            let scale = g.window.scale_factor() as f32;
            let size = [g.config.width, g.config.height];
            actions = draw_overlay(g, &target, no_game, size, None, |ctx| {
                if let (Some(b), Some((rect, pic))) = (bezel.as_mut(), bezel_at) {
                    b.paint(ctx, rect, pic, size[0] as f32, scale);
                }
                if let Some((x, y)) = crosshair {
                    draw_crosshair(ctx, egui::pos2(x / scale, y / scale));
                }
                let prefs = menu::Prefs { fullscreen: &mut fullscreen, for_all: &mut for_all };
                let actions = menu.ui(ctx, &mut values, prefs, &view, bindings.as_deref_mut(), &navs);
                if let Some(f) = fade {
                    let painter =
                        ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("game fade")));
                    painter.rect_filled(ctx.content_rect(), 0.0, egui::Color32::from_black_alpha((f * 255.0) as u8));
                    ctx.request_repaint();
                }
                // Over the game or the menu, fading to show it.
                splash.ui(ctx);
                actions
            });
            self.set_prefs(fullscreen, for_all);
            if let Some(set) = self.session.as_ref().map(|s| s.game.id) {
                self.edit_values(set, values);
            }
        }
        // Not counting `present`, which can wait for vsync.
        let draw_time = t_draw.elapsed();
        self.gpu.as_ref().unwrap().renderer.queue().present(frame);
        self.stats.drawn += 1;
        self.stats.drawing += draw_time;
        self.note_cost(ran, emulate_time, draw_time);
        for action in actions {
            match action {
                menu::Action::Resume => self.set_menu(false),
                menu::Action::Save(n) => self.save_slot(n),
                menu::Action::Load(n) => {
                    self.load_slot(n);
                    self.set_menu(false);
                }
                menu::Action::Games => self.open_launcher(),
                menu::Action::Bindings => {
                    if let Some(s) = &self.session {
                        settings::save_bindings(s.game, &s.bindings);
                    }
                }
                menu::Action::Quit => self.quit(el),
            }
        }
        self.status();
    }

    /// The player-wide settings, as the launcher or menu left them.
    fn set_prefs(&mut self, fullscreen: bool, for_all: bool) {
        if (fullscreen, for_all) == (self.settings.fullscreen, self.settings.for_all) {
            return;
        }
        if fullscreen != self.settings.fullscreen {
            if let Some(g) = &self.gpu {
                g.window.set_fullscreen(fullscreen.then_some(Fullscreen::Borderless(None)));
            }
        }
        (self.settings.fullscreen, self.settings.for_all) = (fullscreen, for_all);
        self.settings.save();
    }

    /// The launcher's preview: drop it when the selection has moved (after
    /// fading it out), start getting the selected game's ready once it has
    /// been selected for `launcher::SETTLE`, and start it when it is.
    fn manage_preview(&mut self, now: Instant) {
        let selected = self.launcher.selected();
        let wanted = selected.map(|g| g.id.to_string());
        if wanted != self.preview.wanted {
            self.preview.wanted = wanted.clone();
            self.previews.cancel_loads();
        }
        let showing = self.session.as_ref().map(|s| s.game.id);
        if showing.is_some() && showing != wanted.as_deref() {
            let since = *self.preview.leaving.get_or_insert(now);
            if now - since >= FADE_OUT {
                self.session = None;
                self.preview.leaving = None;
            }
        } else {
            self.preview.leaving = None;
        }
        let settled = now - self.launcher.selected_at >= launcher::SETTLE;
        if let Some(game) = selected.filter(|g| settled && !self.preview.failed.contains_key(g.id)) {
            if self.session.is_none() && !self.previews.pending(game.id) {
                self.previews.request(game, &self.settings.rom, self.values.geo_mode(), self.opts.interp);
            }
        }
        for ev in self.previews.poll() {
            match ev {
                preview::Event::Ready(r) => {
                    if self.session.is_none() && wanted.as_deref() == Some(r.game.id) {
                        self.start_preview(*r);
                    } else if r.made {
                        println!(
                            "preview       {}: made the attract state in {:.0} ms",
                            r.game.id,
                            r.took.as_secs_f64() * 1000.0
                        );
                    }
                }
                preview::Event::Failed(set, e) => {
                    eprintln!("preview       {set}: {e}");
                    self.preview.failed.insert(set, e);
                }
            }
        }
        // The sound fades in once the preview shows.
        if let (Some(s), Some(shown)) = (&self.session, self.preview.shown) {
            let leaving = self.preview.leaving.map(|t| 1.0 - (now - t).as_secs_f32() / FADE_OUT.as_secs_f32());
            let f = ((now - shown).as_secs_f32() / SOUND_FADE.as_secs_f32()).min(1.0) * leaving.unwrap_or(1.0);
            if let Some(snd) = &s.sound {
                if f < 1.0 || leaving.is_some() || self.redraws.is_multiple_of(30) {
                    let db = PREVIEW_DB + fade_db(f);
                    snd.set_mix(self.values.sfx_db + db, self.values.music_db + db);
                }
            }
        }
    }

    /// One redraw with the launcher open.
    fn launcher_frame(&mut self, el: &ActiveEventLoop) {
        let now = Instant::now();
        if let Some(set) = self.launcher.selected().map(|g| g.id) {
            self.use_values(set);
        }
        self.manage_preview(now);
        let t0 = Instant::now();
        let (ran, emulate_time) = self.run_due();
        let Some(g) = &mut self.gpu else { return };
        let frame = match g.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            _ => {
                g.surface.configure(g.renderer.device(), &g.config);
                self.stats.surface_misses += 1;
                return;
            }
        };
        let t_draw = Instant::now();
        let target = frame.texture.create_view(&Default::default());
        let size = [g.config.width, g.config.height];
        let ppp = g.window.scale_factor() as f32;
        let actions = self.draw_launcher_at(&target, size, ppp, t0);
        let draw_time = t_draw.elapsed();
        self.gpu.as_ref().unwrap().renderer.queue().present(frame);
        self.stats.drawn += 1;
        self.stats.drawing += draw_time;
        self.note_cost(ran, emulate_time, draw_time);
        for action in actions {
            match action {
                launcher::Action::Play { set, resume } => {
                    self.launcher.open = false;
                    self.launcher_bindings = None;
                    self.session = None;
                    self.preview =
                        PreviewState { failed: std::mem::take(&mut self.preview.failed), ..Default::default() };
                    self.start_game(&set, resume);
                    if self.session.is_some() {
                        self.game_fade = Some(Instant::now());
                    }
                }
                launcher::Action::Rescan(rom) => {
                    self.settings.rom = rom.into();
                    self.settings.save();
                    self.scan();
                }
                launcher::Action::ResetGame(set) => {
                    self.settings.reset_game(&set);
                    self.use_values(&set);
                }
                launcher::Action::Quit => self.quit(el),
            }
        }
        self.status();
    }

    /// Draw the launcher into `target` (`size` pixels at `ppp` pixels per
    /// point): the backdrop, the preview, and the launcher over them.
    fn draw_launcher(&mut self, target: &wgpu::TextureView, size: [u32; 2], ppp: f32, t0: Instant) {
        self.draw_launcher_at(target, size, ppp, t0);
    }

    fn draw_launcher_at(
        &mut self,
        target: &wgpu::TextureView,
        size: [u32; 2],
        ppp: f32,
        t0: Instant,
    ) -> Vec<launcher::Action> {
        let now = Instant::now();
        let secs = (now - self.started).as_secs_f64();
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32) / ppp);
        let l = launcher::layout(screen);
        // The backdrop's horizon sits behind the cards' tops.
        let horizon = (l.cards_y - l.card.y * 0.62) / screen.height();
        let g = self.gpu.as_mut().unwrap();
        let params = backdrop::Params {
            size: [size[0] as f32, size[1] as f32],
            horizon,
            scale: ppp * l.u,
            phase: (secs * 0.18).fract() as f32,
            drift: (secs * 0.004 % 1000.0) as f32,
            level: 1.0,
            _pad: 0.0,
        };
        g.backdrop.draw(g.renderer.device(), g.renderer.queue(), target, &params);
        // The preview: the game drawn as the window would show it, at the
        // preview's size.
        let hero = l.hero;
        let pw = (hero.width() * ppp).round().max(16.0) as u32;
        let ph = (hero.height() * ppp).round().max(16.0) as u32;
        let mut preview_view = None;
        if self.session.as_ref().is_some_and(|s| s.preview) {
            let (view, origin) = self.fit(pw, ph);
            let g = self.gpu.as_mut().unwrap();
            let format = g.config.format;
            if g.preview.as_ref().is_none_or(|p| p.size != [pw, ph]) {
                let texture = g.renderer.device().create_texture(&wgpu::TextureDescriptor {
                    label: Some("preview"),
                    size: wgpu::Extent3d { width: pw, height: ph, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                });
                let view = texture.create_view(&Default::default());
                let device = g.renderer.device();
                let id = match &g.preview {
                    Some(p) => {
                        g.overlay.renderer.update_egui_texture_from_wgpu_texture(
                            device,
                            &view,
                            wgpu::FilterMode::Linear,
                            p.id,
                        );
                        p.id
                    }
                    None => g.overlay.renderer.register_native_texture(device, &view, wgpu::FilterMode::Linear),
                };
                g.preview = Some(PreviewTarget { _texture: texture, view, id, size: [pw, ph] });
            }
            let pt = g.preview.as_ref().unwrap();
            let (pview, id) = (pt.view.clone(), pt.id);
            self.draw_session(&pview, &view, origin, format, t0);
            let shown = *self.preview.shown.get_or_insert(now);
            let fade_in = ((now - shown).as_secs_f32() / launcher::FADE_IN.as_secs_f32()).min(1.0);
            let fade_out = self.preview.leaving.map_or(1.0, |t| 1.0 - (now - t).as_secs_f32() / FADE_OUT.as_secs_f32());
            let bezel = self.values.bezel && view.pad < 1.0 && (view.width as u32) + 1 < pw;
            preview_view = Some(launcher::PreviewView {
                texture: id,
                alpha: (fade_in * fade_out).clamp(0.0, 1.0),
                picture: [
                    origin.0 as f32 / pw as f32,
                    origin.1 as f32 / ph as f32,
                    view.width as f32 / pw as f32,
                    view.height as f32 / ph as f32,
                ],
                bezel,
            });
        }
        let selected = self.launcher.selected();
        let resume = selected.and_then(|g| std::fs::metadata(resume_path(g.id)).and_then(|m| m.modified()).ok());
        let progress = selected.and_then(|g| self.previews.progress(g.id));
        let failed = selected.and_then(|g| self.preview.failed.get(g.id)).cloned();
        let has_own = selected.is_some_and(|g| self.settings.has_own(g.id));
        let view = launcher::View {
            preview: preview_view,
            progress: progress.filter(|_| preview_view.is_none()),
            no_preview: failed.as_deref(),
            resume,
            unsupported: self.unsupported,
        };
        if self.launcher.panel == launcher::Panel::Controls && self.launcher_bindings.is_none() {
            self.launcher_bindings = selected.map(settings::load_bindings);
        }
        if self.launcher.panel != launcher::Panel::Controls {
            self.launcher_bindings = None;
        }
        let navs = std::mem::take(&mut self.navs);
        let mut values = self.values.clone();
        let mut for_all = self.settings.for_all;
        let sources = self.sources;
        let (launcher, menu, splash) = (&mut self.launcher, &mut self.menu, &mut self.splash);
        let bindings = &mut self.launcher_bindings;
        let title = selected.map_or("", |g| g.title);
        let animating = launcher.animating() || self.session.is_some() || progress.is_some();
        let g = self.gpu.as_mut().unwrap();
        let mut changed = false;
        let actions = draw_overlay(g, target, false, size, Some(ppp), |ctx| {
            let actions = launcher.ui(ctx, navs.clone(), &view, &mut values, &sources, &mut for_all, has_own);
            if launcher.panel == launcher::Panel::Controls {
                if let Some(b) = bindings.as_mut() {
                    menu.controls = true;
                    changed |= menu.controls_page(ctx, b, title, &navs);
                    if !menu.controls {
                        launcher.panel = launcher::Panel::Settings;
                    }
                }
            } else {
                menu.capture = None;
            }
            menu.notices(ctx);
            splash.ui(ctx);
            if animating {
                ctx.request_repaint();
            }
            actions
        });
        if changed {
            if let (Some(game), Some(b)) = (selected, &self.launcher_bindings) {
                settings::save_bindings(game, b);
            }
        }
        self.set_prefs(self.settings.fullscreen, for_all);
        if let Some(set) = selected.map(|g| g.id) {
            self.edit_values(set, values);
        }
        actions
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
            "{mode} · {:.0}% speed · {:.0} fps ({} repeats, longest gap {:.0} ms) · emulate {:.1} ms/frame · draw {:.2} ms{audio}{}",
            st.emulated as f64 / secs / FPS * 100.0,
            st.drawn as f64 / secs,
            st.repeats,
            st.longest_gap.as_secs_f64() * 1000.0,
            st.emulating.as_secs_f64() * 1000.0 / st.emulated.max(1) as f64,
            st.drawing.as_secs_f64() * 1000.0 / st.drawn.max(1) as f64,
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
        } + &match st.shading_as {
            Some((level, lit, polys)) => format!(
                " · shading {}{}, {lit}/{polys} polygons lit, {:.2} ms a draw",
                level.name(),
                if self.applied.shading_fine { " fine" } else { "" },
                st.shading.as_secs_f64() * 1000.0 / st.shaded.max(1) as f64
            ),
            None => String::new(),
        } + &match self.session.as_ref().map(|s| (&s.run_ahead, s.run_ahead_dropped)) {
            // Of the emulate time above: copying the game, and its frames ahead.
            Some((Some(ra), _)) if st.ahead > 0 => format!(
                " · run-ahead {} ({:.1} ms of that, copy {:.2} ms)",
                ra.frames(),
                st.run_ahead.as_secs_f64() * 1000.0 / st.ahead as f64,
                ra.stats.copying.as_secs_f64() * 1000.0 / ra.stats.forks.max(1) as f64,
            ),
            Some((None, true)) => " · run-ahead off (too slow)".to_string(),
            _ => String::new(),
        };
        if let Some(g) = &self.gpu {
            let title = match &self.session {
                Some(s) if s.preview => format!("reModel · preview: {} · {status}", s.game.title),
                Some(s) => format!("reModel: {} · {}x{} · {status}", s.game.title, g.view.width, g.view.height),
                None if self.launcher.open => "reModel".to_string(),
                None => format!("reModel: no game · {status}"),
            };
            g.window.set_title(&title);
        }
        let reports = self.stats.reports + 1;
        if reports.is_multiple_of(5) && !self.menu.open {
            println!("status        {}{status}", if self.launcher.open { "launcher · " } else { "" });
        }
        let misses = self.stats.surface_misses;
        self.stats = Stats { reports, surface_misses: misses, ..Stats::new() };
    }

    /// Open or close the menu; the game pauses while it's open.
    fn set_menu(&mut self, open: bool) {
        self.menu.open = open;
        // Keys held when the menu opened would stay pressed otherwise; the
        // pad's Start and Select are kept from the game until released.
        self.keys.clear();
        self.gate.menu_changed();
        self.pad_nav = pad::PadNav::default();
        self.navs.clear();
        self.menu.pad_used = false;
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
            let what = if s.preview { "preview" } else { "frames" };
            println!("{what:<14}{}, state hash {hash:016x}", s.machine.frames);
            if let Some(d) = &s.machine.bus.drive {
                println!("drive board   {}", d.describe());
            }
        }
        self.quit(el);
        true
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        self.previews.cancel_all();
        self.leave_game();
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

/// egui's look in the logo's colours: the menu, the Controls page and the
/// launcher's widgets.
fn style(ctx: &egui::Context) {
    use launcher::{CYAN, FG, LINE, PANEL};
    ctx.all_styles_mut(|s| {
        let v = &mut s.visuals;
        *v = egui::Visuals::dark();
        v.window_fill = PANEL;
        v.panel_fill = PANEL;
        v.window_stroke = egui::Stroke::new(1.0, LINE);
        v.window_corner_radius = 12.0.into();
        v.selection.bg_fill = egui::Color32::from_rgba_unmultiplied(47, 230, 239, 90);
        v.selection.stroke = egui::Stroke::new(1.0, CYAN);
        v.hyperlink_color = CYAN;
        v.override_text_color = Some(FG);
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, CYAN);
        v.widgets.active.bg_stroke = egui::Stroke::new(1.5, CYAN);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.item_spacing = egui::vec2(10.0, 7.0);
    });
}

/// Draw egui over `target` (`size` pixels, at `ppp` pixels per point, else
/// the window's scaling), building the UI with `ui`; returns what it
/// returned. `clear` clears to black first (no game under it).
fn draw_overlay<T: Default>(
    g: &mut Gpu,
    target: &wgpu::TextureView,
    clear: bool,
    size: [u32; 2],
    ppp: Option<f32>,
    mut ui: impl FnMut(&egui::Context) -> T,
) -> T {
    let o = &mut g.overlay;
    let mut raw = o.state.take_egui_input(&g.window);
    let ppp = ppp.unwrap_or(g.window.scale_factor() as f32);
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
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
        // The app id (Wayland) and class (X11) the desktop entry names
        // (`make install-desktop`), so the window gets its icon.
        #[cfg(all(unix, not(target_os = "macos")))]
        let attrs = {
            use winit::platform::{wayland::WindowAttributesExtWayland, x11::WindowAttributesExtX11};
            let attrs = WindowAttributesExtWayland::with_name(attrs, "remodel", "remodel");
            WindowAttributesExtX11::with_name(attrs, "remodel", "remodel")
        };
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
            required_features: gpu::features(&adapter),
            ..Default::default()
        }))
        .expect("create device");
        let max_msaa = gpu::max_msaa(&adapter);
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
        style(&ctx);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui_renderer = egui_wgpu::Renderer::new(&device, config.format, Default::default());
        let backdrop = backdrop::Backdrop::new(&device, config.format);
        let mut renderer = gpu::Renderer::from_device(device, queue, name);
        renderer.set_max_msaa(max_msaa);
        self.max_anisotropy = gpu::max_anisotropy(&adapter);
        renderer.set_max_anisotropy(self.max_anisotropy);
        self.launcher.max_anisotropy = self.max_anisotropy;
        // `from_device` assumes it; the GL fallback may not have it.
        self.temporal_supported = gpu::temporal_supported(&adapter);
        renderer.set_temporal_supported(self.temporal_supported);
        self.launcher.temporal_supported = self.temporal_supported;
        if !self.temporal_supported {
            println!("temporal AA   not on this GPU (no 16-bit float targets)");
        }
        let (view, origin) = self.fit(config.width, config.height);
        // Redraws drive everything; Wayland sends none until asked.
        window.request_redraw();
        let refresh = refresh_hz(&window);
        let overlay = Overlay { ctx, state, renderer: egui_renderer };
        self.gpu =
            Some(Gpu { window, surface, config, renderer, view, origin, refresh, overlay, backdrop, preview: None });
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
                if pressed && !event.repeat && code == KeyCode::F11 {
                    let fullscreen = !self.settings.fullscreen;
                    return self.set_prefs(fullscreen, self.settings.for_all);
                }
                // The launcher reads its keys from egui.
                if self.launcher.open {
                    return;
                }
                if pressed && !event.repeat {
                    match code {
                        KeyCode::Escape => return self.set_menu(!self.menu.open),
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
            WindowEvent::MouseInput { state, button, .. } if !self.menu.open && !to_menu && !self.launcher.open => {
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
        settings,
        values: GameSettings::default(),
        sources: [Source::Default; KEYS.len()],
        applied: GameSettings::default(),
        values_for: String::new(),
        session: None,
        games: Vec::new(),
        unsupported: 0,
        menu: menu::Menu::default(),
        launcher: launcher::Launcher::default(),
        previews: preview::Previews::default(),
        preview: PreviewState::default(),
        launcher_bindings: None,
        splash,
        bezel: None,
        gpu: None,
        keys: HashSet::new(),
        gilrs,
        pad: None,
        pad_nav: pad::PadNav::default(),
        navs: Vec::new(),
        gate: pad::Gate::default(),
        mouse: None,
        mouse_used: false,
        mouse_buttons: [false; 2],
        stick: model2::input::StickCursor::default(),
        epoch: Instant::now(),
        smooth_at: Instant::now(),
        game_fade: None,
        redraws: 0,
        last_emulate: None,
        frame_cost: 0.0,
        behind: false,
        behind_frames: 0,
        stats: Stats::new(),
        started: Instant::now(),
        pack: PackState::default(),
        max_anisotropy: 16,
        temporal_supported: true,
    };
    if play.settings.rom == Settings::default().rom {
        // So there's an obvious place to put sets.
        let _ = std::fs::create_dir_all(&play.settings.rom);
    }
    play.scan();
    // `--game`: straight into it; else the launcher.
    match play.opts.game.clone() {
        Some(set) if play.games.iter().any(|g| g.id == set) => play.start_game(&set, false),
        Some(set) => {
            println!("no {set} in {}: choose a game", play.settings.rom.display());
            play.menu.notify(format!("No {set} in {}", play.settings.rom.display()));
            play.launcher.open = true;
        }
        None => play.launcher.open = true,
    }
    let event_loop = EventLoop::new().expect("event loop");
    event_loop.run_app(&mut play).expect("event loop");
}
