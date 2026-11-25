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
mod link_variants;
mod manxtt;
mod motoraid;
mod overrev;
mod pltkids;
mod rchase2;
mod schamp;
mod segawski;
mod sgt24h;
mod skisuprg;
mod skytargt;
mod srallyc;
mod stcc;
mod topskatr;
mod variants;
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
pub use motoraid::MOTORAID;
pub use overrev::OVERREV;
pub use pltkids::PLTKIDS;
pub use rchase2::RCHASE2;
pub use schamp::SCHAMP;
pub use segawski::SEGAWSKI;
pub use sgt24h::SGT24H;
pub use skisuprg::SKISUPRG;
pub use skytargt::SKYTARGT;
pub use srallyc::SRALLYC;
pub use stcc::STCC;
pub use topskatr::TOPSKATR;
pub use variants::{PARENT_LABELS, VARIANTS};
pub use vcop::VCOP;
pub use vcop2::VCOP2;
pub use vf2::VF2;
pub use von::VON;
pub use vstriker::VSTRIKER;
pub use waverunr::WAVERUNR;
pub use zerogun::ZEROGUN;

/// The set's tours (`remodel tour`, docs/uplift-studio.md, "Tours"): lines
/// of `LABEL|LENGTH|INPUT`, input scripts and frame numbers only. Tours are
/// frame counts from power-on, so a variant (another program, other timing)
/// has none until some are made: it does not use its parent's. Empty where
/// there are none.
pub fn tours(game: &Game) -> &'static str {
    match game.id {
        "topskatr" => include_str!("tours/topskatr.txt"),
        "vf2" => include_str!("tours/vf2.txt"),
        "daytona" => include_str!("tours/daytona.txt"),
        _ => "",
    }
}

/// The set's pokes (`remodel poke`, docs/uplift/content-coverage.md): lines
/// of `NAME|ADDR:VALUE[:b|:w|:l]|what it does, how it was found`, RAM writes
/// a tour can hold (`poke=NAME@from-to`) to reach content without skill.
/// Found by running the game; never applied unless a tour asks. Empty where
/// there are none.
pub fn pokes(game: &Game) -> &'static str {
    match game.id {
        "topskatr" => include_str!("pokes/topskatr.txt"),
        _ => "",
    }
}

/// The set's named game values (`remodel ram values`,
/// docs/uplift-studio.md, "Named values"): lines of
/// `NAME|ADDR:WIDTH[:signed|:float]|what it is, how it was found`, work-RAM
/// words found by running the game with `remodel ram search`. Empty where
/// there are none.
pub fn values(game: &Game) -> &'static str {
    match game.id {
        "topskatr" => include_str!("values/topskatr.txt"),
        _ => "",
    }
}

pub static ALL: &[&Game] = &[
    &TOPSKATR, &SKISUPRG, &STCC, &SEGAWSKI, &WAVERUNR, &OVERREV, &HOTD, &BEL, &SGT24H, &DYNABB97, &DOA, &INDY500,
    &FVIPERS, &SCHAMP, &VSTRIKER, &LASTBRNX, &SRALLYC, &VF2, &DAYTONA, &VCOP, &DESERT, &VCOP2, &SKYTARGT, &MANXTT,
    &MOTORAID, &GUNBLADE, &DYNAMCOP, &ZEROGUN, &PLTKIDS, &RCHASE2, &VON,
];

/// Games whose board model runs but which don't show a game yet. The CLI
/// can run them (`find`), but every list built from [`ALL`] (the player's
/// and the browser's game lists, `remodel games`, the posters and
/// bezels) leaves them out. A game moves to `ALL` when it plays.
pub static IN_PROGRESS: &[&Game] = &[];

/// The title (a supported game, not a variant of one) `id`.
pub fn title(id: &str) -> Option<&'static Game> {
    ALL.iter().chain(IN_PROGRESS).copied().find(|g| g.id == id)
}

/// The game `id`, if it is one a front end lists: a title ([`ALL`]) or a
/// variant of one, resolved.
pub fn playable(id: &str) -> Option<&'static Game> {
    ALL.iter().copied().find(|g| g.id == id).or_else(|| crate::variants::get(id).and_then(crate::variants::resolve))
}

/// The game `id`: a title, or a variant resolved into a full [`Game`] (its
/// parent's manifest with its swaps and overrides: [`crate::variants`]).
pub fn find(id: &str) -> Result<&'static Game, Error> {
    title(id)
        .or_else(|| crate::variants::get(id).and_then(crate::variants::resolve))
        .ok_or_else(|| Error::UnknownGame(id.to_string()))
}
