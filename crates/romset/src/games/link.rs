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
        "manxtt" | "manxttc" => Some(&MANXTT),
        "indy500" => Some(&INDY500),
        "motoraid" => Some(&MOTORAID),
        "waverunr" => Some(&WAVERUNR),
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
        crc: RecordCrc::ccitt(0, 2, 36, 0xffff, true),
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
        crc: RecordCrc::ccitt(8, 0x0a, 0x80, 0, false),
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

/// `manxtt` and its clone `manxttc` (docs/games/manxtt.md, link-board.md section
/// 14): GAME ASSIGNMENTS LINK TYPE at byte `0x0B` of the 56-byte settings
/// record (`srallyc`'s kind: CRC-16 at 0 over bytes 2-55, start `0xFFFF`,
/// complemented, the length word `0x38` at 2), 0 Not Link, 1 Master, 2 Slave. The
/// bytes are what the game's own test menu writes at the defaults. The menu offers
/// two machines; the hub ran four. The block's identity bytes are taken to be
/// `srallyc`'s (*unconfirmed*).
static MANXTT: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0xcf, 0x0f, 0x38, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x01, 0x01, 0x01,
            0x02, 0x02, 0x02, 0x02, 0x01, 0x01, 0x01, 0x01, 0x00, 0x20, 0xe0, 0x20, 0xe0, 0x80, 0x20, 0xe0, 0xff, 0xff,
            0xf2, 0xff, 0xff, 0xff, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00,
            0xff, 0xff,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::ccitt(0, 2, 0x38, 0xffff, true),
    },
    role_off: 0x0b,
    roles: &[1, 2],
    car_off: None,
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04), (0x24, 0x80)],
    ghost_sum: None,
    free_play: Some(MANXTT_FREE_PLAY),
};

/// `manxtt` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY"
/// (docs/games/manxtt.md): what the game's own test menu changed in the record
/// going from its default #1 (1 coin 1 credit), found by running it: the
/// setting's number (byte `0x2E`, 0-based) and two bytes the game derives from it.
static MANXTT_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x04, on: 0x02, off: 0x01 },
    FreePlayByte { at: 0x24, on: 0xfa, off: 0xf2 },
    FreePlayByte { at: 0x2e, on: 0x1a, off: 0x00 },
];

/// The start value of the "S32A" records' checksum in these games (`docs/hw/eeprom-records.md`).
const S32A_SEED: u32 = 0xdebd_ec00;

/// `indy500` (docs/games/indy500.md, docs/hw/eeprom-records.md, link-board.md section 14): the
/// "S32A" settings record in the EEPROM: a header, then the 36-byte record at `0x08`
/// (checksum word, write counter, the settings from `0x10`) and its copy at `0x2C`. NETWORK
/// TYPE is byte `0x1E` (0 STAND ALONE, 1 MASTER, 2 SLAVE) and CABINET ID byte `0x1F`
/// (they must differ between cabinets: two at one ID show NETWORK ERROR and
/// restart the check). The bytes are what the game's own test menu writes at the defaults.
static INDY500: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x07, 0x09, 0x00, 0x00, 0x3f, 0x0f, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x0b, 0x02,
            0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x00, 0x02, 0x01, 0x00, 0x02, 0x03, 0x00, 0x00, 0x00, 0x40, 0x80, 0xc0,
            0x40, 0x80, 0xc0, 0x40, 0x80, 0xc0, 0x80, 0x00,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::s32a(0x08, 36, 34, S32A_SEED),
    },
    role_off: 0x1e,
    roles: &[1, 2],
    car_off: Some(0x1f),
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04), (0x24, 0x80)],
    ghost_sum: None,
    free_play: Some(INDY500_FREE_PLAY),
};

/// `indy500` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY"
/// (docs/games/indy500.md): what the game's own test menu changed in the record
/// going from its default #12 (2 coins 1 credit), found by running it: the
/// setting's number (byte `0x10`, 0-based) and two bytes the game derives from it
/// (and the write counter, which the game raises by itself).
static INDY500_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x0b },
    FreePlayByte { at: 0x11, on: 0x01, off: 0x02 },
    FreePlayByte { at: 0x15, on: 0x01, off: 0x00 },
];

