//! Per-game manifests. Adding a game means adding a module here and listing
//! it in `ALL`.

mod bel;
mod doa;
mod dynabb97;
mod fvipers;
mod hotd;
mod indy500;
mod overrev;
mod overrevb;
mod schamp;
mod segawski;
mod sgt24h;
mod skisuprg;
mod stcc;
mod topskatr;
mod waverunr;

use crate::{Error, Game};

pub use bel::BEL;
pub use doa::DOA;
pub use dynabb97::DYNABB97;
pub use fvipers::FVIPERS;
pub use hotd::HOTD;
pub use indy500::INDY500;
pub use overrev::OVERREV;
pub use overrevb::OVERREVB;
pub use schamp::SCHAMP;
pub use segawski::SEGAWSKI;
pub use sgt24h::SGT24H;
pub use skisuprg::SKISUPRG;
pub use stcc::STCC;
pub use topskatr::TOPSKATR;
pub use waverunr::WAVERUNR;

pub static ALL: &[&Game] = &[
    &TOPSKATR, &SKISUPRG, &STCC, &SEGAWSKI, &WAVERUNR, &OVERREV, &HOTD, &BEL, &OVERREVB, &SGT24H, &DYNABB97, &DOA,
    &INDY500, &FVIPERS, &SCHAMP,
];

pub fn find(id: &str) -> Result<&'static Game, Error> {
    ALL.iter().copied().find(|g| g.id == id).ok_or_else(|| Error::UnknownGame(id.to_string()))
}
