//! Settings kept between runs, and where the player keeps its files.
//!
//! Run from the reModel source tree (an `out/` directory in the current
//! directory), files go under `out/` next to the other per-game output;
//! otherwise in the platform's data folder: `$XDG_DATA_HOME/remodel`
//! (`~/.local/share/remodel`) on Linux, `~/Library/Application
//! Support/remodel` on macOS, `%APPDATA%\remodel` on Windows.
//!
//! A game's settings ([`GameSettings`]: sound mix, display, speed, smooth
//! motion, run-ahead, anti-aliasing, temporal anti-aliasing and its
//! stipple, depth buffer, smooth shading, its crease angle and finer
//! levels, texture
//! pack and its filtering, material maps and their rule, geometrizer, and
//! the picture preset that sets many of them) come in layers, lowest first, as
//! in the browser build (`crates/web/settings.js`): the built-in defaults, the defaults for
//! all games (`play-settings.txt`), the game's own (`<set>/settings.txt`)
//! and the command line (this run only). A layer keeps only what it sets,
//! so changing the defaults for all games reaches every game that hasn't
//! set its own.

use model2::enhance::TexFilter;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The picture presets: a choice of the rows that change how the game
/// looks (and, with Enhanced, moves). Custom is any other combination.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Preset {
    /// Everything that changes the picture off: the arcade look.
    #[default]
    Original,
    /// 4x multisampling and mipmapped texture filtering only.
    Sharp,
    /// Per game (`romset::Recommended`): what its documentation says is
    /// right for it, with smooth motion and run-ahead.
    Enhanced,
    Custom,
}

impl Preset {
    /// Every preset, in the order the menu offers them.
    pub const ALL: [Preset; 4] = [Preset::Original, Preset::Sharp, Preset::Enhanced, Preset::Custom];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Original => "original",
            Preset::Sharp => "sharp",
            Preset::Enhanced => "enhanced",
            Preset::Custom => "custom",
        }
    }

    pub fn parse(s: &str) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.name() == s)
    }
}

/// The rows a preset describes the picture by: a preset matches when all
/// of them are what it sets. Changing one of them under a preset makes it
/// Custom.
pub const PICTURE_KEYS: [&str; 13] = [
    "aa",
    "temporal",
    "stipple",
    "depth",
    "shading",
    "crease",
    "shading_fine",
    "shading_balance",
    "pack",
    "tex_filter",
    "materials",
    "materials_relief",
    "bloom",
];

/// The rows choosing a preset sets: the picture's, smooth motion and
/// run-ahead (the last two don't count against a preset matching).
const PRESET_KEYS: [&str; 15] = [
    "aa",
    "temporal",
    "stipple",
    "depth",
    "shading",
    "crease",
    "shading_fine",
    "shading_balance",
    "pack",
    "tex_filter",
    "materials",
    "materials_relief",
    "bloom",
    "smooth",
    "run_ahead",
];

/// The settings a game is played with.
#[derive(Clone, Debug, PartialEq)]
pub struct GameSettings {
    /// The picture preset. The rows it sets are kept in the fields below;
    /// a preset is stored as itself, not as its rows (`Settings::edit`).
    pub preset: Preset,
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
    /// as vertex with the chip geometrizer). Not forced off by the chip:
    /// it works from the polygon stream alone.
    pub shading: gpu::Shading,
    /// Smooth shading's crease angle in degrees (faces meeting at more
    /// keep a hard edge; 0: nothing smoothed), or `gpu::CREASE_AUTO`: the
    /// game's own (`romset::Shading::crease`). By flag or file only.
    pub crease: u8,
    /// Smooth shading's finer levels (`gpu::Quality::shading_fine`): a lit
    /// pixel blends the two colour columns around its brightness instead
    /// of the game's 64 levels (no banding on large smooth surfaces).
    pub shading_fine: bool,
    /// Smooth shading's brightness balance (`gpu::Quality::shading_balance`,
    /// on by default): one brightness offset per smooth surface keeps its
    /// mean the game's; off, bumpy surfaces come out brighter.
    pub shading_balance: bool,
    /// Draw the game's texture pack (`<data dir>/<set>/texpack/`, or
    /// `--texture-pack`) over its own textures.
    pub pack: bool,
    /// Filtering of texture pack images: off, mipmaps, or mipmaps with
    /// anisotropic filtering (no effect without a pack).
    pub tex_filter: TexFilter,
    /// Material maps (`gpu::Quality::materials`, off by default): the
    /// pack's normal maps change the brightness of surfaces the game lit
    /// with the exact lighting layer. Needs the texture pack on (and one
    /// with maps) and the native geometrizer; independent of `shading`.
    pub materials: bool,
    /// How material maps change brightness (`gpu::Relief`): relief at a
    /// strength (2 by default), or the faithful rule (`off`).
    pub materials_relief: gpu::Relief,
    /// Bloom (`gpu::Quality::bloom`): strength in percent, 0 for none. The
    /// bright parts of the 3D layer glow; GPU only, never part of a preset
    /// but Original's "off".
    pub bloom: u8,
    /// The geometrizer on its chip (the game's own program) rather than
    /// the native implementation. Applies when the game starts.
    pub geo_chip: bool,
}

