//! Settings kept between runs, and where the player keeps its files.
//!
//! Run from the Remodel source tree (an `out/` directory in the current
//! directory), files go under `out/` next to the other per-game output;
//! otherwise under `$XDG_DATA_HOME/remodel` (`~/.local/share/remodel`).

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub rom: PathBuf,
    pub game: String,
    /// Mix in dB: sound board (effects, announcer), music board.
    pub sfx_db: f32,
    pub music_db: f32,
    pub hud_edges: bool,
    pub exact: bool,
    pub fullscreen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            rom: default_rom(),
            game: "topskatr".into(),
            sfx_db: model2::audio::DEFAULT_MIX.0,
            music_db: model2::audio::DEFAULT_MIX.1,
            hud_edges: true,
            exact: false,
            fullscreen: false,
        }
    }
}

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

impl Settings {
    /// Saved settings, or the defaults.
    pub fn load() -> Settings {
        let mut s = Settings::default();
        for line in std::fs::read_to_string(path()).unwrap_or_default().lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "rom" => s.rom = PathBuf::from(v),
                "game" => s.game = v.to_string(),
                "sfx_db" => s.sfx_db = v.parse().unwrap_or(s.sfx_db),
                "music_db" => s.music_db = v.parse().unwrap_or(s.music_db),
                "hud_edges" => s.hud_edges = v == "true",
                "exact" => s.exact = v == "true",
                "fullscreen" => s.fullscreen = v == "true",
                _ => {}
            }
        }
        s
    }

    pub fn save(&self) {
        let text = format!(
            "# remodel-play settings\nrom = {}\ngame = {}\nsfx_db = {}\nmusic_db = {}\nhud_edges = {}\nexact = {}\nfullscreen = {}\n",
            self.rom.display(),
            self.game,
            self.sfx_db,
            self.music_db,
            self.hud_edges,
            self.exact,
            self.fullscreen
        );
        let p = path();
        if let Some(d) = p.parent() {
            let _ = std::fs::create_dir_all(d);
        }
        let _ = std::fs::write(p, text);
    }
}
