//! Per-game manifests. Adding a game means adding a module here and listing
//! it in `ALL`.

mod bel;
mod daytona;
mod desert;
mod doa;
mod dynabb97;
mod dynamcop;
mod fvipers;
mod gunblade;
mod hotd;
mod indy500;
mod lastbrnx;
mod link;
mod manxtt;
mod manxttc;
mod motoraid;
mod overrev;
mod overrevb;
mod pltkids;
mod rchase2;
mod schamp;
mod segawski;
mod sfight;
mod sgt24h;
mod skisuprg;
mod skytargt;
mod srallyc;
mod stcc;
mod topskatr;
mod vcop;
mod vcop2;
mod vf2;
mod von;
mod vstriker;
mod waverunr;
mod zerogun;

use crate::{Error, Game, Role, RomFile};

/// One chip of a set, for a game's `files` list.
const fn rom(name: &'static str, size: usize, sha1: &'static str, role: Role) -> RomFile {
    RomFile { name, size, sha1, role }
}

pub use bel::BEL;
pub use daytona::DAYTONA;
pub use desert::DESERT;
pub use doa::DOA;
pub use dynabb97::DYNABB97;
pub use dynamcop::DYNAMCOP;
pub use fvipers::FVIPERS;
pub use gunblade::GUNBLADE;
pub use hotd::HOTD;
pub use indy500::INDY500;
pub use lastbrnx::LASTBRNX;
pub use link::link_play;
pub use manxtt::MANXTT;
pub use manxttc::MANXTTC;
pub use motoraid::MOTORAID;
pub use overrev::OVERREV;
pub use overrevb::OVERREVB;
pub use pltkids::PLTKIDS;
pub use rchase2::RCHASE2;
pub use schamp::SCHAMP;
pub use segawski::SEGAWSKI;
pub use sfight::SFIGHT;
pub use sgt24h::SGT24H;
pub use skisuprg::SKISUPRG;
pub use skytargt::SKYTARGT;
pub use srallyc::SRALLYC;
pub use stcc::STCC;
pub use topskatr::TOPSKATR;
pub use vcop::VCOP;
pub use vcop2::VCOP2;
pub use vf2::VF2;
pub use von::VON;
pub use vstriker::VSTRIKER;
pub use waverunr::WAVERUNR;
pub use zerogun::ZEROGUN;

pub static ALL: &[&Game] = &[
    &TOPSKATR, &SKISUPRG, &STCC, &SEGAWSKI, &WAVERUNR, &OVERREV, &HOTD, &BEL, &OVERREVB, &SGT24H, &DYNABB97, &DOA,
    &INDY500, &FVIPERS, &SCHAMP, &SFIGHT, &VSTRIKER, &LASTBRNX, &SRALLYC, &VF2, &DAYTONA, &VCOP, &DESERT, &VCOP2,
    &SKYTARGT, &MANXTT, &MANXTTC, &MOTORAID, &GUNBLADE, &DYNAMCOP, &ZEROGUN, &PLTKIDS, &RCHASE2, &VON,
];

/// Games whose board model runs but which don't show a game yet. The CLI
/// can run them (`find`), but every list built from [`ALL`] (the player's
/// and the browser's game lists, `remodel games`, the posters and
/// bezels) leaves them out. A game moves to `ALL` when it plays.
pub static IN_PROGRESS: &[&Game] = &[];

/// The game `id`, if it is one a front end lists ([`ALL`]).
pub fn playable(id: &str) -> Option<&'static Game> {
    ALL.iter().copied().find(|g| g.id == id)
}

pub fn find(id: &str) -> Result<&'static Game, Error> {
    ALL.iter().chain(IN_PROGRESS).copied().find(|g| g.id == id).ok_or_else(|| Error::UnknownGame(id.to_string()))
}
