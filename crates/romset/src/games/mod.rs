//! Per-game manifests. Adding a game means adding a module here and listing
//! it in `ALL`.

mod bel;
mod hotd;
mod overrev;
mod overrevb;
mod segawski;
mod skisuprg;
mod stcc;
mod topskatr;
mod waverunr;

use crate::{Error, Game};

pub use bel::BEL;
pub use hotd::HOTD;
pub use overrev::OVERREV;
pub use overrevb::OVERREVB;
pub use segawski::SEGAWSKI;
pub use skisuprg::SKISUPRG;
pub use stcc::STCC;
pub use topskatr::TOPSKATR;
pub use waverunr::WAVERUNR;

pub static ALL: &[&Game] = &[&TOPSKATR, &SKISUPRG, &STCC, &SEGAWSKI, &WAVERUNR, &OVERREV, &HOTD, &BEL, &OVERREVB];

pub fn find(id: &str) -> Result<&'static Game, Error> {
    ALL.iter().copied().find(|g| g.id == id).ok_or_else(|| Error::UnknownGame(id.to_string()))
}
