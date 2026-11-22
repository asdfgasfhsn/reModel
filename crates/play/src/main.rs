//! remodel-play: Model 2 games in a native window.
//!
//! The main thread runs the window (winit), input, the board model and the
//! menu (egui, over the game). Drawing (the GPU renderer, presented with
//! vsync) runs on the draw thread, from a copy of the frame, alongside the
//! next frame's emulation (`draw`; on the main thread with
//! `--no-overlap`). The geometrizer runs on its own thread (the board
//! model's default) and the sound boards on another (`sound`), playing
//! through the system's audio API.
//!
//! Pacing: with a display near the game's 57.524 Hz (60 Hz, or 120 Hz at
//! one game frame per two refreshes) the game runs exactly one frame per
//! refresh, so motion is perfectly even; that's 4.3% fast at 60 Hz, and the
//! audio is resampled to match. With exact speed, or other refresh rates,
//! frames follow a steady clock at 57.524 per second instead. Either way
//! the audio queue corrects the drift between the clocks by stretching the
//! audio by at most 1% (inaudible), never by stalling the game. When the
//! game persistently runs below full speed the audio follows the frames it
//! actually runs (`model2::pacing::Pace`), so it slows with it instead of
//! running dry.
//!
//! The logo shows for two seconds at startup, fading out over the game
//! booting underneath (`splash`). With no game given, the launcher opens
//! (`launcher`): the games as a carousel or a grid with filters, the
//! selected one's attract mode live (`preview`) over a wireframe backdrop
//! (`backdrop`), its settings and saves, and a quick resume of the game last
//! played. Esc opens the menu (and pauses): save
//! states, sound mix, display, back to the games. Settings persist
//! (`settings`). Beside a 4:3 picture in a wider window, the game's bezel
//! (`bezel`). With the texture pack setting on, replacement textures come
//! from the game's pack directory, decoded in the background (`pack`).

mod art;
mod backdrop;
mod bezel;
mod controls;
mod draw;
mod focus;
mod hints;
mod launcher;
mod library;
mod link;
mod log;
mod menu;
mod netpage;
mod netplay;
mod pack;
mod pad;
mod perf;
mod photo;
mod preview;
mod saves;
mod settings;
mod sound;
mod splash;
mod steer;
mod sticklog;
mod sync;
mod theme;

use gilrs::Gilrs;
use model2::pacing::{FPS, MAX_CATCH_UP};
use settings::{GameSettings, Settings, Source, KEYS};
use sound::Sound;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{CursorIcon, Fullscreen, Window, WindowId};