/// `motoraid` (docs/games/motoraid.md, docs/hw/eeprom-records.md, link-board.md section 14): the
/// "S32A" settings record in the EEPROM: a header, then the 36-byte record at `0x08`
/// (checksum word, write counter, the settings from `0x10`) and its copy at `0x2C`. NETWORK
/// TYPE is byte `0x1E` (0 STAND ALONE, 1 MASTER, 2 SLAVE) and CABINET ID byte `0x1F`
/// (they must differ between cabinets: two at one ID show NETWORK ERROR and
/// restart the check). The bytes are what the game's own test menu writes at the defaults.
static MOTORAID: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x92, 0xbc, 0x00, 0x00, 0x73, 0x16, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x0b, 0x02,
            0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x00, 0x03, 0x00, 0x00, 0x00, 0x40, 0x80, 0xc0,
            0x40, 0x80, 0xc0, 0x40, 0x80, 0xc0, 0x80, 0x00,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::s32a(0x08, 36, 34, S32A_SEED),
    },
    role_off: 0x1e,
    roles: &[1, 2],
    car_off: Some(0x1f),
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04), (0x24, 0x80)],
    ghost_sum: None,
    free_play: None,
};

/// `waverunr` (docs/games/waverunr.md, docs/hw/eeprom-records.md, link-board.md section 14): the
/// "S32A" settings record in the EEPROM: a header, then the 36-byte record at `0x08`
/// (checksum word, write counter, the settings from `0x10`) and its copy at `0x2C`. NETWORK
/// TYPE is byte `0x1E` (0 STAND ALONE, 1 MASTER, 2 SLAVE) and CABINET ID byte `0x1F`
/// (they must differ between cabinets: two at one ID show NETWORK ERROR and
/// restart the check). The bytes are what the game's own test menu writes at the defaults.
static WAVERUNR: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x17, 0xa9, 0x00, 0x00, 0xb7, 0xdf, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x10, 0x03,
            0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x02, 0x03, 0x00, 0x00, 0x00, 0x40, 0x80, 0xc0,
            0x40, 0x80, 0xc0, 0x40, 0x80, 0xe0, 0x80, 0x00,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::s32a(0x08, 36, 34, S32A_SEED),
    },
    role_off: 0x1e,
    roles: &[1, 2],
    car_off: Some(0x1f),
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04), (0x24, 0x80)],
    ghost_sum: None,
    free_play: None,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_play_sets_the_game_bytes_and_comes_out_again() {
        for id in ["srallyc", "daytona", "manxtt", "indy500"] {
            let p = link_play(id).unwrap();
            let Some(fp) = p.free_play else { continue };
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

    #[test]
    fn the_records_the_games_wrote_carry_their_own_checksums() {
        // Each default record is what the game's own test menu saved; its checksum field is
        // the game's, so the algorithm must reproduce it (both copies, for the S32A records).
        for id in ["daytona", "manxtt", "indy500", "motoraid", "waverunr"] {
            let p = link_play(id).unwrap();
            let n = p.settings.record.len();
            let rebuilt = p.settings.image(None, p.role_off, p.settings.record[p.role_off], &[]);
            // Rebuilding the image through the store changes nothing: the game's checksum is ours.
            assert_eq!(rebuilt[..n], *p.settings.record, "{id}");
            p.settings.crc.check(&rebuilt).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }

    #[test]
    fn s32a_roles_go_into_both_copies_with_a_good_checksum() {
        for id in ["indy500", "motoraid", "waverunr"] {
            let p = link_play(id).unwrap();
            let n = p.settings.crc.copy.unwrap();
            let a = p.settings.image(None, p.role_off, 1, &[(p.car_off.unwrap(), 0)]);
            let b = p.settings.image(None, p.role_off, 2, &[(p.car_off.unwrap(), 1)]);
            for (img, role, car) in [(&a, 1, 0), (&b, 2, 1)] {
                p.settings.crc.check(img).unwrap();
                for copy in [0, n] {
                    assert_eq!(img[p.role_off + copy], role, "{id}");
                    assert_eq!(img[p.car_off.unwrap() + copy], car, "{id}");
                }
            }
            assert_ne!(a, b);
            assert_eq!(&a[..4], b"S32A");
        }
    }
}
