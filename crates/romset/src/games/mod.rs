//! Per-game manifests. Adding a game means adding a module here and listing
//! it in `ALL`.

mod skisuprg;
mod topskatr;

use crate::{Error, Game};

pub use skisuprg::SKISUPRG;
pub use topskatr::TOPSKATR;

pub static ALL: &[&Game] = &[&TOPSKATR, &SKISUPRG];

pub fn find(id: &str) -> Result<&'static Game, Error> {
    ALL.iter().copied().find(|g| g.id == id).ok_or_else(|| Error::UnknownGame(id.to_string()))
}
