use crate::controls::button;
use crate::{
    Board, Controls, Dropout, Game, PadButton, Protection, ProtectionInterface, ProtectionReply, Role, RomFile,
};

/// Dead or Alive (Tecmo, Japan), Model 2B-CRX. Pairs are listed low
/// half-word first, each in an 8 MB slot, laid out as dynabb97's: program
/// `.15/.16`; main data `.11/.12` then `.9/.10`, a second bank `.7/.8` and
/// `.5/.6`; polygons `.17/.21`, `.18/.22`, `.19/.23`; textures `.27/.25`;
/// four sample ROMs, no copro data, no music board. The Model 2A chips also
/// in `doa.zip` belong to the 2A clone `doaa`. The game checks for a
/// protection device at boot and, if it isn't answered, stops itself
/// during the first fight (docs/games/doa.md).
pub static DOA: Game = Game {
    id: "doa",
    title: "Dead or Alive",
    year: 1996,
    board: Board::Model2B,
    // The attract demo: two fighters in the ring.
    attract_frame: 2000,
    input_lag: 1,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    // The boot check (i960 0x5A0): command 6, then the 48 bytes at 0x2810
    // the game compares the reply with. Without them it arms a countdown
    // when a game starts and stops itself mid-fight (docs/games/doa.md).
    protection: Some(Protection {
        interface: ProtectionInterface::Tables,
        replies: &[ProtectionReply { command: 6, rom: 0x2810, len: 0x30 }],
    }),
    // The dump of mpr-19324.19 (the low half of the third polygon pair,
    // Kasumi's and other fighters' models) reads 60% of the 1s on data line
    // 9 as 0: its last quarter, the same data as the other two pairs' last
    // quarters, differs from them only there, always 1 to 0. In attribute
    // words that is link bit 9, which drew Kasumi as scattered shards
    // (docs/games/doa.md).
    dropouts: &[Dropout { chip: "mpr-19324.19", bit: 9 }],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended { shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls a work RAM word (0x54B150) until an
    // interrupt handler sets it, counting passes in g0 (docs/games/doa.md).
    idle_loops: &[crate::IdleLoop { start: 0x27e0, end: 0x27f4 }],
    packed: &[],
    // A coin-start run's `make sound` averages -30 dBFS RMS (peak -11),
    // louder than the games here that play at 0 dB (about -31 to -38):
    // brought down towards their middle (docs/hw/sound.md §6).
    volume_db: -3.0,
    files: &[
        RomFile {
            name: "epr-19379b.15",
            size: 0x80000,
            sha1: "c675a344f74d0118907fb5292495883c0c30c719",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19380b.16",
            size: 0x80000,
            sha1: "49250886f66db9fd37d88bc22c8f22046f74f043",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19318.11",
            size: 0x400000,
            sha1: "45b5ccf67c91014daf6bf3c4bd8ec372b246e404",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19319.12",
            size: 0x400000,
            sha1: "448b45d30cc7a71395a49a2c5789989fd7b7b4e7",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19316.9",
            size: 0x400000,
            sha1: "77ce5d8aa98bdbc97ae08a452f584b30d8885cfc",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19317.10",
            size: 0x400000,
            sha1: "3aa9d2f8afad74b5626ce2cf2d7a86aef8cac80b",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19314.7",
            size: 0x400000,
            sha1: "6a1680d6380321279b0d701e4b47d4ae712f3b72",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19315.8",
            size: 0x400000,
            sha1: "ec56df14f0847daf9bd0435f785a8946c94d2988",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19312.5",
            size: 0x200000,
            sha1: "a60fb9e7c0731004d0f0ff28c4cde272b21dd658",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19313.6",
            size: 0x200000,
            sha1: "9f375b3f4a8884163ffcf364989499f2cd21e18b",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19322.17",
            size: 0x400000,
            sha1: "1b87f6337b4286fd738856da899462e7baa92601",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19325.21",
            size: 0x400000,
            sha1: "8b31e292160b88df9c77b36096914d09ab8b6086",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19323.18",
            size: 0x400000,
            sha1: "8c0530824bb8ecb007021ee6e93412597bb0ecd6",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19326.22",
            size: 0x400000,
            sha1: "a154eb128604aac9e35438d8811971133eab94a1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19324.19",
            size: 0x400000,
            sha1: "1857ffc58697997ee22436586c398eb0c3daba6c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19327.23",
            size: 0x400000,
            sha1: "8ed74c7afd95fc7a4df0f01a47479b6f44e3073c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19321.27",
            size: 0x400000,
            sha1: "344839640d9814263fa5ed00c2043cd6f18d5cb2",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19320.25",
            size: 0x400000,
            sha1: "4c3250b9abe39fc5c8fd0fcdb5fb7ea131434516",
            role: Role::Textures,
        },
        RomFile {
            name: "epr-19328.30",
            size: 0x80000,
            sha1: "54db969fa54cf3c502d77aa6a6aaeef5d7db9f04",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-19329.31",
            size: 0x200000,
            sha1: "7a341b15afa489aa95af70cb34ac3934b1a7d887",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19330.32",
            size: 0x200000,
            sha1: "dc5870cd93da2babe5fc9c03b252fc6ea6e45721",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19331.33",
            size: 0x200000,
            sha1: "0f42458829ae85fffcedd42cd9f728a7a3d75f1c",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19332.34",
            size: 0x200000,
            sha1: "00e5677da30527b862e238f10762a5cbfbabde2b",
            role: Role::Samples,
        },
    ],
};

/// Two players, each with an 8-way stick and three buttons (hold, punch,
/// kick). Ports read active low; which bit is which comes from the game's
/// input test (docs/games/doa.md). One coin is one credit.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("hold", 1, 0x01, &["KeyZ"], &[PadButton::West]).labelled("Hold"),
        button("punch", 1, 0x02, &["KeyX"], &[PadButton::South]).labelled("Punch"),
        button("kick", 1, 0x04, &["KeyV"], &[PadButton::East]).labelled("Kick"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("hold2", 2, 0x01, &[], &[]),
        button("punch2", 2, 0x02, &[], &[]),
        button("kick2", 2, 0x04, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    // One coin, start, then punch and kick every half second: that picks
    // the character (Kasumi) and fights.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,punch@1400-99999/30,kick@1415-99999/30",
};
