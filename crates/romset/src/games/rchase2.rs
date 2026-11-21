use super::rom;
use crate::controls::{button, AnalogInput};
use crate::{Board, Controls, Game, PadAxis, PadButton, Role};

/// Rail Chase 2 (Sega, 1995), Model 2B-CRX: the program writes both SHARCs'
/// host windows (`0x8C0000`, `0x840000`) and the 2B sound UART (`0x9C0000`)
/// (`docs/games/rchase2.md`). Pairs are listed low half-word first: program
/// `.15/.16` then `.13/.14`; main data `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6`
/// **packed** back to back from `0x2000000` (2 MB chips: the pairs sit at
/// `0x2000000`, `0x2400000`, `0x2800000`, `0x2C00000`, where the program's
/// pointers land); polygons `.17/.21` then `.18/.22` packed; textures
/// `.27/.25`; sound program `.31`, samples `.32` then `.34`. The memory
/// test passes every ROM in this layout. No copro data ROMs: both SHARC
/// programs are its own (the geometrizer's is the shared one the native
/// geometrizer runs). The ride's drive board is linked "by network"
/// (`DriveBoard::RideLink`); the controls are a stick and a shot button a
/// player, not a mounted gun.
pub static RCHASE2: Game = Game {
    id: "rchase2",
    title: "Rail Chase 2",
    year: 1995,
    genre: "Rail shooter",
    kind: crate::Kind::Shooting,
    board: Board::Model2B,
    // The title screen over the cliff-side track.
    attract_frame: 4000,
    scenes: &[],
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: Some(crate::DriveBoard::RideLink),
    link_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/rchase2.md).
    idle_loops: &[crate::IdleLoop { start: 0xd798, end: 0xd7b0 }],
    // In 8 MB slots the program's pointers (0x2600000, 0x2C00000...) would
    // land in repeats of the first pair.
    packed: &[Role::MainData, Role::Polygons],
    // A coin-start run's `make sound` averages -28.5 dBFS RMS (peak -6),
    // louder than the games here that play at 0 dB: brought down as
    // schamp's (docs/hw/sound.md §6).
    volume_db: -4.0,
    files: &[
        rom("epr-18045a.15", 0x80000, "9eb0f2cdab8c10fda9edc0ddc439263af3903cdc", Role::Program),
        rom("epr-18046a.16", 0x80000, "fee8436399fb97ad5b8357b81e69bd5c27af1dde", Role::Program),
        rom("epr-18074a.13", 0x80000, "d41cb8efd9fd65eea9e7aefadebfd0a27ef145fb", Role::Program),
        rom("epr-18075a.14", 0x80000, "519fdb5a978b6e82989b9841c6b59819f0d417cb", Role::Program),
        rom("mpr-18037.11", 0x200000, "8eb45e46bd14a2ffbdaac47d381a1ea9b9a03ca2", Role::MainData),
        rom("mpr-18038.12", 0x200000, "cbfa687839b6cad6a5ace45b44b95c45e4cfab0d", Role::MainData),
        rom("mpr-18039.9", 0x200000, "dd1ff9c682778de1c6c09e7a5cbc95a8149488c4", Role::MainData),
        rom("mpr-18040.10", 0x200000, "33e56486ec6b953341552b6bc21dc66f6f8aaf74", Role::MainData),
        rom("mpr-18041.7", 0x200000, "c6ea14b3bdefdc59603bd2fc152ac0421fae4d6f", Role::MainData),
        rom("mpr-18042.8", 0x200000, "0204ba86af2707bc9e277cac68dd9ef759189c23", Role::MainData),
        rom("mpr-18043.5", 0x200000, "82833bf4cb1f367aea5fec6cffb7023cbbd3c8cb", Role::MainData),
        rom("mpr-18044.6", 0x200000, "62e95ceea6f71eedbebae59e188aac03e6129e62", Role::MainData),
        rom("mpr-18031.17", 0x200000, "2d0339dd7eeb2625f78e2fbe4ebdc976967175a4", Role::Polygons),
        rom("mpr-18032.21", 0x200000, "9510104975192a0ef1750251636daff7f089feb9", Role::Polygons),
        rom("mpr-18033.18", 0x200000, "7dee991f0c43de9bfe17ae44767f65f12e83c811", Role::Polygons),
        rom("mpr-18034.22", 0x200000, "48227544209412fca3035e85a00d33ea654dc7b5", Role::Polygons),
        rom("mpr-18035.27", 0x200000, "c1f8dda4781dea00bd97dbf9ecfbb626dadd2c35", Role::Textures),
        rom("mpr-18036.25", 0x200000, "e39644a08aa631dbdcfc7c0dc356e73f6a4412a9", Role::Textures),
        rom("epr-18047.31", 0x80000, "424d5e5a7787d0d4c68aa919ba7d575babfd1ce0", Role::AudioCpu),
        rom("mpr-18029.32", 0x200000, "ef40c11008c75d04159772ad30f02cdb8c5464f3", Role::Samples),
        rom("mpr-18030.34", 0x200000, "bae0060aec3c15f08342f11df665c05c5703523d", Role::Samples),
    ],
};

/// Per player a stick (two ADC channels) and a shot button, from the game's
/// input test (docs/games/rchase2.md): port 0 the coins, test, service and
/// starts, port 1 the shots. The stick's X channel reads high to the left
/// and its Y high upwards (the reticle in play), centre 0x80, to 0x40 and
/// 0xC0 at the ends. Ports read active low. One coin a credit; a one-player
/// ride costs two.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("shot", 1, 0x01, &["KeyX", "Space"], &[PadButton::South, PadButton::RightShoulder]).labelled("Shot"),
        button("shot2", 1, 0x02, &[], &[]),
    ],
    analogs: &[
        AnalogInput {
            name: "stickx",
            channel: 1,
            centre: 0x80,
            span: -0x40,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
        },
        AnalogInput {
            name: "sticky",
            channel: 3,
            centre: 0x80,
            span: -0x40,
            axis: PadAxis::LeftStickY,
            keys_neg: &["ArrowUp"],
            keys_pos: &["ArrowDown"],
            pad_neg: &[PadButton::DPadUp],
            pad_pos: &[PadButton::DPadDown],
            key_ramp: None,
        },
        // Player 2's stick: the same on the next channels, unbound.
        AnalogInput {
            name: "stick2x",
            channel: 0,
            centre: 0x80,
            span: -0x40,
            axis: PadAxis::None,
            keys_neg: &[],
            keys_pos: &[],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        AnalogInput {
            name: "stick2y",
            channel: 2,
            centre: 0x80,
            span: -0x40,
            axis: PadAxis::None,
            keys_neg: &[],
            keys_pos: &[],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
    ],
    guns: &[],
    shifter: None,
    // Two coins (the one-player ride costs two credits) and start, then a
    // shot every 20 frames with the stick centred.
    coin_start: "coin1@1100-1104,coin1@1150-1154,start@1250-1254,shot@1400-99999/20",
};
