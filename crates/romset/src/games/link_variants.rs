//! Link play data of class B variants of `daytona` and `indy500` (docs/variants.md,
//! "Link play and free play of the variants"): each variant's own settings record, as its
//! program's test menu writes it, and the bytes its own test menu changed (found by running it).
//! The parents' data and the other variants' are in `link.rs`.

use crate::{FreePlayByte, LinkPlay, LinkSettings, RecordCrc, SettingsStore};

/// The start value of the "S32A" records' checksum (`docs/hw/eeprom-records.md`).
const S32A_SEED: u32 = 0xdebd_ec00;

/// `daytonas` (Saturn Ads), version 4: the I/O board's 128-byte record as the parent's (link ID at byte `0x0B`,
/// CAR NUMBER `0x0C`, CRC-16 at 8: CCITT, start 0, over `0x0A`-`0x7F`), with this program's own version byte
/// and defaults, as the game wrote them (docs/games/daytona.md).
pub static DAYTONAS: LinkPlay = LinkPlay {
    max_nodes: 8,
    settings: LinkSettings {
        store: SettingsStore::IoBoard,
        record: &[
            0x53, 0x45, 0x47, 0x41, 0x40, 0x82, 0x04, 0x00, 0xb2, 0x3e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x14, 0x1c, 0x00, 0x01,
            0x00, 0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00,
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
    ghost_keep: &[(0x0b, 0x0d), (0x40, 0xe8)],
    ghost_sum: Some(0x3c),
    free_play: Some(DAYTONA_FREE_PLAY),
};

/// `daytonase` (Special Edition), version 3: the I/O board's 128-byte record as the parent's (link ID at byte `0x0B`,
/// CAR NUMBER `0x0C`, CRC-16 at 8: CCITT, start 0, over `0x0A`-`0x7F`), with this program's own version byte
/// and defaults, as the game wrote them (docs/games/daytona.md).
pub static DAYTONASE: LinkPlay = LinkPlay {
    max_nodes: 8,
    settings: LinkSettings {
        store: SettingsStore::IoBoard,
        record: &[
            0x53, 0x45, 0x47, 0x41, 0x40, 0x82, 0x03, 0x00, 0x04, 0xad, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x14, 0x1c, 0x00, 0x01,
            0x00, 0x01, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0x00, 0x00,
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
    ghost_keep: &[(0x0b, 0x0d), (0x40, 0xe8)],
    ghost_sum: Some(0x3c),
    free_play: Some(DAYTONA_FREE_PLAY),
};

/// COIN ASSIGNMENT, COIN/CREDIT SETTING #27 "FREE PLAY" in `daytonas` and `daytonase` (docs/games/daytona.md),
/// as the parent's (setting number at `0x10`, `0x14`) but byte `0x7C` counts 1 -> 2 where the parent's goes
/// 0 -> 1: the game raises it at every save (any changed setting does it, link ID and car number too).
static DAYTONA_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x00 },
    FreePlayByte { at: 0x14, on: 0x01, off: 0x00 },
    FreePlayByte { at: 0x7c, on: 0x02, off: 0x01 },
];

/// `indy500d` (Deluxe, Rev A): the "S32A" record in the EEPROM, 44 bytes at `0x08` and its copy at `0x34`
/// (docs/hw/eeprom-records.md), NETWORK TYPE at byte `0x28` (0 STAND ALONE, 1 MASTER, 2 SLAVE)
/// and CABINET ID at `0x29`, as its own test menu writes them (docs/games/indy500.md).
pub static INDY500D: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x11, 0x07, 0x00, 0x00, 0x4e, 0x9f, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x10, 0x03,
            0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x28, 0x35, 0x27, 0x12, 0x11, 0x18, 0x23, 0x00, 0x00,
            0x16, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x80, 0x4e, 0x9f,
            0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x10, 0x03, 0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x02, 0x01, 0x28,
            0x35, 0x27, 0x12, 0x11, 0x18, 0x23, 0x00, 0x00, 0x16, 0x1a, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xff, 0x00,
            0x80, 0xff, 0x00, 0x80, 0xff, 0x80,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::s32a(0x08, 44, 42, S32A_SEED),
    },
    role_off: 0x28,
    roles: &[1, 2],
    car_off: Some(0x29),
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04), (0x24, 0x80)],
    ghost_sum: None,
    free_play: Some(INDY500D_FREE_PLAY),
};

