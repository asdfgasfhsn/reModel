//! The player's texture pack: the directory reader is `model2::dirpack`
//! (shared with `remodel pack-log`); this is where a game's pack lives.

use std::path::PathBuf;

pub use model2::dirpack::DirPack;

/// Where `game`'s pack is kept: `<data dir>/<set>/texpack/` (`<root>/<set>/texpack/`
/// with a texture packs folder chosen in System settings), or for a clone
/// without one, its parent's (keys are the textures' content, so the same
/// images serve every version).
pub fn game_dir(game: &romset::Game) -> PathBuf {
    let root = crate::settings::packs_root().unwrap_or_else(crate::settings::data_dir);
    let dir = |set: &str| root.join(set).join("texpack");
    romset::pack_sets(game).map(dir).find(|d| d.is_dir()).unwrap_or_else(|| dir(game.id))
}
