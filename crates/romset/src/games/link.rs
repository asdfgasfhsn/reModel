//! Per-game link play data (`LinkPlay`): where a game keeps the setting that
//! makes a cabinet a link master or slave, and how its link RAM is laid out
//! (`docs/hw/link-board.md`). Used by `model2::link::LinkHub` and `remodel
//! link-run`; games without an entry have no linked play.

use crate::{EEPROM_BYTES, FreePlayByte, LinkPlay, LinkSettings, RecordCrc, SettingsStore};

/// The linked-play data of game `id`, if it has been looked at.
pub fn link_play(id: &str) -> Option<&'static LinkPlay> {
    if let Some(v) = crate::variants::get(id) {
        return match v.link {
            crate::variants::VariantLink::Parent => link_play(v.parent),
            crate::variants::VariantLink::None => None,
            crate::variants::VariantLink::Own(p) => Some(p),
        };
    }
    match id {
        "srallyc" => Some(&SRALLYC),
        "daytona" => Some(&DAYTONA),
        "manxtt" => Some(&MANXTT),
        "indy500" => Some(&INDY500),
        "motoraid" => Some(&MOTORAID),
        "waverunr" => Some(&WAVERUNR),
        "von" => Some(&VON),
        "stcc" => Some(&STCC),
        "skisuprg" => Some(&SKISUPRG),
        "sgt24h" => Some(&SGT24H),
        "overrev" => Some(&OVERREV),
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
    node_bytes: None,
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
    node_bytes: None,
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
    node_bytes: None,
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
    node_bytes: None,
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
    free_play: Some(MOTORAID_FREE_PLAY),
    node_bytes: None,
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
    free_play: Some(WAVERUNR_FREE_PLAY),
    node_bytes: None,
};

/// `motoraid` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY" (docs/games/motoraid.md): what the
/// game's own test menu changed in the record going from its default #12 (2 coins 1 credit), found by
/// running it (8 presses of the test button): the setting's number (byte `0x10`, 0-based) and two bytes
/// the game derives from it.
static MOTORAID_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x0b },
    FreePlayByte { at: 0x11, on: 0x01, off: 0x02 },
    FreePlayByte { at: 0x15, on: 0x01, off: 0x00 },
];

/// `waverunr` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY" (docs/games/waverunr.md): from the
/// default #17 (3 coins 1 credit), 6 presses of the test button; the same three bytes as `motoraid`'s.
static WAVERUNR_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x10 },
    FreePlayByte { at: 0x11, on: 0x01, off: 0x03 },
    FreePlayByte { at: 0x15, on: 0x01, off: 0x00 },
];

/// `sgt24h`'s settings image: the EEPROM's 128 bytes (the record the game's own test menu saves
/// at the defaults, in the first `0x48`: the record and its copy) and, from [`EEPROM_BYTES`], the backup SRAM's first
/// `0x14c` bytes as the game leaves them after a first boot (its marker `0xAD85`, the coin
/// settings, the bookkeeping at zero): the coin settings, FREE PLAY among them, are there, not in
/// the EEPROM (docs/games/sgt24h.md).
static SGT24H_IMAGE: [u8; EEPROM_BYTES + 0x14c] = {
    const EEPROM: [u8; 0x48] = [
        0x53, 0x33, 0x32, 0x41, 0x06, 0x49, 0x00, 0x00, 0xd8, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x85, 0xad,
        0x01, 0x00, 0x00, 0x00, 0x00, 0x02, 0x00, 0x01, 0x01, 0x01, 0x00, 0x30, 0x80, 0xd0, 0x48, 0x80, 0xc8, 0x48,
        0x80, 0xc8, 0x00, 0x00, 0xa6, 0x04, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
        0x00, 0x02, 0x00, 0x01, 0x01, 0x01, 0x00, 0x30, 0x80, 0xd0, 0x48, 0x80, 0xc8, 0x48, 0x80, 0xc8, 0x00, 0x00,
    ];
    const SRAM: [u8; 0x14c] = [
        0x85, 0xad, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff, 0xff, 0xff,
        0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0xff,
        0xff, 0xff, 0xff, 0xff, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x32, 0x00, 0x1e, 0x00, 0x32, 0x00,
        0x1e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00,
        0x32, 0x00, 0x1e, 0x00, 0x32, 0x00, 0x1e, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x6d, 0x03, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x02, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
    ];
    let mut a = [0xff; EEPROM_BYTES + 0x14c];
    let mut i = 0;
    while i < EEPROM.len() {
        a[i] = EEPROM[i];
        i += 1;
    }
    i = 0;
    while i < SRAM.len() {
        a[EEPROM_BYTES + i] = SRAM[i];
        i += 1;
    }
    a
};

