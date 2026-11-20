//! Per-game link play data (`LinkPlay`): where a game keeps the setting that
//! makes a cabinet a link master or slave, and how its link RAM is laid out
//! (`docs/hw/link-board.md`). Used by `model2::link::LinkHub` and `remodel
//! link-run`; games without an entry have no linked play.

use crate::{FreePlayByte, LinkPlay, LinkSettings, RecordCrc, SettingsStore};

/// The linked-play data of game `id`, if it has been looked at.
pub fn link_play(id: &str) -> Option<&'static LinkPlay> {
    match id {
        "srallyc" => Some(&SRALLYC),
        "daytona" => Some(&DAYTONA),
        _ => None,
    }
}

/// `srallyc` (docs/hw/link-board.md sections 2 and 4): LINK TYPE at byte
/// `0x0B` of the 38-byte settings record (4-byte header: CRC-16 over the
/// next 34 bytes with start value `0xFFFF`, complemented; then the
/// settings), 1 CAR 1 the master, 2-4 CAR 2-4. The bytes are what the
/// game's own first boot writes, LINK TYPE 0 (NOTLINK).
static SRALLYC: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x00, 0x00, 0x24, 0x00, 0x02, 0, 0, 0, 0x01, 0, 0, 0, 0, 0, 0x01, 0x04, 0, 0, 0x80, 0x3f, 0xf2, 0xff, 0xff,
            0xff, 0x01, 0, 0x02, 0, 0, 0, 0x0b, 0, 0x01, 0, 0x01, 0, 0xff, 0xff,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc { at: 0, from: 2, to: 36, init: 0xffff, complement: true },
    },
    role_off: 0x0b,
    roles: &[1, 2, 3, 4],
    car_off: None,
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: Some(0x18),
    pos_off: Some(0x28),
    // The node and car numbers (0x00-0x03) and the car's pose and wheels (0x24-0x7f).
    ghost_keep: &[(0x00, 0x04), (0x24, 0x80)],
    ghost_sum: None,
    free_play: Some(SRALLYC_FREE_PLAY),
};

/// `srallyc` COIN ASSIGNMENTS, COIN/CREDIT SETTING #26 "FREE PLAY"
/// (docs/games/srallyc.md): what the game's own test menu changed in the
/// record going from its default #12 (2 coins 1 credit), found by running
/// it: the setting's number (byte `0x1E`, 0-based) and three bytes the
/// game derives from it.
static SRALLYC_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x04, on: 0x03, off: 0x02 },
    FreePlayByte { at: 0x14, on: 0xfa, off: 0xf2 },
    FreePlayByte { at: 0x1a, on: 0x01, off: 0x02 },
    FreePlayByte { at: 0x1e, on: 0x19, off: 0x0b },
];

/// `daytona` (docs/hw/link-board.md section 10): the link ID at byte `0x0B` of
/// the I/O board's 128-byte record ("SEGA", version, CRC-16 at 8 over bytes
/// `0x0A`-`0x7F`: CCITT, start 0, not complemented), 0 SINGLE, 1 MASTER, 2 SLAVE.
static DAYTONA: LinkPlay = LinkPlay {
    max_nodes: 8,
    settings: LinkSettings {
        store: SettingsStore::IoBoard,
        record: &[
            0x53, 0x45, 0x47, 0x41, 0x40, 0x82, 0x02, 0x00, 0x31, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x14, 0x1c, 0x00, 0x01,
            0x00, 0x01, 0x04, 0x01,
        ],
        fill: 0,
        size: 128,
        crc: RecordCrc { at: 8, from: 0x0a, to: 0x80, init: 0, complement: false },
    },
    role_off: 0x0b,
    roles: &[1, 2],
    car_off: Some(0x0c),
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    // The role and node ID bytes of the header (0x0b-0x0c) and the car's body (0x40-0xe7); the
    // header's checksum word at 0x3c is the sum of the other 15 words.
    ghost_keep: &[(0x0b, 0x0d), (0x40, 0xe8)],
    ghost_sum: Some(0x3c),
    free_play: Some(DAYTONA_FREE_PLAY),
};

/// `daytona` COIN ASSIGNMENT, COIN/CREDIT SETTING #27 "FREE PLAY"
/// (docs/games/daytona.md): what the game's own test menu changed in the
/// record going from its default #1 (1 coin 1 credit), found by running it:
/// the setting's number (byte `0x10`, 0-based) and two bytes the game
/// derives from it.
static DAYTONA_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x00 },
    FreePlayByte { at: 0x14, on: 0x01, off: 0x00 },
    FreePlayByte { at: 0x7c, on: 0x01, off: 0x00 },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_play_sets_the_game_bytes_and_comes_out_again() {
        for id in ["srallyc", "daytona"] {
            let p = link_play(id).unwrap();
            let fp = p.free_play.unwrap();
            let plain = p.settings.image(None, p.role_off, 0, &[]);
            let base = p.base_with_free_play(None, true);
            let on = p.settings.image(base.as_deref(), p.role_off, 0, &[]);
            for b in fp {
                assert_eq!(plain[b.at], b.off, "{id}: byte {:#x} at the defaults", b.at);
                assert_eq!(on[b.at], b.on, "{id}: byte {:#x} on free play", b.at);
            }
            let at = p.settings.crc.at;
            assert_eq!(p.settings.crc.compute(&on), u16::from_le_bytes([on[at], on[at + 1]]), "{id}: checksum");
            assert_ne!(plain, on);
            // A session's saved settings go back to the defaults.
            let mut saved = on.clone();
            p.unset_free_play(&mut saved);
            assert_eq!(saved, plain, "{id}");
            // With nothing of free play in them, nothing is touched.
            assert!(p.free_play_set(Some(&plain), false).is_empty());
            // Free play off over a file a free play session wrote restores the defaults.
            if p.settings.store == SettingsStore::Eeprom {
                let off = p.base_with_free_play(Some(&on), false).unwrap();
                assert_eq!(p.settings.image(Some(&off), p.role_off, 0, &[]), plain, "{id}");
            }
        }
    }
}
