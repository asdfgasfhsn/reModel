use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, DriveBoard, Game, PadAxis, PadButton, Role};

/// Sega Ski Super G, Model 2C-CRX. Same layout as `topskatr` plus a second
/// program pair (`.13/.14`, at `0x100000`) and twice the polygon ROM; no
/// MPEG music board. Pairs are listed low half-word first.
pub static SKISUPRG: Game = Game {
    id: "skisuprg",
    title: "Sega Ski Super G",
    year: 1996,
    genre: "Skiing",
    kind: crate::Kind::Sports,
    board: Board::Model2C,
    attract_frame: 3750,
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: Some(DriveBoard::SkiPlatform),
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/skisuprg.md).
    idle_loops: &[crate::IdleLoop { start: 0x44864, end: 0x44878 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-19489.15", 0x80000, "a38faeb97c65b379ad05f7311b55217118c8d2be", Role::Program),
        rom("epr-19490.16", 0x80000, "1ac9172cf0b4d6a3488483ffa490a4ca5d410927", Role::Program),
        rom("epr-19551.13", 0x80000, "23f45858559776a70b3b57f4cb2840f44e6a6531", Role::Program),
        rom("epr-19552.14", 0x80000, "b234f3b65e8fabfb6ec7ca62dd9a1d2935b2e95a", Role::Program),
        rom("mpr-19494.11", 0x400000, "bdbb7d9e91a7742ff5a908b6244adbed291e5e7f", Role::MainData),
        rom("mpr-19495.12", 0x400000, "21ca5e7e543595a4691aacdbcdd2af21d464d939", Role::MainData),
        rom("mpr-19492.9", 0x400000, "dbd1359817933313c6d74d3a1450682e8ce5857a", Role::MainData),
        rom("mpr-19493.10", 0x400000, "e29e50c7fc39bd4945f993ceaa100358054efc5a", Role::MainData),
        rom("mpr-19501.27", 0x400000, "cede0dc5c8d9fbfa8de01fe864b3cc101abf67d7", Role::Textures),
        rom("mpr-19500.25", 0x400000, "31f104e3022b5bc7ed7c667eb801a57949a06c93", Role::Textures),
        rom("mpr-19496.17", 0x400000, "d4b511b90c0a6e27d6097cb25ff005f68d5fa83c", Role::Polygons),
        rom("mpr-19497.21", 0x400000, "4b20bab36462f9506fa2601c2545051ca49de7f5", Role::Polygons),
        rom("mpr-19498.18", 0x400000, "b8a1cc117875c3919a78eedb60a06926288d9b95", Role::Polygons),
        rom("mpr-19499.22", 0x400000, "40f3f2844244c3f1c8792aa262872243ad20fd69", Role::Polygons),
        rom("mpr-19502.29", 0x400000, "3b8a4da2dc00a1eac41b48cbdc322ea1c31b8b29", Role::CoproData),
        rom("mpr-19503.30", 0x400000, "2213485a00cef0bcef11b67f00027c4159c5e2f5", Role::CoproData),
        rom("mpr-19504.32", 0x400000, "d05d9ceb7fd09fa8991c0df4d1c57eb621460e30", Role::Samples),
        rom("mpr-19505.34", 0x400000, "f6e521bedf298808a768f6fdcb0b60b320a66d04", Role::Samples),
        rom("epr-19491.31", 0x80000, "045244a4eebc45f149aecf47f090cede1813477b", Role::AudioCpu),
    ],
};

/// The ski platform cabinet, as far as the game's code and scripted runs
/// show (docs/games/skisuprg.md). Ports read active low.
/// One credit is two coins by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        // The three panel buttons: they pick the left, middle and right
        // choice on the select screens; blue also starts a game.
        button("blue", 0, 0x40, &["Enter", "KeyA"], &[PadButton::Start, PadButton::West]),
        button("red", 0, 0x80, &["KeyS"], &[PadButton::East]),
        button("green", 0, 0x10, &["KeyD"], &[PadButton::South]),
        // Step a selection right / left and confirm it, in the select
        // screens' other mode (unconfirmed which switches these are).
        button("selright", 0, 0x20, &["KeyE"], &[PadButton::RightShoulder]).labelled("▶"),
        button("selleft", 1, 0x01, &["KeyQ"], &[PadButton::LeftShoulder]).labelled("◀"),
        button("decide", 1, 0x10, &["KeyW"], &[PadButton::North]),
    ],
    analogs: &[
        // The skis' two angle sensors; both turn the skier, in opposite
        // senses: right is edge up and swing down (confirmed by scripted
        // runs). Which angle each one measures is unconfirmed. Swing's
        // calibrated centre is what boot sends the drive board.
        AnalogInput {
            name: "edge",
            channel: 0,
            centre: 0x80,
            span: 112,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
        },
        AnalogInput {
            name: "swing",
            channel: 1,
            centre: 0x80,
            span: -112,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
        },
        // Read and calibrated like the others; the skier's physics adds
        // (value - 0x80) to the speed outside 0x70-0x8F, but no scripted
        // run has shown an effect yet. Unconfirmed.
        AnalogInput {
            name: "push",
            channel: 2,
            centre: 0x80,
            span: 112,
            axis: PadAxis::Triggers,
            keys_neg: &["ArrowDown"],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
    ],
    guns: &[],
    shifter: None,
    coin_start: "coin1@1100-1104,coin1@1120-1124,blue@1250-1254",
};
