use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, DriveBoard, Game, PadAxis, PadButton, Role};

/// Sega Touring Car Championship, Model 2C-CRX, with the newest of the
/// three program pairs MAME-style dumps carry (`epr-19570/19571`; the
/// `19272a/19273a` and `19272b/19273b` pairs are older revisions with the
/// same strings, not supported yet). Pairs are listed low half-word first.
/// Each bank's pairs fill 8 MB slots, smaller pairs repeating: main data
/// `.9/.10`, the second polygon pair `.18/.22` and the second texture pair
/// `.28/.26` are 1 MB, and the game addresses `.9/.10` and `.18/.22` 4 MB
/// into their slots (docs/games/stcc.md).
///
/// Beyond topskatr's layout it has a DSB1 music board (Z80 program `.2s`,
/// MPEG ROMs `.57s`-`.60s`; no board model yet, and the game runs without
/// one) and `epr-18643a.7`, a Z80 program for another cabinet board whose
/// role is unconfirmed.
pub static STCC: Game = Game {
    id: "stcc",
    title: "Sega Touring Car Championship",
    year: 1996,
    genre: "Racing",
    kind: crate::Kind::Racing,
    board: Board::Model2C,
    attract_frame: 1050,
    input_lag: 1,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: Some(DriveBoard::Wheel),
    link_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended { shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in r4
    // (docs/games/stcc.md).
    idle_loops: &[crate::IdleLoop { start: 0x763d4, end: 0x763ec }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-19570.15", 0x80000, "6f1e75d86a37b8579f968b2fb51d32a1a860697f", Role::Program),
        rom("epr-19571.16", 0x80000, "f9154cd9e954b16f7f45019b4758b1f971190437", Role::Program),
        rom("mpr-19257.11", 0x400000, "31d360dc435336942f70365d0491a2ccfc24c4c0", Role::MainData),
        rom("mpr-19258.12", 0x400000, "9c8304a1f856d1ded869ed2b86de52129510f019", Role::MainData),
        rom("epr-19270.9", 0x80000, "0490f3abc97af16e05f0dc9623e8fc635b1d4262", Role::MainData),
        rom("epr-19271.10", 0x80000, "49e7a1e6478122b4f0e679d7b336fb34044b503b", Role::MainData),
        rom("mpr-19254.27", 0x200000, "a9bdbab7b4b265c9118cf27fd45ca94f4516d5c6", Role::Textures),
        rom("mpr-19253.25", 0x200000, "f4d8a4f8278eec6d528bd947b91ebeb5223559d5", Role::Textures),
        rom("epr-19269.28", 0x80000, "fe711709e70b3743b2a0318b823d859f233d3ff8", Role::Textures),
        rom("epr-19268.26", 0x80000, "b89d39ed19a146d1e94e52682f67d2cd23d8df7f", Role::Textures),
        rom("mpr-19251.17", 0x400000, "6d472d03cd3caeb66be929c74ae63c32d305a3db", Role::Polygons),
        rom("mpr-19252.21", 0x400000, "654d5cdf44e7e1e788b26593f418ce76a5c1165a", Role::Polygons),
        rom("epr-19266.18", 0x80000, "afbbc0328bd36c34c69f0f54404dfd6a64036417", Role::Polygons),
        rom("epr-19267.22", 0x80000, "f134482ed0fcfc7b3eea39947da47081301a111a", Role::Polygons),
        rom("mpr-19255.29", 0x200000, "e6b3d8422613d22db50cf6c251f9a21356d96653", Role::CoproData),
        rom("mpr-19256.30", 0x200000, "86b2b8bb6074352f72eb81e616093a1ba6f5163f", Role::CoproData),
        rom("mpr-19259.32", 0x400000, "6e57e6e6e785b0f14bb5e221a44d518dbde7ad65", Role::Samples),
        rom("mpr-19261.34", 0x400000, "4bebcfba68b0cc2fa0bcacfaaf2d2e8af3625c5d", Role::Samples),
        rom("epr-19274.31", 0x20000, "bad26e2c994f2d4db5d9be0e34cf21a8bf5aa7e9", Role::AudioCpu),
        rom("epr-19275.2s", 0x20000, "347080858fbfe9955002f382603a1b86a52d26d5", Role::Dsb1Cpu),
        rom("mpr-19262.57s", 0x200000, "8d6872bdd46eaf2076c10d18c10af8ccbd3b10e8", Role::MpegData),
        rom("mpr-19263.58s", 0x200000, "a17b49050f1ecf1970477b12201cc3b58b31d89c", Role::MpegData),
        rom("mpr-19264.59s", 0x200000, "9e0969a1e49ec1462f69cd0f0f9ce630d66174ce", Role::MpegData),
        rom("mpr-19265.60s", 0x200000, "bedd37314ecab424b5b27030e1e7dc1b596303f3", Role::MpegData),
        rom("epr-18643a.7", 0x20000, "8182e05a2ffebd590a936c1359c81e60caa79c2a", Role::Unknown),
    ],
};

/// The twin/deluxe driving cabinet. Ports read active low. Which bit is
/// which comes from the game's input test; the effect of each, except the
/// unchecked ones noted, was confirmed by scripted runs (docs/games/stcc.md). One credit is one coin by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter"], &[PadButton::Start]),
        // Sequential shifter, used with the manual gearbox (pick MT by
        // holding the brake while deciding on the car select screen).
        button("shiftup", 1, 0x10, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Gear +"),
        button("shiftdown", 1, 0x20, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Gear −"),
        // View buttons: view 1 ("zoom in") changes the camera while racing;
        // view 2 ("zoom out") is from the input test only, not yet tried.
        button("view1", 1, 0x01, &["KeyV"], &[PadButton::North]).labelled("View"),
        button("view2", 1, 0x02, &["KeyB"], &[PadButton::South]).labelled("View 2"),
    ],
    analogs: &[
        // The wheel: 0x30 steers left, 0xD0 right. It also moves the
        // selection on the menus.
        AnalogInput {
            name: "wheel",
            channel: 0,
            centre: 0x80,
            span: 112,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
        },
        // The pedals read 0x00 released and 0xFF fully down. The gas pedal
        // also confirms a choice on the menus.
        AnalogInput {
            name: "gas",
            channel: 1,
            centre: 0x00,
            span: 255,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        AnalogInput {
            name: "brake",
            channel: 2,
            centre: 0x00,
            span: 255,
            axis: PadAxis::LeftTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
    ],
    guns: &[],
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254",
};
