//! Settings kept between runs, and where the player keeps its files.
//!
//! Run from the reModel source tree (an `out/` directory in the current
//! directory), files go under `out/` next to the other per-game output;
//! otherwise under `$XDG_DATA_HOME/remodel` (`~/.local/share/remodel`).
//!
//! A game's settings ([`GameSettings`]: sound mix, display, speed, smooth
//! motion, run-ahead, anti-aliasing, temporal anti-aliasing and its
//! stipple, depth buffer, smooth shading and its crease angle, texture
//! pack and its filtering, geometrizer) come in layers, lowest first, as
//! in the browser build (`crates/web/settings.js`): the built-in defaults, the defaults for
//! all games (`play-settings.txt`), the game's own (`<set>/settings.txt`)
//! and the command line (this run only). A layer keeps only what it sets,
//! so changing the defaults for all games reaches every game that hasn't
//! set its own.

use model2::enhance::TexFilter;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The settings a game is played with.
#[derive(Clone, Debug, PartialEq)]
pub struct GameSettings {
    /// Mix in dB: sound board (effects, announcer), music board.
    pub sfx_db: f32,
    pub music_db: f32,
    pub hud_edges: bool,
    /// Draw the game wider than its 4:3 frame to fill a wide window (else
    /// the 4:3 picture, with the bezel beside it if `bezel`).
    pub wide: bool,
    /// The game's bezel art beside a 4:3 picture in a wider window.
    pub bezel: bool,
    pub exact: bool,
    /// Smooth motion (`model2::smooth`; the native geometrizer only).
    pub smooth: bool,
    /// Frames shown ahead of the game (`model2::runahead`), 0 for none.
    pub run_ahead: u32,
    /// Anti-aliasing: samples a pixel (multisampling), 0 for none; 4 or 8,
    /// the GPU's own limit applying (docs/enhancements.md).
    pub aa: u32,
    /// Temporal anti-aliasing (`gpu::Quality::temporal`): the 3D layer's
    /// render scale in percent, 100 for TAA alone, 50 to 99 to upscale; 0
    /// for none. Needs the native geometrizer (motion data comes from
    /// smooth motion's recording); multisampling is off while it's on.
    pub temporal: u8,
    /// With temporal on: keep the games' checkerboard (screen door)
    /// translucency as a pattern, rather than an even 50% blend.
    pub stipple_keep: bool,
    /// Depth buffer for the 3D layer instead of the game's draw order.
    pub depth: bool,
    /// Smooth shading (`gpu::Quality::shading`, `model2::light`): flat
    /// (the game's own, one brightness a polygon), vertex or pixel (drawn
    /// as vertex until the exact lighting exists). Not forced off by the
    /// chip: it works from the polygon stream alone.
    pub shading: gpu::Shading,
    /// Smooth shading's crease angle in degrees (faces meeting at more
    /// keep a hard edge; 0: nothing smoothed), or `gpu::CREASE_AUTO`: the
    /// game's own (`romset::Shading::crease`). By flag or file only.
    pub crease: u8,
    /// Draw the game's texture pack (`<data dir>/<set>/texpack/`, or
    /// `--texture-pack`) over its own textures.
    pub pack: bool,
    /// Filtering of texture pack images: off, mipmaps, or mipmaps with
    /// anisotropic filtering (no effect without a pack).
    pub tex_filter: TexFilter,
    /// The geometrizer on its chip (the game's own program) rather than
    /// the native implementation. Applies when the game starts.
    pub geo_chip: bool,
}

impl Default for GameSettings {
    fn default() -> Self {
        GameSettings {
            sfx_db: model2::audio::DEFAULT_MIX.0,
            music_db: model2::audio::DEFAULT_MIX.1,
            hud_edges: true,
            wide: true,
            bezel: true,
            exact: false,
            smooth: false,
            run_ahead: 0,
            aa: 0,
            temporal: 0,
            stipple_keep: false,
            depth: false,
            shading: gpu::Shading::Flat,
            crease: gpu::CREASE_AUTO,
            pack: false,
            tex_filter: TexFilter::Off,
            geo_chip: false,
        }
    }
}

