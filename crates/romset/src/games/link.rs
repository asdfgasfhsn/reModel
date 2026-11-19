//! Per-game link play data (`LinkPlay`): where a game keeps the setting that
//! makes a cabinet a link master or slave, and how its link RAM is laid out
//! (`docs/hw/link-board.md`). Used by `model2::link::LinkHub` and `remodel
//! link-run`; games without an entry have no linked play.

use crate::{LinkPlay, LinkSettings, RecordCrc, SettingsStore};

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
};

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
};