/// `indy500to` (Twin, older): the "S32A" record in the EEPROM, 36 bytes at `0x08` and its copy at `0x2C`
/// (docs/hw/eeprom-records.md), NETWORK TYPE at byte `0x1E` (0 STAND ALONE, 1 MASTER, 2 SLAVE)
/// and CABINET ID at `0x1F`, as its own test menu writes them (docs/games/indy500.md).
pub static INDY500TO: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x09, 0x08, 0x00, 0x00, 0xc9, 0xbd, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x0b, 0x02,
            0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x80, 0xff, 0x00,
            0x80, 0xff, 0x00, 0x80, 0xff, 0x80, 0x00, 0x00, 0xc9, 0xbd, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x0b, 0x02,
            0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x01, 0x00, 0x01, 0x03, 0x00, 0x00, 0x00, 0x80, 0xff, 0x00,
            0x80, 0xff, 0x00, 0x80, 0xff, 0x80, 0x00, 0x00,
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
    free_play: Some(INDY500TO_FREE_PLAY),
};

/// COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY" in `indy500d` (default #17, 3 coins 1 credit):
/// the setting's number at byte `0x10` and two derived bytes, as the parent's (docs/games/indy500.md).
static INDY500D_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x10 },
    FreePlayByte { at: 0x11, on: 0x01, off: 0x03 },
    FreePlayByte { at: 0x15, on: 0x01, off: 0x00 },
];

/// The same in `indy500to` (default #12, 2 coins 1 credit, as the parent's).
static INDY500TO_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x0b },
    FreePlayByte { at: 0x11, on: 0x01, off: 0x02 },
    FreePlayByte { at: 0x15, on: 0x01, off: 0x00 },
];

#[cfg(test)]
mod tests {
    use crate::games::link_play;
    use crate::SettingsStore;

    const IDS: [&str; 4] = ["daytonas", "daytonase", "indy500d", "indy500to"];

    #[test]
    fn the_variants_have_their_own_link_data() {
        for id in IDS {
            let p = link_play(id).unwrap_or_else(|| panic!("{id}: no link data"));
            let parent = link_play(crate::variants::title_of(id)).unwrap();
            assert_eq!(p.max_nodes, parent.max_nodes, "{id}");
            assert!(!std::ptr::eq(p, parent), "{id}");
        }
        // `dayton93`'s program has no link code (docs/variants.md).
        assert!(link_play("dayton93").is_none());
    }

    #[test]
    fn the_records_the_games_wrote_carry_their_own_checksums() {
        for id in IDS {
            let p = link_play(id).unwrap();
            let n = p.settings.record.len();
            let rebuilt = p.settings.image(None, p.role_off, p.settings.record[p.role_off], &[]);
            assert_eq!(rebuilt[..n], *p.settings.record, "{id}");
            p.settings.crc.check(&rebuilt).unwrap_or_else(|e| panic!("{id}: {e}"));
        }
    }

    #[test]
    fn free_play_sets_the_game_bytes_and_comes_out_again() {
        for id in IDS {
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
            let mut saved = on.clone();
            p.unset_free_play(&mut saved);
            assert_eq!(saved, plain, "{id}");
            if p.settings.store == SettingsStore::Eeprom {
                let off = p.base_with_free_play(Some(&on), false).unwrap();
                assert_eq!(p.settings.image(Some(&off), p.role_off, 0, &[]), plain, "{id}");
            }
        }
    }

    #[test]
    fn roles_go_into_the_record_with_a_good_checksum() {
        for id in IDS {
            let p = link_play(id).unwrap();
            let car = p.car_off.unwrap();
            for (role, id_byte) in [(1, 0), (2, 1)] {
                let img = p.settings.image(None, p.role_off, role, &[(car, id_byte)]);
                p.settings.crc.check(&img).unwrap_or_else(|e| panic!("{id}: {e}"));
                assert_eq!((img[p.role_off], img[car]), (role, id_byte), "{id}");
                if let Some(n) = p.settings.crc.copy {
                    assert_eq!((img[p.role_off + n], img[car + n]), (role, id_byte), "{id}: the copy");
                }
            }
        }
    }
}