/// Every game setting's key, in the order the panels show them.
pub const KEYS: [&str; 17] = [
    "wide",
    "bezel",
    "hud_edges",
    "exact",
    "smooth",
    "run_ahead",
    "aa",
    "temporal",
    "stipple",
    "depth",
    "shading",
    "crease",
    "pack",
    "tex_filter",
    "sfx_db",
    "music_db",
    "geo",
];

/// The anti-aliasing levels (samples a pixel; 0 is off).
pub const AA_LEVELS: [u32; 3] = [0, 4, 8];

/// The texture filter settings (label, value), most anisotropy last.
const TEX_FILTERS: [(&str, &str); 6] =
    [("Off", "off"), ("Mip", "mip"), ("2x", "2"), ("4x", "4"), ("8x", "8"), ("16x", "16")];

/// The texture filter settings a GPU allowing anisotropic filtering up to
/// `max_anisotropy` times (`gpu::max_anisotropy`) can use.
pub fn tex_filters(max_anisotropy: u8) -> &'static [(&'static str, &'static str)] {
    let n = TEX_FILTERS.iter().take_while(|f| f.1.parse::<u8>().map_or(true, |n| n <= max_anisotropy)).count();
    &TEX_FILTERS[..n]
}

/// Most frames the player runs ahead.
pub const MAX_RUN_AHEAD: u32 = 3;

impl GameSettings {
    /// Setting `key` as text (as the files keep it).
    pub fn get(&self, key: &str) -> String {
        match key {
            "sfx_db" => self.sfx_db.to_string(),
            "music_db" => self.music_db.to_string(),
            "hud_edges" => self.hud_edges.to_string(),
            "wide" => self.wide.to_string(),
            "bezel" => self.bezel.to_string(),
            "exact" => self.exact.to_string(),
            "smooth" => self.smooth.to_string(),
            "run_ahead" => self.run_ahead.to_string(),
            "aa" => self.aa.to_string(),
            "temporal" => match self.temporal {
                0 => "off".to_string(),
                100 => "taa".to_string(),
                n => n.to_string(),
            },
            "stipple" => if self.stipple_keep { "keep" } else { "blend" }.to_string(),
            "depth" => self.depth.to_string(),
            "shading" => self.shading.name().to_string(),
            "crease" => match self.crease {
                gpu::CREASE_AUTO => "auto".to_string(),
                n => n.to_string(),
            },
            "pack" => self.pack.to_string(),
            "tex_filter" => self.tex_filter.to_string(),
            "geo" => if self.geo_chip { "chip" } else { "native" }.to_string(),
            _ => String::new(),
        }
    }

