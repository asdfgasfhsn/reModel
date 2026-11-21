use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, Scene};

/// Model 2C-CRX. Roles and interleave order follow the board wiring;
/// pairs are listed low half-word first (note textures: `.27` before `.25`).
pub static TOPSKATR: Game = Game {
    id: "topskatr",
    title: "Top Skater",
    year: 1997,
    genre: "Skateboarding",
    kind: crate::Kind::Sports,
    board: Board::Model2C,
    attract_frame: 1650,
    // Named moments of a coin-start run (frame numbers are the machine's own; `remodel scene verify` checks them).
    scenes: &[
        Scene { name: "coin-up", title: "Credit in, press start", input: "coin-start", frame: 1200, tags: &["menu"] },
        Scene {
            name: "course-select",
            title: "Novice or Expert course",
            input: "coin-start",
            frame: 1500,
            tags: &["select"],
        },
        Scene { name: "skater-select", title: "Skater select", input: "coin-start", frame: 2100, tags: &["select"] },
        Scene {
            name: "race-go",
            title: "The GO of the Novice race",
            input: "coin-start",
            frame: 3150,
            tags: &["play", "hud"],
        },
        Scene {
            name: "race-course",
            title: "Novice course, first stretch",
            input: "coin-start",
            frame: 3300,
            tags: &["play", "hud"],
        },
    ],
    input_lag: 0,
    run_ahead_max: 0,
    // Window 4 holds the speed gauge and the time-limit ring (confirmed with
    // REMODEL_PROBE); the course is window 2 and the sky window 0.
    hud_windows: &[4],
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
    recommended: crate::Recommended { depth: true, shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0 (through
    // g6) (docs/games/topskatr.md).
    idle_loops: &[crate::IdleLoop { start: 0x4820c, end: 0x48224 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-19755a.15", 0x80000, "5396da414beeb918e6f38f25a43dd76345a0c8ed", Role::Program),
        rom("epr-19756a.16", 0x80000, "06d0f609257ba476e6bd3b956e0850e7167429ce", Role::Program),
        rom("mpr-19735.11", 0x400000, "49afc91467f08befaf34e743cbe823de3e3c9d85", Role::MainData),
        rom("mpr-19736.12", 0x400000, "de2c739f71e51166263446b9f6a566866ab8bee8", Role::MainData),
        rom("mpr-19737.9", 0x400000, "71d5ba434328a81969bfdc71ac1160c5ff3ae9d3", Role::MainData),
        rom("mpr-19738.10", 0x400000, "68c9db242ef7e8f98979e968a09e4b093bc5d470", Role::MainData),
        rom("mpr-19740.27", 0x400000, "c90fa3b42d87291ea459ccc137f3a2f3eb7efec0", Role::Textures),
        rom("mpr-19739.25", 0x400000, "a82744bff5dcdfae296c7c3e8c3fbfda26324e85", Role::Textures),
        rom("mpr-19741.17", 0x200000, "8664059f157626e4bbdcf8357e3d30b37d3c25b8", Role::Polygons),
        rom("mpr-19742.21", 0x200000, "3e68aec090f36a60b3b70bc90f09e2f9ce088718", Role::Polygons),
        rom("mpr-19743.29", 0x200000, "a5f6b24e6526d0d2ef9c526c273c018d1e0fed59", Role::CoproData),
        rom("mpr-19744.30", 0x200000, "4952b764e6bf6cd735018738c5eff08781ee2315", Role::CoproData),
        rom("mpr-19745.32", 0x400000, "415f9d0793a697cb1719bbd96370f4a741866527", Role::Samples),
        rom("mpr-19746.34", 0x400000, "ca76f211d68b6b55678a4d7949bfd2ddef1b1710", Role::Samples),
        rom("mpr-19747.18s", 0x400000, "4c67c1e1d58a3034bbd711252a78689db9f235bb", Role::MpegData),
        rom("mpr-19748.20s", 0x400000, "fd4da4cf40c4342c6263cf22eee5968292a4d2c0", Role::MpegData),
        rom("mpr-19749.22s", 0x400000, "6ee6b2eb2ea400bdb9c0a9b4a126b4b86886e813", Role::MpegData),
        rom("mpr-19750.24s", 0x400000, "40e2a2980c89049c339fefd48bf7aac79962cd2e", Role::MpegData),
        rom("mpr-19759.31", 0x80000, "7b205085965d6694f8e75e29c4028f7cb6f631ab", Role::AudioCpu),
        rom("mpr-19760.2s", 0x20000, "a302209bfe0f1491dff2da64b32cfaa13c3d3304", Role::MpegCpu),
    ],
};

/// The skateboard cabinet. Ports read active low. Coin, test and service
/// follow the common Model 2 layout; the game buttons and both analog
/// channels were confirmed by scripted presses (docs/games/topskatr.md).
/// Test and service are not yet checked.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("selright", 0, 0x10, &["KeyE"], &[PadButton::RightShoulder]).labelled("▶"),
        button("jumpfront", 0, 0x20, &["KeyZ", "ArrowUp"], &[PadButton::South]).labelled("Jump"),
        button("start", 0, 0x40, &["Enter"], &[PadButton::Start]),
        button("selleft", 0, 0x80, &["KeyQ"], &[PadButton::LeftShoulder]).labelled("◀"),
        button("jumptail", 1, 0x01, &["KeyX"], &[PadButton::West]).labelled("Tail"),
    ],
    analogs: &[
        // Steering: 0x10 carves right, 0xF0 left, 0x80 centred.
        AnalogInput {
            name: "curving",
            channel: 0,
            centre: 0x80,
            span: -112,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
            steering: true,
        },
        // The board pressed down at the back: the skater crouches and slides.
        AnalogInput {
            name: "slide",
            channel: 1,
            centre: 0x80,
            span: 112,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown", "Space"],
            pad_neg: &[],
            pad_pos: &[PadButton::East],
            key_ramp: None,
            steering: false,
        },
    ],
    guns: &[],
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254",
};
