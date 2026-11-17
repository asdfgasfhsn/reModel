use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, Shifter};

/// Desert Tank, the original Model 2 board (`docs/games/desert.md`,
/// `docs/hw/model2.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13` then
/// `.14/.15`, 512 KB from 0, as vcop's; main data `.10/.11`, `.8/.9`,
/// `.6/.7` packed from `0x2000000` as daytona's; polygons **`.16/.21` then
/// `.17/.20`** (not daytona's `.16/.20`, `.17/.21`: so joined, the halves
/// make floats, and the models draw), textures `.25/.24`; the TGP's data
/// ROM the 512 KB EPROMs `.28/.29`, `.28` the low half (it begins with an
/// identity matrix, as daytona's). The sound board is daytona's:
/// `epr-16985.7` its 68000 program, the first MultiPCM `.32` alone, the
/// second `.4` then `.5`. The ramps `16310`-`16312` are unused.
pub static DESERT: Game = Game {
    id: "desert",
    title: "Desert Tank",
    year: 1994,
    genre: "Tank shooter",
    kind: crate::Kind::Shooting,
    board: Board::Model2,
    attract_frame: 1500,
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
    // The wait for the next frame: polls a byte the vblank handler changes
    // (at the address in r3) until it differs from the value read before
    // (docs/games/desert.md).
    idle_loops: &[crate::IdleLoop { start: 0x2fc1c, end: 0x2fc24 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        rom("epr-16976.12", 0x20000, "f3e5f22ef1f3ff9c9a1ff7352cdad3e2c2977a51", Role::Program),
        rom("epr-16977.13", 0x20000, "cec8eb8d4b52c387d5750ee5a0c6e6ce7c0fe80d", Role::Program),
        rom("epr-16970.14", 0x20000, "75133b03a450518bae27d62f0a1c37451c8c49a0", Role::Program),
        rom("epr-16971.15", 0x20000, "ca7bd1e01e396b8b6a0925e767cc714729e0fd42", Role::Program),
        rom("mpr-16974.10", 0x200000, "79deb3877d0ffc8ee75c01d3bf0a6dd71cc2b552", Role::MainData),
        rom("mpr-16975.11", 0x200000, "f8ab28c95d421978b1517adeacf09e7ee203d8f6", Role::MainData),
        rom("mpr-16972.8", 0x200000, "9c8a1d8aec8f9e5504e5aac0390dfb3770ab8616", Role::MainData),
        rom("mpr-16973.9", 0x200000, "c83bce7f7b0a15bd14b99e829640b7dd9948e671", Role::MainData),
        rom("epr-16978.6", 0x80000, "a1133df608b0fbb9c53bbeb29138650c87845d2c", Role::MainData),
        rom("epr-16979.7", 0x80000, "0c851dedd5c42b026195faed7d028924698a8b27", Role::MainData),
        rom("mpr-16968.16", 0x200000, "411214ed65ce966040d4299b50bfaa40f7f5f266", Role::Polygons),
        rom("mpr-16964.21", 0x200000, "845c34f95a49e06e3996b0c67aa73b4886fa8996", Role::Polygons),
        rom("mpr-16969.17", 0x200000, "03a9f601764d06cb0b2daaadf4f8433f327abd4a", Role::Polygons),
        rom("mpr-16965.20", 0x200000, "c04f369961f908bac16fad8e32b863202390c205", Role::Polygons),
        rom("mpr-16967.25", 0x200000, "00ef320988609e98c8af383b68d845e3be8d0a03", Role::Textures),
        rom("mpr-16966.24", 0x200000, "33e72139ad6c2990428e3fa041dbcdf39aca1c7a", Role::Textures),
        rom("epr-16981.28", 0x80000, "32d0f9e685667ae9fddacea0b9f4ad6fb3a6fdad", Role::TgpData),
        rom("epr-16980.29", 0x80000, "e889556e0f1ea80de52afff563b0923f87cef7ab", Role::TgpData),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-16985.7", 0x20000, "785752d761c648d1177c5f0cfa3e9fa44135d6dc", Role::AudioCpu),
        rom("mpr-16986.32", 0x200000, "33bcaddfc7d8fe899707e663299e8f04e9004d51", Role::MultiPcmA),
        rom("mpr-16988.4", 0x200000, "5351c6bd2d75df57ff92960e7f90493d95d9dfb9", Role::MultiPcmB),
        rom("mpr-16989.5", 0x200000, "35bd2bfd08514ba6f235cda2605c171cd51fd78e", Role::MultiPcmB),
    ],
};

const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        // The three view buttons: VR1 (blue), VR2 (green), VR3 (red).
        button("view1", 0, 0x20, &["KeyV"], &[PadButton::North]).labelled("View 1"),
        button("view2", 0, 0x40, &["KeyB"], &[]).labelled("View 2"),
        button("view3", 0, 0x80, &["KeyN"], &[]).labelled("View 3"),
        // The drive lever (SHIFT in the input test): set is forward, clear
        // reverse; latched by the shifter below.
        button("forward", 1, 0x01, &["KeyW"], &[PadButton::DPadUp]).labelled("Forward"),
        button("reverse", 1, 0x00, &["KeyS"], &[PadButton::DPadDown]).labelled("Reverse"),
        button("leverup", 1, 0x00, &[], &[]),
        button("leverdown", 1, 0x00, &[], &[]),
        button("mgun", 1, 0x10, &["Space"], &[PadButton::South, PadButton::RightShoulder]).labelled("Machine gun"),
        button("cannon", 1, 0x20, &["KeyZ"], &[PadButton::East, PadButton::LeftShoulder]).labelled("Cannon"),
    ],
    analogs: &[
        AnalogInput {
            name: "handle",
            channel: 0,
            centre: 0x80,
            span: 127,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
        },
        AnalogInput {
            name: "accel",
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
            name: "turret",
            channel: 2,
            centre: 0x80,
            span: 127,
            axis: PadAxis::RightStickX,
            keys_neg: &["KeyA"],
            keys_pos: &["KeyD"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
    ],
    guns: &[],
    // The drive lever: two positions, forward and (as "neutral") reverse.
    shifter: Some(Shifter { gears: &["forward"], neutral: "reverse", up: "leverup", down: "leverdown" }),
    // One coin a credit, start, the pedal on SELECT A MISSION (it picks the
    // mission under the handle: BEGINNER, the desert), then in play the
    // lever forward, the pedal down, the machine gun every 10 frames and
    // the cannon every 60 (docs/games/desert.md).
    coin_start: "coin1@1100-1104,start@1300-1304,accel=0xff@1420-1440,forward@1700-99999,accel=0xff@1700-99999,mgun@1700-99999/10,cannon@1700-99999/60",
};
