use crate::{Board, Game, Role, RomFile};

/// Sega Ski Super G, Model 2C-CRX. Same layout as `topskatr` plus a second
/// program pair (`.13/.14`, at `0x100000`) and twice the polygon ROM; no
/// MPEG music board. Pairs are listed low half-word first.
pub static SKISUPRG: Game = Game {
    id: "skisuprg",
    title: "Sega Ski Super G",
    board: Board::Model2C,
    hud_windows: &[],
    files: &[
        RomFile {
            name: "epr-19489.15",
            size: 0x80000,
            sha1: "a38faeb97c65b379ad05f7311b55217118c8d2be",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19490.16",
            size: 0x80000,
            sha1: "1ac9172cf0b4d6a3488483ffa490a4ca5d410927",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19551.13",
            size: 0x80000,
            sha1: "23f45858559776a70b3b57f4cb2840f44e6a6531",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19552.14",
            size: 0x80000,
            sha1: "b234f3b65e8fabfb6ec7ca62dd9a1d2935b2e95a",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19494.11",
            size: 0x400000,
            sha1: "bdbb7d9e91a7742ff5a908b6244adbed291e5e7f",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19495.12",
            size: 0x400000,
            sha1: "21ca5e7e543595a4691aacdbcdd2af21d464d939",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19492.9",
            size: 0x400000,
            sha1: "dbd1359817933313c6d74d3a1450682e8ce5857a",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19493.10",
            size: 0x400000,
            sha1: "e29e50c7fc39bd4945f993ceaa100358054efc5a",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19501.27",
            size: 0x400000,
            sha1: "cede0dc5c8d9fbfa8de01fe864b3cc101abf67d7",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19500.25",
            size: 0x400000,
            sha1: "31f104e3022b5bc7ed7c667eb801a57949a06c93",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19496.17",
            size: 0x400000,
            sha1: "d4b511b90c0a6e27d6097cb25ff005f68d5fa83c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19497.21",
            size: 0x400000,
            sha1: "4b20bab36462f9506fa2601c2545051ca49de7f5",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19498.18",
            size: 0x400000,
            sha1: "b8a1cc117875c3919a78eedb60a06926288d9b95",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19499.22",
            size: 0x400000,
            sha1: "40f3f2844244c3f1c8792aa262872243ad20fd69",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19502.29",
            size: 0x400000,
            sha1: "3b8a4da2dc00a1eac41b48cbdc322ea1c31b8b29",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19503.30",
            size: 0x400000,
            sha1: "2213485a00cef0bcef11b67f00027c4159c5e2f5",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19504.32",
            size: 0x400000,
            sha1: "d05d9ceb7fd09fa8991c0df4d1c57eb621460e30",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19505.34",
            size: 0x400000,
            sha1: "f6e521bedf298808a768f6fdcb0b60b320a66d04",
            role: Role::Samples,
        },
        RomFile {
            name: "epr-19491.31",
            size: 0x80000,
            sha1: "045244a4eebc45f149aecf47f090cede1813477b",
            role: Role::AudioCpu,
        },
    ],
};