    /// Set `key` from text; false (and unchanged) if it isn't a setting or
    /// the value doesn't parse.
    pub fn set(&mut self, key: &str, v: &str) -> bool {
        let flag = |v: &str| match v {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        };
        let db = |v: &str| v.parse::<f32>().ok().filter(|x| x.is_finite()).map(|x| x.clamp(-40.0, 24.0));
        match key {
            "sfx_db" => db(v).map(|x| self.sfx_db = x).is_some(),
            "music_db" => db(v).map(|x| self.music_db = x).is_some(),
            "hud_edges" => flag(v).map(|x| self.hud_edges = x).is_some(),
            "wide" => flag(v).map(|x| self.wide = x).is_some(),
            "bezel" => flag(v).map(|x| self.bezel = x).is_some(),
            "exact" => flag(v).map(|x| self.exact = x).is_some(),
            "smooth" => flag(v).map(|x| self.smooth = x).is_some(),
            "run_ahead" => v.parse::<u32>().ok().filter(|n| *n <= MAX_RUN_AHEAD).map(|n| self.run_ahead = n).is_some(),
            "aa" => v.parse::<u32>().ok().filter(|n| AA_LEVELS.contains(n)).map(|n| self.aa = n).is_some(),
            "temporal" => gpu::Quality::parse_temporal(v).map(|n| self.temporal = n).is_some(),
            "stipple" => gpu::Stipple::parse(v).map(|s| self.stipple_keep = s == gpu::Stipple::Keep).is_some(),
            "depth" => flag(v).map(|x| self.depth = x).is_some(),
            "shading" => gpu::Shading::parse(v).map(|x| self.shading = x).is_some(),
            "crease" => model2::light::parse_crease(v).map(|n| self.crease = n).is_some(),
            "pack" => flag(v).map(|x| self.pack = x).is_some(),
            "tex_filter" => TexFilter::parse(v).map(|f| self.tex_filter = f).is_some(),
            "geo" => match model2::geo::GeoMode::parse(v) {
                Some(model2::geo::GeoMode::Chip) => {
                    self.geo_chip = true;
                    true
                }
                Some(model2::geo::GeoMode::Native) => {
                    self.geo_chip = false;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    pub fn geo_mode(&self) -> model2::geo::GeoMode {
        if self.geo_chip {
            model2::geo::GeoMode::Chip
        } else {
            model2::geo::GeoMode::Native
        }
    }

    /// Smooth motion as it runs: it needs the native geometrizer.
    pub fn smooth_on(&self) -> bool {
        self.smooth && !self.geo_chip
    }

    /// Temporal anti-aliasing as it runs: it needs the native geometrizer
    /// (its motion data is smooth motion's). The GPU may still not have it
    /// (`gpu::temporal_supported`).
    pub fn temporal_on(&self) -> bool {
        self.temporal > 0 && !self.geo_chip
    }

    /// Smooth motion's recording: for its in-betweens, or for temporal
    /// anti-aliasing's motion data (in-betweens are drawn only with
    /// `smooth_on`).
    pub fn motion_on(&self) -> bool {
        self.smooth_on() || self.temporal_on()
    }

    /// Smooth shading's side channel (`Geometrizer::set_light`): wanted
    /// whenever shading can light anything (the game's own lighting per
    /// polygon for the exact layer, block facts with the chip).
    pub fn light_on(&self) -> bool {
        self.shading != gpu::Shading::Flat && self.crease != 0
    }

    /// The renderer's quality for these settings (`supported`: the GPU
    /// has temporal anti-aliasing). Multisampling is asked for as set; the
    /// renderer turns it off while temporal is on.
    pub fn quality(&self, supported: bool) -> gpu::Quality {
        gpu::Quality {
            msaa: self.aa,
            depth: self.depth,
            tex_filter: self.tex_filter,
            temporal: if self.temporal_on() && supported { self.temporal } else { 0 },
            stipple: if self.stipple_keep { gpu::Stipple::Keep } else { gpu::Stipple::Blend },
            shading: self.shading,
            crease: self.crease,
        }
    }
}

/// One layer of game settings: the keys it sets, as text.
pub type Layer = BTreeMap<String, String>;

/// Where a game setting's value came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Default,
    AllGames,
    Game,
    CommandLine,
}

/// The settings kept between runs (`play-settings.txt`) and this run's
/// command-line overrides.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub rom: PathBuf,
    /// The last game played.
    pub game: String,
    /// Games played, most recent first.
    pub recent: Vec<String>,
    pub fullscreen: bool,
    /// The launcher's settings panel saves for all games (else for the
    /// game shown).
    pub for_all: bool,
    /// The defaults for all games.
    pub defaults: Layer,
    /// Each game's own settings, read when first needed.
    games: BTreeMap<String, Layer>,
    /// The command line's, for this run only (never saved).
    pub cli: Layer,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rom: default_rom(),
            game: "topskatr".into(),
            recent: Vec::new(),
            fullscreen: false,
            for_all: true,
            defaults: Layer::new(),
            games: BTreeMap::new(),
            cli: Layer::new(),
        }
    }
}

/// Most games kept in `Settings::recent`.
const RECENT: usize = 32;

/// The ROM folder until one is chosen: `rom` in the source tree (or
/// wherever there's a `rom/` folder), else `roms` beside the settings.
fn default_rom() -> PathBuf {
    if Path::new("rom").is_dir() || Path::new("out").is_dir() {
        PathBuf::from("rom")
    } else {
        data_dir().join("roms")
    }
}

/// Where saves, EEPROMs and settings go.
pub fn data_dir() -> PathBuf {
    if Path::new("out").is_dir() {
        return PathBuf::from("out");
    }
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("remodel")
}

fn path() -> PathBuf {
    data_dir().join("play-settings.txt")
}

fn game_path(set: &str) -> PathBuf {
    data_dir().join(set).join("settings.txt")
}

/// `key = value` lines (blank lines and `#` comments skipped).
fn lines(text: &str) -> impl Iterator<Item = (&str, &str)> {
    text.lines()
        .filter(|l| !l.trim_start().starts_with('#'))
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.trim(), v.trim()))
}

