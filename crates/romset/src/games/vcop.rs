use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, Gun, GunSource, PadButton, Reload, Role};

/// Virtua Cop (revision B), the original Model 2 board (`docs/games/vcop.md`,
/// `docs/hw/model2.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13` then
/// `.14/.15`, 512 KB from 0 (code runs from both); main data `.10/.11`,
/// `.8/.9`, `.6/.7` packed from `0x2000000` as daytona's (`.6/.7` hold the
/// TGP program); one polygon pair `.16/.20` and one texture pair
/// `.25/.24`; the TGP's tables as every 2A set; no TGP data ROM. The sound
/// board is daytona's: `epr-17170.7` its 68000 program, the first
/// MultiPCM `.32` then `.33` (1 MB chips, each in a 2 MB socket), the
/// second `.4` then `.5`; `.8` unknown. The zip's `epr-17166a`/`17167a`
/// are `vcopa`'s and the ramps `16310`-`16312` are unused.
pub static VCOP: Game = Game {
    id: "vcop",
    title: "Virtua Cop",
    year: 1994,
    genre: "Light gun",
    kind: crate::Kind::Shooting,
    board: Board::Model2,
    attract_frame: 1500,
    scenes: &[],
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    link_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0, as
    // hotd's does (docs/games/vcop.md).
    idle_loops: &[crate::IdleLoop { start: 0x9124, end: 0x9138 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons],
    volume_db: 0.0,
    files: &[
        rom("epr-17166b.12", 0x20000, "0a9e0be447d3591e82efd40ef4acbfe7ae211579", Role::Program),
        rom("epr-17167b.13", 0x20000, "95db029bc4206a44ea216afbcd1c19689f79115a", Role::Program),
        rom("epr-17160a.14", 0x20000, "40ec09cda984bb80969bfae2278432153137c213", Role::Program),
        rom("epr-17161a.15", 0x20000, "b0ceb1206edaa507ec15723497fcd447a511f423", Role::Program),
        rom("mpr-17164.10", 0x200000, "e60deec1e79d207d37d3f4ddd83a1b2125c411ac", Role::MainData),
        rom("mpr-17165.11", 0x200000, "23327137b36c98dfb9175ea9d36478e7385dfac2", Role::MainData),
        rom("mpr-17162.8", 0x200000, "0894c9bcdedeb09f921419a309858e242cb8db3a", Role::MainData),
        rom("mpr-17163.9", 0x200000, "cf99a5bb4f343d59c8d6f5716287b6e16bef6412", Role::MainData),
        rom("epr-17168a.6", 0x80000, "14591c7015aaf126755be584aa94c04e6de222fa", Role::MainData),
        rom("epr-17169a.7", 0x80000, "5b86a9a68c2b52f942aa8d858ee7a491f546a921", Role::MainData),
        rom("mpr-17159.16", 0x200000, "1458d01d49936a0b8d497b62ff9ea940ca753b37", Role::Polygons),
        rom("mpr-17156.20", 0x200000, "8814cd329609cc8a188fedd770230bb9a5d00361", Role::Polygons),
        rom("mpr-17158.25", 0x200000, "e95d4166bd4b26c5f21b85821b410f53045f4309", Role::Textures),
        rom("mpr-17157.24", 0x200000, "0cb62d4f28b5ad8a7e4c82b0ca8aea3037b05455", Role::Textures),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-17170.7", 0x20000, "a2c3d14d9266449ebfc6d976a956e0a8a602cfb0", Role::AudioCpu),
        rom("epr-17171.8", 0x20000, "1da3cb52d64f52d03a8de9954afffbc6e1549a5b", Role::Unknown),
        rom("mpr-17172.32", 0x100000, "0e872158faeb8c0404b10cdf0a3fa36f89a5093e", Role::MultiPcmA),
        rom("mpr-17173.33", 0x100000, "a56f436ea6dfe0968b73ae7bc92bb2f4c612460d", Role::MultiPcmA),
        rom("mpr-17174.4", 0x200000, "69807157baf6e3679adc95633c82b0236db01247", Role::MultiPcmB),
        rom("mpr-17175.5", 0x200000, "741f80a8ff8165ffe171dc568e0da4ad0bde4809", Role::MultiPcmB),
    ],
};

/// Two light guns and two start buttons, from the game's input test and
/// GUN MARK CHECK (docs/games/vcop.md). Register 8 holds coin 1 and 2,
/// test, service and the starts, register 9 bits 0 and 1 the triggers.
/// The guns' registers at `0x1C00100` have the layout of hotd's gun
/// counters (Y in 0/1, X in 2/3 for player 1, then player 2's; a "sees no
/// screen" bit a player in 8); the ranges are the game's default
/// calibration, which maps them to the view's edges.
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
    guns: &[
        Gun {
            name: "gun1",
            source: GunSource::Counters { x: 2, y: 0, offscreen_bit: 0 },
            x_range: (131, 630),
            y_range: (36, 425),
            trigger: "trigger1",
            reload: Reload::Offscreen,
            keys_reload: &["KeyR"],
        },
        Gun {
            name: "gun2",
            source: GunSource::Counters { x: 6, y: 4, offscreen_bit: 1 },
            x_range: (128, 627),
            y_range: (39, 425),
            trigger: "trigger2",
            reload: Reload::Offscreen,
            keys_reload: &[],
        },
    ],
    shifter: None,
    // One coin a credit, start, then shoot STAGE 1 (BEGINNER) on the stage
    // select; in play aim at the middle and either side in turn, fire every
    // 20 frames and shoot off screen (reload) every seventh shot. The game
    // takes a shot's aim two frames after the trigger (in its flash), so
    // the aims and the off-screen pulses come two frames after their
    // trigger pulses (docs/games/vcop.md).
    coin_start: "coin1@1100-1104,start@1300-1304,gun1=95:190@1350-1420,trigger1@1400-1404,gun1=248:170@1500-6400,gun1=140:190@1582-6400/120,gun1=360:190@1642-6400/120,gun1=off@1562-6400/140,trigger1@1560-6400/20",
};
