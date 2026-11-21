use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role};

/// Cyber Troopers Virtual-On (USA, Sega), Model 2B-CRX: `indy500`'s
/// geometrizer code and a copro program of its own, which comes in the
/// program's second pair (docs/games/von.md). Pairs are listed low
/// half-word first: program `vo-prog0.usa/vo-prog1.usa` then `.13/.14`
/// (512 KB chips, back to back); main data `.11/.12`, `.9/.10`; polygons
/// `.17/.21`, `.18/.22`; textures `.27/.25`, `.28/.26`; copro data
/// `.29/.30`; samples `.32`, `.34`. `epr-18643a.7` is the link board's Z80
/// program, which the i960 never reads: the game needs the board itself
/// fitted, which the board model answers (`LinkBoard::Idle`). The
/// Japanese clone `vonj` (`epr-18664b.15/epr-18665b.16`) isn't supported.
pub static VON: Game = Game {
    id: "von",
    title: "Cyber Troopers Virtual-On",
    year: 1996,
    genre: "Mech fighting",
    kind: crate::Kind::Fighting,
    board: Board::Model2B,
    // The attract demo: a robot's sword swing under the "GO AGAINST OTHER
    // PLAYER!" banner.
    attract_frame: 1500,
    scenes: &[],
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    link_board: Some(crate::LinkBoard::Idle),
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2), counting passes in g0 (docs/games/von.md).
    idle_loops: &[crate::IdleLoop { start: 0x29a3c, end: 0x29a50 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("vo-prog0.usa", 0x80000, "8289be295f021acbf0c903513ba97ae7de50dedb", Role::Program),
        rom("vo-prog1.usa", 0x80000, "b89cc814b02b4ab5e37c75ee1a9cf57b88b63053", Role::Program),
        rom("epr-18666.13", 0x80000, "b67131b0158a58138380734dd5b9394b70010026", Role::Program),
        rom("epr-18667.14", 0x80000, "1e9f23f4052ab1b0275307cc80e51352f13bc319", Role::Program),
        rom("mpr-18648.11", 0x400000, "ded2e4b295be08970d13c387818c570c3afe8109", Role::MainData),
        rom("mpr-18649.12", 0x400000, "cdee1ba9247eda4282442d0522f8de7d7c86e1e6", Role::MainData),
        rom("mpr-18650.9", 0x400000, "5096db1da1f7e175000e89fca2a1dd3fd53030ea", Role::MainData),
        rom("mpr-18651.10", 0x400000, "f65984614111b12dd414db80751efe64fcf5ef16", Role::MainData),
        rom("mpr-18662.29", 0x200000, "991bbe9dcbef8bfa96682e9d142623fc9b7c0879", Role::CoproData),
        rom("mpr-18663.30", 0x200000, "a684e13c0afe2ef3f3108ae9b73389121368fc4e", Role::CoproData),
        rom("mpr-18654.17", 0x400000, "9f009f44e62ae0f9dec7a34a163bc186d1c4cbbd", Role::Polygons),
        rom("mpr-18655.21", 0x400000, "af512c994bedbdaf3a5eeed607e771dcd87810fc", Role::Polygons),
        rom("mpr-18656.18", 0x400000, "eb71ada331576f2a7219d238ea07a61bcbf6381a", Role::Polygons),
        rom("mpr-18657.22", 0x400000, "a918c2a3de78a08104480097edfb9d6aeaeda873", Role::Polygons),
        rom("mpr-18660.27", 0x200000, "0a4908be654bad4f00d7d58f0e42f631996911c9", Role::Textures),
        rom("mpr-18658.25", 0x200000, "c8626c879bfcf7abd095cac5dc03a04ae8629423", Role::Textures),
        rom("mpr-18661.28", 0x200000, "64ea7b2f86745954e0b8a15d71203444705240a2", Role::Textures),
        rom("mpr-18659.26", 0x200000, "e9b756e5b4b1c19e52e47af03c773fee544be420", Role::Textures),
        rom("epr-18670.31", 0x80000, "4fd997e379a8cdb94ec3b1986b3ab443fc6fa12a", Role::AudioCpu),
        rom("mpr-18652.32", 0x400000, "e592f9e97abe0a7bc9009d8327b93da9bc43749c", Role::Samples),
        rom("mpr-18653.34", 0x400000, "197bc8adc823e93128c1cebf69361a7c7297f808", Role::Samples),
        rom("epr-18643a.7", 0x20000, "8182e05a2ffebd590a936c1359c81e60caa79c2a", Role::Unknown),
    ],
};

/// Twin sticks, from the game's Input Test (docs/games/von.md): one player
/// has two 8-way levers, each with SHOT and DASH, and a single START. Port
/// 0 is the coins, test, service and start; port 1 the left lever and port
/// 2 the right one, each with SHOT `0x01`, DASH `0x02` and the directions
/// on the high nibble. On the gamepad the left lever is the left stick
/// (and the d-pad) and the right lever the right stick, each stick read
/// as an 8-way lever (`StickDirs`); the shoulders are the shots and the
/// triggers the dashes, with the face buttons for the same four on a pad
/// whose fingers are on them (X and A the left lever's shot and dash, B and
/// Y the right's). On the keyboard the left lever is WASD and the right one
/// the arrows.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        // The left lever.
        button("lshot", 1, 0x01, &["KeyQ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Left shot"),
        button("ldash", 1, 0x02, &["KeyE"], &[PadButton::LeftTrigger, PadButton::South]).labelled("Left dash"),
        button("ldown", 1, 0x10, &["KeyS"], &[PadButton::DPadDown, PadButton::LeftStickDown]).labelled("Left down"),
        button("lup", 1, 0x20, &["KeyW"], &[PadButton::DPadUp, PadButton::LeftStickUp]).labelled("Left up"),
        button("lright", 1, 0x40, &["KeyD"], &[PadButton::DPadRight, PadButton::LeftStickRight]).labelled("Left right"),
        button("lleft", 1, 0x80, &["KeyA"], &[PadButton::DPadLeft, PadButton::LeftStickLeft]).labelled("Left left"),
        // The right lever.
        button("rshot", 2, 0x01, &["Period"], &[PadButton::RightShoulder, PadButton::East]).labelled("Right shot"),
        button("rdash", 2, 0x02, &["Slash"], &[PadButton::RightTrigger, PadButton::North]).labelled("Right dash"),
        button("rdown", 2, 0x10, &["ArrowDown"], &[PadButton::RightStickDown]).labelled("Right down"),
        button("rup", 2, 0x20, &["ArrowUp"], &[PadButton::RightStickUp]).labelled("Right up"),
        button("rright", 2, 0x40, &["ArrowRight"], &[PadButton::RightStickRight]).labelled("Right right"),
        button("rleft", 2, 0x80, &["ArrowLeft"], &[PadButton::RightStickLeft]).labelled("Right left"),
    ],
    analogs: &[],
    guns: &[],
    shifter: None,
    // One coin and start; the machine and the stage choose themselves. In
    // the fight both levers hold forward, the shots fire and the dashes
    // go in turn.
    coin_start: "coin1@1100-1104,start@1250-1254,lup@2000-99999,rup@2000-99999,lshot@2000-99999/6,rshot@2003-99999/6,ldash@2100-99999/90",
};
