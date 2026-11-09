use crate::controls::button;
use crate::{Board, Controls, Game, Gun, GunSource, PadButton, Reload, Role, RomFile};

/// The House of the Dead, Model 2C-CRX (Japan). Pairs are listed low
/// half-word first, each bank's pairs in 8 MB slots. Program `.15/.16` and
/// `.13/.14`; main data `.11/.12` at `0x2000000` and `.9/.10` at
/// `0x2800000` (which holds both MB86235 programs), and a second data bank
/// as in segawski: `.7/.8` at `0x6000000` and the 1 MB `.5/.6` at
/// `0x6800000`. The memory test passes every ROM in this layout. Two light
/// guns, read through the 2C gun interface (`model2::gun`; README, "What's
/// known about hotd"). No drive or music board.
pub static HOTD: Game = Game {
    id: "hotd",
    title: "The House of the Dead",
    year: 1996,
    board: Board::Model2C,
    // The attract demo: a zombie close up, with the HUD.
    attract_frame: 4080,
    input_lag: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    packed: &[],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-19696.15",
            size: 0x80000,
            sha1: "be0bd34a9216375c7204445f084f6c74c4d3b0c8",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19697.16",
            size: 0x80000,
            sha1: "0b14f9a81272f79a5b294bc024711042c5fb2637",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19694.13",
            size: 0x80000,
            sha1: "3d688be98f78fe40c2af1e91df6decd500400ae9",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19695.14",
            size: 0x80000,
            sha1: "bc96ab2a4ba7f30c0b89814acc8931c8bf800a82",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19704.11",
            size: 0x400000,
            sha1: "24e63f4392847f288971469cd10448536eb435d4",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19705.12",
            size: 0x400000,
            sha1: "bee4f43b3ad15d93a2f9f07b873c9cf5d228e2f9",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19702.9",
            size: 0x400000,
            sha1: "b64afb17d9c97277d8c4f20811f14f65a61cbb56",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19703.10",
            size: 0x400000,
            sha1: "e5c45ea5621f99661a87ffe88e24764d2bbcb51e",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19700.7",
            size: 0x400000,
            sha1: "94440839d3325176c2d03f39a78949d0ef040bba",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19701.8",
            size: 0x400000,
            sha1: "933770546d46abca400e7f524eff2ae89241e56d",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-19698.5",
            size: 0x80000,
            sha1: "77cb53f8730fdb55080b70910ab8c750d79acb02",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-19699.6",
            size: 0x80000,
            sha1: "9dab483c60624dddba8085e94a4325739592ec17",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19718.27",
            size: 0x400000,
            sha1: "3ebac2aeb1467939337c9a5c87ad9c293560dae2",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19716.25",
            size: 0x400000,
            sha1: "f602cabd879c69afee544848feafb9fb9f5d51e2",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19719.28",
            size: 0x400000,
            sha1: "fe6622b5917f9a99c097fd60d9446ac6b481fa75",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19717.26",
            size: 0x400000,
            sha1: "927ac9cad22f87b339cc86043678470ff139ce1f",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19715.17",
            size: 0x400000,
            sha1: "0a61b091bb0bc659f0cbca8ad401d0925a1dc2ea",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19711.21",
            size: 0x400000,
            sha1: "4167428a2a903aea2c14631ccf924afb81338b89",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19714.18",
            size: 0x400000,
            sha1: "70b4c1627db80e6734112c02265495e2b4a53278",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19710.22",
            size: 0x400000,
            sha1: "3cc59bb4910aa5382e95762f63325c06b763bd23",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19713.19",
            size: 0x400000,
            sha1: "b6d0be283c25235249186751c7f025a7c38d2f36",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19709.23",
            size: 0x400000,
            sha1: "c92571e35960f27dc8b0b059f12167026d0666d1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19712.20",
            size: 0x400000,
            sha1: "25a0d921c8662043c5860dc7a226d4895ff9fff6",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19708.24",
            size: 0x400000,
            sha1: "d3cae450186bc62fd746b14d6a05cb397efcfe40",
            role: Role::Polygons,
        },
        RomFile {
            name: "epr-19707.29",
            size: 0x80000,
            sha1: "6d060378d0f801b04d12e7ee874f2fa0572992d9",
            role: Role::CoproData,
        },
        RomFile {
            name: "epr-19706.30",
            size: 0x80000,
            sha1: "08d3e733ba9989fcd32290634171c73f26ab6e2b",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19721.32",
            size: 0x400000,
            sha1: "6836973a687c59dd80f8e6c30d33155e306be199",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19722.34",
            size: 0x400000,
            sha1: "405a892bc368ba862ba71bda7525b421d6973c0e",
            role: Role::Samples,
        },
        RomFile {
            name: "epr-19720.31",
            size: 0x80000,
            sha1: "1edaed489a3518ddad85728e416319f940ea02bb",
            role: Role::AudioCpu,
        },
    ],
};

/// Two light guns, from the game's input test and gun setting screens.
/// Ports read active low. One credit is two coins by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("trigger1", 1, 0x01, &["Space"], &[]).labelled("Trigger"),
        button("trigger2", 1, 0x02, &[], &[]).labelled("Trigger 2"),
    ],
    analogs: &[],
    // Each gun's X and Y are counter pairs of the gun interface (Y first:
    // the game reads counters 0/1 as player 1's Y), and counter 8 has a
    // "sees no screen" bit per player; shooting with it set reloads. The
    // ranges put GUN MARK CHECK's mark on the aimed pixel with the game's
    // default calibration (a fresh EEPROM).
    guns: &[
        Gun {
            name: "gun1",
            source: GunSource::Counters { x: 2, y: 0, offscreen_bit: 0 },
            x_range: (141, 629),
            y_range: (56, 410),
            trigger: "trigger1",
            reload: Reload::Offscreen,
            keys_reload: &["KeyR"],
        },
        Gun {
            name: "gun2",
            source: GunSource::Counters { x: 6, y: 4, offscreen_bit: 1 },
            x_range: (131, 630),
            y_range: (56, 410),
            trigger: "trigger2",
            reload: Reload::Offscreen,
            keys_reload: &[],
        },
    ],
    // Two coins and start, then aim at the middle and either side in turn,
    // fire every 20 frames and shoot off screen (reload) every seventh shot.
    shifter: None,
    coin_start: "coin1@1100-1104,coin1@1120-1124,start@1250-1254,gun1=248:170@2200-6400,gun1=140:190@2280-6400/120,gun1=360:190@2340-6400/120,gun1=off@2260-6400/140,trigger1@2260-6400/20",
};