/// `sgt24h` COIN ASSIGNMENTS, FREE PLAY ON (docs/games/sgt24h.md): the byte at `0x12` of the backup
/// SRAM (offset `0x80 + 0x12` in the settings image) goes from 0 to 1; found by running the game's
/// test menu and diffing the SRAM.
static SGT24H_FREE_PLAY: &[FreePlayByte] = &[FreePlayByte { at: EEPROM_BYTES + 0x12, on: 0x01, off: 0x00 }];

/// `sgt24h` (docs/games/sgt24h.md, docs/hw/eeprom-records.md section 3, link-board.md section
/// 14): GAME ASSIGNMENTS LINK TYPE and LINK MAX in the "S32A" record's bytes `0x19`-`0x1C`:
/// `0x19` LINK MAX (the number itself, 2 to 4; 1 with NOT LINK), `0x1A` 0 once a type is chosen
/// (1 NOT LINK), `0x1B` 1 for CAR NO1 (the master) and 0 for the slaves, `0x1C` the car number
/// minus one (0-3). The record is 32 bytes (copies at `0x08` and `0x28`), its checksum the byte
/// sum of the 30 bytes after it, and the second copy has the marker word `0xAD85` at `0x10`
/// zeroed. The bytes are what the game's own test menu writes at the defaults. Blocks are `0x380`
/// bytes (own at `0x2000`, received from `0x2380`) and the game writes its ID to `0x1A10001`.
/// `sgt24h` (docs/games/sgt24h.md, docs/hw/eeprom-records.md section 3, link-board.md section
/// 14): GAME ASSIGNMENTS LINK TYPE and LINK MAX in the "S32A" record's bytes `0x19`-`0x1C`:
/// `0x19` LINK MAX (the number itself, 2 to 4; 1 with NOT LINK), `0x1A` 0 once a type is chosen
/// (1 NOT LINK), `0x1B` 1 for CAR NO1 (the master) and 0 for the slaves, `0x1C` the car number
/// minus one (0-3). The record is 32 bytes (copies at `0x08` and `0x28`), its checksum the byte
/// sum of the 30 bytes after it, and the second copy has the marker word `0xAD85` at `0x10`
/// zeroed. The bytes are what the game's own test menu writes at the defaults. Blocks are `0x380`
/// bytes (own at `0x2000`, received from `0x2380`) and the game writes its ID to `0x1A10001`.
static SGT24H: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &SGT24H_IMAGE,
        fill: 0xff,
        size: SGT24H_IMAGE.len(),
        crc: RecordCrc::byte_sum(0x08, 32, 30, 8),
    },
    role_off: 0x1b,
    roles: &[1, 0],
    car_off: Some(0x1c),
    tx: 0x2000,
    rx: 0x2380,
    block: 0x380,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04)],
    ghost_sum: None,
    free_play: Some(SGT24H_FREE_PLAY),
    node_bytes: Some(|_, nodes| vec![(0x19, nodes as u8), (0x1a, 0)]),
};

