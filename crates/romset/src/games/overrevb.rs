use super::rom;
use crate::{Board, Game, Role};

/// Over Rev, Model 2B-CRX (Jaleco): the same game as `overrev` on the
/// earlier board, with two ADSP-21062 SHARCs in place of the MB86235s.
/// Only the program pair (`epr-19992b/19993b`, in `overrev.zip`) differs:
/// every data, polygon, texture and sound chip is the 2C set's, in the
/// same order, and so are the controls.
pub static OVERREVB: Game = Game {
    id: "overrevb",
    title: "Over Rev",
    year: 1997,
    board: Board::Model2B,
    // The attract demo: two cars drifting past the grandstand (its attract
    // runs a few frames behind overrev's).
    attract_frame: 1720,
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: super::overrev::CONTROLS,
    drive_board: None,
    // A clone: its chips can sit in the parent set as MAME merges them.
    parent: Some("overrev"),
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/overrevb.md).
    idle_loops: &[crate::IdleLoop { start: 0x12c44, end: 0x12c58 }],
    packed: &[],
    // Its voices mostly play at DISDL 3: the sound board averages about
    // 5.5 dB below the other SCSP-only games (docs/hw/sound.md §6).
    volume_db: 6.0,
    files: &[
        rom("epr-19992b.15", 0x80000, "40d18ee284ea2e038f7e3d04db56e793ab3e3dd5", Role::Program),
        rom("epr-19993b.16", 0x80000, "a718c32ca27ec1fb5ed2d7d3797ea7e906510a04", Role::Program),
        rom("mpr-19996.11", 0x400000, "6b439fd2b113b64df9378ef8180a17aa6fa975c5", Role::MainData),
        rom("mpr-19997.12", 0x400000, "dbf9af938afd0599d345c42c1df242e575c14de9", Role::MainData),
        rom("mpr-19994.9", 0x400000, "b99c2f3f2a682966d792917dfcb8ed8e53bc0b7a", Role::MainData),
        rom("mpr-19995.10", 0x400000, "4336a12a07a67f94091b4a9b491bab02c375dd15", Role::MainData),
        rom("mpr-20001.27", 0x200000, "b3cb89fadb42afed13be4f229d7158dee487978a", Role::Textures),
        rom("mpr-20000.25", 0x200000, "9bf7c754a29eef47fa49b5567980601895127306", Role::Textures),
        rom("mpr-19998.17", 0x200000, "8be19bf42dbb157d6acde62a2018ef4c0d41aab4", Role::Polygons),
        rom("mpr-19999.21", 0x200000, "ad29e4270b4a2f82189fbab83358eb1200f43777", Role::Polygons),
        rom("mpr-20003.32", 0x400000, "c890bbaebbbb07b62bcb8a3a8edded9fa0ec9a1e", Role::Samples),
        rom("mpr-20004.34", 0x400000, "e5bb30702fc853ccc03316be07a334269d3ebb4a", Role::Samples),
        rom("epr-20002.31", 0x80000, "30b1bbaf348d6a6b9ee2fdf82a0749baa025e0bf", Role::AudioCpu),
    ],
};
