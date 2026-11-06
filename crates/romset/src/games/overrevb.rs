use crate::{Board, Game, Role, RomFile};

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
    hud_windows: &[],
    controls: super::overrev::CONTROLS,
    drive_board: None,
    // A clone: its chips can sit in the parent set as MAME merges them.
    parent: Some("overrev"),
    backup_ram: &[],
    packed: &[],
    // Its voices mostly play at DISDL 3: the sound board averages about
    // 5.5 dB below the other SCSP-only games (docs/hw/sound.md §6).
    volume_db: 6.0,
    files: &[
        RomFile {
            name: "epr-19992b.15",
            size: 0x80000,
            sha1: "40d18ee284ea2e038f7e3d04db56e793ab3e3dd5",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19993b.16",
            size: 0x80000,
            sha1: "a718c32ca27ec1fb5ed2d7d3797ea7e906510a04",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19996.11",
            size: 0x400000,
            sha1: "6b439fd2b113b64df9378ef8180a17aa6fa975c5",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19997.12",
            size: 0x400000,
            sha1: "dbf9af938afd0599d345c42c1df242e575c14de9",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19994.9",
            size: 0x400000,
            sha1: "b99c2f3f2a682966d792917dfcb8ed8e53bc0b7a",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19995.10",
            size: 0x400000,
            sha1: "4336a12a07a67f94091b4a9b491bab02c375dd15",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20001.27",
            size: 0x200000,
            sha1: "b3cb89fadb42afed13be4f229d7158dee487978a",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-20000.25",
            size: 0x200000,
            sha1: "9bf7c754a29eef47fa49b5567980601895127306",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19998.17",
            size: 0x200000,
            sha1: "8be19bf42dbb157d6acde62a2018ef4c0d41aab4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19999.21",
            size: 0x200000,
            sha1: "ad29e4270b4a2f82189fbab83358eb1200f43777",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20003.32",
            size: 0x400000,
            sha1: "c890bbaebbbb07b62bcb8a3a8edded9fa0ec9a1e",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-20004.34",
            size: 0x400000,
            sha1: "e5bb30702fc853ccc03316be07a334269d3ebb4a",
            role: Role::Samples,
        },
        RomFile {
            name: "epr-20002.31",
            size: 0x80000,
            sha1: "30b1bbaf348d6a6b9ee2fdf82a0749baa025e0bf",
            role: Role::AudioCpu,
        },
    ],
};