/// `overrev` and its clone `overrevb` (docs/games/overrev.md, link-board.md section 14): the
/// "S32A" record in the EEPROM, 52 bytes (copies at `0x08` and `0x3C`). GAME ASSIGNMENTS
/// FOUNDATIONAL SETTING: LINK MAX byte `0x36` (0 NOT LINK, 1 2 LINKS, 2 3 LINKS, 3 4 LINKS) and
/// LINK TYPE byte `0x35` (0 MASTER CarNo.1, 1 SLAVE CarNo.2, ...). The bytes are what the game's own
/// test menu writes at the defaults. Blocks as `sgt24h`'s; the master does not write `0x1A14002`.
static OVERREV: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x06, 0x49, 0x00, 0x00, 0x93, 0x77, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x01, 0x01, 0x01, 0x00, 0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x00,
            0x80, 0xff, 0x00, 0x00, 0x02, 0x02, 0x01, 0x02, 0x01, 0x03, 0x03, 0x02, 0x06, 0x02, 0x02, 0x01, 0x03, 0x00,
            0x00, 0x02, 0x02, 0x01, 0x01, 0x00,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::s32a(0x08, 52, 50, 0xdebd_eb00),
    },
    role_off: 0x36,
    roles: &[1],
    car_off: Some(0x35),
    tx: 0x2000,
    rx: 0x2380,
    block: 0x380,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04)],
    ghost_sum: None,
    free_play: Some(OVERREV_FREE_PLAY),
    node_bytes: Some(|_, nodes| vec![(0x36, nodes as u8 - 1)]),
};

/// `overrev` COIN ASSIGNMENTS, COIN/CREDIT SETTING # "FREEPLAY" (docs/games/overrev.md): the setting's
/// list has 15 entries (0 to 14, `1 COIN 1 CREDIT` the default) and ends in FREEPLAY, 14 presses of the
/// test button from the default: the setting's number (byte `0x10`) and one byte derived from it.
static OVERREV_FREE_PLAY: &[FreePlayByte] =
    &[FreePlayByte { at: 0x10, on: 0x0e, off: 0x00 }, FreePlayByte { at: 0x11, on: 0x01, off: 0x00 }];

/// `von`'s settings in the backup SRAM (docs/games/von.md, link-board.md section 15): the record
/// is 0x20C bytes starting `SEGA.AM#3.VRON.`, its length word, a checksum word at `0x14` over
/// `0x16..0x38` (the operator settings, NETWORK LINK ATTRIBUTE at `0x28`), then bookkeeping and
/// the ranking tables, and a second copy right after. The bytes are what the game writes at its
/// first boot, its NETWORK LINK ATTRIBUTE stand-alone (2).
const fn von_record() -> [u8; 0x418] {
    let mut r = [0u8; 0x418];
    let head: [u8; 0x38] = [
        b'S', b'E', b'G', b'A', b'.', b'A', b'M', b'#', b'3', b'.', b'V', b'R', b'O', b'N', b'.', 0, //
        0x0c, 0x02, 0, 0, 0x02, 0xa0, 0x11, 0x0b, 0x11, 0x11, 0x11, 0, 0, 0, 0, 0, //
        0x01, 0, 0x02, 0, 0, 0x03, 0x01, 0, 0x02, 0, 0x01, 0, 0, 0, 0, 0, //
        0x01, 0, 0x01, 0, 0, 0x01, 0, 0xff,
    ];
    let mut i = 0;
    while i < 0x38 {
        r[i] = head[i];
        i += 1;
    }
    let mut k = 0x60;
    while k < 0x64 {
        r[k] = 0xff;
        k += 1;
    }
    // The ranking tables: ten 8-byte entries, then ten of 12 ("MSB", a score).
    let mut e = 0;
    while e < 10 {
        let at = 0x144 + 8 * e;
        (r[at], r[at + 1], r[at + 2], r[at + 4], r[at + 5], r[at + 6]) = (b'M', b'S', b'B', 0xd0, 0x64, 0x04);
        let at = 0x194 + 12 * e;
        (r[at], r[at + 1], r[at + 2], r[at + 4]) = (b'M', b'S', b'B', 5);
        e += 1;
    }
    // The second copy.
    let mut j = 0;
    while j < 0x20c {
        r[0x20c + j] = r[j];
        j += 1;
    }
    r
}

static VON_RECORD: [u8; 0x418] = von_record();

