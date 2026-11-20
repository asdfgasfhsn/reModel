//! LAN linked play: each machine on the network runs its own linked cabinet
//! and the link board's blocks travel between the hosts over UDP
//! (`docs/hw/link-board.md` section 12, `docs/play.md` "Network link play").
//!
//! Lockstep with a fixed delay D: at frame boundary N every host contributes
//! its node's record; a node sees the others' records of frame N - (D - 1)
//! and its own at once, and a host whose records for that frame have not all
//! arrived waits (no guessing), so every host's machine is exactly what
//! `remodel link-run` gives that node with the same D. The game's machines
//! are not touched here: this crate carries `model2::link::NodeRec`s.
//!
//! * [`wire`]: the packet formats, parsed defensively.
//! * [`net`]: the UDP socket and fault injection (loss, delay, jitter, duplication).
//! * [`mdns`]: mDNS / DNS-SD discovery (host advert, browser, merged list).
//! * [`names`]: player user names.
//! * [`setup`]: the lobby ([`Host`], [`Join`], [`Discovery`]) and the agreement.
//! * [`session`]: records, acknowledgements, state checks, liveness, the end.
//! * [`driver`]: [`NetLink`], the boundary exchange over a hub.

pub mod driver;
pub mod mdns;
pub mod names;
pub mod net;
pub mod session;
pub mod setup;
pub mod wire;

pub use driver::{NetLink, StallStats, Step};
pub use mdns::{blocker, merge, Advert, Browser, Lobby, Mine, Txt};
pub use names::{default_name, display_names, name_or_default, sanitize_name, MAX_NAME};
pub use net::{lan_address, Faults, Net};
pub use session::{End, Event, Session, Stats};
pub use setup::{delay_for, Config, Discovery, Found, Host, HostStatus, Join, JoinStatus, DEFAULT_PORT};
pub use wire::{fnv, Params, FNV_START};

/// A hash of the program ROMs: the same ROMs on every host.
pub fn rom_hash(program: &[u8]) -> u64 {
    fnv(FNV_START, program)
}

/// A hash of the settings image node `node` of a game runs with (`base`: the settings the
/// session starts from; `extra`: bytes set on top, a test): what a host checks against
/// what the master sent.
pub fn settings_hash(play: &romset::LinkPlay, base: Option<&[u8]>, node: usize, extra: &[(usize, u8)]) -> u64 {
    let role = *play.roles.get(node).or(play.roles.last()).unwrap();
    let set: Vec<(usize, u8)> =
        play.car_off.map(|o| (o, node as u8)).into_iter().chain(extra.iter().copied()).collect();
    fnv(FNV_START, &play.settings.image(base, play.role_off, role, &set))
}
