use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role, RomFile};

/// Sonic Championship (USA), Model 2B-CRX: Sega's, uploading the
/// geometrizer program the other 2B games share and a copro program of its
/// own, built like `fvipers`'. Pairs are listed low half-word first:
/// program `.15/.16`; main data `.11/.12`, `.9/.10`; the 1 MB `.7/.8` in
/// the second data bank (with the SHARC programs); polygons `.17/.21`,
/// `.18/.22`; textures `.27/.25`, `.28/.26`; copro data `.29/.30`
/// (`fvipers`' chips). The program and `.7/.8` chips of the clone `sfight`
/// (Sonic the Fighters) in `schamp.zip` aren't supported (docs/games/schamp.md).
pub static SCHAMP: Game = Game {
    id: "schamp",
    title: "Sonic Championship",
    year: 1996,
    board: Board::Model2B,
    // The attract demo: two fighters in the desert ring.
    attract_frame: 3500,
    input_lag: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    // It uploads its textures to 0x11100000 and 0x11300000 only.
    texture_ram_mirrored: true,
    shading: crate::Shading::DEFAULT,
    packed: &[],
    // A coin-start run's `make sound` averages -28 dBFS RMS (peak -8), louder
    // than the games here that play at 0 dB (about -31 to -38): brought
    // down towards their middle, as doa (docs/hw/sound.md §6).
    volume_db: -4.0,
    files: &[
        RomFile {
            name: "epr-19141.15",
            size: 0x80000,
            sha1: "2372412d49349894c99d545313c12413c2d1ec86",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19142.16",
            size: 0x80000,
            sha1: "9456fb9a847e01548fc30d36ef161325788653d5",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19007.11",
            size: 0x400000,
            sha1: "5343a9a2502052e3587424c984bd48caa7564849",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19008.12",
            size: 0x400000,
            sha1: "39ad2e9431543ea6cbc0307bc39933cf64956a74",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19005.9",
            size: 0x400000,
            sha1: "300c9cdef199f31255bacb95399e9c75be73f817",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19006.10",
            size: 0x400000,
            sha1: "37a4ff13cfccfda587ca59a9ef08b5914d2c28d4",
            role: Role::MainData,
        },
        RomFile {
            name: "epr-19143.7",
            size: 0x80000,
            sha1: "8c9d871d4639563f8298a7f93032e07a8f863faa",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-19144.8",
            size: 0x80000,
            sha1: "950ad9174196e776881439545983f91655922a49",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19015.29",
            size: 0x200000,
            sha1: "9914be9925b86af6af670745b5eba3a9e4f24af9",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19016.30",
            size: 0x200000,
            sha1: "a6f0f589ad174a34493ee24dc0cb509ead3aed70",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19009.17",
            size: 0x400000,
            sha1: "5af3a90c87ec8a90a8fc58ae469ef23ec6e6213c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19012.21",
            size: 0x400000,
            sha1: "8e13a0bb34e187a340b38d76ab15ff6fe4bae764",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19010.18",
            size: 0x400000,
            sha1: "e3318ef0eb0168998e139e527339c7c667c17fb1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19013.22",
            size: 0x400000,
            sha1: "a6c4b2b3bf8efc6f6263f73d6f4cacf9785010c1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19019.27",
            size: 0x400000,
            sha1: "c29bedb41b14d63c6067ae12ad009deaafca2aa4",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19017.25",
            size: 0x400000,
            sha1: "52fad61412040c90c7dd300c0fd7aa5b8d5af441",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19020.28",
            size: 0x400000,
            sha1: "7b9a75caa8c5b12ba54c6f4f746d80b165ee97ab",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19018.26",
            size: 0x400000,
            sha1: "9c707a7c2cffc5eff19f9919ddfae7300842fd19",
            role: Role::Textures,
        },
        RomFile {
            name: "epr-19021.31",
            size: 0x80000,
            sha1: "21290389cd8bd9e52ed438152cc6cb5793f809d3",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-19022.32",
            size: 0x200000,
            sha1: "43a21609b49926a227558d4938088526acf1fe42",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19023.33",
            size: 0x200000,
            sha1: "696dc85e066fb27c7618e52e0acd0d00451e4589",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19024.34",
            size: 0x200000,
            sha1: "b431bd85c973aa0a4d6032ac98fb057139f142a2",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19025.35",
            size: 0x200000,
            sha1: "b666d31f9be26eb0cdcb71041a3c3c08d5aa41e1",
            role: Role::Samples,
        },
    ],
};

/// Two players, each with an 8-way stick and three buttons (punch, kick,
/// barrier). Ports read active low; which bit is which comes from the game's
/// input test (docs/games/schamp.md). One coin is one
/// credit.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("punch", 1, 0x01, &["KeyX"], &[PadButton::South]).labelled("Punch"),
        button("kick", 1, 0x02, &["KeyV"], &[PadButton::East]).labelled("Kick"),
        button("barrier", 1, 0x04, &["KeyZ"], &[PadButton::West]).labelled("Barrier"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("punch2", 2, 0x01, &[], &[]),
        button("kick2", 2, 0x02, &[], &[]),
        button("barrier2", 2, 0x04, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    // One coin, start, then punch and kick every half second: that picks
    // the fighter under the cursor (Sonic) and fights.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,punch@1400-99999/30,kick@1415-99999/30",
};