/// `von` (docs/games/von.md, link-board.md section 15): NETWORK LINK ATTRIBUTE is the backup SRAM
/// byte `0x1D00028`, 0 SLAVE, 1 MASTER, 2 NOLINK; two machines (the game rejects any other
/// total). The SRAM record's checksum is the game's own routine's (`SumKind::Table`).
static VON: LinkPlay = LinkPlay {
    max_nodes: 2,
    settings: LinkSettings {
        store: SettingsStore::BackupRam,
        record: &VON_RECORD,
        fill: 0xff,
        size: 0x418,
        crc: RecordCrc::table(0x14, 0x16, 0x38, 0xdebd_eb00, 0x20c),
    },
    role_off: 0x28,
    roles: &[1, 0],
    car_off: None,
    tx: 0x2000,
    rx: 0x2700,
    block: 0x700,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04)],
    ghost_sum: None,
    free_play: Some(VON_FREE_PLAY),
    node_bytes: None,
};

/// `von` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY" (docs/games/von.md): what the
/// game's own test menu changed in the record going from its default #1 (1 coin 1 credit), found
/// by running it: the setting's number (byte `0x2A`, 1-based here) and two flags the game derives
/// from it (the checksum and the second copy follow).
static VON_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x2a, on: 0x1b, off: 0x01 },
    FreePlayByte { at: 0x34, on: 0x01, off: 0x00 },
    FreePlayByte { at: 0x36, on: 0x01, off: 0x00 },
];

/// `stcc` (docs/games/stcc.md, docs/hw/eeprom-records.md section 4, link-board.md section 16):
/// the `SEGASTCb` record in the EEPROM: the magic, a CRC-16 at `0x08` (CCITT, start `0xFFFF`,
/// complemented, over `0x10`-`0x7F`), a write counter at `0x0C`, then the settings from `0x10`.
/// LINK TYPE is bits 5-7 of byte `0x10` (0 STAND ALONE, 1-7 CAR 1-7; bit 0 is another setting,
/// which stays at its default 1). The bytes are what the game writes at its first boot.
static STCC: LinkPlay = LinkPlay {
    max_nodes: 7,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x45, 0x47, 0x41, 0x53, 0x54, 0x43, 0x62, 0x0f, 0xa6, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0xf1, 0xff, 0x50, 0xff, 0xff, 0xff, 0x02, 0x00, 0xfc, 0xff, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x01, 0x00, 0x01, 0x00, 0x09, 0x00,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::ccitt(8, 0x10, 0x80, 0xffff, true),
    },
    role_off: 0x10,
    roles: &[0x21, 0x41, 0x61, 0x81, 0xa1, 0xc1, 0xe1],
    car_off: None,
    tx: 0x2000,
    rx: 0x21c0,
    block: 0x1c0,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x04)],
    ghost_sum: None,
    free_play: Some(STCC_FREE_PLAY),
    node_bytes: None,
};

/// `stcc` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY" (docs/games/stcc.md): what the
/// game's own test menu changed in the record going from its default #1 (1 coin 1 credit), found
/// by running it: the setting's word (bytes `0x18`-`0x19`; the write counter and the checksum
/// follow).
static STCC_FREE_PLAY: &[FreePlayByte] =
    &[FreePlayByte { at: 0x18, on: 0x8a, off: 0x02 }, FreePlayByte { at: 0x19, on: 0x06, off: 0x00 }];

/// The same in `stcca` and `stccb`, whose records are four bytes shorter there.
static STCCAB_FREE_PLAY: &[FreePlayByte] =
    &[FreePlayByte { at: 0x14, on: 0x8a, off: 0x02 }, FreePlayByte { at: 0x15, on: 0x06, off: 0x00 }];

/// `stcca` (Rev A) and `stccb` (Rev B): the record's magic is `SEGASTCa` and it is four bytes
/// shorter (the settings from `0x10` are `01 00 f1/d1 50 02 00 fc ff ...`), the checksum rule, the
/// role byte (`0x10`, bits 5-7) and the link RAM are the parent's (docs/games/stcc.md).
pub static STCCA: LinkPlay = LinkPlay {
    settings: LinkSettings {
        record: &[
            0x53, 0x45, 0x47, 0x41, 0x53, 0x54, 0x43, 0x61, 0x7a, 0xd3, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0xf1, 0x50, 0x02, 0x00, 0xfc, 0xff, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
            0x09, 0x00,
        ],
        ..STCC.settings
    },
    free_play: Some(STCCAB_FREE_PLAY),
    ..STCC
    node_bytes: None,
};