impl Default for GameSettings {
    fn default() -> Self {
        GameSettings {
            preset: Preset::Original,
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
            shading_fine: false,
            shading_balance: true,
            pack: false,
            tex_filter: TexFilter::Off,
            materials: false,
            materials_relief: gpu::Relief::default(),
            bloom: 0,
            geo_chip: false,
        }
    }
}

/// Every game setting's key, in the order the panels show them.
pub const KEYS: [&str; 23] = [
    "preset",
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
    "shading_fine",
    "shading_balance",
    "pack",
    "tex_filter",
    "materials",
    "materials_relief",
    "bloom",
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
            "preset" => self.preset.name().to_string(),
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
            "shading_fine" => self.shading_fine.to_string(),
            "shading_balance" => self.shading_balance.to_string(),
            "pack" => self.pack.to_string(),
            "tex_filter" => self.tex_filter.to_string(),
            "materials" => self.materials.to_string(),
            "materials_relief" => self.materials_relief.name(),
            "bloom" => match self.bloom {
                0 => "off".to_string(),
                n => n.to_string(),
            },
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
            "preset" => Preset::parse(v).map(|p| self.preset = p).is_some(),
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
            "shading_fine" => flag(v).map(|x| self.shading_fine = x).is_some(),
            "shading_balance" => flag(v).map(|x| self.shading_balance = x).is_some(),
            "pack" => flag(v).map(|x| self.pack = x).is_some(),
            "tex_filter" => TexFilter::parse(v).map(|f| self.tex_filter = f).is_some(),
            "materials" => flag(v).map(|x| self.materials = x).is_some(),
            "materials_relief" => gpu::Relief::parse(v).map(|r| self.materials_relief = r).is_some(),
            "bloom" => gpu::Quality::parse_bloom(v).map(|n| self.bloom = n).is_some(),
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

    /// Choose preset `p`: set the rows it describes (all of them, smooth
    /// motion and run-ahead too) for `game` (Enhanced is per game). Custom
    /// keeps the rows as they are.
    pub fn apply_preset(&mut self, p: Preset, game: Option<&romset::Game>) {
        self.preset = p;
        if p == Preset::Custom {
            return;
        }
        let base = GameSettings::default();
        for k in PRESET_KEYS {
            self.set(k, &base.get(k));
        }
        if p == Preset::Original {
            return;
        }
        self.aa = 4;
        self.tex_filter = TexFilter::Mip;
        if p == Preset::Enhanced {
            let rec = game.map_or(romset::Recommended::DEFAULT, |g| g.recommended);
            self.temporal = if rec.temporal { 100 } else { 0 };
            self.depth = rec.depth;
            self.shading = if rec.shading { gpu::Shading::Vertex } else { gpu::Shading::Flat };
            self.smooth = true;
            self.run_ahead = game.map_or(0, |g| g.run_ahead_recommended() as u32);
        }
    }

    /// Whether the picture rows are what preset `p` sets for `game`.
    pub fn picture_is(&self, p: Preset, game: Option<&romset::Game>) -> bool {
        let mut want = self.clone();
        want.apply_preset(p, game);
        PICTURE_KEYS.iter().all(|k| want.get(k) == self.get(k))
    }

    /// These settings, with a preset whose rows no longer all match
    /// changed to Custom (the rows win).
    pub fn settled(mut self, game: Option<&romset::Game>) -> GameSettings {
        if self.preset != Preset::Custom && !self.picture_is(self.preset, game) {
            self.preset = Preset::Custom;
        }
        self
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
        gpu::Quality::shading_lit(self.shading, self.crease)
    }

    /// Material maps as asked for: on, with the texture pack on. Whether
    /// they show depends on the pack having maps and on the exact
    /// lighting layer (the native geometrizer), which the renderer finds
    /// out frame by frame (`gpu::PackStats::lamped`).
    pub fn materials_on(&self) -> bool {
        self.materials && self.pack
    }

    /// The geometrizer's lighting side channel (`Geometrizer::set_light`):
    /// for smooth shading, or for material maps (their lamps come from it,
    /// with shading flat too).
    pub fn side_channel_on(&self) -> bool {
        self.light_on() || self.materials_on()
    }

    /// The renderer's quality for these settings (`supported`: the GPU
    /// has temporal anti-aliasing's 16-bit float targets, which bloom
    /// needs too). Multisampling is asked for as set; the
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
            shading_fine: self.shading_fine,
            shading_balance: self.shading_balance,
            materials: self.materials_on(),
            materials_relief: self.materials_relief,
            bloom: if supported { self.bloom } else { 0 },
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
    // XDG_DATA_HOME, else ~/.local/share, on Linux; Application Support on
    // macOS; the roaming AppData on Windows.
    dirs::data_dir().unwrap_or_else(|| PathBuf::from(".")).join("remodel")
}

