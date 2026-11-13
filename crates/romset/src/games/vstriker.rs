use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role, RomFile};

/// Virtua Striker (Rev A), Model 2B-CRX: Sega's, with geometrizer and copro
/// programs of its own. Pairs are listed low half-word first: program
/// `.15/.16` then `.13/.14` (128 KB chips); main data `.11/.12`, `.9/.10`
/// and the 1 MB `.7/.8` (with the SHARC programs) **packed** back to back
/// from `0x2000000`, so `.7/.8` sits at `0x2800000`; polygons `.17/.21`,
/// `.18/.22`, packed every 4 MB; textures `.27/.25`. Its textures go to the
/// upper half of each texture RAM bank, as `fvipers`'. The zip's chips
/// without the `a` suffix belong to the original revision `vstrikro`,
/// which isn't supported (docs/games/vstriker.md).
pub static VSTRIKER: Game = Game {
    id: "vstriker",
    title: "Virtua Striker",
    year: 1994,
    board: Board::Model2B,
    // The attract mode's sweep over the stadium before kick-off.
    attract_frame: 1200,
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    // It uploads its textures to 0x11100000 and 0x11300000 only: in 2 MB
    // banks the pitch, kits and stands drew black.
    texture_ram_mirrored: true,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended { temporal: false, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/vstriker.md).
    idle_loops: &[crate::IdleLoop { start: 0x3d54, end: 0x3d68 }],
    // In 8 MB slots the copro and geometrizer uploads read zeros (the
    // program fetches them from 0x2825F30) and the stands, boards and pitch
    // lines (polygon models from 0x900000 on) were missing.
    packed: &[Role::MainData, Role::Polygons],
    // A coin-start run's `make sound` averages -32 dBFS RMS (peak -13), with
    // the games here that play at 0 dB (docs/hw/sound.md §6).
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "ep18068a.15",
            size: 0x20000,
            sha1: "2127bde1de3cd6663c31cf2126847815234e09a4",
            role: Role::Program,
        },
        RomFile {
            name: "ep18069a.16",
            size: 0x20000,
            sha1: "3cbeac09d503a19c5950cf70e3b329f791acfa13",
            role: Role::Program,
        },
        RomFile {
            name: "ep18066a.13",
            size: 0x20000,
            sha1: "33266e6372e73f670688f58e51081ec5a7deec11",
            role: Role::Program,
        },
        RomFile {
            name: "ep18067a.14",
            size: 0x20000,
            sha1: "56c8d1a365985886dffeddf24d692ce6b377760a",
            role: Role::Program,
        },
        RomFile {
            name: "mp18055.11",
            size: 0x200000,
            sha1: "40d45af7e58fa48b6afa85071c2bd1d4b5b5ffa5",
            role: Role::MainData,
        },
        RomFile {
            name: "mp18056.12",
            size: 0x200000,
            sha1: "744e5a02abd82fbeb875c5cd30c5543570140cff",
            role: Role::MainData,
        },
        RomFile {
            name: "mp18053.9",
            size: 0x200000,
            sha1: "000e9edfed49cc3dcc136f80e044dcd2b42378ce",
            role: Role::MainData,
        },
        RomFile {
            name: "mp18054.10",
            size: 0x200000,
            sha1: "c5afa62100a93e160aa96b327a260cc7fee51fdc",
            role: Role::MainData,
        },
        RomFile {
            name: "ep18070a.7",
            size: 0x80000,
            sha1: "12ead9b782e092346b7cd5a7343b302f546fe066",
            role: Role::MainData,
        },
        RomFile {
            name: "ep18071a.8",
            size: 0x80000,
            sha1: "3b35522ab8e1fdfa327245fef797e3d7c0cceb85",
            role: Role::MainData,
        },
        RomFile {
            name: "mp18057.17",
            size: 0x200000,
            sha1: "fe73e4ea310e13b172e49d39c7eafba8f9052e67",
            role: Role::Polygons,
        },
        RomFile {
            name: "mp18059.21",
            size: 0x200000,
            sha1: "fd127d33bc5a78b81aaa7d5886beca2192a62867",
            role: Role::Polygons,
        },
        RomFile {
            name: "mp18058.18",
            size: 0x200000,
            sha1: "fe783c5bc94c2581fd990f0f0a705bdc5c05a386",
            role: Role::Polygons,
        },
        RomFile {
            name: "mp18060.22",
            size: 0x200000,
            sha1: "bca83f024d85c97ca59fae8d9097fc510ec0fc7f",
            role: Role::Polygons,
        },
        RomFile {
            name: "mp18062.27",
            size: 0x200000,
            sha1: "0810364934dee8d5035cef623d01dfbacc64bf2b",
            role: Role::Textures,
        },
        RomFile {
            name: "mp18061.25",
            size: 0x200000,
            sha1: "c917046c2d98af17c59ceb0ea4f89d215cc0ead8",
            role: Role::Textures,
        },
        RomFile {
            name: "ep18072.31",
            size: 0x20000,
            sha1: "4f6d70d6e0d7b469c5f2527efb08f208f4aa017e",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mp18063.32",
            size: 0x200000,
            sha1: "da0bc8b3822b01087b6f9de0446cab1eb6617e8e",
            role: Role::Samples,
        },
        RomFile {
            name: "mp18064.33",
            size: 0x200000,
            sha1: "108b23bb57e3133c555083aa4f9bc573ac6e3152",
            role: Role::Samples,
        },
        RomFile {
            name: "mp18065.34",
            size: 0x200000,
            sha1: "2db7eabf4318881a67b10dba24f6f0cd68940ace",
            role: Role::Samples,
        },
    ],
};

/// Two players, each with an 8-way stick and three buttons: short pass
/// (green on the cabinet, also the sliding tackle when defending), shoot
/// (magenta; held, it charges a power bar) and long pass (cyan). Ports read
/// active low; which bit is which comes from the input test and the
/// attract mode's how-to-play (docs/games/vstriker.md). One
/// coin is one credit.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("pass", 1, 0x01, &["KeyZ"], &[PadButton::South]).labelled("Short pass"),
        button("shoot", 1, 0x02, &["KeyV"], &[PadButton::East]).labelled("Shoot"),
        button("long", 1, 0x04, &["KeyX"], &[PadButton::West]).labelled("Long pass"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("pass2", 2, 0x01, &[], &[]),
        button("shoot2", 2, 0x02, &[], &[]),
        button("long2", 2, 0x04, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    // One coin, start, then a short pass every half second: that takes the
    // team under the cursor (Denmark) and plays a match against France.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,pass@1400-99999/30",
};