/// `stccb`: as [`STCCA`], one settings byte (`0x12`) different.
pub static STCCB: LinkPlay = LinkPlay {
    settings: LinkSettings {
        record: &[
            0x53, 0x45, 0x47, 0x41, 0x53, 0x54, 0x43, 0x61, 0x16, 0x60, 0xff, 0xff, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00,
            0xd1, 0x50, 0x02, 0x00, 0xfc, 0xff, 0x01, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00,
            0x09, 0x00,
        ],
        ..STCC.settings
    },
    free_play: Some(STCCAB_FREE_PLAY),
    ..STCC
    node_bytes: None,
};

/// `skisuprg` (docs/games/skisuprg.md, docs/hw/eeprom-records.md, link-board.md section 17): the
/// "S32A" record of the 2C Sega games with a 40-byte record at `0x08` (copy at `0x30`, start value
/// `0xDEBDEB00`); NETWORK TYPE is byte `0x20` (0 NO LINK, 1 MASTER, 2 SLAVE) and CABINET ID byte
/// `0x21` (0-based here, shown as 1 to 4). The bytes are what the game's own test menu writes at
/// the defaults.
static SKISUPRG: LinkPlay = LinkPlay {
    max_nodes: 4,
    settings: LinkSettings {
        store: SettingsStore::Eeprom,
        record: &[
            0x53, 0x33, 0x32, 0x41, 0x41, 0x19, 0x00, 0x00, 0x63, 0x8e, 0x00, 0x00, 0x65, 0x00, 0x00, 0x00, 0x0b, 0x00,
            0x02, 0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x28, 0x0a, 0x0a, 0x0a, 0x0a, 0x01, 0x00, 0x00, 0x01, 0x01,
            0x01, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x00, 0x00, 0x63, 0x8e, 0x00, 0x00, 0x65, 0x00,
            0x00, 0x00, 0x0b, 0x00, 0x02, 0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x28, 0x0a, 0x0a, 0x0a, 0x0a, 0x01,
            0x00, 0x00, 0x01, 0x01, 0x01, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x00, 0x80, 0xff, 0x00, 0x00,
        ],
        fill: 0xff,
        size: 128,
        crc: RecordCrc::s32a(0x08, 40, 39, 0xdebd_eb00),
    },
    role_off: 0x20,
    roles: &[1, 2],
    car_off: Some(0x21),
    tx: 0x2000,
    rx: 0x2380,
    block: 0x380,
    id_base: 1,
    state_off: None,
    pos_off: None,
    ghost_keep: &[(0x00, 0x08)],
    ghost_sum: None,
    free_play: Some(SKISUPRG_FREE_PLAY),
    node_bytes: None,
};

/// `skisuprg` COIN ASSIGNMENTS, COIN/CREDIT SETTING #27 "FREE PLAY" (docs/games/skisuprg.md): what
/// the game's own test menu changed in the record going from its default #12 (2 coins 1 credit),
/// found by running it: the setting's number (byte `0x10`, 0-based) and two bytes the game derives
/// from it (and the write counter, which the game raises by itself).
static SKISUPRG_FREE_PLAY: &[FreePlayByte] = &[
    FreePlayByte { at: 0x10, on: 0x1a, off: 0x0b },
    FreePlayByte { at: 0x11, on: 0x01, off: 0x00 },
    FreePlayByte { at: 0x12, on: 0x01, off: 0x02 },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_play_sets_the_game_bytes_and_comes_out_again() {
        for id in [
            "srallyc", "daytona", "manxtt", "indy500", "motoraid", "waverunr", "von", "stcc", "stcca", "stccb", "skisuprg", "sgt24h",
            "overrev",
        ] {
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
        for id in [
            "daytona", "manxtt", "indy500", "motoraid", "waverunr", "von", "stcc", "stcca", "stccb", "skisuprg", "sgt24h", "overrev",
        ] {
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
        for id in ["indy500", "motoraid", "waverunr", "skisuprg", "overrev"] {
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