/// The game settings among `lines` that parse.
fn layer<'a>(lines: impl Iterator<Item = (&'a str, &'a str)>) -> Layer {
    let mut out = Layer::new();
    for (k, v) in lines {
        if GameSettings::default().set(k, v) {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

fn write(path: &Path, text: &str) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(path, text);
}

impl Settings {
    /// Saved settings, or the defaults.
    pub fn load() -> Settings {
        Settings::parse(&std::fs::read_to_string(path()).unwrap_or_default())
    }

    fn parse(text: &str) -> Settings {
        let mut s = Settings::default();
        for (k, v) in lines(text) {
            match k {
                "rom" => s.rom = PathBuf::from(v),
                "game" => s.game = v.to_string(),
                "recent" => {
                    s.recent = v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect()
                }
                "fullscreen" => s.fullscreen = v == "true",
                "scope" => s.for_all = v != "game",
                _ => {}
            }
        }
        // Game settings at the top level are the defaults for all games
        // (as every setting was before games had their own).
        s.defaults = layer(lines(text));
        s
    }

    fn text(&self) -> String {
        let mut text = format!(
            "# remodel-play settings\nrom = {}\ngame = {}\nrecent = {}\nfullscreen = {}\nscope = {}\n",
            self.rom.display(),
            self.game,
            self.recent.join(","),
            self.fullscreen,
            if self.for_all { "all" } else { "game" },
        );
        if !self.defaults.is_empty() {
            text += "# the defaults for all games (a game's own are in <set>/settings.txt)\n";
        }
        for (k, v) in &self.defaults {
            text += &format!("{k} = {v}\n");
        }
        text
    }

    /// Keep the settings (not the command line's).
    pub fn save(&self) {
        write(&path(), &self.text());
    }

    /// Note `set` as played just now.
    pub fn played(&mut self, set: &str) {
        self.game = set.to_string();
        self.recent.retain(|s| s != set);
        self.recent.insert(0, set.to_string());
        self.recent.truncate(RECENT);
    }

    /// `set`'s own settings layer (read from its file the first time).
    fn game_layer(&mut self, set: &str) -> &mut Layer {
        self.games
            .entry(set.to_string())
            .or_insert_with(|| layer(lines(&std::fs::read_to_string(game_path(set)).unwrap_or_default())))
    }

    /// Whether `set` has settings of its own.
    pub fn has_own(&mut self, set: &str) -> bool {
        !self.game_layer(set).is_empty()
    }

    /// The settings `set` plays with, and where each came from (by
    /// `KEYS` order).
    pub fn resolve(&mut self, set: &str) -> (GameSettings, [Source; KEYS.len()]) {
        let mut v = GameSettings::default();
        let mut source = [Source::Default; KEYS.len()];
        let own = self.game_layer(set).clone();
        for (layer, name) in
            [(&self.defaults, Source::AllGames), (&own, Source::Game), (&self.cli, Source::CommandLine)]
        {
            for (i, k) in KEYS.iter().enumerate() {
                if let Some(x) = layer.get(*k) {
                    if v.set(k, x) {
                        source[i] = name;
                    }
                }
            }
        }
        (v, source)
    }

    /// Keep what changed from what `set` resolves to in `new`: for all
    /// games (removing the game's own copy) or for `set` only, as
    /// `for_all` says. A changed setting the command line gave is the
    /// player's from then on. Saves what changed.
    pub fn edit(&mut self, set: &str, new: &GameSettings) {
        let (all, game) = self.apply_edit(set, new);
        if all {
            self.save();
        }
        if game {
            self.save_game(set);
        }
    }

    /// [`Settings::edit`] without saving: whether the defaults for all
    /// games and the game's own changed.
    fn apply_edit(&mut self, set: &str, new: &GameSettings) -> (bool, bool) {
        let (old, _) = self.resolve(set);
        let base = GameSettings::default();
        let (mut all_changed, mut game_changed) = (false, false);
        for k in KEYS {
            let v = new.get(k);
            if v == old.get(k) {
                continue;
            }
            self.cli.remove(k);
            if self.for_all {
                if v == base.get(k) {
                    self.defaults.remove(k);
                } else {
                    self.defaults.insert(k.to_string(), v);
                }
                all_changed = true;
                game_changed |= self.game_layer(set).remove(k).is_some();
            } else {
                let under = self.defaults.get(k).cloned().unwrap_or_else(|| base.get(k));
                let own = self.game_layer(set);
                if v == under {
                    own.remove(k);
                } else {
                    own.insert(k.to_string(), v);
                }
                game_changed = true;
            }
        }
        (all_changed, game_changed)
    }

    /// Drop `set`'s own settings: it plays with the defaults for all games.
    pub fn reset_game(&mut self, set: &str) {
        self.game_layer(set).clear();
        self.save_game(set);
    }

    fn save_game(&mut self, set: &str) {
        let own = self.game_layer(set).clone();
        let p = game_path(set);
        if own.is_empty() {
            let _ = std::fs::remove_file(p);
            return;
        }
        let mut text = format!("# remodel-play settings for {set} (only where it differs from all games')\n");
        for (k, v) in own {
            text += &format!("{k} = {v}\n");
        }
        write(&p, &text);
    }
}

/// Where a game's remapped controls are kept: only what differs from its
/// manifest (`romset::bindings`), in `<data dir>/<set>/controls.txt`.
fn bindings_path(game: &romset::Game) -> PathBuf {
    data_dir().join(game.id).join("controls.txt")
}

/// The game's bindings: its manifest's, with the player's changes.
pub fn load_bindings(game: &'static romset::Game) -> romset::Bindings {
    let text = std::fs::read_to_string(bindings_path(game)).unwrap_or_default();
    romset::Bindings::with_overrides(&game.controls, &text, romset::bindings::RESERVED_KEYS)
}

/// Keep the player's changes (removing the file when there are none).
pub fn save_bindings(game: &romset::Game, b: &romset::Bindings) {
    let p = bindings_path(game);
    let text = b.overrides();
    if text.is_empty() {
        let _ = std::fs::remove_file(p);
        return;
    }
    write(&p, &format!("# remodel-play controls for {} (only changes from its defaults)\n{text}", game.id));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Settings with `set`'s own layer given (not read from disk).
    fn with_game(mut s: Settings, set: &str, own: &[(&str, &str)]) -> Settings {
        s.games.insert(set.into(), own.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect());
        s
    }

    #[test]
    fn old_files_become_the_defaults_for_all_games() {
        let s = Settings::parse(
            "# remodel-play settings\nrom = rom\ngame = hotd\nsfx_db = 4\nwide = false\nfullscreen = true\n",
        );
        assert_eq!(s.game, "hotd");
        assert!(s.fullscreen && s.for_all);
        assert_eq!(s.defaults.get("sfx_db").map(String::as_str), Some("4"));
        assert_eq!(s.defaults.get("wide").map(String::as_str), Some("false"));
        assert!(!s.defaults.contains_key("rom") && !s.defaults.contains_key("fullscreen"));
        // And back: the same text.
        let again = Settings::parse(&s.text());
        assert_eq!((again.defaults, again.game, again.fullscreen), (s.defaults, s.game, s.fullscreen));
    }

    #[test]
    fn layers_in_order() {
        let mut s = Settings::parse("sfx_db = 4\nmusic_db = -2\nbezel = false\n");
        s = with_game(s, "hotd", &[("sfx_db", "6"), ("wide", "false"), ("smooth", "nonsense")]);
        s.cli.insert("music_db".into(), "-12".into());
        let (v, src) = s.resolve("hotd");
        assert_eq!((v.sfx_db, v.music_db, v.bezel, v.wide, v.smooth), (6.0, -12.0, false, false, false));
        let at = |k: &str| src[KEYS.iter().position(|x| *x == k).unwrap()];
        assert_eq!(at("sfx_db"), Source::Game);
        assert_eq!(at("music_db"), Source::CommandLine);
        assert_eq!(at("bezel"), Source::AllGames);
        assert_eq!(at("smooth"), Source::Default);
        // Another game: no own layer (its file isn't there in a test run's
        // directory, or empty), the rest the same.
        s.games.insert("stcc".into(), Layer::new());
        let (v, _) = s.resolve("stcc");
        assert_eq!((v.sfx_db, v.music_db, v.wide), (4.0, -12.0, true));
    }

    #[test]
    fn edits_go_to_the_chosen_layer() {
        let mut s = with_game(Settings::parse("sfx_db = 4\n"), "hotd", &[("exact", "false")]);
        s.games.insert("stcc".into(), Layer::new());
        s.cli.insert("bezel".into(), "false".into());
        let (mut v, _) = s.resolve("hotd");
        v.smooth = true;
        s.for_all = false;
        edit_quietly(&mut s, "hotd", &v);
        assert_eq!(s.games["hotd"].get("smooth").map(String::as_str), Some("true"));
        assert!(!s.defaults.contains_key("smooth"));
        // For all games: the game's own copy goes, and back to the built-in
        // value means not stored at all.
        s.for_all = true;
        let (mut v, _) = s.resolve("hotd");
        v.exact = true;
        v.sfx_db = model2::audio::DEFAULT_MIX.0;
        edit_quietly(&mut s, "hotd", &v);
        assert_eq!(s.defaults.get("exact").map(String::as_str), Some("true"));
        assert!(!s.games["hotd"].contains_key("exact"));
        assert!(!s.defaults.contains_key("sfx_db"));
        assert!(s.resolve("stcc").0.exact);
        // A command-line value, changed, is the player's from then on.
        let (mut v, _) = s.resolve("stcc");
        assert!(!v.bezel);
        v.bezel = true;
        edit_quietly(&mut s, "stcc", &v);
        assert!(!s.cli.contains_key("bezel"));
        assert!(s.resolve("stcc").0.bezel);
        // Unchanged settings stay where they were (the command line's too).
        s.cli.insert("exact".into(), "true".into());
        let (v, _) = s.resolve("stcc");
        edit_quietly(&mut s, "stcc", &v);
        assert!(s.cli.contains_key("exact"));
    }

    #[test]
    fn run_ahead_is_off_or_up_to_three_frames() {
        let mut v = GameSettings::default();
        assert_eq!(v.run_ahead, 0);
        assert!(v.set("run_ahead", "2") && v.run_ahead == 2);
        assert_eq!(v.get("run_ahead"), "2");
        assert!(!v.set("run_ahead", "4") && !v.set("run_ahead", "on"));
        assert_eq!(v.run_ahead, 2);
    }

    #[test]
    fn pack_and_filter() {
        let mut v = GameSettings::default();
        assert!(!v.pack && v.tex_filter == TexFilter::Off);
        assert!(v.set("pack", "true") && v.pack);
        for f in ["off", "mip", "2", "4", "8", "16"] {
            assert!(v.set("tex_filter", f));
            assert_eq!(v.get("tex_filter"), f);
        }
        assert!(!v.set("tex_filter", "3") && v.tex_filter == TexFilter::Aniso(16));
        // Through the files.
        let s = Settings::parse(&with_game(Settings::parse("tex_filter = mip\npack = true\n"), "x", &[]).text());
        assert_eq!(s.defaults.get("tex_filter").map(String::as_str), Some("mip"));
        assert_eq!(s.defaults.get("pack").map(String::as_str), Some("true"));
        // Choices: anisotropy only where the GPU has it.
        assert_eq!(tex_filters(1).iter().map(|f| f.1).collect::<Vec<_>>(), ["off", "mip"]);
        assert_eq!(tex_filters(16).len(), 6);
        assert_eq!(tex_filters(4).last().unwrap().1, "4");
    }

    #[test]
    fn temporal_and_stipple() {
        let mut v = GameSettings::default();
        assert_eq!((v.temporal, v.stipple_keep), (0, false));
        assert_eq!((v.get("temporal").as_str(), v.get("stipple").as_str()), ("off", "blend"));
        // Off: the quality is what it was before temporal existed.
        let q = v.quality(true);
        assert_eq!(q, gpu::Quality { msaa: 0, depth: false, tex_filter: TexFilter::Off, ..Default::default() });
        assert!(!v.motion_on());
        for (text, n) in [("taa", 100), ("75", 75), ("67", 67), ("50", 50), ("off", 0)] {
            assert!(v.set("temporal", text));
            assert_eq!((v.temporal, v.get("temporal")), (n, text.to_string()));
        }
        assert!(!v.set("temporal", "49") && !v.set("temporal", "101") && !v.set("temporal", "x"));
        assert!(v.set("stipple", "keep") && v.stipple_keep && v.get("stipple") == "keep");
        assert!(!v.set("stipple", "on") && v.stipple_keep);
        // On: smooth motion's recording too, not its in-betweens.
        v.set("temporal", "67");
        assert!(v.temporal_on() && v.motion_on() && !v.smooth_on());
        assert_eq!(v.quality(true).temporal, 67);
        assert_eq!(v.quality(true).stipple, gpu::Stipple::Keep);
        assert_eq!(v.quality(false).temporal, 0);
        // Not with the chip.
        v.geo_chip = true;
        assert!(!v.temporal_on() && !v.motion_on());
        assert_eq!(v.quality(true).temporal, 0);
        // Through the files; unset, nothing is written.
        assert!(Settings::default().text().find("temporal").is_none());
        let s = Settings::parse("temporal = 67\nstipple = keep\n");
        assert_eq!(s.defaults.get("temporal").map(String::as_str), Some("67"));
        let s = Settings::parse(&s.text());
        let (v, _) = with_game(s, "x", &[]).resolve("x");
        assert_eq!((v.temporal, v.stipple_keep), (67, true));
    }

    #[test]
    fn shading_and_crease() {
        let mut v = GameSettings::default();
        assert_eq!((v.shading, v.crease), (gpu::Shading::Flat, gpu::CREASE_AUTO));
        assert_eq!((v.get("shading").as_str(), v.get("crease").as_str()), ("flat", "auto"));
        // Flat: the quality is what it was before shading existed (the
        // renderer then never runs `light::shade`).
        let q = v.quality(true);
        assert_eq!(q, gpu::Quality { msaa: 0, depth: false, tex_filter: TexFilter::Off, ..Default::default() });
        for s in ["vertex", "pixel", "flat"] {
            assert!(v.set("shading", s) && v.get("shading") == s);
        }
        assert!(v.set("shading", "off") && v.shading == gpu::Shading::Flat && v.get("shading") == "flat");
        assert!(!v.set("shading", "smooth") && !v.set("shading", "on"));
        for (text, n) in [("0", 0), ("40", 40), ("180", 180), ("auto", gpu::CREASE_AUTO)] {
            assert!(v.set("crease", text));
            assert_eq!((v.crease, v.get("crease")), (n, text.to_string()));
        }
        assert!(!v.set("crease", "181") && !v.set("crease", "-1") && !v.set("crease", "x"));
        assert_eq!(v.crease, gpu::CREASE_AUTO);
        // Into the renderer's quality, the chip included (not forced off),
        // and alongside temporal.
        v.set("shading", "vertex");
        v.set("crease", "30");
        v.set("temporal", "taa");
        v.geo_chip = true;
        let q = v.quality(true);
        assert_eq!((q.shading, q.crease, q.temporal), (gpu::Shading::Vertex, 30, 0));
        v.geo_chip = false;
        assert_eq!(v.quality(true).temporal, 100);
        assert_eq!(v.quality(true).shading, gpu::Shading::Vertex);
        // Through the files; unset, nothing is written.
        assert!(Settings::default().text().find("shading").is_none());
        let s = Settings::parse("shading = pixel\ncrease = 25\n");
        assert_eq!(s.defaults.get("shading").map(String::as_str), Some("pixel"));
        let s = Settings::parse(&s.text());
        let (v, _) = with_game(s, "x", &[("crease", "auto")]).resolve("x");
        assert_eq!((v.shading, v.crease), (gpu::Shading::Pixel, gpu::CREASE_AUTO));
        // Back to the defaults: nothing stored.
        let mut s = with_game(Settings::parse("shading = vertex\ncrease = 25\n"), "x", &[]);
        let (mut v, _) = s.resolve("x");
        (v.shading, v.crease) = (gpu::Shading::Flat, gpu::CREASE_AUTO);
        edit_quietly(&mut s, "x", &v);
        assert!(!s.defaults.contains_key("shading") && !s.defaults.contains_key("crease"));
    }

    /// `edit` without saving (the layers it would touch are given).
    fn edit_quietly(s: &mut Settings, set: &str, v: &GameSettings) {
        s.apply_edit(set, v);
    }

    #[test]
    fn recent_first() {
        let mut s = Settings::default();
        for g in ["a", "b", "a", "c"] {
            s.played(g);
        }
        assert_eq!(s.recent, ["c", "a", "b"]);
        assert_eq!(Settings::parse(&s.text()).recent, s.recent);
    }
}