const USAGE: &str = "\
usage: remodel-play [options]

  -V, --version      print the version and exit
  --game ID          start this set straight away (default: the launcher,
                     to choose one)
  --rom DIR          ROM folder: MAME sets as zips or folders (default: the
                     last one used; else rom in the reModel source tree,
                     or roms in the data folder anywhere else: see
                     README.txt)
  --mix SFX:MUSIC    sound board and music board gains in dB (default 8:-8)
  --no-sound         don't run the sound boards
  --hud center|edges wide windows: keep the HUD centred, or move each HUD
                     element to the nearest screen edge (default edges)
  --aspect fit|4:3   fit the window, drawing the game wider than its frame
                     (the default), or keep its 4:3 picture
  --bezel on|off     the game's bezel beside a 4:3 picture in a wider
                     window (default on)
  --players N        link play: N machines of the game (2 to 4, where the
                     game can) linked on one screen, one controller each:
                     the keyboard plays P1, gamepads join P2 up as a
                     button is pressed (docs/play.md, \"Link play\")
  --input2 S .. --input4 S
                     link play: scripted inputs for players 2 to 4
                     (--input is player 1's, and everyone's without their own)
  --link-layout auto|side|stacked
                     link play: two players side by side or stacked (default
                     auto: whichever gives each picture more room)
  --free-play        start with the game's own FREE PLAY coin setting (games that
  --no-free-play     have one; the default is on in link play, off in a single
                     game; docs/play.md, \"Free play\")
  --link-audio p1|mix|pan|p1only
                     link play: whose sound you hear: player 1's machine
                     loudest (default), all equal, spread left to right,
                     or player 1's only (the others' sound boards off)
  --link-keyboard on|off
                     link play: whether the keyboard plays P1 (default on);
                     off, gamepads take the seats from P1
  --host [PORT]      network link play with --game: host a session on this
                     machine (default port 47890; the lobby shows its
                     address) and wait for the other machines
                     (docs/play.md, \"Network link play\")
  --join HOST:PORT   network link play with --game: join the host there
  --net-players N    with --host: start when N players (this one included)
                     have joined, without pressing Start
  --name NAME        network link play: the name the other players see, for
                     this run (default: the one set on the launcher's Link
                     play page, first the login name); at most 16 characters
  --interp           run the i960 on the interpreter even if this build
                     has recompiled code
  --no-jit           don't use the JIT (the default runs the game's code
                     translated to this machine's as it runs, docs/jit.md,
                     on background workers, with the blocks it translated
                     kept per game and translated again at boot); the
                     recompiled code if this build has it, else the
                     interpreter (--jit is accepted and changes nothing)
  --jit-warm         a game with no hot-block list yet gets
                     one from a quiet run in the background (coin and
                     start, no window or sound), for its next boot
  --catalogue [on|off]
                     observe the game into the catalogue as it is played
                     (the Catalogue setting): every distinct texture, tile
                     pattern, 2D layout and object it shows, and the
                     texture RAM it loads, sampled every 60 frames and when
                     the screen changes, the work done off the game's
                     thread (docs/play.md, \"Catalogue\")
  --catalogue-dir D  where (default out/GAME/catalogue, or the data
                     folder's GAME/catalogue)
  --catalogue-every N  frames between samples (default 60)
  --perf-log         print a line for each redraw that took longer to
                     emulate than its frames' share of real time, with
                     what the JITs did in it, and for each that came over
                     1.5 refreshes after the one before, where the time
                     went; frame times and the late redraws summed up on
                     leaving the game (docs/play.md, Slowdowns)
  --no-idle-skip     run the game's idle loops instruction by instruction
                     instead of skipping through them (same game, slower;
                     docs/idle-skip.md)
  --no-overlap       draw each frame on the main thread after emulating
                     it, instead of on the draw thread alongside the next
                     frame's emulation (for comparison; docs/play.md, \"The
                     frame loop\")
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
  --shading-balance on|off
                     with --shading: keep each smooth surface's mean
                     brightness the game's (one offset per surface); off
                     uses the game's formula at the smoothed normals
                     alone, so bumpy surfaces come out brighter. Default on
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
  --materials on|off the texture pack's material maps (normal maps): the
                     surfaces' bumps and grooves lit by the game's own
                     lights. Needs --pack, a pack with maps and the native
                     geometrizer (no effect with --geo chip); works with
                     any --shading (docs/texture-packs.md). Default off
  --materials-relief off|S
                     with --materials: relief at strength S (0.1 to 8),
                     the bumps' light change relative to the surface's
                     brightness (shows on fully lit scenery too), or off
                     for the faithful rule (the map's normal in the game's
                     own lighting sum: little shows where the game lights
                     to full). Default 2
  --bloom off|N      bloom: the 3D layer's bright parts glow, N percent
                     strong (1 to 100; 25 light, 50 medium, 100 strong).
                     The HUD and tile layers are left out, it works with
                     any anti-aliasing and is GPU only. Default off
  --crt off|N        the CRT look (scanlines, mask, glow, vignette) over the
                     finished picture, N percent (1 to 100; 60 is the CRT
                     style). GPU only. Default off
  --exact            run at the arcade's exact 57.524 frames per second
                     (default: one frame per display refresh when the
                     refresh is 60 or 120 Hz, which is 4.3% fast but smooth)
  --fullscreen       start fullscreen
  --seconds N        quit after N seconds (for timing runs)
  --frames N         quit when the game reaches frame N (the game stops
                     there; with --shot, a picture of that frame, to set
                     beside the CLI's of the same frame)
  --no-splash        don't show the logo at startup
  --input SCRIPT     scripted inputs, as for `remodel run` (for timing runs),
                     e.g. coin-start (the game's own coin-up and start)
  --shot FILE        with --seconds or --frames: at the end, draw the last
                     frame (the game, bezel included, or the launcher) as
                     a window would show it to a PNG
  --shot-size WxH[@S] the --shot window's size in pixels (default 1920x1080)
                     and its display scaling (e.g. 3840x2160@2)
  --nav LIST         scripted gamepad input, for checking with --shot:
                     NAME@SECONDS, comma separated; names left, right, up,
                     down (the d-pad), accept (A), back (B), continue (Y),
                     saves (X), settings (the menu button), prev, next
                     (shoulders), lt, rt (triggers), start, select, guide; a+b
                     presses both, @S:H holds them H seconds
  --photo-history N  photo mode's time scrub keeps the last N seconds of
                     the game to go back through (save states every
                     second and the inputs between, compressed on another
                     thread; 0: none, photo mode then freezes two frames
                     on). Default 10
  --photo LIST       scripted photo mode, for checking: at game frame F,
                     open it, set KEY=VALUE..., take a photo, close it:
                     F[:KEY=VALUE,...], sessions separated by ; (keys in
                     docs/photo-mode.md, e.g. 1500:orbit=30,dof=1,res=4k)
  --photo-dir DIR    where photos go (default: reModel/<set>/ in your
                     pictures folder)
  --fast             for checking (with --frames, --input, --photo): run
                     the game as fast as it goes, drawing nothing (a
                     hidden window gets about a redraw a second)
  --stick-log FILE   what the gamepad's sticks read, a CSV line a game
                     frame (raw, tuned, smoothed), and at exit how noisy
                     each stick is at rest: hold it still for 5 s, then
                     steer (docs/play.md, Measuring a pad)
  --sound-log FILE   every byte the game sends its sound boards, a line
                     each: the set, the game frame, the bus cycle and
                     the byte in hex (control-register writes marked
                     ctl), to see whether a sound or music command was
                     sent when one is missing (docs/play.md, Slowdowns)
  --pack-log FILE    for checking a texture pack: for each frame drawn,
                     every texture, tile pattern and region looked up in
                     the pack and whether its replacement or the original
                     was drawn, and why (`remodel pack-flicker FILE`
                     reports what alternates; docs/texture-packs.md,
                     \"Flicker\")
  --pack-fixes LIST  which stabilisations of the pack's replacements are on:
                     all (default), none, or surface, hold, regions,
                     preload separated by commas (to compare runs)
  --frame-log FILE   for checking: each picture presented, read back and
                     hashed, a line each: the shown machine's frame, game
                     or ahead (run-ahead's machine), -late if drawn with
                     the frame before's polygons, the hash. Slow; keeps
                     run-ahead on when the game falls behind

The on|off options (--smooth, --depth, --shading-fine, --shading-balance,
--pack, --materials) also work bare, as on.

The game's settings (mix, HUD, aspect, bezel, speed, smooth motion,
run-ahead, anti-aliasing, temporal anti-aliasing and stipple, depth buffer,
shading, crease, finer levels and brightness balance, texture pack and
filtering, material maps and their rule, bloom, geometrizer) given here are for this run only, over the
defaults for all games and the game's own, which the menu (from the launcher
too) keeps.
--rom and --fullscreen are remembered.

keys: Esc menu (pause), F5 quick save, F9 quick load, F8 photo mode, F11
fullscreen. The
game's controls depend on the game (docs/play.md); C coins up and arrows
steer. Gamepads work too: Guide, or Start and Select together, opens the
menu. The menu's Controls page remaps them per game.
";

/// The game's HUD windows, a bit each: smooth shading leaves them flat
/// (`gpu::Renderer::set_light_rules`).
fn hud_mask(game: &romset::Game) -> u64 {
    model2::light::window_mask(game.hud_windows)
}

/// What is left of a fade `total` long, `elapsed` into it: 1 falling to 0
/// (and below, once it is over).
fn fade_left(elapsed: Duration, total: Duration) -> f32 {
    1.0 - elapsed.as_secs_f32() / total.as_secs_f32()
}
/// Narrowest and widest views; windows beyond are letterboxed.
const ASPECTS: (f32, f32) = (4.0 / 3.0, 32.0 / 9.0);
/// The launcher's preview plays this much quieter than the game.
const PREVIEW_DB: f32 = -9.0;
/// The preview's sound fades in over this, its picture out over `FADE_OUT`.
const SOUND_FADE: Duration = Duration::from_millis(1200);
/// Following the display: a gap between redraws longer than this is a
/// stall (a driver or window-system hitch), counted as one refresh rather
/// than caught up (as the audio rate control, `sound::STALL`).
const REFRESH_STALL: f64 = 0.1;
/// The refreshes at `hz` that went by in the `dt` seconds since the last
/// redraw, with `part` the fraction of one carried between calls: 1 for
/// the first redraw or after a stall (`REFRESH_STALL`), 0 for an extra
/// early one.
fn refreshes_gone(part: &mut f64, dt: f64, hz: f64) -> u64 {
    if dt <= 0.0 || dt > REFRESH_STALL {
        *part = 0.0;
        return 1;
    }
    let r = *part + dt * hz;
    let n = r.round().max(0.0);
    *part = r - n;
    n as u64
}

/// How often a network game's held frame boundary is tried again between
/// redraws (the others' records are read from the socket then).
const NET_POLL: Duration = Duration::from_micros(500);

/// The sooner of the loop's own wake-up times, if it has any.
fn el_flow_deadline(a: Option<Instant>, b: Option<Instant>) -> Option<Instant> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Frames run in one redraw to catch up with the game's time (the least
/// the player's pacing allows; `model2::pacing::MAX_CATCH_UP` bounds it).
const CATCH_UP: u32 = 2;

/// Draws in a row that panicked before the player gives up and quits
/// (rather than show a frozen window).
const DRAW_FAILURES_TO_QUIT: u32 = 120;
const FADE_OUT: Duration = Duration::from_millis(200);
/// Seconds of game the player may run behind before run-ahead is turned
/// off.
const RUN_AHEAD_GRACE: f64 = 3.0;

/// Redraws of the game to measure before saying how much run-ahead the
/// computer has time for.
const COST_FRAMES: u32 = 120;
/// A game started from the launcher fades in from black.
const GAME_FADE: Duration = Duration::from_millis(400);

/// Options for this run only (the game settings go into the settings'
/// command-line layer).
struct Options {
    sound: bool,
    interp: bool,
    /// The CPUs on the JITs (the game's own sessions); on unless
    /// `--no-jit` or `--interp`.
    jit: bool,
    /// `--jit-warm`: warm a game's missing hot-block list in the
    /// background.
    jit_warm: bool,
    /// `--perf-log`: slow redraws and frame times (`perf`).
    perf_log: bool,
    /// `--catalogue-dir`, `--catalogue-every`.
    catalogue_dir: Option<PathBuf>,
    catalogue_every: u64,
    seconds: Option<f64>,
    /// `--frames`: quit at this game frame.
    frames: Option<u64>,
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
    /// Draw on the draw thread alongside the next frame's emulation (not
    /// `--no-overlap`).
    overlap: bool,
    /// `--frame-log`: where to log each presented picture's hash.
    frame_log: Option<PathBuf>,
    /// `--pack-log`: where to log what the texture pack's lookups gave in
    /// each frame drawn (`remodel pack-flicker` reports on it), and
    /// `--pack-fixes`: which stabilisations of the pack's replacements are
    /// on.
    pack_log: Option<PathBuf>,
    pack_fixes: model2::packlog::Fixes,
    /// `--sound-log`: where to log the sound link's bytes.
    sound_log: Option<PathBuf>,
    /// `--stick-log`: write what the pad's sticks read, a line a frame.
    stick_log: Option<PathBuf>,
    /// `--photo`: scripted photo mode sessions, in frame order.
    photo: std::collections::VecDeque<photo::Script>,
    /// `--photo-dir`: where photos go.
    photo_dir: Option<PathBuf>,
    /// `--photo-history`: seconds of the game kept for the time scrub.
    photo_history: f64,
    /// `--fast`: run as fast as the game goes, drawing nothing.
    fast: bool,
    /// `--players`: link play with this many machines (0: a single game).
    players: usize,
    /// `--input2` to `--input4`, by player (index 0 and 1 unused).
    link_inputs: Vec<Option<String>>,
    /// `--link-keyboard`: the keyboard plays player 1.
    link_keyboard: bool,
    /// `--host [PORT]` / `--join ADDR`: network link play.
    net: Option<NetMode>,
    /// `--net-players`.
    net_players: usize,
    /// `--name`: the player's name for this run.
    name: Option<String>,
}

/// Network link play from the command line.
#[derive(Clone, Debug)]
enum NetMode {
    Host(u16),
    Join(String),
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
                "continue" => Button::North,
                "saves" => Button::West,
                "settings" => Button::Start,
                "prev" => Button::LeftTrigger,
                "next" => Button::RightTrigger,
                "lt" => Button::LeftTrigger2,
                "rt" => Button::RightTrigger2,
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

/// An on/off option's value: `on` or `off` if that comes next, else the
/// bare flag, which means on (as before the options took a value).
fn opt_bool(args: &mut std::iter::Peekable<impl Iterator<Item = String>>) -> bool {
    match args.peek().map(String::as_str) {
        Some("on") => {
            args.next();
            true
        }
        Some("off") => {
            args.next();
            false
        }
        _ => true,
    }
}

fn options(s: &mut Settings) -> Result<Options, String> {
    let mut o = Options {
        sound: true,
        interp: false,
        jit: true,
        jit_warm: false,
        perf_log: false,
        catalogue_dir: None,
        catalogue_every: 60,
        seconds: None,
        frames: None,
        input: None,
        game: None,
        splash: true,
        shot: None,
        shot_size: ([1920, 1080], 1.0),
        nav: Default::default(),
        texture_pack: None,
        overlap: true,
        frame_log: None,
        pack_log: None,
        pack_fixes: Default::default(),
        sound_log: None,
        stick_log: None,
        photo: Default::default(),
        photo_dir: None,
        photo_history: 10.0,
        fast: false,
        players: 0,
        link_inputs: vec![None; 4],
        link_keyboard: true,
        net: None,
        net_players: 0,
        name: None,
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
            "--jit" => o.jit = true,
            "--no-jit" => o.jit = false,
            "--jit-warm" => o.jit_warm = true,
            "--perf-log" => o.perf_log = true,
            "--catalogue" => {
                let on = opt_bool(&mut args);
                cli("catalogue", &on.to_string());
            }
            "--catalogue-dir" => o.catalogue_dir = Some(value()?.into()),
            "--catalogue-every" => {
                o.catalogue_every =
                    value()?.parse().ok().filter(|&n| n > 0).ok_or("--catalogue-every wants a number above 0")?
            }
            "--no-idle-skip" => model2::idle::set_default(false),
            "--smooth" => {
                let on = opt_bool(&mut args);
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
                    _ => {
                        return Err(format!(
                            "--run-ahead wants 0 to {} (a game may take fewer: it warns above its own)",
                            settings::MAX_RUN_AHEAD
                        ))
                    }
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
                let on = opt_bool(&mut args);
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
                let on = opt_bool(&mut args);
                cli("shading_fine", &on.to_string());
            }
            "--shading-balance" => {
                let on = opt_bool(&mut args);
                cli("shading_balance", &on.to_string());
            }
            "--pack" => {
                let on = opt_bool(&mut args);
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
            "--materials" => {
                let on = opt_bool(&mut args);
                cli("materials", &on.to_string());
            }
            "--materials-relief" => {
                let Some(r) = gpu::Relief::parse(&value()?) else {
                    return Err("--materials-relief wants off or a strength from 0.1 to 8".into());
                };
                cli("materials_relief", &r.name());
            }
            "--bloom" => {
                let v = value()?;
                if gpu::Quality::parse_bloom(&v).is_none() {
                    return Err("--bloom wants off or a strength from 1 to 100".into());
                }
                cli("bloom", &v);
            }
            "--crt" => {
                let v = value()?;
                if gpu::Quality::parse_crt(&v).is_none() {
                    return Err("--crt wants off or a strength from 1 to 100".into());
                }
                cli("crt", &v);
            }
            "--no-splash" => o.splash = false,
            "--no-overlap" => o.overlap = false,
            "--fast" => o.fast = true,
            "--frame-log" => o.frame_log = Some(PathBuf::from(value()?)),
            "--pack-log" => o.pack_log = Some(PathBuf::from(value()?)),
            "--pack-fixes" => o.pack_fixes = model2::packlog::Fixes::parse(&value()?)?,
            "--sound-log" => o.sound_log = Some(PathBuf::from(value()?)),
            "--stick-log" => o.stick_log = Some(PathBuf::from(value()?)),
            "--photo" => {
                let mut v: Vec<photo::Script> = photo::parse_scripts(&value()?)?;
                v.sort_by_key(|p| p.at);
                o.photo = v.into();
            }
            "--photo-dir" => o.photo_dir = Some(PathBuf::from(value()?)),
            "--photo-history" => {
                o.photo_history = value()?
                    .parse::<f64>()
                    .ok()
                    .filter(|s| (0.0..=120.0).contains(s))
                    .ok_or("--photo-history wants seconds, 0 to 120")?
            }
            "--fullscreen" => fullscreen = true,
            "--exact" => {
                cli("exact", "true");
            }
            "--input" => o.input = Some(value()?),
            "--input2" | "--input3" | "--input4" => {
                let n = (a.as_bytes()[7] - b'0') as usize - 1;
                o.link_inputs[n] = Some(value()?);
            }
            "--players" => {
                o.players = value()?.parse().ok().filter(|n| (2..=4).contains(n)).ok_or("--players wants 2 to 4")?
            }
            "--link-keyboard" => o.link_keyboard = opt_bool(&mut args),
            "--host" => {
                let port = match args.peek().and_then(|p| p.trim_start_matches(':').parse::<u16>().ok()) {
                    Some(p) => {
                        args.next();
                        p
                    }
                    None => linknet::DEFAULT_PORT,
                };
                o.net = Some(NetMode::Host(port));
            }
            "--join" => o.net = Some(NetMode::Join(value()?)),
            "--name" => {
                let n = linknet::sanitize_name(&value()?);
                if n.is_empty() {
                    return Err("--name wants a name (printable characters, at most 16)".into());
                }
                o.name = Some(n);
            }
            "--net-players" => {
                o.net_players =
                    value()?.parse().ok().filter(|n| (2..=8).contains(n)).ok_or("--net-players wants 2 to 8")?
            }
            "--link-layout" => {
                let v = value()?;
                if link::Layout::parse(&v).is_none() {
                    return Err("--link-layout wants auto, side or stacked".into());
                }
                cli("link_layout", &v);
            }
            "--link-audio" => {
                let v = value()?;
                if link::Audio::parse(&v).is_none() {
                    return Err("--link-audio wants p1, mix, pan or p1only".into());
                }
                cli("link_audio", &v);
            }
            "--free-play" => {
                cli("free_play", "on");
            }
            "--no-free-play" => {
                cli("free_play", "off");
            }
            "--seconds" => o.seconds = Some(value()?.parse().map_err(|_| "bad --seconds")?),
            "--frames" => o.frames = Some(value()?.parse().map_err(|_| "bad --frames")?),
            "-h" | "--help" => return Err(String::new()),
            "-V" | "--version" => {
                println!("remodel-play {}", remodel_version::VERSION);
                std::process::exit(0);
            }
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
    if s.cli.get("materials").is_some_and(|v| v == "true") && s.cli.get("geo").is_some_and(|v| v == "chip") {
        eprintln!("--materials: no effect with --geo chip (the lighting they need comes from the native geometrizer)");
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
    /// Started on the geometrizer's chip (the setting applies on starting).
    geo_chip: bool,
    /// Photo mode's time scrub: the game's last seconds (None: off, or the
    /// launcher's preview).
    history: Option<model2::photo::History>,
    /// Photo mode's machine, made the first time it opens.
    studio: Option<model2::photo::Studio>,
    /// `--perf-log`'s counters (the game's own sessions).
    perf: Option<perf::PerfLog>,
    /// The JITs' hot-block list and its file (`--jit`), and when it was
    /// last written.
    jit_list: Option<(PathBuf, Arc<model2::jit::HotList>)>,
    jit_saved: Instant,
    /// Observing into the catalogue (the Catalogue setting): the worker
    /// finishes its session when this is dropped.
    catalogue: Option<model2::catalogue::Catalogue>,
    /// Link play: the other machines and the hub (`machine` is player 1's).
    link: Option<link::LinkSet>,
    /// Network link play: this machine is one cabinet of a session whose
    /// others are on other machines.
    net: Option<netplay::NetPlay>,
    /// The game was started on free play (its settings image carries the free
    /// play bytes in memory only: saving the EEPROM takes them out again).
    free_play: bool,
}

/// Where `set`'s JIT hot-block list is kept (docs/jit.md, "Hot-block
/// list"): beside its other files in a source tree (`out/SET/jit`), else
/// in the user's cache folder (it can always be made again).
fn jit_list_path(set: &str) -> PathBuf {
    let dir = if Path::new("out").is_dir() {
        settings::data_dir().join(set).join("jit")
    } else {
        dirs::cache_dir().unwrap_or_else(settings::data_dir).join("remodel").join("jit").join(set)
    };
    dir.join(model2::jit::hotlist::FILE)
}

/// `--jit-warm`: make `game`'s hot-block list from a quiet coin-start run
/// on a thread of its own (no window, sound or saves; its translations
/// are the workers'), written to `path` when it is done, for the next
/// boot. The game being played isn't touched.
fn jit_warm(
    game: &'static romset::Game,
    rom: PathBuf,
    geo: model2::geo::GeoMode,
    list: Arc<model2::jit::HotList>,
    path: PathBuf,
) {
    let spawned = std::thread::Builder::new().name("jit-warm".into()).spawn(move || {
        // A whole second machine: it must not take cores from the game.
        model2::jit::lower_priority();
        let t = Instant::now();
        let run = romset::RomSet::load(game, &rom, false)
            .map_err(|e| e.to_string())
            .and_then(|roms| model2::jit_warm::run(&roms, list.clone(), model2::jit_warm::STEPS, geo));
        match run.and_then(|_| list.save(&path).map_err(|e| e.to_string())) {
            Ok(_) => println!(
                "jit warm      {}: {} blocks listed in {:.1} s -> {}",
                game.id,
                list.len(),
                t.elapsed().as_secs_f64(),
                path.display()
            ),
            Err(e) => eprintln!("jit warm      {}: {e}", game.id),
        }
    });
    if let Err(e) = spawned {
        eprintln!("jit warm: {e}");
    }
}

/// Where a game's resume state goes: written on leaving it, offered by
/// the launcher as Continue.
fn resume_path(set: &str) -> PathBuf {
    saves::resume_file(set)
}

impl Session {
    fn dir(&self) -> PathBuf {
        settings::data_dir().join(self.game.id)
    }

    fn slot_path(&self, slot: u8) -> PathBuf {
        saves::slot_file(&self.key(), slot)
    }

    /// The name its saves and resume state go under: the game's, or for
    /// linked machines the linked set's own, so link play never touches
    /// the single game's.
    fn key(&self) -> String {
        match (&self.link, &self.net) {
            (Some(l), _) => format!("{}-link{}", self.game.id, l.n()),
            (_, Some(_)) => format!("{}-net", self.game.id),
            _ => self.game.id.to_string(),
        }
    }

    /// Write the hot-block list if it grew (docs/jit.md, "Hot-block
    /// list"): on leaving the game and every half a minute.
    fn save_jit_list(&mut self) {
        self.jit_saved = Instant::now();
        if let Some((path, list)) = &self.jit_list {
            if let Err(e) = list.save(path) {
                eprintln!("{}: can't keep the JIT's hot-block list: {e}", path.display());
            }
        }
    }

    fn save_eeprom(&self) {
        if self.net.is_some() {
            // The session's settings are the master's: nothing of the player's own is touched.
            return;
        }
        if let Some(l) = &self.link {
            // The linked set's own files, one for each machine.
            let buses = std::iter::once(&self.machine).chain(&l.machines).map(|m| &m.bus);
            for (i, bus) in buses.enumerate().filter(|(_, b)| b.eeprom.dirty) {
                let _ = std::fs::create_dir_all(&l.dir);
                let _ = std::fs::write(l.dir.join(format!("eeprom-{i}.bin")), bus.eeprom.save());
            }
            return;
        }
        let bus = &self.machine.bus;
        if bus.eeprom.dirty && !self.preview {
            let _ = std::fs::create_dir_all(self.dir());
            let mut image = bus.eeprom.save();
            if let (true, Some(play)) = (self.free_play, link::play_for(self.game.id)) {
                play.unset_free_play(&mut image);
            }
            let _ = std::fs::write(self.dir().join("eeprom.bin"), image);
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
        for m in self.link.iter_mut().flat_map(|l| l.machines.iter_mut()) {
            m.bus.geo.set_light(on);
        }
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

    /// Link play's sound: the mode's levels, and the sound boards it leaves
    /// out stopped. A mode that wants a board that isn't running gets it when
    /// the game next starts.
    fn set_link_audio(&mut self, audio: link::Audio) {
        let Some(l) = &mut self.link else { return };
        l.audio = audio;
        let n = l.n();
        for (i, slot) in l.sounds.iter_mut().enumerate() {
            if audio.gains(i + 1, n).is_none() {
                *slot = None;
            }
        }
        l.apply_audio(self.sound.as_ref());
    }

    /// The whole machine, sound boards included, as a save file.
    fn save_state(&mut self) -> Vec<u8> {
        let live = self.sound.as_ref().map(|snd| snd.save());
        let live = match &mut self.link {
            // The other machines, the hub and the board flags go in the tail.
            Some(l) => Some(l.save(&mut self.machine, live)),
            None => live,
        };
        model2::save_session_parts(&mut self.machine, live)
    }
}

/// egui over the game: its context and window input (its renderer is
/// the drawer's, `draw::Core::egui`).
struct Overlay {
    ctx: egui::Context,
    state: egui_winit::State,
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
    config: wgpu::SurfaceConfiguration,
    view: model2::video::View,
    origin: (u32, u32),
    /// The display's refresh rate (Hz), if known.
    refresh: Option<f64>,
    overlay: Overlay,
    preview: Option<PreviewTarget>,
    /// The surface, renderer and the frame drawn: on the draw thread while
    /// a draw is in flight (`draw::Drawer::core` waits for it).
    drawer: draw::Drawer,
    /// The texture pack counters of the last frame drawn.
    pack_stats: gpu::PackStats,
    /// Link play: each machine's view and where it is in the window.
    link: Option<LinkView>,
}

/// Link play's layout of the window (`Play::link_views`).
struct LinkView {
    /// Each machine's view and its top-left corner in the window.
    views: Vec<(model2::video::View, (u32, u32))>,
    /// The cell of the window each has, and the fourth quadrant, empty,
    /// of three.
    cells: Vec<link::Cell>,
    empty: Option<link::Cell>,
}

/// Status line counters for the current one-second window.
struct Stats {
    start: Instant,
    emulated: u32,
    drawn: u32,
    emulating: Duration,
    /// Host time drawing (not waiting for vsync), on the draw thread or
    /// the main thread.
    drawing: Duration,
    /// Main thread time copying the frame for drawing and laying out the
    /// overlay, and waiting for the draw thread to finish the frame before.
    capturing: Duration,
    waiting: Duration,
    /// Bytes the frame copies moved.
    copied: usize,
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
            capturing: Duration::ZERO,
            waiting: Duration::ZERO,
            copied: 0,
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
    /// The save slots as the menu lists them, read when it opens and
    /// after a save (None: read again).
    slot_cache: Option<Vec<Option<saves::SlotInfo>>>,
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
    /// The sticks as 8-way levers (`romset::StickDirs`), with their memory.
    stick_dirs: romset::StickDirs,
    /// Real time that emulated frame 0 corresponds to.
    epoch: Instant,
    /// Smooth motion: when the newest game frame was first drawn.
    smooth_at: Instant,
    /// A game started from the launcher: since when (fading in).
    game_fade: Option<Instant>,
    redraws: u64,
    /// Draws that panicked (`draw::run_guarded`), in all and in a row: at
    /// `DRAW_FAILURES_TO_QUIT` in a row the player quits.
    draw_failures: u64,
    draw_failures_in_row: u32,
    /// The display as last logged (`describe_display`).
    display: String,
    /// Audio underruns as last logged, and when.
    underruns_logged: u64,
    underrun_logged_at: Option<Instant>,
    /// Following the display: refreshes counted since the game started
    /// following it, and the fraction of one carried over (see `emulate`).
    refreshes: u64,
    refresh_part: f64,
    /// When `emulate` last ran (for the audio rate control).
    last_emulate: Option<Instant>,
    /// A network game: what the redraw left due and its catch-up room, the
    /// frames run between redraws not yet counted for the audio, and the last
    /// log line (`REMODEL_NET_LOG`).
    net_due: u64,
    net_budget: u32,
    net_credit: u32,
    net_poll_at: Instant,
    net_log_at: Option<Instant>,
    /// Host time per game frame (emulating, waiting for the geometrizer and
    /// drawing, not waiting for vsync; with the drawing on the draw thread,
    /// the busier of the two threads), smoothed, in seconds; and whether
    /// that is too slow to follow the display (then the clock paces).
    sync: sync::Sync,
    /// What a displayed frame costs without run-ahead, smoothed: emulating
    /// a frame, the main thread's copy of it and overlay, and drawing, in
    /// seconds, and the redraws they come from (for `run_ahead_room`).
    emulate_cost: f64,
    overhead_cost: f64,
    draw_cost: f64,
    cost_frames: u32,
    /// The host time drawing the last frame drawn (`draw::Report::draw`).
    last_draw: Duration,
    /// Overlap: emulate the next frame while the draw thread draws, when
    /// the draw is done or at this time, whichever comes first
    /// (`next_start`); waiting for it now.
    waiting: Option<Instant>,
    /// Smoothed host time of a redraw's emulation with frames, run-ahead
    /// included, in seconds (`next_start`).
    emulate_redraw: f64,
    /// `--frame-log`: the file, and what the picture in flight is (the
    /// shown machine's frame, whether that is run-ahead's, whether its
    /// polygons are the frame before's).
    frame_log: Option<std::io::BufWriter<std::fs::File>>,
    /// `--sound-log`: the file.
    sound_log: Option<std::io::BufWriter<std::fs::File>>,
    /// `--stick-log`: the log.
    stick_log: Option<sticklog::StickLog>,
    /// The steering smoothing of each player's pad (seat 0 is the
    /// session's own), stepped once an emulated frame.
    steer_filters: Vec<steer::SteerFilter>,
    /// The Controls page's own, stepped as the page is drawn (at 60 a second
    /// by the clock), and when it was.
    steer_preview: std::cell::RefCell<(steer::SteerFilter, Instant)>,
    drawing: Option<(u64, bool, bool)>,
    /// For the draw thread to wake the event loop.
    proxy: Option<winit::event_loop::EventLoopProxy<Wake>>,
    /// The host time the last `emulate` spent on the game's own frames,
    /// not the run-ahead.
    real_emulate: Duration,
    stats: Stats,
    started: Instant,
    /// The texture pack set on the renderer.
    pack: PackState,
    look: LookState,
    /// Anisotropic filtering the GPU has (`gpu::max_anisotropy`).
    max_anisotropy: u8,
    /// The GPU can do temporal anti-aliasing (`gpu::temporal_supported`).
    temporal_supported: bool,
    /// Photo mode, while it's open.
    photo: Option<photo::Mode>,
}

/// The texture pack the renderer draws with (`Play::sync_pack`).
#[derive(Default)]
struct PackState {
    /// The directory wanted (None: no pack).
    dir: Option<PathBuf>,
    /// Images and material map files in it, or why it can't be used.
    loaded: Option<Result<(usize, usize), String>>,
}

/// The look the renderer draws with (`Play::look_current`): the base look
/// from its file (watched for changes) or the built-in Comic defaults.
struct LookState {
    path: Option<PathBuf>,
    mtime: Option<std::time::SystemTime>,
    base: Arc<model2::look::Look>,
    /// Bumped when the base changes.
    generation: u64,
    checked: Option<Instant>,
    /// The look built from `base` and the tuning, and what it was built
    /// from.
    built: Option<LookBuilt>,
}

/// A look built from (base generation, look on, tuning), and the result.
type LookBuilt = (u64, bool, std::collections::BTreeMap<String, String>, Option<Arc<model2::look::Look>>);

impl Default for LookState {
    fn default() -> Self {
        LookState {
            path: None,
            mtime: None,
            base: Arc::new(model2::look::Look::comic()),
            generation: 0,
            checked: None,
            built: None,
        }
    }
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

/// For the log: the monitor the window is on, its refresh rate and scale.
fn describe_display(window: &Window) -> String {
    let m = window.current_monitor();
    let name = m.as_ref().and_then(|m| m.name()).unwrap_or_else(|| "an unknown monitor".into());
    let size = m.as_ref().map(|m| m.size());
    format!(
        "{name}{}, {}, scale {}",
        size.map_or(String::new(), |s| format!(" ({}x{})", s.width, s.height)),
        refresh_hz(window).map_or("refresh rate unknown".to_string(), |hz| format!("{hz:.2} Hz")),
        window.scale_factor()
    )
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
        self.launcher.player_name = self.opts.name.clone().unwrap_or_else(|| self.settings.player_name.clone());
        self.launcher.set_versions(&self.settings.versions);
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
            // For the log: what the launcher or menu changed.
            let changed: Vec<String> = KEYS
                .iter()
                .filter(|k| v.get(k) != self.values.get(k))
                .map(|k| format!("{k} {} -> {}", self.values.get(k), v.get(k)))
                .collect();
            println!(
                "settings      {set}: {} (saved for {})",
                changed.join(", "),
                if self.settings.for_all { "all games" } else { "this game" }
            );
            self.settings.edit(set, &v);
        }
        self.use_values(set);
    }

    /// Open the launcher, leaving the game (if any) where it was.
    fn open_launcher(&mut self) {
        let set = self.session.as_ref().map(|s| s.game.id);
        self.leave_game();
        self.launcher.set_versions(&self.settings.versions);
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
        self.underruns_logged = 0;
        self.leave_photo();
        let Some(mut s) = self.session.take() else { return };
        if s.link.is_some() {
            // The renderers of the other machines go, and the frames are drawn into
            // a cleared target again.
            if let Some(g) = &mut self.gpu {
                let core = g.drawer.core();
                core.ensure_views(1, 1, true);
                core.renderer.set_keep_target(false);
                core.forget_frame();
            }
        }
        if let Some(p) = &s.perf {
            println!("{}", p.summary());
            println!("{}", p.gaps_summary());
        }
        s.save_eeprom();
        s.save_jit_list();
        if let Some(n) = &mut s.net {
            println!("{}", n.summary());
            // The other players go on without us, at once rather than after their timeout.
            n.link.sess.bye("left");
        }
        if s.preview || s.script.is_some() || s.net.is_some() {
            return;
        }
        let data = s.save_state();
        let path = resume_path(&s.key());
        let written = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, data));
        if let Err(e) = written {
            eprintln!("{}: can't keep the resume state: {e}", path.display());
        }
    }

    /// Set the i960 up on `machine` the way the options say: the
    /// interpreter (`--interp`), the JIT with the game's hot-block list
    /// (the default; `warm`: a game with no list yet gets one from a quiet
    /// run, `--jit-warm`), or the recompiled code. Games and the
    /// launcher's previews both start this way. Returns the list and where
    /// it is kept, if the JIT is on.
    fn setup_cpu(
        &self,
        machine: &mut model2::Machine,
        game: &'static romset::Game,
        geo: model2::geo::GeoMode,
        warm: bool,
    ) -> Option<(PathBuf, Arc<model2::jit::HotList>)> {
        if self.opts.interp {
            machine.disable_native();
            return None;
        }
        if !self.opts.jit {
            return None;
        }
        let path = jit_list_path(game.id);
        let list = Arc::new(model2::jit::HotList::load(&path));
        if warm && list.is_empty() && self.opts.jit_warm {
            jit_warm(game, self.settings.rom.clone(), geo, list.clone(), path.clone());
        }
        match machine.enable_jit_with(model2::jit::Config::default(), Some(list.clone())) {
            Ok(()) => Some((path, list)),
            Err(e) => {
                eprintln!("warning: no JIT: {e}; the i960 runs without it");
                None
            }
        }
    }

    /// A machine of `game` set up the way the game's settings say: the CPU
    /// (`setup_cpu`), the geometrizer, and the lighting side channel.
    /// Linked machines have no smooth motion (nor run-ahead).
    fn new_machine(
        &self,
        roms: &romset::RomSet,
        game: &'static romset::Game,
        values: &GameSettings,
        warm: bool,
        linked: bool,
    ) -> (model2::Machine, Option<(PathBuf, Arc<model2::jit::HotList>)>) {
        let mut machine = model2::Machine::new(roms);
        let jit_list = self.setup_cpu(&mut machine, game, values.geo_mode(), warm);
        machine.bus.geo.set_mode(values.geo_mode());
        machine.bus.geo.set_smooth(values.motion_on() && !linked);
        // Smooth shading and material maps light from what the
        // geometrizer knew (run-ahead's machine is made like this one).
        machine.bus.geo.set_light(values.side_channel_on());
        // Drawing can go ahead with the last frame's polygons when the
        // geometrizer is late (`frame`).
        machine.bus.geo.set_keep_last(true);
        (machine, jit_list)
    }

    /// Start `set` from power-on, or from a saved state (`from`), leaving
    /// the current game first.
    /// `start_game` for `players` machines linked on one screen (0 or 1: a
    /// single game).
    fn start_game_n(&mut self, set: &str, from: library::Start, players: usize) {
        self.start_game_inner(set, from, players, None)
    }

    /// Start `set` as one cabinet of an agreed network session.
    fn start_net_game(&mut self, set: &str, session: linknet::Session, params: linknet::Params) {
        self.start_game_inner(set, library::Start::New, 0, Some((session, params)))
    }

    fn start_game_inner(
        &mut self,
        set: &str,
        from: library::Start,
        players: usize,
        net: Option<(linknet::Session, linknet::Params)>,
    ) {
        self.leave_game();
        self.steer_filters.clear();
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
            let linked = match players {
                0 | 1 => None,
                n => Some(
                    link::play_for(game.id)
                        .filter(|lp| n <= link::max_players(lp))
                        .ok_or_else(|| format!("{} can't link {n} cabinets on one screen", game.title))?,
                ),
            };
            let net_play = match &net {
                Some(_) => {
                    Some(link::play_for(game.id).ok_or_else(|| format!("{} can't link over a network", game.title))?)
                }
                None => None,
            };
            let (mut machine, jit_list) =
                self.new_machine(&roms, game, &values, true, linked.is_some() || net_play.is_some());
            // The game's own EEPROM, or in link play the linked set's own (the
            // player's single-player settings are only where it starts from,
            // never written): each cabinet has its own, with its LINK TYPE set.
            let dir = settings::data_dir().join(game.id);
            let link_dir = settings::data_dir().join(format!("{}-link{players}", game.id));
            let mine = std::fs::read(dir.join("eeprom.bin")).ok();
            // Free play: the game's own coin setting (for a network session, in the master's settings).
            let free_play = values.free_play.on(linked.is_some() || net_play.is_some());
            match linked {
                None if net.is_some() => {
                    // A network session's settings are the master's, never the player's own.
                    let (sess, p) = net.as_ref().unwrap();
                    let node = p.hosts[sess.me].nodes[0] as usize;
                    link::load_settings(&mut machine.bus, net_play.unwrap(), p.base.as_deref(), node, p.nodes());
                }
                None => {
                    match link::play_for(game.id).filter(|_| free_play) {
                        // The game's own settings with its FREE PLAY coin setting, in memory only.
                        Some(play) => {
                            let base = play.base_with_free_play(mine.as_deref(), true);
                            let role = base.as_ref().map_or(play.settings.record[play.role_off], |b| b[play.role_off]);
                            let image = play.settings.image(base.as_deref(), play.role_off, role, &[]);
                            machine.bus.settings_load(play.settings.store, &image);
                        }
                        None => {
                            if let Some(bytes) = &mine {
                                machine.bus.eeprom.load(bytes);
                            }
                        }
                    }
                }
                Some(lp) => {
                    let own = std::fs::read(link_dir.join("eeprom-0.bin")).ok().or(mine.clone());
                    let base = lp.base_with_free_play(own.as_deref(), free_play);
                    link::load_settings(&mut machine.bus, lp, base.as_deref(), 0, players);
                }
            }
            let sound = if self.opts.sound {
                Sound::new(&roms, (values.sfx_db, values.music_db))
                    .inspect_err(|e| eprintln!("{e}; playing without sound"))
                    .ok()
                    .map(|mut snd| {
                        if net_play.is_some() {
                            snd.set_net_play();
                        }
                        snd
                    })
            } else {
                None
            };
            println!(
                "game          {} ({}), i960 {}{}",
                game.title,
                game.id,
                if machine.jit_on() {
                    "on the JIT"
                } else if machine.is_native() {
                    "recompiled"
                } else {
                    "interpreted"
                },
                sound.as_ref().map_or(String::new(), |s| format!(", audio {} Hz", s.rate))
            );
            println!("settings      {}", settings::describe(game.id, &values, &self.sources));
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
                geo_chip: values.geo_chip,
                history: None,
                studio: None,
                perf: None,
                jit_list,
                jit_saved: Instant::now(),
                catalogue: None,
                link: None,
                free_play: free_play && net_play.is_none() && linked.is_none(),
                net: None,
            };
            if let Some((sess, params)) = net {
                let np = netplay::NetPlay::new(sess, &params, net_play.unwrap());
                println!(
                    "net link play {} node {} of {}, delay D = {}, players {:?}",
                    game.title, np.node, np.nodes, np.delay, np.names
                );
                s.net = Some(np);
            }
            if let Some(lp) = linked {
                let mut machines = Vec::new();
                let mut sounds = Vec::new();
                for i in 1..players {
                    let (mut m, _) = self.new_machine(&roms, game, &values, false, true);
                    let own = std::fs::read(link_dir.join(format!("eeprom-{i}.bin"))).ok().or(mine.clone());
                    let base = lp.base_with_free_play(own.as_deref(), free_play);
                    link::load_settings(&mut m.bus, lp, base.as_deref(), i, players);
                    machines.push(m);
                    sounds.push(if self.opts.sound && values.link_audio.gains(i, players).is_some() {
                        Sound::new(&roms, (values.sfx_db, values.music_db)).ok()
                    } else {
                        None
                    });
                }
                let mut set =
                    link::LinkSet::new(machines, model2::link::LinkHub::for_game(players, 1, lp), sounds, link_dir);
                set.audio = values.link_audio;
                set.keyboard = self.opts.link_keyboard;
                for i in 1..players {
                    let spec = self.opts.link_inputs.get(i).cloned().flatten().or_else(|| self.opts.input.clone());
                    if let Some(spec) = spec {
                        set.scripts[i - 1] = Some(
                            model2::input::InputScript::parse(&spec, &game.controls)
                                .map_err(|e| format!("--input{}: {e}", i + 1))?,
                        );
                    }
                }
                set.apply_audio(s.sound.as_ref());
                println!(
                    "link play     {players} cabinets of {}, one thread each; EEPROMs in {}; keyboard {}",
                    game.title,
                    set.dir.display(),
                    if set.keyboard { "plays P1, gamepads P2 up" } else { "off, gamepads play P1 up" }
                );
                s.link = Some(set);
            }
            if values.catalogue {
                let mut st = model2::catalogue::Settings::new(
                    self.opts.catalogue_dir.clone().unwrap_or_else(|| dir.join("catalogue")),
                    game.id,
                );
                st.every = self.opts.catalogue_every;
                st.label = "played".into();
                st.windows = (game.hud_windows.to_vec(), game.hud_element_windows.to_vec());
                match model2::catalogue::Catalogue::start(st.clone(), &mut s.machine.bus) {
                    Ok(c) => {
                        println!(
                            "catalogue     observing into {} (session {}, a sample every {} frames)",
                            st.dir.display(),
                            c.session,
                            st.every
                        );
                        s.catalogue = Some(c);
                    }
                    Err(e) => eprintln!("catalogue: {}: {e}; not observing", st.dir.display()),
                }
            }
            if self.opts.perf_log {
                s.perf = Some(perf::PerfLog::new(game.id, &s.machine));
            }
            // Photo mode's time scrub: the game's last seconds.
            if self.opts.photo_history > 0.0 && s.link.is_none() && s.net.is_none() {
                s.history = Some(model2::photo::History::new(self.opts.photo_history, 60));
            }
            // A run-ahead the command line asked for beyond what the game's
            // lag can use only judders: it's kept, and said.
            if self.settings.cli.contains_key("run_ahead") && values.run_ahead > game.run_ahead_max as u32 {
                eprintln!(
                    "warning: --run-ahead {} is more than {} can use (at most {}: docs/play.md, \"Run-ahead\"); \
                     more only judders",
                    values.run_ahead, game.id, game.run_ahead_max
                );
            }
            if s.link.is_none() && s.net.is_none() {
                s.set_run_ahead(values.run_ahead, values.motion_on(), Some(&roms), &self.settings.rom);
            }
            Ok(s)
        });
        match started {
            Ok(s) => {
                self.reset_cost();
                self.bezel =
                    bezel::image(s.game).map(|image| Bezel { size: image.size, image: Some(image), texture: None });
                self.settings.played(set);
                self.settings.save();
                // A crosshair pointer over gun games.
                if let Some(g) = &mut self.gpu {
                    let gun = !s.game.controls.guns.is_empty();
                    g.window.set_cursor(if gun { CursorIcon::Crosshair } else { CursorIcon::Default });
                    let core = g.drawer.core();
                    core.forget_frame();
                }
                self.session = Some(s);
                self.epoch = Instant::now();
                self.refit();
                let key = self.session.as_ref().map_or(set.to_string(), |s| s.key());
                match from {
                    library::Start::New => {}
                    library::Start::Resume => self.load_file(&resume_path(&key), "Continued"),
                    library::Start::Slot(0) => self.load_file(&saves::slot_file(&key, 0), "Quick loaded"),
                    library::Start::Slot(n) => self.load_file(&saves::slot_file(&key, n), &format!("Loaded slot {n}")),
                }
            }
            Err(e) => {
                eprintln!("{set}: {e}");
                self.menu.error(format!("Can't start {set}: {e}"));
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
        let geo = self.values.geo_mode();
        // The i960 as in a game (previews were interpreted: with smooth
        // motion on, that was 2 to 3 times the game's emulation time).
        self.setup_cpu(&mut machine, game, geo, false);
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
            let core = g.drawer.core();
            core.forget_frame();
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
            geo_chip: self.values.geo_chip,
            history: None,
            studio: None,
            perf: None,
            jit_list: None,
            jit_saved: Instant::now(),
            catalogue: None,
            link: None,
            free_play: false,
            net: None,
        });
        self.preview.shown = None;
        self.preview.leaving = None;
        self.sync.reset();
        self.reset_cost();
    }

    fn save_slot(&mut self, slot: u8) {
        if self.session.as_ref().is_some_and(|s| s.net.is_some()) {
            return self.menu.error("Save states are off in a network game".to_string());
        }
        let Some(s) = self.session.as_mut().filter(|s| !s.preview) else { return };
        let data = s.save_state();
        let path = s.slot_path(slot);
        let result = path.parent().map_or(Ok(()), std::fs::create_dir_all).and_then(|_| std::fs::write(&path, data));
        println!(
            "save          {} frame {}: {} ({})",
            s.game.id,
            s.machine.frames,
            path.display(),
            result.as_ref().map_or_else(|e| format!("failed: {e}"), |_| "ok".into())
        );
        let name = if slot == 0 { "Quick saved".to_string() } else { format!("Saved to slot {slot}") };
        match result {
            Ok(()) => {
                self.keep_thumbnail(&path, slot);
                self.menu.notify(name);
            }
            Err(e) => self.menu.error(format!("Save failed: {e}")),
        }
        self.slot_cache = None;
    }

    /// The thumbnail of the state just saved to `save`: the game's frame
    /// as it is, drawn small, handed to the menu for the slots page and
    /// encoded to a PNG beside the state on another thread. A game that
    /// can't be drawn now leaves no thumbnail (and drops an older one).
    fn keep_thumbnail(&mut self, save: &Path, slot: u8) {
        let thumb = saves::thumb_path(save);
        match self.thumbnail() {
            Some((w, h, rgba)) => {
                self.menu.fresh_thumb(slot, [w as usize, h as usize], rgba.clone());
                saves::write_thumb(thumb, w, h, rgba);
            }
            None => {
                let _ = std::fs::remove_file(thumb);
            }
        }
    }

    /// The game's frame as a `saves::THUMB_SIZE` picture (4:3, bezel and
    /// menu left out), RGBA. Drawn from the machine as `--shot` is, in a
    /// small texture of its own with temporal anti-aliasing off, so the
    /// frame being shown is left alone but for a fresh start of its
    /// anti-aliasing history on the next redraw.
    fn thumbnail(&mut self) -> Option<(u32, u32, Vec<u8>)> {
        let (w, h) = saves::THUMB_SIZE;
        if self.photo.is_some() || self.launcher.open {
            return None;
        }
        let (Some(g), Some(s)) = (&mut self.gpu, &mut self.session) else { return None };
        if s.preview {
            return None;
        }
        let format = g.config.format;
        let hud = if self.values.hud_edges { model2::video::Hud::Edges } else { model2::video::Hud::Center };
        let view = model2::video::View::new(h as usize, ASPECTS.0, hud)
            .with_hud_windows(s.game.hud_windows, s.game.hud_element_windows);
        let (w, h) = (w.min(view.width as u32).max(1), h.min(view.height as u32).max(1));
        let core = g.drawer.core();
        let texture = core.renderer.device().create_texture(&wgpu::TextureDescriptor {
            label: Some("thumbnail"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let target = texture.create_view(&Default::default());
        let r = &mut core.renderer;
        r.set_keep_target(false);
        r.forget_ram();
        r.reset_history();
        r.set_light_rules(&s.game.shading, hud_mask(s.game));
        let mut quality = self.values.quality(self.temporal_supported);
        if quality.temporal > 0 {
            quality.temporal = 0;
            quality.msaa = quality.msaa.max(4);
        }
        r.set_quality(quality);
        r.present_bus(&mut s.shown().bus, None, &view, &target, format, (0, 0));
        let mut rgba = r.read_texture(&texture, w, h);
        // The next redraw starts from scratch (its frame copy, RAM and
        // anti-aliasing history), as after a load.
        core.forget_frame();
        s.drew_ahead = s.run_ahead.as_ref().is_some_and(|ra| ra.is_ahead_of(&s.machine));
        if format == wgpu::TextureFormat::Bgra8Unorm || format == wgpu::TextureFormat::Bgra8UnormSrgb {
            rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2));
        }
        Some((w, h, rgba))
    }

    fn load_slot(&mut self, slot: u8) {
        let Some(path) = self.session.as_ref().filter(|s| !s.preview).map(|s| s.slot_path(slot)) else { return };
        if !path.is_file() {
            return self.menu.warn(if slot == 0 { "No quick save yet" } else { "That slot is empty" });
        }
        let done = if slot == 0 { "Quick save loaded".to_string() } else { format!("Loaded slot {slot}") };
        self.load_file(&path, &done);
    }

    /// Load the save file at `path` into the game, saying `done` if it did.
    fn load_file(&mut self, path: &Path, done: &str) {
        if self.session.as_ref().is_some_and(|s| s.net.is_some()) {
            return self.menu.error("Save states are off in a network game".to_string());
        }
        self.steer_filters.clear();
        let Some(s) = &mut self.session else { return };
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => return self.menu.error(format!("Can't read {}: {e}", path.display())),
        };
        let result = model2::load_session_parts(&mut s.machine, &data).and_then(|live| {
            // Link play: the tail holds the other machines, the hub and the board flags.
            let live = match &mut s.link {
                Some(l) => l.load(&mut s.machine, &live)?,
                None => live,
            };
            match &mut s.sound {
                Some(snd) if !live.is_empty() => snd.load(live, &s.machine.bus),
                _ => Ok(()),
            }
        });
        println!(
            "load          {} {}: {}",
            s.game.id,
            path.display(),
            result.as_ref().map_or_else(|e| format!("failed: {e}"), |_| format!("ok, at frame {}", s.machine.frames))
        );
        match result {
            Ok(()) => {
                self.epoch = Instant::now() - Duration::from_secs_f64(s.machine.frames as f64 / FPS);
                // A jump the motion data doesn't know of; memory the
                // frame copy's versions don't describe.
                if let Some(g) = &mut self.gpu {
                    let core = g.drawer.core();
                    core.renderer.reset_history();
                    core.frame.forget();
                    core.views.iter_mut().for_each(|v| v.frame.forget());
                }
                self.menu.notify(done);
            }
            Err(e) => {
                // The machine may be half loaded: start the game afresh.
                let set = s.game.id.to_string();
                let players = s.link.as_ref().map_or(0, |l| l.n());
                self.session = None;
                self.start_game_n(&set, library::Start::New, players);
                self.menu.error(format!("Load failed: {e}"));
            }
        }
    }

    /// The save slots (quick save first): when and by what each was saved,
    /// its thumbnail. Read from disk when the menu opens and after a save.
    fn slot_infos(&mut self) -> Vec<Option<saves::SlotInfo>> {
        let Some(s) = &self.session else { return Vec::new() };
        let cache = self
            .slot_cache
            .get_or_insert_with(|| (0..menu::SLOTS).map(|n| saves::read_info(&s.slot_path(n))).collect());
        cache.clone()
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
            if let Some(s) = self.session.as_mut().filter(|s| s.link.is_none()) {
                // Setting it again tries again after it was dropped.
                s.run_ahead_dropped &= old.run_ahead == new.run_ahead;
                let frames = if s.run_ahead_dropped { 0 } else { new.run_ahead };
                s.set_run_ahead(frames, motion(&new), None, &rom);
            }
        }
        if old.side_channel_on() != new.side_channel_on() && !preview {
            if let Some(s) = &mut self.session {
                s.set_light(new.side_channel_on());
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
        if old.hud_edges != new.hud_edges || old.wide != new.wide || old.link_layout != new.link_layout {
            self.refit();
        }
        if old.link_audio != new.link_audio && !preview {
            if let Some(s) = &mut self.session {
                s.set_link_audio(new.link_audio);
            }
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
        let b = self
            .bezel
            .as_ref()
            .filter(|_| self.values.bezel && self.session.as_ref().is_some_and(|s| !s.preview && s.link.is_none()))?;
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
        let texture = g.drawer.core().renderer.device().create_texture(&wgpu::TextureDescriptor {
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
        if self.photo.is_some() {
            // Photo mode as the window shows it: the photo and its dock.
            let (view, origin) = self.fit(w, h);
            let job = self.photo_job(1, 4);
            let g = self.gpu.as_mut().unwrap();
            if let Some(job) = &job {
                photo::draw(g.drawer.core(), job, &target, &view, origin, format);
            }
            let menu = &mut self.menu;
            if let Some(m) = &mut self.photo {
                let line = photo::status(m, (view.width as u32, view.height as u32));
                let picture = photo::picture_rect(origin, &view, ppp);
                draw_overlay(g, &target, false, [w, h], Some(ppp), |ctx| {
                    menu.photo_notices(ctx);
                    m.ui(ctx, picture, &line);
                });
            }
        } else if self.launcher.open {
            self.draw_launcher(&target, [w, h], ppp, Instant::now());
        } else {
            self.sync_pack();
            let (view, origin) = self.fit(w, h);
            let link_views = self.link_views(w, h);
            let at = self.bezel_at(&view, origin, w);
            let slots = self.slot_infos();
            let pack_text = self.pack_text();
            let materials_text = self.materials_text();
            let look_now = self.look_current();
            let has_bezel = self.bezel.is_some();
            let room = self.session.as_ref().and_then(|s| self.run_ahead_room(s.game.id));
            let (Some(g), Some(s)) = (&mut self.gpu, &mut self.session) else { return Err("no game".into()) };
            let core = g.drawer.core();
            let mut quality = self.values.quality(self.temporal_supported);
            if let (Some(l), Some(lv)) = (&mut s.link, &link_views) {
                // Link play: each machine in its own view of one target.
                quality.temporal = 0;
                core.ensure_views(l.n(), self.max_anisotropy, self.temporal_supported);
                core.renderer.clear(&target);
                let machines = std::iter::once(&mut s.machine).chain(l.machines.iter_mut());
                let renderers =
                    std::iter::once(&mut core.renderer).chain(core.views.iter_mut().map(|v| &mut v.renderer));
                for ((m, r), (view, origin)) in machines.zip(renderers).zip(&lv.views) {
                    r.set_keep_target(true);
                    r.forget_ram();
                    r.reset_history();
                    r.set_light_rules(&s.game.shading, hud_mask(s.game));
                    r.set_quality(quality);
                    r.set_look(look_now.clone());
                    m.bus.geo.sync();
                    r.present_bus(&mut m.bus, None, view, &target, format, *origin);
                }
                // The next redraw starts these renderers from scratch.
                core.forget_frame();
                core.views.iter_mut().for_each(|v| v.frame.forget());
            } else {
                let r = &mut core.renderer;
                r.set_keep_target(false);
                r.forget_ram();
                // A one-off frame: with temporal on, the plain (unjittered) one.
                r.reset_history();
                r.set_light_rules(&s.game.shading, hud_mask(s.game));
                r.set_quality(quality);
                r.set_look(look_now.clone());
                r.present_bus(&mut s.shown().bus, None, &view, &target, format, origin);
            }
            s.drew_ahead = s.run_ahead.as_ref().is_some_and(|ra| ra.is_ahead_of(&s.machine));
            let (bezel, menu) = (&mut self.bezel, &mut self.menu);
            let mut values = self.values.clone();
            let (mut fullscreen, mut for_all) = (self.settings.fullscreen, self.settings.for_all);
            let keeps = menu::Keeps {
                all_games: self.settings.all_games_preset(s.game.id),
                own_picture: self.settings.has_own_picture(s.game.id),
            };
            let menu_view = menu::View {
                playing: Some(s.game.title),
                saves: &slots,
                game: Some(s.game),
                picture: Some([origin.0 as f32, origin.1 as f32, view.width as f32, view.height as f32]),
                keeps,
                has_bezel,
                has_own: false,
                run_ahead_dropped: s.run_ahead_dropped,
                run_ahead_room: room,
                max_anisotropy: self.max_anisotropy,
                temporal_supported: self.temporal_supported,
                pack: pack_text.as_deref(),
                materials: materials_text.as_deref(),
                look: look_now.as_deref(),
                pad: controls::PadLive::default(),
                link: None,
                running: false,
            };
            let lui = link_ui(Some(&*s), self.gilrs.as_ref(), link_views.as_ref());
            draw_overlay(g, &target, false, [w, h], Some(ppp), |ctx| {
                if let (Some(b), Some((rect, pic))) = (bezel.as_mut(), at) {
                    b.paint(ctx, rect, pic, w as f32, ctx.pixels_per_point());
                }
                if let Some((cells, views, empty, seats, title)) = &lui {
                    link_labels(ctx, ctx.pixels_per_point(), cells, views, *empty, seats, title);
                }
                // The menu as it is (drawing it changes nothing).
                if menu.open {
                    let prefs = menu::Prefs { fullscreen: &mut fullscreen, for_all: &mut for_all };
                    menu.ui(ctx, &mut values, prefs, &menu_view, Some(&mut s.bindings), &[]);
                }
            });
        }
        let g = self.gpu.as_mut().unwrap();
        let mut rgba = g.drawer.core().renderer.read_texture(&texture, w, h);
        if format == wgpu::TextureFormat::Bgra8Unorm {
            rgba.as_chunks_mut::<4>().0.iter_mut().for_each(|p| p.swap(0, 2));
        }
        std::fs::write(path, png::encode_rgba(w as usize, h as usize, &rgba))
            .map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The look to draw with: None with the setting off (or for the
    /// launcher's preview). Its file is the texture pack's `look.txt` (pack
    /// on), else `<data dir>/<set>/look.txt`, else the built-in Comic
    /// defaults; the file is watched (every 0.4 s) and read again when it
    /// changes, so an edit shows at once, and the player's tuning
    /// (`GameSettings::look_tuning`) goes over it.
    fn look_current(&mut self) -> Option<Arc<model2::look::Look>> {
        let s = self.session.as_ref().filter(|s| !s.preview)?;
        if !self.values.look {
            return None;
        }
        let now = Instant::now();
        if self.look.checked.is_none_or(|t| now.duration_since(t) > Duration::from_millis(400)) {
            self.look.checked = Some(now);
            let pack_dir =
                self.values.pack.then(|| self.opts.texture_pack.clone().unwrap_or_else(|| pack::game_dir(s.game)));
            let own = settings::data_dir().join(s.game.id);
            let path = pack_dir.into_iter().chain([own]).map(|d| d.join(model2::look::FILE)).find(|p| p.is_file());
            let mtime = path.as_ref().and_then(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok());
            if path != self.look.path || mtime != self.look.mtime {
                let loaded = match &path {
                    Some(p) => model2::look::Look::load(p),
                    None => Ok(None),
                };
                match loaded {
                    Ok(l) => {
                        if let Some(l) = &l {
                            println!(
                                "look          {}",
                                path.as_ref().map_or(String::new(), |p| p.display().to_string())
                            );
                            if let Some(n) = l.notice() {
                                println!("look          {n}");
                            }
                        }
                        self.look.base = Arc::new(l.unwrap_or_else(model2::look::Look::comic));
                    }
                    Err(e) => {
                        // Keep what was drawn, and say why.
                        eprintln!("look: {e}");
                        self.menu.warn(format!("Look: {e}"));
                    }
                }
                self.look.path = path;
                self.look.mtime = mtime;
                self.look.generation += 1;
            }
        }
        let key = (self.look.generation, true, self.values.look_tuning.clone());
        let fresh = self.look.built.as_ref().is_some_and(|b| (b.0, b.1, &b.2) == (key.0, key.1, &key.2));
        if !fresh {
            let l = self.values.effective_look(&self.look.base).map(Arc::new);
            self.look.built = Some((key.0, key.1, key.2, l));
        }
        self.look.built.as_ref().and_then(|b| b.3.clone())
    }

    /// Set the texture pack the settings ask for on the renderer, when
    /// that changed: the game's (or `--texture-pack`'s) with the setting
    /// on, none for the launcher's preview.
    fn sync_pack(&mut self) {
        let wanted = match &self.session {
            Some(s) if !s.preview && self.values.pack && s.link.is_none() => {
                Some(self.opts.texture_pack.clone().unwrap_or_else(|| pack::game_dir(s.game)))
            }
            _ => None,
        };
        if wanted == self.pack.dir {
            return;
        }
        let Some(g) = &mut self.gpu else { return };
        let renderer = &mut g.drawer.core().renderer;
        self.pack = PackState { dir: wanted.clone(), loaded: None };
        let Some(dir) = wanted else {
            renderer.set_pack(None);
            return;
        };
        let opened = if dir.is_dir() {
            pack::DirPack::open(&dir)
        } else {
            Err(format!("no texture pack: {} not found", dir.display()))
        };
        match opened {
            Ok(p) => {
                renderer.set_pack_fixes(self.opts.pack_fixes);
                if let Some(path) = &self.opts.pack_log {
                    match model2::packlog::PackLog::create(path) {
                        Ok(l) => renderer.set_pack_log(Some(l)),
                        Err(e) => eprintln!("--pack-log: {}: {e}", path.display()),
                    }
                }
                let maps = if p.maps() > 0 { format!(", {} material maps", p.maps()) } else { String::new() };
                println!("texture pack  {}: {} images{maps}", dir.display(), p.len());
                self.pack.loaded = Some(Ok((p.len(), p.maps())));
                renderer.set_pack(Some(Box::new(p)));
            }
            Err(e) => {
                eprintln!("{e}");
                self.menu.warn(format!("{}{}", e[..1].to_uppercase(), &e[1..]));
                self.pack.loaded = Some(Err(e));
                renderer.set_pack(None);
            }
        }
    }

    /// The menu's line on the texture pack: where it is and what it
    /// replaced in the last frame, or why there is none.
    fn pack_text(&self) -> Option<String> {
        let g = self.gpu.as_ref()?;
        let dir = self.pack.dir.as_ref()?;
        Some(match self.pack.loaded.as_ref()? {
            Ok((images, _)) => {
                let st = g.pack_stats;
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

    /// The menu's line on material maps while the setting is on: what they
    /// changed in the last frame, or why they have no effect (no pack, a
    /// pack without maps, the chip, no polygon lit by the exact layer).
    fn materials_text(&self) -> Option<String> {
        let v = &self.values;
        if !v.materials {
            return None;
        }
        let s = self.session.as_ref()?;
        if s.preview {
            return None;
        }
        let rule = match v.materials_relief.strength() {
            Some(_) => format!("relief {}", v.materials_relief.name()),
            None => "faithful".to_string(),
        };
        Some(match (&self.pack.loaded, &self.gpu) {
            _ if !v.pack => "No effect: needs the texture pack on".to_string(),
            (Some(Err(_)), _) | (None, _) => "No effect: no texture pack loaded".to_string(),
            (Some(Ok((_, 0))), _) => "No effect: this texture pack has no material maps".to_string(),
            _ if s.geo_chip => {
                "No effect with the game's chip: material maps need the native geometrizer's lighting".to_string()
            }
            (Some(Ok((_, maps))), Some(g)) => {
                let st = g.pack_stats;
                if st.textured > 0 && st.lamped == 0 {
                    "No effect in this frame: no textured polygon had the game's exact lighting \
                     (unlit, or the lighting check fell back)"
                        .to_string()
                } else {
                    format!(
                        "{rule}: maps on {} of {} lit textured polygons ({} materials drawn, {} held, {} MB; \
                         {maps} map files)",
                        st.material_polys,
                        st.lamped,
                        st.materials,
                        st.material_held,
                        st.material_bytes >> 20
                    )
                }
            }
            _ => return None,
        })
    }

    fn refit(&mut self) {
        if let Some((w, h)) = self.gpu.as_ref().map(|g| (g.config.width, g.config.height)) {
            let (mut view, mut origin) = self.fit(w, h);
            let link = self.link_views(w, h);
            // The menu's picture and the pointer follow player 1's view.
            if let Some(l) = &link {
                (view, origin) = l.views[0];
            }
            let g = self.gpu.as_mut().unwrap();
            (g.view, g.origin) = (view, origin);
            g.link = link;
        }
    }

    /// Link play's views in a window of `w` x `h` pixels (None: a single
    /// game): a cell each, a 4:3 view fitted in it
    /// ([`link::cells`], [`link::fit`]); each view's height is its render
    /// resolution, so the internal resolution follows the viewport.
    fn link_views(&self, w: u32, h: u32) -> Option<LinkView> {
        let s = self.session.as_ref()?;
        let l = s.link.as_ref()?;
        let (cells, empty) = link::cells(l.n(), (w, h), self.values.link_layout);
        let hud = if self.values.hud_edges { model2::video::Hud::Edges } else { model2::video::Hud::Center };
        let views = cells
            .iter()
            .map(|c| {
                let (vh, origin) = link::fit(*c);
                let view = model2::video::View::new(vh as usize, ASPECTS.0, hud)
                    .with_hud_windows(s.game.hud_windows, s.game.hud_element_windows);
                (view, origin)
            })
            .collect();
        Some(LinkView { views, cells, empty })
    }

    fn resize(&mut self, w: u32, h: u32) {
        if let Some(g) = &mut self.gpu {
            g.config.width = w.max(1);
            g.config.height = h.max(1);
            let core = g.drawer.core();
            core.surface.configure(core.renderer.device(), &g.config);
            // A resize can mean the window moved to another monitor.
            g.refresh = refresh_hz(&g.window);
            let display = describe_display(&g.window);
            if display != self.display {
                println!("display       {display}");
                self.display = display;
            }
            // The temporal targets are made again at the new size.
            core.renderer.reset_history();
        }
        self.refit();
    }

    /// Game frames per display refresh when the game follows the display
    /// (1 at about 60 Hz, 1/2 at about 120 Hz), or None to follow the clock.
    fn per_refresh(&self) -> Option<f64> {
        let hz = self.gpu.as_ref()?.refresh?;
        if self.values.exact || self.sync.behind {
            return None;
        }
        [1.0, 0.5].into_iter().find(|k| (0.95..=1.1).contains(&(hz * k / FPS)))
    }

    /// Take the gamepads' events (the last used one's) and `--nav`'s due
    /// now: see `pad_event`. Then the held direction's repeat, and Start
    /// and Select held long enough to open the menu.
    fn poll_pads(&mut self) {
        let now = Instant::now();
        let mut events: Vec<(pad::Event, bool, Option<usize>)> = Vec::new();
        let link = !self.launcher.open && self.session.as_ref().is_some_and(|s| s.link.is_some());
        if let Some(g) = &mut self.gilrs {
            while let Some(ev) = g.next_event() {
                match ev.event {
                    gilrs::EventType::Connected => println!("controller    connected: {}", g.gamepad(ev.id).name()),
                    gilrs::EventType::Disconnected => {
                        println!("controller    disconnected: {}", g.gamepad(ev.id).name());
                        if let Some(l) = self.session.as_mut().and_then(|s| s.link.as_mut()) {
                            if let Some(seat) = l.leave(ev.id) {
                                println!("link play     player {} lost their controller", seat + 1);
                            }
                        }
                    }
                    _ => {}
                }
                if link {
                    // Link play: a gamepad's first button press takes a seat (and is
                    // not the game's); each seat's pad has its own gate; the menu
                    // follows the pad used last.
                    let l = self.session.as_mut().and_then(|s| s.link.as_mut()).unwrap();
                    if matches!(ev.event, gilrs::EventType::ButtonPressed(..))
                        && l.seat_of(ev.id).is_none()
                        && !self.menu.open
                    {
                        if let Some(seat) = l.join(ev.id) {
                            let name = g.gamepad(ev.id).name().to_string();
                            println!("link play     player {} plays on {name}", seat + 1);
                            self.menu.notify(format!("Player {} joined on {name}", seat + 1));
                            self.pad = Some(ev.id);
                            continue;
                        }
                    }
                    let seat = l.seat_of(ev.id);
                    if matches!(ev.event, gilrs::EventType::ButtonPressed(..) | gilrs::EventType::AxisChanged(..)) {
                        self.pad = Some(ev.id);
                    }
                    if seat.is_some() || Some(ev.id) == self.pad {
                        events.extend(pad::Event::from_gilrs(&ev.event).map(|e| (e, false, seat)));
                    }
                    continue;
                }
                if matches!(ev.event, gilrs::EventType::ButtonPressed(..) | gilrs::EventType::AxisChanged(..))
                    && self.pad != Some(ev.id)
                {
                    self.pad = Some(ev.id);
                    self.gate = pad::Gate::default();
                }
                if Some(ev.id) == self.pad {
                    events.extend(pad::Event::from_gilrs(&ev.event).map(|e| (e, false, None)));
                }
            }
        }
        let secs = (now - self.started).as_secs_f64();
        while self.opts.nav.front().is_some_and(|n| n.0 <= secs) {
            events.push((self.opts.nav.pop_front().unwrap().1, true, None));
        }
        for (ev, scripted, seat) in events {
            // The seat's own gate stands in for the player's for this event.
            let swapped = seat.is_some_and(|i| self.swap_gate(i));
            self.pad_event(ev, scripted, now);
            if let (true, Some(i)) = (swapped, seat) {
                self.swap_gate(i);
            }
        }
        if (self.launcher.open || self.menu.open) && self.menu.capture.is_none() {
            self.navs.extend(self.pad_nav.repeat(now));
        }
        if !self.launcher.open && !self.menu.open && self.photo.is_none() {
            let mut open = self.gate.tick(now);
            if let Some(l) = self.session.as_mut().and_then(|s| s.link.as_mut()) {
                for seat in &mut l.seats {
                    open |= seat.gate.tick(now);
                }
            }
            if open {
                self.set_menu(true);
                self.menu.pad_used = true;
            }
        }
    }

    /// Link play: put seat `i`'s gate in `self.gate` and the player's in
    /// its place (call again to put them back). False without link play.
    fn swap_gate(&mut self, i: usize) -> bool {
        let Some(l) = self.session.as_mut().and_then(|s| s.link.as_mut()) else { return false };
        std::mem::swap(&mut self.gate, &mut l.seats[i].gate);
        true
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
        if self.photo.is_some() {
            if let Event::Pressed(b) = ev {
                self.gate.keep(b);
                self.photo_button(b);
            }
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
        // Sticks resting off centre, or moved on the way to a button: they
        // bind only as a lever's directions, in the lists that take them.
        if matches!(c, romset::bindings::Capture::Axis(..))
            && !axis_slot
            && b.accepts(slot) != romset::bindings::Accepts::Inputs
        {
            return;
        }
        match b.capture(slot, at, c, romset::bindings::RESERVED_KEYS) {
            Ok(()) => {
                settings::save_bindings(game, b);
                self.menu.capture = None;
                // Two actions on one key or button: say so.
                if let Some(text) = menu::conflict_text(b, slot) {
                    self.menu.warn(text);
                }
            }
            Err(e) => self.menu.warn(e),
        }
    }

    /// Keyboard and the last used gamepad onto the cabinet's controls, by
    /// the game's bindings (`romset::Controls`), or the `--input` script.
    fn apply_inputs(&mut self) {
        self.poll_pads();
        let pad = self.gilrs.as_ref().and_then(|g| self.pad.map(|id| g.gamepad(id)));
        let mouse_aim = self.mouse_aim();
        let Some(s) = &mut self.session else { return };
        if let Some(l) = s.link.as_mut() {
            // Link play: each player drives their machine, by their seat's devices.
            let tuning = self.values.tuning;
            let none = HashSet::new();
            let gilrs = self.gilrs.as_ref();
            let machines = std::iter::once(&mut s.machine).chain(l.machines.iter_mut());
            for (i, (m, seat)) in machines.zip(l.seats.iter_mut()).enumerate() {
                let script = if i == 0 { s.script.as_ref() } else { l.scripts[i - 1].as_ref() };
                if let Some(script) = script {
                    script.apply(m.frames, &mut m.bus);
                    continue;
                }
                let pad = gilrs.and_then(|g| seat.pad.map(|id| g.gamepad(id)));
                let dirs = controls::read_sticks(&mut seat.dirs, pad.as_ref(), tuning);
                let keys = if i == 0 && l.keyboard { &self.keys } else { &none };
                if self.steer_filters.len() <= i {
                    self.steer_filters.resize(i + 1, steer::SteerFilter::default());
                }
                let raw = pad.as_ref().map(controls::raw_axes);
                let steer = self.steer_filters[i].step(raw.as_ref(), tuning);
                let devices = controls::Devices { keys, pad, gate: &seat.gate, tuning, dirs, steer };
                model2::input::apply_player(&mut m.bus, &[(&s.bindings, &devices)]);
            }
            return;
        }
        // The menu open over a running game (a network game's): the controls
        // are the menu's, the game's let go.
        let in_menu = self.menu.open;
        let none = HashSet::new();
        let (pad, keys) = if in_menu { (None, &none) } else { (pad, &self.keys) };
        match &s.script {
            Some(script) => script.apply(s.machine.frames, &mut s.machine.bus),
            None => {
                let tuning = self.values.tuning;
                let dirs = controls::read_sticks(&mut self.stick_dirs, pad.as_ref(), tuning);
                if self.steer_filters.is_empty() {
                    self.steer_filters.push(steer::SteerFilter::default());
                }
                let raw = pad.as_ref().map(controls::raw_axes);
                let steer = self.steer_filters[0].step(raw.as_ref(), tuning);
                if let (Some(log), Some(raw)) = (&mut self.stick_log, &raw) {
                    let b = &s.bindings;
                    log.game(|a| {
                        (0..b.controls().analogs.len()).any(|i| b.controls().analogs[i].steering && b.axis(i).axis == a)
                    });
                    log.frame(s.machine.frames, raw, &steer, tuning);
                }
                let devices = controls::Devices { keys, pad, gate: &self.gate, tuning, dirs, steer };
                let bus = &mut s.machine.bus;
                let b = &s.bindings;
                model2::input::apply_player(bus, &[(b, &devices)]);
                if !bus.controls.guns.is_empty() {
                    use model2::input::{apply_gun, held, GunState};
                    let (reload0, reload1) = (held(&devices, b.reload(0)), held(&devices, b.reload(1)));
                    let mut stick = self.stick.update(&devices, b);
                    if self.mouse_used && !in_menu {
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

    /// What the pad reads now, for the Controls page.
    fn pad_live(&self) -> controls::PadLive {
        let pad = self.gilrs.as_ref().and_then(|g| self.pad.map(|id| g.gamepad(id)));
        let mut live = controls::PadLive::of(pad.as_ref());
        // The page's smoothed reading: the filter stepped for the time since
        // the last draw, at 60 steps a second (at most a few at once).
        let mut p = self.steer_preview.borrow_mut();
        if live.connected {
            let steps = (p.1.elapsed().as_secs_f32() * 60.0).round().clamp(1.0, 4.0) as u32;
            p.1 = Instant::now();
            for _ in 0..steps {
                live.smoothed = p.0.step(Some(&live.axes), self.values.tuning);
            }
        } else {
            p.0.reset();
        }
        live
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

    /// Run the frames that are due (unless paused in the menu). `retry`: a
    /// network game's held boundary tried again between redraws (`net_retry`):
    /// the frames the last redraw left due, within the catch-up it had left, and
    /// none of that redraw's bookkeeping.
    fn emulate(&mut self, retry: bool) -> Result<u32, i960::cpu::Stop> {
        let now = Instant::now();
        if !retry {
            self.redraws += 1;
        }
        // A network session that ended (a player left, the connection was lost, the
        // states differ): back to the launcher with the reason, unless this is a
        // scripted run, which reports it as it stops.
        if self.opts.frames.is_none() {
            let over = self.session.as_mut().and_then(|s| s.net.as_mut()).and_then(|n| n.ended.take());
            if let Some(why) = over {
                println!("net link play ended: {why}");
                self.leave_game();
                self.set_menu(false);
                self.launcher.open = true;
                self.menu.error(format!("The network game ended: {why}"));
                return Ok(0);
            }
        }
        // A network game doesn't pause for the menu: the others would wait for
        // it. The game runs on underneath with this player's controls let go.
        let net_game = self.session.as_ref().is_some_and(|s| s.net.is_some() && !s.preview);
        if (self.menu.open && !net_game) || self.session.is_none() || self.photo.is_some() {
            // A network game keeps its link alive while paused (the others wait for it).
            if let Some(n) = self.session.as_mut().and_then(|s| s.net.as_mut()) {
                n.keepalive();
            }
            // Paused: keep the clock with the game for when it resumes.
            if let Some(s) = &self.session {
                self.epoch = now - Duration::from_secs_f64(s.machine.frames as f64 / FPS);
            }
            return Ok(0);
        }
        let frames = self.session.as_ref().map_or(0, |s| s.machine.frames);
        // Follow the display (one frame per refresh, or per two) or the clock.
        let (due, speed) = match self.per_refresh() {
            _ if retry => (self.net_due, 1.0),
            // `--fast`: as fast as it goes.
            _ if self.opts.fast => (frames + MAX_CATCH_UP as u64, 1.0),
            // A frame per refresh gone by since the last redraw, not per
            // redraw: a redraw that comes late (the surface waited past a
            // vblank) runs the frames of the refreshes it missed, so the
            // game keeps its speed and the audio its pitch.
            Some(k) => {
                let hz = self.gpu.as_ref().and_then(|g| g.refresh).unwrap_or(60.0);
                let dt = self.last_emulate.map_or(0.0, |l| (now - l).as_secs_f64());
                let n = refreshes_gone(&mut self.refresh_part, dt, hz);
                let before = self.refreshes;
                self.refreshes += n;
                let f = if k == 1.0 { n } else { self.refreshes / 2 - before / 2 };
                (frames + f, hz * k / FPS)
            }
            None => (((now - self.epoch).as_secs_f64() * FPS) as u64, 1.0),
        };
        // `--frames`: the game stops at that frame.
        let due =
            self.opts.frames.filter(|_| self.session.as_ref().is_some_and(|s| !s.preview)).map_or(due, |n| due.min(n));
        // Late: at most one frame more than due is run to catch up, the rest
        // of the lateness dropped (the clock moves on below), rather than a
        // sprint of several frames in one redraw that shows as the game
        // running fast.
        let catch_up = if retry {
            self.net_budget
        } else if self.opts.fast {
            MAX_CATCH_UP
        } else {
            MAX_CATCH_UP.min(CATCH_UP)
        };
        let mut ran = 0;
        let t_real = Instant::now();
        let held = |s: &Session| s.net.as_ref().is_some_and(|n| n.pending);
        while self.session.as_ref().is_some_and(|s| s.machine.frames < due || held(s)) && ran < catch_up {
            if self.session.as_ref().is_some_and(|s| !s.preview && !held(s)) {
                self.apply_inputs();
                // The pad opened the menu: pause now (not in a network game).
                if self.menu.open && !net_game {
                    break;
                }
            }
            let s = self.session.as_mut().unwrap();
            // Photo mode's history: the inputs this frame runs with, and a
            // save state now and then (nothing in the machine changes).
            if let Some(h) = &mut s.history {
                h.before_frame(&mut s.machine);
            }
            if let Some(n) = &mut s.net {
                // One cabinet of a network session: run the frame once, then pass the
                // boundary when the other machines' records are in; until then the frame
                // is held (the screen keeps the last one) and tried again next time.
                if !n.pending {
                    s.machine.run_slice(u64::MAX)?;
                }
                if !n.boundary(&mut s.machine) {
                    break;
                }
            } else {
                match &mut s.link {
                    // Every machine a frame, alongside each other, then the link blocks across.
                    Some(l) => l.step(&mut s.machine)?,
                    None => {
                        s.machine.run_slice(u64::MAX)?;
                    }
                }
            }
            if let Some(c) = &mut s.catalogue {
                c.tick(&mut s.machine.bus);
            }
            if let Some(log) = &mut self.sound_log {
                use std::io::Write;
                for &(cycle, control, byte) in &s.machine.bus.uart_log {
                    let ctl = if control { " ctl" } else { "" };
                    let _ = writeln!(log, "{} {} {cycle} {byte:02x}{ctl}", s.game.id, s.machine.frames);
                }
                let _ = log.flush();
            }
            match &mut s.sound {
                Some(snd) => snd.send(&mut s.machine.bus),
                None => s.machine.bus.uart_log.clear(),
            }
            if let Some(l) = &mut s.link {
                l.send_sound();
            }
            ran += 1;
        }
        if retry {
            // Between redraws: what ran is counted with the next redraw's, and the
            // draw shows it then.
            self.net_credit += ran;
            self.net_budget -= ran;
            return Ok(ran);
        }
        self.net_due = due;
        self.net_budget = catch_up - ran;
        self.real_emulate = t_real.elapsed();
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
        if let Some(s) = self.session.as_mut() {
            if dt > 0.0 {
                let others = s.link.iter_mut().flat_map(|l| l.sounds.iter_mut().flatten());
                // A network game: frames the retries ran since the last redraw count
                // as this one's, and a short hold is not slowness (`sound::net_feed`).
                let credit = std::mem::take(&mut self.net_credit);
                let held = s.net.as_ref().and_then(|n| n.held_for());
                let due_f = dt * FPS * speed;
                let ran_f = match &s.net {
                    Some(_) => sound::net_feed(ran as f64 + credit as f64, due_f, held.map(|h| h.as_secs_f64() * FPS)),
                    None => ran as f64,
                };
                for snd in s.sound.iter_mut().chain(others) {
                    snd.set_speed(speed, dt, ran_f, due_f);
                }
            }
        }
        self.net_log(ran, dt);
        // For the log: the audio running dry, as it happens (once a
        // second at most), with what the game was doing.
        if let Some(s) = self.session.as_ref().filter(|s| !s.preview) {
            if let Some(snd) = &s.sound {
                let underruns = snd.queue.lock().unwrap().underruns;
                if underruns > self.underruns_logged
                    && self.underrun_logged_at.is_none_or(|t| t.elapsed().as_secs_f64() >= 1.0)
                {
                    println!(
                        "underrun      {} frame {}: the audio ran dry ({} so far; {:.0} ms queued now, {:.1} ms since the last redraw)",
                        s.game.id,
                        s.machine.frames,
                        underruns,
                        snd.queued() * 1000.0,
                        dt * 1000.0
                    );
                    self.underruns_logged = underruns;
                    self.underrun_logged_at = Some(Instant::now());
                }
            }
        }
        let frames = self.session.as_ref().map_or(0, |s| s.machine.frames);
        if self.per_refresh().is_some() || frames < due {
            // Following the display (or fell behind): keep the clock current.
            self.epoch = now - Duration::from_secs_f64(frames as f64 / FPS);
        }
        Ok(ran)
    }

    /// `REMODEL_NET_LOG=1`: a line a second of how a network game keeps
    /// its speed and its audio (for measuring, docs/play.md).
    fn net_log(&mut self, ran: u32, dt: f64) {
        static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        if !*ON.get_or_init(|| std::env::var_os("REMODEL_NET_LOG").is_some()) {
            return;
        }
        let Some(s) = self.session.as_ref().filter(|s| !s.preview) else { return };
        let Some(n) = &s.net else { return };
        let _ = (ran, dt);
        let at = *self.net_log_at.get_or_insert_with(Instant::now);
        if at.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.net_log_at = Some(Instant::now());
        let st = &n.link.stalls;
        let (step, queued) = s.sound.as_ref().map_or((1.0, 0.0), |snd| (snd.step(), snd.queued() * 1000.0));
        println!(
            "netlog        frame {} boundaries {} held {} held_ms {:.0} audio_step {step:.4} queued_ms {queued:.0}",
            s.machine.frames,
            st.boundaries,
            st.stalls,
            st.stall_time.as_secs_f64() * 1000.0
        );
    }

    /// Emulate what's due and keep the stats; false if the game stopped.
    fn run_due(&mut self) -> (u32, Duration) {
        self.run_due_as(false)
    }

    /// A network game is holding its frame boundary for the others' records.
    fn net_held(&self) -> bool {
        !self.opts.fast
            && !self.launcher.open
            && self.session.as_ref().is_some_and(|s| !s.preview && s.net.as_ref().is_some_and(|n| n.pending))
    }

    /// A network game's held boundary, tried again between redraws: when the
    /// others' records have come the frame runs now, not at the next vblank
    /// (about_to_wait calls this every `NET_POLL` while it is held).
    fn net_retry(&mut self) {
        let held = self.session.as_ref().is_some_and(|s| !s.preview && s.net.as_ref().is_some_and(|n| n.pending));
        if held && !self.launcher.open && self.photo.is_none() {
            self.net_poll_at = Instant::now();
            self.run_due_as(true);
        }
    }

    fn run_due_as(&mut self, retry: bool) -> (u32, Duration) {
        let t0 = Instant::now();
        let span = perf::spans::begin();
        let result = self.emulate(retry);
        perf::spans::end(perf::spans::Kind::Emulate, span);
        if let (Ok(1..), Some(s)) = (&result, &self.session) {
            // When the newest frame was due: on the clock, or now when
            // following the display (which sets the clock to now).
            self.smooth_at = self.epoch + Duration::from_secs_f64(s.machine.frames as f64 / FPS);
        }
        let emulate_time = t0.elapsed();
        if let (Ok(ran), Some(s), false) = (&result, &mut self.session, retry) {
            if let Some(p) = &mut s.perf {
                p.redraw(&s.machine, *ran, emulate_time, s.sound.as_ref().map(|snd| snd.queued()));
            }
        }
        match result {
            Ok(ran) => {
                let st = &mut self.stats;
                st.emulating += emulate_time;
                st.emulated += ran;
                if ran == 0 && !retry && !self.menu.open && self.photo.is_none() {
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
                        self.menu.error(format!("The game stopped: {e:?}"));
                        self.open_launcher();
                    }
                }
                (0, emulate_time)
            }
        }
    }

    /// Copy the session's current frame for drawing (into the drawer's
    /// `draw::Core::frame`, so the drawer must be idle) and say how to draw
    /// it: this frame's polygons if the geometrizer has them in time for
    /// this refresh (`t0`, when the redraw started), else the previous
    /// frame's; with smooth motion, in-betweens. None without a session.
    ///
    /// The game's machine or run-ahead's, whichever is shown: a switch
    /// between them starts the renderer's video RAM and temporal
    /// accumulation afresh, as their versions and frames are each
    /// machine's own.
    fn capture_game(&mut self, t0: Instant) -> Option<draw::GameJob> {
        self.sync_pack();
        let look = self.look_current();
        let (Some(g), Some(s)) = (&mut self.gpu, &mut self.session) else { return None };
        // When the geometrizer is the slowest part, drawing the previous
        // frame's polygons lets it run alongside the next frame instead of
        // after it, at the cost of the polygons lagging a frame.
        let budget = frame_budget(g.refresh, self.values.exact);
        let period = frame_budget(g.refresh, self.values.exact || self.sync.behind).as_secs_f64();
        let ahead = s.run_ahead.as_ref().is_some_and(|ra| ra.is_ahead_of(&s.machine));
        let switched = ahead != s.drew_ahead;
        s.drew_ahead = ahead;
        let core = g.drawer.core();
        if switched {
            core.frame.forget();
        }
        // Anti-aliasing, the depth buffer and smooth shading (by the game's
        // rules): the game's settings, not the launcher preview's (flat).
        // Every image (game frame, in-between, late, run-ahead) is shaded
        // by the renderer from the polygons it draws.
        let mut quality =
            if s.preview { gpu::Quality::default() } else { self.values.quality(self.temporal_supported) };
        // Linked machines draw without temporal anti-aliasing (no motion data) and
        // smooth motion.
        let linked = s.link.is_some();
        if linked {
            quality.temporal = 0;
        }
        let bus = &mut s.shown().bus;
        let span = perf::spans::begin();
        let late = !bus.geo.wait_until(t0 + budget.mul_f64(0.8));
        perf::spans::end(perf::spans::Kind::Geometry, span);
        core.frame.capture(bus, late);
        self.stats.copied += core.frame.bytes;
        if late {
            self.stats.geo_late += 1;
        }
        // Smooth motion's in-betweens: by its setting, not just its
        // recording (which temporal anti-aliasing turns on too); only on a
        // display faster than the game, and not when it's already behind.
        let fast = g.refresh.is_none_or(|hz| hz > FPS * 1.2)
            && !self.sync.behind
            && !self.menu.open
            && !linked
            && self.values.smooth_on();
        // With smooth shading or material maps, an in-between gets its own
        // lighting side channel (its replay records it as the game frame's
        // was), so it is shaded at the level of the frames around it.
        // (In-betweens are of the geometrizer's newest frame: not when late.)
        let lit = (quality.shading != gpu::Shading::Flat || quality.materials)
            && !late
            && !linked
            && bus.geo.engine().light_on();
        // Link play: the other machines' frames, each in its own copy and renderer.
        if let Some(l) = &mut s.link {
            core.ensure_views(l.n(), self.max_anisotropy, self.temporal_supported);
            for (m, v) in l.machines.iter_mut().zip(core.views.iter_mut()) {
                let late = !m.bus.geo.wait_until(t0 + budget.mul_f64(0.8));
                v.frame.capture(&mut m.bus, late);
                self.stats.copied += v.frame.bytes;
                self.stats.geo_late += late as u32;
            }
        }
        // For the frame log: what this picture is.
        self.drawing = Some((s.shown().frames, ahead, late));
        Some(draw::GameJob {
            forget_ram: switched,
            reset_history: switched,
            quality,
            look,
            rules: (&s.game.shading, hud_mask(s.game)),
            smooth: fast.then_some((self.smooth_at, period)),
            lit,
            compare: None,
            link: g.link.as_ref().map(|l| l.views.clone()),
        })
    }

    /// Account for the draw just done (`draw::Drawer::take_report`): the
    /// status line's counters, the frame log.
    fn take_report(&mut self) {
        let Some(g) = &mut self.gpu else { return };
        let Some(r) = g.drawer.take_report() else { return };
        let drawing = self.drawing.take();
        if let Some(msg) = r.failed {
            // A draw that panicked (draw::run_guarded): say so once, carry
            // on, and give up cleanly if every draw fails.
            if self.draw_failures == 0 {
                eprintln!("draw failed: {msg} (the player carries on; please report it)");
                self.menu.error(format!("A draw failed: {msg}"));
            }
            self.draw_failures += 1;
            self.draw_failures_in_row += 1;
            return;
        }
        self.draw_failures_in_row = 0;
        if r.missed {
            self.stats.surface_misses += 1;
            return;
        }
        self.stats.drawn += 1;
        self.stats.drawing += r.draw;
        self.last_draw = r.draw;
        if let Some(gr) = r.game {
            if let Some(t) = gr.smooth {
                self.stats.smooth += 1;
                self.stats.smoothing += t;
            }
            // Smooth shading: what the frame was drawn as, and its cost (a
            // repeat of a frame costs nothing: the renderer keeps its
            // shading).
            if let Some((st, eff)) = gr.shading {
                self.stats.shaded += 1;
                self.stats.shading += Duration::from_secs_f32(st.ms.max(0.0) / 1000.0);
                self.stats.shading_as = Some((eff, st.lit, st.polys - st.hidden));
            }
            g.pack_stats = gr.pack;
        }
        if let (Some(log), Some(hash)) = (&mut self.frame_log, r.hash) {
            use std::io::Write;
            let line = match drawing {
                Some((frame, ahead, late)) => format!(
                    "{frame} {}{} {hash:016x}",
                    if ahead { "ahead" } else { "game" },
                    if late { "-late" } else { "" }
                ),
                None => format!("- - {hash:016x}"),
            };
            let _ = writeln!(log, "{line}");
        }
    }

    /// Smooth the frame cost (`ran` frames emulated in `emulate`, the frame
    /// copied and the overlay laid out in `overhead`, drawn in `draw`):
    /// below full speed when following the display, pace by the clock
    /// instead, running frames as fast as they can be (dropping to half the
    /// refresh rate otherwise). With the drawing on the draw thread, a
    /// frame costs the busier thread's time, not the sum.
    fn note_cost(&mut self, ran: u32, emulate: Duration, overhead: Duration, draw: Duration) {
        let Some(g) = &self.gpu else { return };
        // (The launcher draws on this thread.)
        let overlap = g.drawer.overlaps() && !self.launcher.open;
        if ran > 0 {
            // What run-ahead would add to: the game's own frames, the rest
            // of the main thread's work and the drawing, as measured with
            // or without it.
            let e = self.real_emulate.as_secs_f64() / ran as f64;
            let (o, d) = (overhead.as_secs_f64(), draw.as_secs_f64());
            let k = if self.cost_frames == 0 { 1.0 } else { 0.05 };
            self.emulate_cost += (e - self.emulate_cost) * k;
            self.overhead_cost += (o - self.overhead_cost) * k;
            self.draw_cost += (d - self.draw_cost) * k;
            self.cost_frames = self.cost_frames.saturating_add(1);
            let main = emulate + overhead;
            let busy = if overlap { main.max(draw) } else { main + draw };
            let cost = busy.as_secs_f64() / ran as f64;
            let budget = frame_budget(g.refresh, false).as_secs_f64();
            if let Some(change) = self.sync.note(cost, ran, budget) {
                let frame = self.session.as_ref().map_or(0, |s| s.machine.frames);
                println!(
                    "pacing        frame {frame}: {} ({:.2} ms a frame against {:.2})",
                    match change {
                        sync::Change::Behind => "too slow to sync, paced by the clock",
                        sync::Change::Synced => "synced to the display again",
                    },
                    self.sync.cost * 1000.0,
                    budget * 1000.0
                );
            }
            // For `next_start`: a redraw's emulation, run-ahead included.
            let k = if self.emulate_redraw == 0.0 { 1.0 } else { 0.1 };
            self.emulate_redraw += (emulate.as_secs_f64() - self.emulate_redraw) * k;
        }
        // Run-ahead the machine can't keep up with: off (until it's set
        // again), rather than the game running slow. Counted in frames
        // run, so time in the menu doesn't count.
        // (Not while `--frame-log` reads every picture back.)
        let keep = self.frame_log.is_some();
        if self.sync.behind_frames as f64 > RUN_AHEAD_GRACE * FPS && !keep {
            if let Some(s) = self.session.as_mut().filter(|s| s.run_ahead.is_some()) {
                s.run_ahead = None;
                s.run_ahead_dropped = true;
                s.machine.bus.geo.set_smooth(self.values.motion_on());
                self.sync.reset();
                self.menu.warn("Run-ahead is off: this computer can't keep up with it");
                println!("run-ahead     off: too slow to keep up");
            }
        }
    }

    /// Forget the frame costs (a new game is running).
    fn reset_cost(&mut self) {
        (self.emulate_cost, self.overhead_cost, self.draw_cost, self.cost_frames) = (0.0, 0.0, 0.0, 0);
        self.emulate_redraw = 0.0;
    }

    /// The frames of run-ahead this computer has time for, once the game
    /// has run long enough to tell (`hints::FrameCost`); None for a game
    /// other than `set`.
    fn run_ahead_room(&self, set: &str) -> Option<u32> {
        let (g, s) = (self.gpu.as_ref()?, self.session.as_ref()?);
        if s.game.id != set || self.cost_frames < COST_FRAMES {
            return None;
        }
        let cost = hints::FrameCost {
            emulate: self.emulate_cost,
            overhead: self.overhead_cost,
            draw: self.draw_cost,
            budget: frame_budget(g.refresh, false).as_secs_f64(),
            // The launcher draws its preview on the main thread.
            overlap: g.drawer.overlaps() && !self.launcher.open,
        };
        Some(cost.run_ahead_room())
    }

    /// With the draw thread still drawing the frame before: when to start
    /// this redraw anyway (None: now). The next frame's emulation starts
    /// once that draw is done, as it would after it on one thread, or, if
    /// that leaves too little of the refresh for it, when waiting longer
    /// would miss the next one (its emulation, at the time the last ones
    /// took, and a margin, before a refresh after the draw started, which
    /// is after the surface gave it a texture, past any wait for vsync):
    /// then they overlap. The event loop runs in between, so the controls
    /// read are as fresh as without the draw thread; the draw thread wakes
    /// it when it starts drawing and when it's done.
    fn next_start(&mut self) -> Option<Instant> {
        let g = self.gpu.as_mut()?;
        g.drawer.busy_since()?;
        if g.drawer.done() {
            return None;
        }
        let Some(since) = g.drawer.drawing_since() else {
            // Still waiting for the surface: it wakes the loop.
            return Some(Instant::now() + Duration::from_secs(1));
        };
        // A refresh (one game frame or none at 120 Hz), or a game frame.
        let budget = g.refresh.map_or(1.0 / FPS, |hz| 1.0 / hz);
        let e = self.emulate_redraw;
        let margin = 0.001 + 0.1 * e;
        let at = since + Duration::from_secs_f64((budget - e - margin).max(0.0));
        (Instant::now() < at).then_some(at)
    }

    /// `--fast`: the frames due and `--photo`'s sessions, nothing drawn.
    fn fast_frame(&mut self) {
        self.poll_pads();
        if self.launcher.open {
            return;
        }
        self.run_due();
        self.photo_script();
        self.status();
    }

    /// One redraw: run the game, draw it and the menu, report status.
    fn frame(&mut self, el: &ActiveEventLoop) {
        // Also while the game isn't reading them (the menu's open, or the
        // launcher), and before it runs (the pad may open the menu).
        self.poll_pads();
        if self.launcher.open {
            return self.launcher_frame(el);
        }
        self.waiting = self.next_start();
        if self.waiting.is_some() {
            perf::spans::mark(perf::spans::Kind::Defer);
            return;
        }
        let t0 = Instant::now();
        let (ran, emulate_time) = self.run_due();
        if self.launcher.open {
            return;
        }
        if let Some(s) = self.session.as_mut().filter(|s| s.jit_saved.elapsed() > Duration::from_secs(30)) {
            s.save_jit_list();
        }
        // `--photo`: a scripted photo session once its frame is reached.
        self.photo_script();
        if self.photo.is_some() {
            return self.photo_frame();
        }
        let slots = if self.menu.open { self.slot_infos() } else { Vec::new() };
        let crosshair = if self.menu.visible() { None } else { self.stick_crosshair() };
        let bezel_at = self.gpu.as_ref().and_then(|g| self.bezel_at(&g.view, g.origin, g.config.width));
        let Some(g) = &mut self.gpu else { return };
        // The frame before, if it is still being drawn: its core (and the
        // copy of its frame) is needed for this one.
        let t_wait = Instant::now();
        g.drawer.finish();
        self.stats.waiting += t_wait.elapsed();
        self.take_report();
        let t_capture = Instant::now();
        let game = self.capture_game(t0);
        let mut actions = Vec::new();
        let fade = self.game_fade.map(|t| fade_left(t.elapsed(), GAME_FADE)).filter(|f| *f > 0.0);
        if fade.is_none() {
            self.game_fade = None;
        }
        let mut overlay = None;
        let mut compare = None;
        let linked = self.session.as_ref().is_some_and(|s| s.link.is_some());
        let netted = self.session.as_ref().is_some_and(|s| s.net.is_some());
        if self.menu.visible()
            || self.session.is_none()
            || self.splash.active()
            || crosshair.is_some()
            || bezel_at.is_some()
            || fade.is_some()
            || linked
            || netted
        {
            let pack_text = self.pack_text();
            let pad_live = self.pad_live();
            let materials_text = self.materials_text();
            let look_now = self.look_current();
            let has_bezel = self.bezel.is_some();
            let room = self.session.as_ref().and_then(|s| self.run_ahead_room(s.game.id));
            let keeps = match self.session.as_ref().map(|s| s.game.id) {
                Some(id) => menu::Keeps {
                    all_games: self.settings.all_games_preset(id),
                    own_picture: self.settings.has_own_picture(id),
                },
                None => menu::Keeps::default(),
            };
            let g = self.gpu.as_mut().unwrap();
            let picture = self
                .session
                .as_ref()
                .filter(|s| !s.preview)
                .map(|_| [g.origin.0 as f32, g.origin.1 as f32, g.view.width as f32, g.view.height as f32]);
            let view = menu::View {
                playing: self.session.as_ref().map(|s| s.game.title),
                saves: &slots,
                game: self.session.as_ref().map(|s| s.game),
                picture,
                keeps,
                has_bezel,
                has_own: false,
                run_ahead_dropped: self.session.as_ref().is_some_and(|s| s.run_ahead_dropped),
                run_ahead_room: room,
                max_anisotropy: self.max_anisotropy,
                temporal_supported: self.temporal_supported,
                pack: pack_text.as_deref(),
                materials: materials_text.as_deref(),
                look: look_now.as_deref(),
                pad: pad_live,
                link: self.session.as_ref().and_then(|s| s.link.as_ref()).map(|l| l.n()),
                running: self.session.as_ref().is_some_and(|s| s.net.is_some() && !s.preview),
            };
            let link_ui = link_ui(self.session.as_ref(), self.gilrs.as_ref(), g.link.as_ref());
            let net_line =
                self.session.as_ref().filter(|s| !s.preview).and_then(|s| s.net.as_ref()).map(|n| n.status());
            let navs = launcher::Nav::view_as_back(&std::mem::take(&mut self.navs));
            let (menu, splash, bezel) = (&mut self.menu, &mut self.splash, &mut self.bezel);
            let mut values = self.values.clone();
            let (mut fullscreen, mut for_all) = (self.settings.fullscreen, self.settings.for_all);
            let no_game = self.session.is_none();
            let mut bindings = self.session.as_mut().map(|s| &mut s.bindings);
            let scale = g.window.scale_factor() as f32;
            let size = [g.config.width, g.config.height];
            let paint;
            (paint, actions) = run_overlay(g, no_game, size, None, |ctx| {
                if let (Some(b), Some((rect, pic))) = (bezel.as_mut(), bezel_at) {
                    b.paint(ctx, rect, pic, size[0] as f32, scale);
                }
                if let Some((x, y)) = crosshair {
                    draw_crosshair(ctx, egui::pos2(x / scale, y / scale));
                }
                if let Some((cells, views, empty, seats, title)) = &link_ui {
                    link_labels(ctx, scale, cells, views, *empty, seats, title);
                }
                if let Some(line) = &net_line {
                    net_status(ctx, line);
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
            overlay = Some(paint);
            compare = self.menu.wipe_rect().map(|rect| draw::Compare {
                before: GameSettings::default().quality(self.temporal_supported),
                rect,
                size: (size[0], size[1]),
            });
            self.set_prefs(fullscreen, for_all);
            if let Some(set) = self.session.as_ref().map(|s| s.game.id) {
                self.edit_values(set, values);
            }
        }
        let overhead = t_capture.elapsed();
        self.stats.capturing += overhead;
        // Draw it: on the draw thread, while the next redraw emulates, or
        // here and now.
        let g = self.gpu.as_mut().unwrap();
        let mut game = game;
        if let Some(gj) = game.as_mut().filter(|g| g.link.is_none()) {
            gj.compare = compare;
        }
        let job = draw::Job {
            game,
            photo: None,
            view: g.view,
            origin: g.origin,
            config: g.config.clone(),
            overlay,
            hash: self.frame_log.is_some(),
        };
        g.drawer.submit(job);
        // `--perf-log`: a gap line if this redraw came late (by the
        // display's refresh, or the game's frame if that isn't known).
        let (refresh, inline) = (Duration::from_secs_f64(1.0 / g.refresh.unwrap_or(FPS)), !g.drawer.overlaps());
        if let Some(s) = &mut self.session {
            if let Some(p) = &mut s.perf {
                p.submitted(&s.machine, refresh, inline);
            }
        }
        if !g.drawer.overlaps() {
            self.take_report();
        }
        self.note_cost(ran, emulate_time, overhead, self.last_draw);
        for action in actions {
            match action {
                menu::Action::Resume => self.set_menu(false),
                menu::Action::Save(n) => self.save_slot(n),
                menu::Action::Load(n) => {
                    self.load_slot(n);
                    self.set_menu(false);
                }
                menu::Action::Games => self.open_launcher(),
                menu::Action::Reset => self.reset_game(),
                menu::Action::Photo => {
                    self.set_menu(false);
                    if let Err(e) = self.enter_photo() {
                        self.menu.error(format!("Photo mode: {e}"));
                    }
                }
                menu::Action::Bindings => {
                    if let Some(s) = &self.session {
                        settings::save_bindings(s.game, &s.bindings);
                    }
                }
                menu::Action::Quit => self.quit(el),
                // The launcher's.
                menu::Action::ResetOwn => {}
            }
        }
        self.status();
    }

    /// Start the game again from power-on (the menu's Reset game), keeping
    /// its EEPROM but not its resume state.
    fn reset_game(&mut self) {
        if self.session.as_ref().is_some_and(|s| s.net.is_some()) {
            return self.menu.error("A network game can't be reset from one machine".to_string());
        }
        let Some(s) = self.session.as_ref().filter(|s| !s.preview) else { return };
        s.save_eeprom();
        let set = s.game.id.to_string();
        let players = s.link.as_ref().map_or(0, |l| l.n());
        self.session = None;
        self.set_menu(false);
        self.start_game_n(&set, library::Start::New, players);
        self.menu.notify("Game reset");
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
        let selected = self.launcher.selected().filter(|_| self.launcher.wants_preview());
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
            let leaving = self.preview.leaving.map(|t| fade_left(now - t, FADE_OUT));
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
        // The launcher draws on this thread: the game's last frame first.
        g.drawer.finish();
        self.take_report();
        let g = self.gpu.as_mut().unwrap();
        let core = g.drawer.core();
        let frame = match core.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            outcome => {
                draw::recover_surface(core, &g.config, &outcome);
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
        self.gpu.as_mut().unwrap().drawer.core().renderer.queue().present(frame);
        self.stats.drawn += 1;
        self.stats.drawing += draw_time;
        self.note_cost(ran, emulate_time, Duration::ZERO, draw_time);
        for action in actions {
            match action {
                launcher::Action::Play { set, from } => self.play_from_launcher(&set, from, 0),
                launcher::Action::PlayLink { set, players } => {
                    self.play_from_launcher(&set, library::Start::New, players)
                }
                launcher::Action::SetPlayerName(name) => {
                    // An edit on the Link play page replaces `--name` and is kept.
                    self.opts.name = None;
                    let name = linknet::name_or_default(&name);
                    if name != self.settings.player_name {
                        self.settings.player_name = name;
                        self.settings.save();
                    }
                }
                launcher::Action::NetHost { set } => self.net_lobby(&set, true),
                launcher::Action::NetJoin { set } => self.net_lobby(&set, false),
                launcher::Action::NetJoinAt { set, addr } => self.net_lobby_for(&set, None, Some(addr)),
                launcher::Action::PlayNet { set, session, params } => {
                    self.launcher.open = false;
                    self.launcher_bindings = None;
                    self.set_menu(false);
                    self.session = None;
                    self.preview =
                        PreviewState { failed: std::mem::take(&mut self.preview.failed), ..Default::default() };
                    self.start_net_game(&set, *session, *params);
                    if self.session.is_some() {
                        self.game_fade = Some(Instant::now());
                    }
                }
                launcher::Action::Version { title, set } => {
                    self.settings.versions.insert(title, set);
                    self.settings.save();
                }
                launcher::Action::Settings => {
                    self.set_menu(true);
                    self.menu.pad_used = self.launcher.pad_used;
                }
                launcher::Action::SettingsAt(page, tab) => {
                    self.set_menu(true);
                    self.menu.pad_used = self.launcher.pad_used;
                    self.menu.open_at(page, tab);
                }
                launcher::Action::CloseSettings => self.set_menu(false),
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
        // The view is kept for next time.
        if self.settings.launcher_view != self.launcher.mode {
            self.settings.launcher_view = self.launcher.mode;
            self.settings.save();
        }
        self.status();
    }

    /// Open the launcher's network panel for `set`: the host's lobby (`host`) or the
    /// joiner's page. `--host`/`--join` open it at start with their address.
    fn net_lobby(&mut self, set: &str, host: bool) {
        self.net_lobby_for(set, if host { Some(NetMode::Host(linknet::DEFAULT_PORT)) } else { None }, None)
    }

    fn net_lobby_for(&mut self, set: &str, host: Option<NetMode>, join: Option<String>) {
        let result: Result<netpage::NetPage, String> = (|| {
            let game = romset::games::find(set).map_err(|e| e.to_string())?;
            let play = link::play_for(game.id).ok_or_else(|| format!("{} can't link over a network", game.title))?;
            let roms = romset::RomSet::load(game, &self.settings.rom, false).map_err(|e| e.to_string())?;
            let name = self.opts.name.clone().unwrap_or_else(|| self.settings.player_name.clone());
            let cfg = netplay::config(game, &roms, play, name);
            let (values, _) = self.settings.resolve(set);
            let free_play = values.free_play.on(true);
            let mut page = match (host, join) {
                (Some(NetMode::Host(port)), _) => {
                    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
                    let h = linknet::Host::bind(cfg.clone(), addr).map_err(|e| {
                        format!("Can't listen on UDP port {port}: {e} (is another game already hosting on it?)")
                    })?;
                    println!("net link play hosting {} on port {port}", game.id);
                    netpage::NetPage::host(game, cfg, h, self.opts.net_players)
                }
                (_, Some(addr)) => netpage::NetPage::join(game, cfg, &addr),
                _ => netpage::NetPage::pick(
                    game,
                    cfg,
                    String::new(),
                    self.games.iter().map(|g| g.id.to_string()).collect(),
                ),
            };
            page.free_play = free_play;
            page.net_delay = values.net_delay;
            Ok(page)
        })();
        match result {
            Ok(page) => self.launcher.show_net(page),
            Err(e) => {
                eprintln!("{set}: {e}");
                self.menu.error(e);
            }
        }
        self.launcher.open = true;
    }

    /// Start `set` from the launcher (`players` machines linked on one
    /// screen, or 0 for a single game).
    fn play_from_launcher(&mut self, set: &str, from: library::Start, players: usize) {
        self.launcher.open = false;
        self.launcher_bindings = None;
        self.set_menu(false);
        self.session = None;
        self.preview = PreviewState { failed: std::mem::take(&mut self.preview.failed), ..Default::default() };
        self.start_game_n(set, from, players);
        if self.session.is_some() {
            self.game_fade = Some(Instant::now());
        }
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
        // The carousel's floor lies under the posters; the grid's is faint.
        let (horizon, level) = launcher::Layout::backdrop(self.launcher.backdrop_view());
        let g = self.gpu.as_mut().unwrap();
        let core = g.drawer.core();
        let params = backdrop::Params {
            size: [size[0] as f32, size[1] as f32],
            horizon,
            scale: ppp * l.u,
            phase: (secs * 0.18).fract() as f32,
            drift: (secs * 0.004 % 1000.0) as f32,
            level,
            _pad: 0.0,
        };
        core.backdrop.draw(core.renderer.device(), core.renderer.queue(), target, &params);
        // The preview: the game drawn as the window would show it, at the
        // preview's size (the screen's shape at half the size: it plays
        // dimmed behind the carousel).
        let hero = l.hero();
        let pw = (hero.width() * ppp).round().max(16.0) as u32;
        let ph = (hero.height() * ppp).round().max(16.0) as u32;
        let mut preview_view = None;
        if self.session.as_ref().is_some_and(|s| s.preview) {
            let (view, origin) = self.fit(pw, ph);
            let g = self.gpu.as_mut().unwrap();
            let format = g.config.format;
            let core = g.drawer.core();
            if g.preview.as_ref().is_none_or(|p| p.size != [pw, ph]) {
                let texture = core.renderer.device().create_texture(&wgpu::TextureDescriptor {
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
                let device = core.renderer.device();
                let id = match &g.preview {
                    Some(p) => {
                        core.egui.update_egui_texture_from_wgpu_texture(device, &view, wgpu::FilterMode::Linear, p.id);
                        p.id
                    }
                    None => core.egui.register_native_texture(device, &view, wgpu::FilterMode::Linear),
                };
                g.preview = Some(PreviewTarget { _texture: texture, view, id, size: [pw, ph] });
            }
            let pt = g.preview.as_ref().unwrap();
            let (pview, id) = (pt.view.clone(), pt.id);
            if let Some(job) = self.capture_game(t0) {
                let core = self.gpu.as_mut().unwrap().drawer.core();
                let r = draw::draw_game(core, &job, &pview, &view, origin, format);
                self.drawing = None;
                if let Some(t) = r.smooth {
                    self.stats.smooth += 1;
                    self.stats.smoothing += t;
                }
            }
            let shown = *self.preview.shown.get_or_insert(now);
            let fade_in = ((now - shown).as_secs_f32() / launcher::FADE_IN.as_secs_f32()).min(1.0);
            let fade_out = self.preview.leaving.map_or(1.0, |t| fade_left(now - t, FADE_OUT));
            preview_view = Some(launcher::PreviewView {
                texture: id,
                alpha: (fade_in * fade_out).clamp(0.0, 1.0),
                picture: [
                    origin.0 as f32 / pw as f32,
                    origin.1 as f32 / ph as f32,
                    view.width as f32 / pw as f32,
                    view.height as f32 / ph as f32,
                ],
            });
        }
        let selected = self.launcher.selected();
        let resume = selected.and_then(|g| std::fs::metadata(resume_path(g.id)).and_then(|m| m.modified()).ok());
        let progress = selected.and_then(|g| self.previews.progress(g.id));
        let failed = selected.and_then(|g| self.preview.failed.get(g.id)).cloned();
        let has_own = selected.is_some_and(|g| self.settings.has_own(g.id));
        let cont = library::quick_resume(&self.games, &self.settings.recent, |set| library::saved_in(&saves::dir(set)));
        let view = launcher::View {
            preview: preview_view,
            progress: progress.filter(|_| preview_view.is_none()),
            no_preview: failed.as_deref(),
            resume,
            unsupported: self.unsupported,
            cont,
            menu_open: self.menu.open,
            capturing: self.menu.capture.is_some(),
            settings: {
                let (picture, sound, speed) = menu::summary(&self.values);
                launcher::SettingsLine {
                    picture: picture.to_string(),
                    sound,
                    speed,
                    free_play: selected
                        .and_then(|g| link::play_for(g.id))
                        .filter(|p| p.free_play.is_some())
                        .map(|_| self.values.free_play.label().to_string()),
                    has_own,
                }
            },
        };
        // Settings is the menu's side panel over the launcher, for the
        // selected game (its Controls page edits that game's bindings).
        if self.menu.open && self.menu.on_controls() && self.launcher_bindings.is_none() {
            self.launcher_bindings = selected.map(settings::load_bindings);
        }
        if !(self.menu.open && self.menu.on_controls()) {
            self.launcher_bindings = None;
        }
        let keeps = match selected {
            Some(g) => menu::Keeps {
                all_games: self.settings.all_games_preset(g.id),
                own_picture: self.settings.has_own_picture(g.id),
            },
            None => menu::Keeps::default(),
        };
        let pad_live = self.pad_live();
        let menu_view = menu::View {
            playing: None,
            game: selected,
            saves: &[],
            max_anisotropy: self.max_anisotropy,
            temporal_supported: self.temporal_supported,
            has_bezel: selected.is_some_and(|g| bezel::art_set(g).is_some()),
            has_own,
            run_ahead_dropped: selected
                .is_some_and(|g| self.session.as_ref().is_some_and(|s| s.game.id == g.id && s.run_ahead_dropped)),
            run_ahead_room: selected.and_then(|g| self.run_ahead_room(g.id)),
            picture: None,
            keeps,
            pack: None,
            materials: None,
            look: None,
            pad: pad_live,
            link: None,
            running: false,
        };
        let navs = std::mem::take(&mut self.navs);
        let mut values = self.values.clone();
        let (mut fullscreen, mut for_all) = (self.settings.fullscreen, self.settings.for_all);
        let (launcher, menu, splash) = (&mut self.launcher, &mut self.menu, &mut self.splash);
        let bindings = &mut self.launcher_bindings;
        let animating = launcher.animating() || self.session.is_some() || progress.is_some();
        let g = self.gpu.as_mut().unwrap();
        let mut changed = false;
        let actions = draw_overlay(g, target, false, size, Some(ppp), |ctx| {
            let mut actions = launcher.ui(ctx, navs.clone(), &view);
            let prefs = menu::Prefs { fullscreen: &mut fullscreen, for_all: &mut for_all };
            for action in
                menu.ui(ctx, &mut values, prefs, &menu_view, bindings.as_mut(), &launcher::Nav::view_as_back(&navs))
            {
                match action {
                    menu::Action::Resume => actions.push(launcher::Action::CloseSettings),
                    menu::Action::ResetOwn => {
                        actions.extend(selected.map(|g| launcher::Action::ResetGame(g.id.to_string())))
                    }
                    menu::Action::Bindings => changed = true,
                    menu::Action::Quit => actions.push(launcher::Action::Quit),
                    // Nothing's running to save, load, reset or photograph.
                    _ => {}
                }
            }
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
        self.set_prefs(fullscreen, for_all);
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
        let materials = self.materials_text().map_or(String::new(), |t| format!(" · material maps: {t}"));
        let st = &self.stats;
        let snd = self.session.as_ref().and_then(|s| s.sound.as_ref());
        let audio = snd.map_or(String::new(), |s| {
            let underruns = s.queue.lock().unwrap().underruns;
            format!(" · audio {:.0} ms queued, {underruns} underruns", s.queued() * 1000.0)
        });
        let mode = match (self.per_refresh(), self.gpu.as_ref().and_then(|g| g.refresh)) {
            (Some(_), Some(hz)) => format!("synced to {hz:.0} Hz"),
            _ if self.sync.behind => "too slow to sync, paced by the clock".into(),
            _ => "exact speed".into(),
        };
        // Handing each frame over to drawing on the main thread (waiting
        // for the geometrizer, copying the frame, the overlay), and with the
        // draw thread, waiting for it to finish the frame before.
        let overlap = self.gpu.as_ref().is_some_and(|g| g.drawer.overlaps()) && !self.launcher.open;
        let per_draw = |d: Duration| d.as_secs_f64() * 1000.0 / st.drawn.max(1) as f64;
        let copied = st.copied as f64 / st.drawn.max(1) as f64 / (1 << 20) as f64;
        let thread = if overlap {
            format!(
                " on its thread (hand-over {:.2} ms, {copied:.1} MB; waited {:.2} ms)",
                per_draw(st.capturing),
                per_draw(st.waiting)
            )
        } else {
            format!(" (hand-over {:.2} ms, {copied:.1} MB)", per_draw(st.capturing))
        };
        let status = format!(
            "{mode} · {:.0}% speed · {:.0} fps ({} repeats, longest gap {:.0} ms) · emulate {:.1} ms/frame · draw {:.2} ms{thread}{audio}{}",
            st.emulated as f64 / secs / FPS * 100.0,
            st.drawn as f64 / secs,
            st.repeats,
            st.longest_gap.as_secs_f64() * 1000.0,
            st.emulating.as_secs_f64() * 1000.0 / st.emulated.max(1) as f64,
            per_draw(st.drawing),
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
                " · shading {}{}{}, {lit}/{polys} polygons lit, {:.2} ms a draw",
                level.name(),
                if self.applied.shading_fine { " fine" } else { "" },
                if self.applied.shading_balance { "" } else { " unbalanced" },
                st.shading.as_secs_f64() * 1000.0 / st.shaded.max(1) as f64
            ),
            None => String::new(),
        } + &materials
            + &match self.session.as_ref().map(|s| (&s.run_ahead, s.run_ahead_dropped)) {
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
        self.slot_cache = None;
        // Keys held when the menu opened would stay pressed otherwise; the
        // pad's Start and Select are kept from the game until released.
        self.keys.clear();
        self.steer_filters.clear();
        self.gate.menu_changed();
        if let Some(l) = self.session.as_mut().and_then(|s| s.link.as_mut()) {
            l.seats.iter_mut().for_each(|seat| seat.gate.menu_changed());
        }
        self.pad_nav = pad::PadNav::default();
        self.navs.clear();
        self.menu.pad_used = false;
    }

    /// Quit if `--seconds` has run out, reporting the run; true if so.
    fn time_up(&mut self, el: &ActiveEventLoop) -> bool {
        let at_frame = self.opts.frames.is_some_and(|n| {
            self.session
                .as_ref()
                .is_some_and(|s| !s.preview && s.machine.frames >= n && !s.net.as_ref().is_some_and(|n| n.pending))
        });
        // A scripted network run also stops when its session ends.
        let net_over = self.opts.frames.is_some()
            && self.session.as_ref().is_some_and(|s| s.net.as_ref().is_some_and(|n| n.ended.is_some()));
        if !at_frame && !net_over && !self.opts.seconds.is_some_and(|s| self.started.elapsed().as_secs_f64() >= s) {
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
            if let Some(n) = &mut s.net {
                // Where the session stands, as `remodel link-net` prints it, so two players'
                // runs can be compared with `link-run`'s hash for each node.
                println!("node {}        state hash {hash:016x} ({} frames)", n.node, s.machine.frames);
                match n.ended.clone() {
                    Some(e) => println!("net link play ended: {e}"),
                    None => n.link.linger(s.machine.frames, Duration::from_secs(3)),
                }
                let (st, ss) = (&n.link.stalls, &n.link.sess.stats);
                println!(
                    "net link play stalls {} of {} boundaries ({:.1} ms in all, longest {:.1} ms), D {}, round trip {}, sent {:.1} KB, received {:.1} KB",
                    st.stalls,
                    st.boundaries,
                    st.stall_time.as_secs_f64() * 1e3,
                    st.longest.as_secs_f64() * 1e3,
                    n.delay,
                    n.link.sess.worst_rtt().map_or("?".into(), |r| format!("{r:.2} ms")),
                    ss.bytes_tx as f64 / 1024.0,
                    ss.bytes_rx as f64 / 1024.0
                );
            }
            if let Some(l) = &mut s.link {
                let h = l.combined_hash(&mut s.machine);
                println!("link play     {} machines, combined hash {h:016x} (as `remodel link-run` prints it)", l.n());
                if l.steps > 0 {
                    let ms = |d: Duration| d.as_secs_f64() * 1e3 / l.steps as f64;
                    println!(
                        "link step     {:.2} ms a frame (the machines {:.2} ms at the slowest, the rest thread starts, the hub and waiting), {} frames",
                        ms(l.step_total),
                        ms(l.busiest_total),
                        l.steps
                    );
                }
            }
            if let Some(h) = s.history.as_ref().filter(|h| h.taken > 0) {
                println!(
                    "photo history {} save states, {:.2} ms each on the game's thread, {:.1} MB held",
                    h.taken,
                    h.cost.as_secs_f64() * 1000.0 / h.taken as f64,
                    h.bytes() as f64 / (1 << 20) as f64
                );
            }
            if let Some(d) = &s.machine.bus.drive {
                println!("drive board   {}", d.describe());
            }
            if let Some(c) = s.catalogue.take() {
                let ran = s.machine.frames.max(1) as f64;
                let sm = c.finish(&mut s.machine.bus);
                println!(
                    "catalogue     session {}: {} samples ({} dropped), {} entries, {} texture loads; game thread {:.3} ms a frame \
                     (texture watch {:.3}), worker {:.0} ms in all",
                    sm.session,
                    sm.samples,
                    sm.dropped,
                    sm.entries,
                    sm.loads,
                    sm.tick_ms / ran,
                    sm.watch_ms / ran,
                    sm.worker_ms
                );
            }
        }
        self.quit(el);
        true
    }

    /// Ask for the next redraw.
    fn redraw(&mut self) {
        self.waiting = None;
        if let Some(g) = &self.gpu {
            g.window.request_redraw();
        }
    }

    fn quit(&mut self, el: &ActiveEventLoop) {
        self.leave_photo();
        // The picture in flight: drawn, and logged.
        if let Some(g) = &mut self.gpu {
            g.drawer.finish();
        }
        self.take_report();
        self.previews.cancel_all();
        self.leave_game();
        if let Some(l) = &mut self.stick_log {
            print!("{}", l.summary());
        }
        self.settings.save();
        el.exit();
    }
}

/// Network link play's status line: a small pill at the top of the window.
fn net_status(ctx: &egui::Context, line: &str) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("net status")));
    let waiting = line.contains("WAITING");
    let colour = if waiting { egui::Color32::from_rgb(255, 214, 90) } else { egui::Color32::from_white_alpha(200) };
    let galley =
        painter.layout_no_wrap(line.to_string(), egui::FontId::new(13.0, egui::FontFamily::Proportional), colour);
    let at = egui::pos2(ctx.content_rect().center().x, ctx.content_rect().top() + 8.0);
    let rect = egui::Align2::CENTER_TOP.anchor_size(at, galley.size()).expand2(egui::vec2(10.0, 4.0));
    painter.rect_filled(rect, 10.0, egui::Color32::from_black_alpha(150));
    painter.galley(rect.center() - galley.size() / 2.0, galley, colour);
}

/// A crosshair at `at` (points), over everything.
/// What link play's tags show: the cells and views, the empty quadrant, each
/// seat's device, and the game's title.
type LinkUi = (Vec<link::Cell>, Vec<(model2::video::View, (u32, u32))>, Option<link::Cell>, Vec<String>, &'static str);

/// Link play's tags for `session` laid out as `lv` (None without link play).
fn link_ui(session: Option<&Session>, gil: Option<&Gilrs>, lv: Option<&LinkView>) -> Option<LinkUi> {
    let s = session?;
    let (l, v) = (s.link.as_ref()?, lv?);
    let seats: Vec<String> = l
        .seats
        .iter()
        .enumerate()
        .map(|(i, seat)| {
            let pad = seat.pad.and_then(|id| gil.map(|g| g.gamepad(id).name().to_string()));
            match (i == 0 && l.keyboard, pad) {
                (true, Some(p)) => format!("Keyboard, {p}"),
                (true, None) => "Keyboard".to_string(),
                (false, Some(p)) => p,
                (false, None) => "Press a button on a controller to join".to_string(),
            }
        })
        .collect();
    Some((v.cells.clone(), v.views.clone(), v.empty, seats, s.game.title))
}

/// Link play's tags over the views: each player's number and device in its
/// corner, and in the empty fourth quadrant of three players what this is.
/// (`cells` and `views` are in window pixels, `scale` the pixels per point.)
fn link_labels(
    ctx: &egui::Context,
    scale: f32,
    cells: &[link::Cell],
    views: &[(model2::video::View, (u32, u32))],
    empty: Option<link::Cell>,
    seats: &[String],
    title: &str,
) {
    let painter = ctx.layer_painter(egui::LayerId::new(egui::Order::Background, egui::Id::unique("link tags")));
    let pt = |x: u32, y: u32| egui::pos2(x as f32 / scale, y as f32 / scale);
    let font = |size: f32| egui::FontId::new(size, egui::FontFamily::Proportional);
    for (i, ((view, origin), seat)) in views.iter().zip(seats).enumerate() {
        let waiting = seat.starts_with("Press");
        let colour = if waiting { egui::Color32::from_rgb(255, 214, 90) } else { egui::Color32::from_white_alpha(190) };
        let text = format!("P{}  {seat}", i + 1);
        // In the bar above the picture where there is room, else a pill at the foot of it.
        let room = origin.1.saturating_sub(cells.get(i).map_or(0, |c| c.y));
        if room as f32 / scale >= 22.0 {
            painter.text(
                pt(origin.0 + 4, origin.1 - (20.0 * scale) as u32),
                egui::Align2::LEFT_TOP,
                text,
                font(13.0),
                colour,
            );
        } else {
            let galley = painter.layout_no_wrap(text, font(13.0), colour);
            let at = pt(origin.0 + view.width as u32 / 2, origin.1 + view.height as u32 - (10.0 * scale) as u32);
            let rect = egui::Align2::CENTER_BOTTOM.anchor_size(at, galley.size()).expand2(egui::vec2(8.0, 3.0));
            painter.rect_filled(rect, 8.0, egui::Color32::from_black_alpha(150));
            painter.galley(rect.center() - galley.size() / 2.0, galley, colour);
        }
    }
    if let Some(e) = empty {
        let c = pt(e.x + e.w / 2, e.y + e.h / 2);
        painter.rect_filled(
            egui::Rect::from_min_max(pt(e.x, e.y), pt(e.x + e.w, e.y + e.h)),
            0.0,
            egui::Color32::from_rgb(8, 11, 18),
        );
        painter.text(
            c - egui::vec2(0.0, 12.0),
            egui::Align2::CENTER_CENTER,
            "reModel",
            font(26.0),
            egui::Color32::from_white_alpha(70),
        );
        painter.text(
            c + egui::vec2(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            format!("{title}: {} linked", views.len()),
            font(13.0),
            egui::Color32::from_white_alpha(60),
        );
    }
}

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

/// Draw egui over `target` (`size` pixels, at `ppp` pixels per point, else
/// the window's scaling), building the UI with `ui`; returns what it
/// returned. `clear` clears to black first (no game under it).
fn draw_overlay<T: Default>(
    g: &mut Gpu,
    target: &wgpu::TextureView,
    clear: bool,
    size: [u32; 2],
    ppp: Option<f32>,
    ui: impl FnMut(&egui::Context) -> T,
) -> T {
    let (paint, result) = run_overlay(g, clear, size, ppp, ui);
    draw::paint_overlay(g.drawer.core(), &paint, target);
    result
}

/// Lay the overlay out (`ui`) for a `size` pixel target at `ppp` pixels a
/// point (None: the window's) and tessellate it, to paint with
/// `draw::paint_overlay` (`clear`: over nothing).
fn run_overlay<T: Default>(
    g: &mut Gpu,
    clear: bool,
    size: [u32; 2],
    ppp: Option<f32>,
    mut ui: impl FnMut(&egui::Context) -> T,
) -> (draw::OverlayPaint, T) {
    let o = &mut g.overlay;
    let mut raw = o.state.take_egui_input(&g.window);
    let ppp = ppp.unwrap_or(g.window.scale_factor() as f32);
    raw.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(ppp);
    raw.screen_rect =
        Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(size[0] as f32, size[1] as f32) / ppp));
    let mut result = T::default();
    theme::sync(&o.ctx, size[1] as f32 / ppp);
    let out = o.ctx.run_ui(raw, |ui_root| result = ui(ui_root.ctx()));
    o.state.handle_platform_output(&g.window, out.platform_output);
    let jobs = o.ctx.tessellate(out.shapes, out.pixels_per_point);
    let screen = egui_wgpu::ScreenDescriptor { size_in_pixels: size, pixels_per_point: out.pixels_per_point };
    (draw::OverlayPaint { jobs, textures: out.textures_delta, screen, clear }, result)
}

/// The draw thread's call to the event loop (`next_start`).
#[derive(Debug, Clone, Copy)]
struct Wake;

impl ApplicationHandler<Wake> for Play {
    /// `--seconds` is checked here as well as on redraws, and the loop set
    /// to wake at the limit: a hidden window may get almost no redraws
    /// (about one a second on Wayland). Without `--seconds` the loop keeps
    /// winit's default (wait for events), so pacing is unchanged.
    /// Waiting for the draw thread (`next_start`): redraw at the time set,
    /// unless it wakes the loop first.
    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.draw_failures_in_row >= DRAW_FAILURES_TO_QUIT {
            eprintln!("draw failed {} times in a row: quitting", self.draw_failures_in_row);
            self.draw_failures_in_row = 0;
            self.quit(el);
            return;
        }
        // `--fast`: run the game here, not on redraws (a hidden window gets
        // about one a second), and draw nothing.
        if self.opts.fast && self.gpu.is_some() {
            if !self.time_up(el) {
                self.fast_frame();
                el.set_control_flow(ControlFlow::Poll);
            }
            return;
        }
        if let Some(s) = self.opts.seconds {
            if self.time_up(el) {
                return;
            }
            el.set_control_flow(ControlFlow::WaitUntil(self.started + Duration::from_secs_f64(s)));
        } else {
            el.set_control_flow(ControlFlow::Wait);
        }
        if let Some(at) = self.waiting {
            if Instant::now() >= at {
                self.redraw();
            } else if self.opts.seconds.is_none_or(|s| at < self.started + Duration::from_secs_f64(s)) {
                el.set_control_flow(ControlFlow::WaitUntil(at));
            }
        }
        // A network game's held boundary is tried again every `NET_POLL` rather
        // than at the next redraw, so a record that lands in between is run
        // within a millisecond (a timer, not a spin: the loop sleeps between).
        if self.net_held() {
            let next = self.net_poll_at + NET_POLL;
            if Instant::now() >= next {
                self.net_retry();
            }
            if self.net_held() {
                let at = (self.net_poll_at + NET_POLL).max(Instant::now());
                match el_flow_deadline(
                    self.waiting,
                    self.opts.seconds.map(|s| self.started + Duration::from_secs_f64(s)),
                ) {
                    Some(d) if d < at => {}
                    _ => el.set_control_flow(ControlFlow::WaitUntil(at)),
                }
            }
        }
    }

    fn user_event(&mut self, _: &ActiveEventLoop, _: Wake) {
        if self.waiting.is_some() {
            self.redraw();
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
            // Blocks of 4 to 8 MB for the small things; a larger texture
            // gets a block of its own, its size exactly. (The default holds
            // 192 MB in blocks of 128 and 64 MB before the first texture,
            // and `MemoryUsage`'s 8 to 64 MB blocks hold up to one more
            // each of what doesn't fill them.)
            memory_hints: wgpu::MemoryHints::Manual { suballocated_device_memory_block_size: (4 << 20)..(8 << 20) },
            ..Default::default()
        }))
        .expect("create device");
        let max_msaa = gpu::max_msaa(&adapter);
        let size = window.inner_size();
        let mut config =
            surface.get_default_config(&adapter, size.width.max(1), size.height.max(1)).expect("surface config");
        // Colours are already display values: present without sRGB encoding.
        let caps = surface.get_capabilities(&adapter);
        let formats = caps.formats;
        config.format = [wgpu::TextureFormat::Bgra8Unorm, wgpu::TextureFormat::Rgba8Unorm]
            .into_iter()
            .find(|f| formats.contains(f))
            .unwrap_or(config.format.remove_srgb_suffix());
        config.present_mode = wgpu::PresentMode::AutoVsync;
        // For profiling a window nobody sees (the compositor sends a hidden
        // window one redraw a second under vsync): REMODEL_PRESENT=immediate
        // or mailbox, where offered (docs/play.md, "Profiling").
        let unpaced = match std::env::var("REMODEL_PRESENT").as_deref() {
            Ok("immediate") => Some(wgpu::PresentMode::Immediate),
            Ok("mailbox") => Some(wgpu::PresentMode::Mailbox),
            // A bounded run (`--frames`) is a check or a measurement, not
            // for watching: mailbox, which a window nobody sees can't stall
            // (vsynced, it gets about a frame a second).
            _ if self.opts.frames.is_some() => Some(wgpu::PresentMode::Mailbox),
            _ => None,
        }
        .filter(|m| caps.present_modes.contains(m));
        if let Some(m) = unpaced {
            config.present_mode = m;
        }
        // `--frame-log` reads each picture back.
        if self.opts.frame_log.is_some() {
            config.usage |= wgpu::TextureUsages::COPY_SRC;
        }
        surface.configure(&device, &config);
        // What AutoVsync resolves to (wgpu: FIFO relaxed if the surface
        // offers it, else FIFO), with the modes on offer, for slowdown
        // reports (docs/play.md).
        let mode = if let Some(m) = unpaced {
            m
        } else if caps.present_modes.contains(&wgpu::PresentMode::FifoRelaxed) {
            wgpu::PresentMode::FifoRelaxed
        } else {
            wgpu::PresentMode::Fifo
        };
        println!(
            "gpu           {name}; present {mode:?} (offered {:?}), frame latency {}, {:?}",
            caps.present_modes, config.desired_maximum_frame_latency, config.format
        );
        let ctx = egui::Context::default();
        theme::install(&ctx);
        let state = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            None,
            Some(device.limits().max_texture_dimension_2d as usize),
        );
        let egui = egui_wgpu::Renderer::new(&device, config.format, Default::default());
        let backdrop = backdrop::Backdrop::new(&device, config.format);
        let mut renderer = gpu::Renderer::from_device(device, queue, name);
        renderer.set_max_msaa(max_msaa);
        self.max_anisotropy = gpu::max_anisotropy(&adapter);
        renderer.set_max_anisotropy(self.max_anisotropy);
        // `from_device` assumes it; the GL fallback may not have it.
        self.temporal_supported = gpu::temporal_supported(&adapter);
        renderer.set_temporal_supported(self.temporal_supported);
        if !self.temporal_supported {
            println!("temporal AA   not on this GPU (no 16-bit float targets)");
        }
        let (view, origin) = self.fit(config.width, config.height);
        // Redraws drive everything; Wayland sends none until asked.
        window.request_redraw();
        let refresh = refresh_hz(&window);
        self.display = describe_display(&window);
        println!(
            "display       {}; window {}x{}{}",
            self.display,
            config.width,
            config.height,
            if window.fullscreen().is_some() { ", fullscreen" } else { "" }
        );
        let overlay = Overlay { ctx, state };
        let mut core = draw::Core {
            surface,
            renderer,
            egui,
            backdrop,
            frame: gpu::BoardFrame::default(),
            photo_shots: None,
            log: self.opts.perf_log.then(Default::default),
            recovered: None,
            views: Vec::new(),
        };
        core.renderer.set_profile(self.opts.perf_log);
        // The draw thread wakes the event loop as it starts drawing and when
        // it's done (`next_start`).
        let wake = self.opts.overlap.then(|| self.proxy.clone()).flatten().map(|p| {
            Box::new(move || {
                let _ = p.send_event(Wake);
            }) as Box<dyn Fn() + Send>
        });
        let drawer = draw::Drawer::new(core, wake);
        self.gpu = Some(Gpu {
            window,
            config,
            view,
            origin,
            refresh,
            overlay,
            preview: None,
            drawer,
            pack_stats: gpu::PackStats::default(),
            link: None,
        });
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
                            None => self.menu.warn("That key can't be bound: pick another"),
                        },
                        _ => self.menu.warn("That key can't be bound: pick another"),
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
        if self.photo.is_some() {
            if let WindowEvent::CursorMoved { position, .. } = &event {
                self.mouse = Some((position.x, position.y));
            }
            let over_panel = self.gpu.as_ref().is_some_and(|g| g.overlay.ctx.egui_wants_pointer_input());
            if matches!(event, WindowEvent::CursorMoved { .. })
                || !over_panel && matches!(event, WindowEvent::MouseInput { .. } | WindowEvent::MouseWheel { .. })
            {
                self.photo_mouse(&event);
            }
        }
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
                if self.photo.is_some() {
                    return self.photo_key(code, pressed, event.repeat);
                }
                if pressed && !event.repeat && code == photo::KEY && self.session.as_ref().is_some_and(|s| !s.preview) {
                    self.set_menu(false);
                    if let Err(e) = self.enter_photo() {
                        self.menu.error(format!("Photo mode: {e}"));
                    }
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
            WindowEvent::MouseInput { state, button, .. }
                if !self.menu.open && !to_menu && !self.launcher.open && self.photo.is_none() =>
            {
                let i = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    _ => return,
                };
                self.mouse_buttons[i] = state == ElementState::Pressed;
            }
            WindowEvent::RedrawRequested if self.opts.fast => {}
            WindowEvent::RedrawRequested => {
                if self.time_up(el) {
                    return;
                }
                perf::spans::mark(perf::spans::Kind::RedrawStart);
                self.frame(el);
                perf::spans::mark(perf::spans::Kind::RedrawEnd);
                // Unless waiting for the draw thread (`next_start`).
                if self.waiting.is_none() {
                    self.redraw();
                }
            }
            _ => {}
        }
    }
}

fn main() {
    recomp::register();
    model2::set_app_version(remodel_version::VERSION);
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
    let log_path = log::start(&settings::data_dir());
    println!(
        "reModel       {} on {}; JIT workers {}; data {}{}",
        remodel_version::VERSION,
        log::host(),
        model2::jit::workers(),
        settings::data_dir().display(),
        log_path.map_or(String::new(), |p| format!(", log {}", p.display()))
    );
    let gilrs = Gilrs::new().ok();
    if let Some(g) = &gilrs {
        for (_, p) in g.gamepads() {
            println!("controller    {} (used once a button or stick moves)", p.name());
        }
    }
    let splash = splash::Splash::new(opts.splash);
    let launcher_view = settings.launcher_view;
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
        slot_cache: None,
        launcher: {
            let mut l = launcher::Launcher::default();
            l.mode = launcher_view;
            l
        },
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
        stick_dirs: romset::StickDirs::default(),
        epoch: Instant::now(),
        smooth_at: Instant::now(),
        game_fade: None,
        redraws: 0,
        draw_failures: 0,
        draw_failures_in_row: 0,
        display: String::new(),
        underruns_logged: 0,
        underrun_logged_at: None,
        refreshes: 0,
        refresh_part: 0.0,
        last_emulate: None,
        net_due: 0,
        net_budget: 0,
        net_credit: 0,
        net_poll_at: Instant::now(),
        net_log_at: None,
        sync: sync::Sync::default(),
        emulate_cost: 0.0,
        overhead_cost: 0.0,
        draw_cost: 0.0,
        cost_frames: 0,
        last_draw: Duration::ZERO,
        waiting: None,
        emulate_redraw: 0.0,
        frame_log: None,
        sound_log: None,
        stick_log: None,
        steer_filters: Vec::new(),
        steer_preview: std::cell::RefCell::new((steer::SteerFilter::default(), Instant::now())),
        drawing: None,
        proxy: None,
        real_emulate: Duration::ZERO,
        stats: Stats::new(),
        started: Instant::now(),
        pack: PackState::default(),
        look: LookState::default(),
        max_anisotropy: 16,
        temporal_supported: true,
        photo: None,
    };
    if play.settings.rom == Settings::default().rom {
        // So there's an obvious place to put sets.
        let _ = std::fs::create_dir_all(&play.settings.rom);
    }
    play.scan();
    // `--game`: straight into it (an in-progress game too, which the
    // launcher doesn't list); else the launcher.
    let in_progress = |set: &str| {
        romset::games::IN_PROGRESS.iter().any(|g| g.id == set)
            && romset::sets_with_clones(&play.settings.rom).iter().any(|s| s == set)
    };
    match play.opts.game.clone() {
        Some(set) if play.games.iter().any(|g| g.id == set) || in_progress(&set) => {
            let players = play.opts.players;
            match play.opts.net.clone() {
                // Network link play: the lobby (or the connecting page) opens over the launcher.
                Some(NetMode::Join(addr)) => play.net_lobby_for(&set, None, Some(addr)),
                Some(host) => play.net_lobby_for(&set, Some(host), None),
                None => play.start_game_n(&set, library::Start::New, players),
            }
        }
        Some(set) => {
            println!("no {set} in {}: choose a game", play.settings.rom.display());
            play.menu.error(format!("No {set} in {}", play.settings.rom.display()));
            play.launcher.open = true;
        }
        None => play.launcher.open = true,
    }
    let event_loop = EventLoop::<Wake>::with_user_event().build().expect("event loop");
    play.proxy = Some(event_loop.create_proxy());
    if let Some(path) = &play.opts.stick_log {
        match sticklog::StickLog::create(path) {
            Ok(l) => play.stick_log = Some(l),
            Err(e) => {
                eprintln!("--stick-log: {}: {e}", path.display());
                log::finish();
                std::process::exit(1)
            }
        }
    }
    if let Some(path) = &play.opts.sound_log {
        match std::fs::File::create(path) {
            Ok(f) => play.sound_log = Some(std::io::BufWriter::new(f)),
            Err(e) => {
                eprintln!("--sound-log: {}: {e}", path.display());
                {
                    log::finish();
                    std::process::exit(1)
                }
            }
        }
    }
    if let Some(path) = &play.opts.frame_log {
        match std::fs::File::create(path) {
            Ok(f) => play.frame_log = Some(std::io::BufWriter::new(f)),
            Err(e) => {
                eprintln!("--frame-log: {}: {e}", path.display());
                {
                    log::finish();
                    std::process::exit(1)
                }
            }
        }
    }
    event_loop.run_app(&mut play).expect("event loop");
    // The draw thread's last picture, the frame log's last lines.
    drop(play.gpu.take());
    if let Some(mut log) = play.frame_log.take() {
        use std::io::Write;
        let _ = log.flush();
    }
    log::finish();
}

#[cfg(test)]
mod tests {
    use super::refreshes_gone;

    #[test]
    fn counts_refreshes_gone() {
        let mut part = 0.0;
        let ms = |t: f64| t / 1000.0;
        // 60 Hz: on time, late by a vblank, a jittery pair, an extra early one.
        let n: Vec<u64> = [16.7, 16.6, 33.4, 50.0, 24.0, 9.4, 4.0, 12.7]
            .iter()
            .map(|&t| refreshes_gone(&mut part, ms(t), 60.0))
            .collect();
        assert_eq!(n, [1, 1, 2, 3, 1, 1, 0, 1]);
        // Over a long run with jitter none are lost or made up.
        let mut total = 0;
        for i in 0..6000 {
            total += refreshes_gone(&mut part, ms(if i % 2 == 0 { 12.0 } else { 21.333 }), 60.0);
        }
        assert!((5999..=6001).contains(&total), "{total}");
        // A stall counts as one.
        assert_eq!(refreshes_gone(&mut part, 0.5, 60.0), 1);
        assert_eq!(part, 0.0);
    }
}
