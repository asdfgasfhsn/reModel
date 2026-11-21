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

pub use driver::{NetLink, Notice, StallStats, Step};
pub use mdns::{blocker, merge, Advert, Browser, Lobby, Mine, Txt};
pub use names::{default_name, display_names, name_or_default, sanitize_name, MAX_NAME};
pub use net::{lan_address, Faults, Net};
pub use session::{End, Event, Session, Stats};
pub use setup::{delay_for, Config, Discovery, Found, Host, HostStatus, Join, JoinStatus, DEFAULT_PORT};
pub use wire::{fnv, Params, FNV_START};

/// A hash of every chip the game loads, so two hosts agree only on the same ROMs
/// all through: the set id (a variant has its own), then each chip in manifest
/// order, which is by socket and role, as its role, its length and its content
/// (never the file's name, which a clone's chips may share with the parent's or
/// a user may have renamed). Cheap enough to take once when a session is
/// configured: FNV over the set's chips, tens of milliseconds.
pub fn rom_hash(roms: &romset::RomSet) -> u64 {
    let mut h = fnv(FNV_START, roms.game.id.as_bytes());
    for (file, chip) in roms.game.files.iter().zip(&roms.chips) {
        h = fnv(h, format!("|{:?}|{}|", file.role, chip.len()).as_bytes());
        h = fnv(h, chip);
    }
    h
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

#[cfg(test)]
mod tests {
    use super::*;

    fn set(id: &str, edit: impl Fn(usize, &mut Vec<u8>)) -> romset::RomSet {
        let game = romset::games::find(id).unwrap();
        let mut chips: Vec<Vec<u8>> = game.files.iter().map(|f| vec![f.size as u8; 64]).collect();
        for (i, c) in chips.iter_mut().enumerate() {
            edit(i, c);
        }
        romset::RomSet { game, program: Vec::new(), chips }
    }

    #[test]
    fn the_rom_hash_covers_every_chip() {
        let game = romset::games::find("srallyc").unwrap();
        let a = set("srallyc", |_, _| {});
        assert_eq!(rom_hash(&a), rom_hash(&set("srallyc", |_, _| {})), "the same set agrees");
        // A change in any one chip, whatever its role, is a different hash.
        let mut roles = std::collections::HashSet::new();
        for (i, f) in game.files.iter().enumerate() {
            let b = set("srallyc", |j, c| {
                if i == j {
                    c[63] ^= 1
                }
            });
            assert_ne!(rom_hash(&a), rom_hash(&b), "chip {} ({:?}) is not in the hash", f.name, f.role);
            roles.insert(format!("{:?}", f.role));
        }
        assert!(roles.len() > 3, "a set with data, texture and sound chips: {roles:?}");
        // The program alone is no longer what is compared.
        let program_only = set("srallyc", |i, c| {
            if game.files[i].role != romset::Role::Program {
                c[0] ^= 1
            }
        });
        assert_ne!(rom_hash(&a), rom_hash(&program_only));
    }

    #[test]
    fn the_rom_hash_covers_the_set_id() {
        // The same chip contents under another set id: not the same game.
        let a = set("manxtt", |_, _| {});
        let mut b = set("manxtt", |_, _| {});
        b.game = romset::games::find("manxttc").unwrap();
        b.chips.truncate(a.chips.len().min(b.game.files.len()));
        let mut a2 = set("manxtt", |_, _| {});
        a2.chips.truncate(b.chips.len());
        assert_ne!(rom_hash(&a2), rom_hash(&b));
    }
}