fn path() -> PathBuf {
    data_dir().join("play-settings.txt")
}

/// For the log: the settings `set` plays with that aren't the built-in
/// defaults, each with where it came from, and the files read.
pub fn describe(set: &str, v: &GameSettings, sources: &[Source; KEYS.len()]) -> String {
    let from = |src: Source| match src {
        Source::Default => "default",
        Source::AllGames => "all games",
        Source::Game => "this game",
        Source::CommandLine => "command line",
    };
    let set_here: Vec<String> = KEYS
        .iter()
        .zip(sources)
        .filter(|(_, src)| **src != Source::Default)
        .map(|(k, src)| format!("{k} {} ({})", v.get(k), from(*src)))
        .collect();
    let list =
        if set_here.is_empty() { "all defaults".to_string() } else { set_here.join(", ") + "; the rest default" };
    let file = |p: PathBuf| format!("{}{}", p.display(), if p.exists() { "" } else { " (none)" });
    format!("{set}: {list}; files {}, {}", file(path()), file(game_path(set)))
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

/// The game settings among `lines` (read from `file`) that parse. A
/// setting whose value doesn't is left out, so it takes its default, with
/// a warning.
fn layer<'a>(file: &Path, lines: impl Iterator<Item = (&'a str, &'a str)>) -> Layer {
    let mut out = Layer::new();
    for (k, v) in lines {
        if GameSettings::default().set(k, v) {
            out.insert(k.to_string(), v.to_string());
        } else if KEYS.contains(&k) {
            eprintln!("warning: {}: `{k} = {v}` is not a value {k} takes; ignored", file.display());
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
        s.defaults = layer(&path(), lines(text));
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
        self.games.entry(set.to_string()).or_insert_with(|| {
            layer(&game_path(set), lines(&std::fs::read_to_string(game_path(set)).unwrap_or_default()))
        })
    }

    /// Whether `set` has settings of its own.
    pub fn has_own(&mut self, set: &str) -> bool {
        !self.game_layer(set).is_empty()
    }

    /// The settings `set` plays with, and where each came from (by
    /// `KEYS` order).
    pub fn resolve(&mut self, set: &str) -> (GameSettings, [Source; KEYS.len()]) {
        let own = self.game_layer(set).clone();
        Self::stack(&[(&self.defaults, Source::AllGames), (&own, Source::Game), (&self.cli, Source::CommandLine)], set)
    }

    /// `layers` (lowest first) over the built-in defaults. A layer's
    /// preset sets the rows it describes before the layer's own settings
    /// apply, so a row given beside it wins; a preset whose rows then
    /// don't all match becomes Custom.
    fn stack(layers: &[(&Layer, Source)], set: &str) -> (GameSettings, [Source; KEYS.len()]) {
        let game = romset::games::find(set).ok();
        let mut v = GameSettings::default();
        let mut source = [Source::Default; KEYS.len()];
        for (layer, name) in layers {
            if let Some(p) = layer.get("preset").and_then(|p| Preset::parse(p)) {
                v.apply_preset(p, game);
                for (i, k) in KEYS.iter().enumerate() {
                    if *k == "preset" || (p != Preset::Custom && PRESET_KEYS.contains(k)) {
                        source[i] = *name;
                    }
                }
            }
            for (i, k) in KEYS.iter().enumerate() {
                if let Some(x) = layer.get(*k).filter(|_| *k != "preset") {
                    if v.set(k, x) {
                        source[i] = *name;
                    }
                }
            }
        }
        (v.settled(game), source)
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
    ///
    /// A preset other than Custom is kept as itself: the rows it sets are
    /// stored only where they differ from what it sets (smooth motion and
    /// run-ahead changed after choosing it). Custom keeps its rows, each
    /// where it differs from the layers below.
    fn apply_edit(&mut self, set: &str, new: &GameSettings) -> (bool, bool) {
        let game = romset::games::find(set).ok();
        let new = new.clone().settled(game);
        let (old, _) = self.resolve(set);
        let under = if self.for_all {
            GameSettings::default()
        } else {
            let all = self.defaults.clone();
            Self::stack(&[(&all, Source::AllGames)], set).0
        };
        let mut derived = GameSettings::default();
        derived.apply_preset(new.preset, game);
        let changed_preset = new.preset != old.preset;
        let (all_before, own_before) = (self.defaults.clone(), self.game_layer(set).clone());
        for k in KEYS {
            // A new preset sets its rows again, whatever they were.
            let forced = changed_preset && (k == "preset" || PRESET_KEYS.contains(&k));
            let v = new.get(k);
            if v == old.get(k) && !forced {
                continue;
            }
            self.cli.remove(k);
            let baseline =
                if new.preset != Preset::Custom && PRESET_KEYS.contains(&k) { derived.get(k) } else { under.get(k) };
            if self.for_all {
                if v == baseline {
                    self.defaults.remove(k);
                } else {
                    self.defaults.insert(k.to_string(), v);
                }
                self.game_layer(set).remove(k);
            } else {
                let own = self.game_layer(set);
                if v == baseline {
                    own.remove(k);
                } else {
                    own.insert(k.to_string(), v);
                }
            }
        }
        (self.defaults != all_before, *self.game_layer(set) != own_before)
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
    fn bloom_is_off_and_not_a_preset_row() {
        let mut v = GameSettings::default();
        assert_eq!((v.bloom, v.get("bloom").as_str()), (0, "off"));
        for (text, n) in [("off", 0), ("0", 0), ("25", 25), ("100", 100)] {
            assert!(v.set("bloom", text));
            assert_eq!(v.bloom, n);
        }
        assert_eq!(v.get("bloom"), "100");
        assert!(!v.set("bloom", "101") && !v.set("bloom", "-1") && !v.set("bloom", "x"));
        assert_eq!(v.bloom, 100);
        // Only a GPU with the 16-bit float targets has it.
        assert_eq!((v.quality(true).bloom, v.quality(false).bloom), (100, 0));
        // The presets leave it off, and a bloom under one makes it Custom.
        for p in [Preset::Original, Preset::Sharp, Preset::Enhanced] {
            v.apply_preset(p, None);
            assert_eq!(v.bloom, 0, "{p:?}");
        }
        v.bloom = 50;
        assert!(!v.picture_is(Preset::Enhanced, None));
        assert!(Settings::default().text().find("bloom").is_none());
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
        // Finer levels: off by default, a flag, into the quality.
        let mut v = GameSettings::default();
        assert_eq!(v.get("shading_fine"), "false");
        assert!(v.set("shading_fine", "true") && v.shading_fine && v.quality(true).shading_fine);
        assert!(!v.set("shading_fine", "on") && v.shading_fine);
        let s = Settings::parse("shading = vertex\nshading_fine = true\n");
        let (v, _) = with_game(Settings::parse(&s.text()), "x", &[]).resolve("x");
        assert!(v.shading_fine && v.shading == gpu::Shading::Vertex);
        // The brightness balance: on by default, a flag, into the quality.
        let mut v = GameSettings::default();
        assert_eq!(v.get("shading_balance"), "true");
        assert!(v.quality(true).shading_balance);
        assert!(v.set("shading_balance", "false") && !v.shading_balance && !v.quality(true).shading_balance);
        assert!(!v.set("shading_balance", "off") && !v.shading_balance);
        let s = Settings::parse("shading = pixel\nshading_balance = false\n");
        let (v, _) = with_game(Settings::parse(&s.text()), "x", &[]).resolve("x");
        assert!(!v.shading_balance && v.shading == gpu::Shading::Pixel);
    }

    #[test]
    fn materials_and_relief() {
        let mut v = GameSettings::default();
        // Off by default: the quality is what it was before materials.
        assert_eq!((v.get("materials").as_str(), v.get("materials_relief").as_str()), ("false", "2"));
        assert!(!v.quality(true).materials && !v.side_channel_on());
        assert_eq!(v.quality(true), gpu::Quality { msaa: 0, ..Default::default() });
        // On, but no pack: nothing asked of the renderer.
        assert!(v.set("materials", "true") && v.materials && !v.materials_on() && !v.quality(true).materials);
        v.pack = true;
        assert!(v.materials_on() && v.quality(true).materials && v.side_channel_on() && !v.light_on());
        assert!(!v.set("materials", "on") && v.materials);
        // The rule: a strength from 0.1 to 8, or off (faithful).
        for (text, r) in [
            ("off", gpu::Relief::Faithful),
            ("1", gpu::Relief::Diffuse { tenths: 10 }),
            ("0.5", gpu::Relief::Diffuse { tenths: 5 }),
            ("8", gpu::Relief::Diffuse { tenths: 80 }),
            ("2", gpu::Relief::default()),
        ] {
            assert!(v.set("materials_relief", text));
            assert_eq!((v.materials_relief, v.get("materials_relief")), (r, text.to_string()));
            assert_eq!(v.quality(true).materials_relief, r);
        }
        assert!(!v.set("materials_relief", "9") && !v.set("materials_relief", "0") && !v.set("materials_relief", "x"));
        // Through the files; unset, nothing is written.
        assert!(Settings::default().text().find("materials").is_none());
        let s = Settings::parse("pack = true\nmaterials = true\nmaterials_relief = off\n");
        let (v, _) = with_game(Settings::parse(&s.text()), "x", &[]).resolve("x");
        assert!(v.materials_on() && v.materials_relief == gpu::Relief::Faithful);
        // Back to the defaults: nothing stored.
        let mut s = with_game(s, "x", &[]);
        let (mut v, _) = s.resolve("x");
        (v.materials, v.materials_relief) = (false, gpu::Relief::default());
        edit_quietly(&mut s, "x", &v);
        assert!(!s.defaults.contains_key("materials") && !s.defaults.contains_key("materials_relief"));
    }

    #[test]
    fn presets_round_trip_through_the_files() {
        // Unset: Original, nothing written.
        assert_eq!(GameSettings::default().preset, Preset::Original);
        assert!(!Settings::default().text().contains("preset"));
        let mut v = GameSettings::default();
        for p in Preset::ALL {
            assert!(v.set("preset", p.name()) && v.get("preset") == p.name());
        }
        assert!(!v.set("preset", "extra") && v.preset == Preset::Custom);
        // A preset is stored as itself; its rows follow from it per game.
        let mut s = with_game(Settings::parse("preset = enhanced\n"), "topskatr", &[]);
        s.games.insert("vstriker".into(), Layer::new());
        let (v, src) = s.resolve("topskatr");
        assert_eq!((v.preset, v.temporal, v.depth, v.shading), (Preset::Enhanced, 100, true, gpu::Shading::Vertex));
        assert!(v.smooth);
        assert_eq!(src[KEYS.iter().position(|k| *k == "depth").unwrap()], Source::AllGames);
        let (v, _) = s.resolve("vstriker");
        assert_eq!((v.preset, v.aa, v.temporal, v.depth), (Preset::Enhanced, 4, 0, false));
        // Through the text.
        let again = Settings::parse(&s.text());
        assert_eq!(again.defaults.get("preset").map(String::as_str), Some("enhanced"));
        // The game's own preset wins over the defaults for all games.
        let mut s = with_game(s, "topskatr", &[("preset", "sharp")]);
        let (v, _) = s.resolve("topskatr");
        assert_eq!((v.preset, v.aa, v.temporal, v.depth, v.smooth), (Preset::Sharp, 4, 0, false, false));
        // A row given beside a preset (the command line's) wins, and the
        // preset is then Custom with the rows as they are.
        s.cli.insert("aa".into(), "8".into());
        let (v, _) = s.resolve("topskatr");
        assert_eq!((v.preset, v.aa), (Preset::Custom, 8));
        // Settings files from before presets have rows and no preset: Custom.
        let mut old = with_game(Settings::parse("aa = 4\ndepth = true\n"), "x", &[]);
        let (v, _) = old.resolve("x");
        assert_eq!((v.preset, v.aa, v.depth), (Preset::Custom, 4, true));
        let mut none = with_game(Settings::parse("sfx_db = 3\n"), "x", &[]);
        assert_eq!(none.resolve("x").0.preset, Preset::Original);
    }

    #[test]
    fn editing_a_preset_keeps_the_preset_not_its_rows() {
        let mut s = with_game(Settings::default(), "topskatr", &[]);
        s.for_all = false;
        let (mut v, _) = s.resolve("topskatr");
        v.apply_preset(Preset::Enhanced, romset::games::find("topskatr").ok());
        edit_quietly(&mut s, "topskatr", &v);
        assert_eq!(s.games["topskatr"].get("preset").map(String::as_str), Some("enhanced"));
        assert_eq!(s.games["topskatr"].len(), 1, "{:?}", s.games["topskatr"]);
        assert!(s.defaults.is_empty());
        assert_eq!(s.resolve("topskatr").0, v);
        // Smooth motion off after it: kept, as the one row that differs.
        let (mut w, _) = s.resolve("topskatr");
        w.smooth = false;
        edit_quietly(&mut s, "topskatr", &w);
        assert_eq!(s.games["topskatr"].get("smooth").map(String::as_str), Some("false"));
        let (back, _) = s.resolve("topskatr");
        assert_eq!((back.preset, back.smooth, back.depth), (Preset::Enhanced, false, true));
        // Back to Original: nothing stored at all.
        let mut o = back.clone();
        o.apply_preset(Preset::Original, romset::games::find("topskatr").ok());
        edit_quietly(&mut s, "topskatr", &o);
        assert!(s.games["topskatr"].is_empty(), "{:?}", s.games["topskatr"]);
        assert_eq!(s.resolve("topskatr").0, GameSettings::default());
        // Custom keeps its rows, each where it differs from the layers below.
        let mut c = GameSettings::default();
        c.apply_preset(Preset::Custom, None);
        (c.aa, c.depth) = (8, true);
        edit_quietly(&mut s, "topskatr", &c);
        let (got, _) = s.resolve("topskatr");
        assert_eq!((got.preset, got.aa, got.depth, got.temporal), (Preset::Custom, 8, true, 0));
        assert_eq!(s.games["topskatr"].get("preset").map(String::as_str), Some("custom"));
        // For all games: another game follows, with its own Enhanced.
        let mut s = with_game(Settings::default(), "vstriker", &[]);
        s.games.insert("topskatr".into(), Layer::new());
        s.for_all = true;
        let (mut v, _) = s.resolve("topskatr");
        v.apply_preset(Preset::Enhanced, romset::games::find("topskatr").ok());
        edit_quietly(&mut s, "topskatr", &v);
        assert_eq!(s.defaults.len(), 1, "{:?}", s.defaults);
        let (other, _) = s.resolve("vstriker");
        assert_eq!((other.preset, other.temporal, other.aa, other.depth), (Preset::Enhanced, 0, 4, false));
        // A row changed under a preset (as the launcher does) makes it
        // Custom with that row.
        let (mut v, _) = s.resolve("topskatr");
        v.aa = 8;
        edit_quietly(&mut s, "topskatr", &v);
        let (v, _) = s.resolve("topskatr");
        assert_eq!((v.preset, v.aa, v.temporal, v.depth), (Preset::Custom, 8, 100, true));
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
