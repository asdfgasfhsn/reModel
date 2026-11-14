use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, KeyRamp, PadAxis, PadButton, Role, RomFile};

/// Manx TT Superbike (revision D, deluxe cabinet), Model 2A-CRX
/// (`docs/games/manxtt.md`, `docs/hw/model2a.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13` then
/// `.14/.15` (512 KB, as `vf2`'s); main data `.10/.11 .8/.9 .6/.7 .4/.5`
/// packed from `0x2000000`; polygons `.16/.20 .17/.21 .18/.22` (paired so,
/// the halves make the most sane floats: 0.92, 0.88, 0.95 of words);
/// textures `.25` (even words) / `.24`, one pair. The TGP's data ROM is
/// `.28` (low half) with `.29`, **the other way round from `srallyc`'s**:
/// with `.29` low the course data's first word read `0x00B40000` instead of
/// a count, the course searches found nothing and the attract's bike stood
/// at the start line until a runaway speed hung the i960
/// (docs/games/manxtt.md). Samples `.31 .32 .36 .37` back to back, the
/// sound program `epr-18824a.30`, by analogy with `srallyc` (not yet
/// checked against the sound program's table or by ear). `epr-18742.7`
/// (a byte-swapped 68000 program by its vectors) and `mpr-18743.32` are
/// only in the deluxe set and nothing in a run reads them: presumably a
/// second sound board of that cabinet, *unconfirmed*, not modelled.
pub static MANXTT: Game = Game {
    id: "manxtt",
    title: "Manx TT Superbike",
    year: 1995,
    board: Board::Model2A,
    attract_frame: 2700,
    input_lag: 2,
    run_ahead_max: 2,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: Some(crate::DriveBoard::MotionSlider),
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0, as
    // srallyc's does (docs/games/manxtt.md).
    idle_loops: &[crate::IdleLoop { start: 0x3f1c, end: 0x3f34 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-18822d.12",
            size: 0x20000,
            sha1: "0fcf64598384012caea27394280de89a9348a47d",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18823d.13",
            size: 0x20000,
            sha1: "7e3b97e3370e68d92922e8999246064196610270",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18824d.14",
            size: 0x20000,
            sha1: "629db70371ea9986ef75557044b5e98329712418",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18825d.15",
            size: 0x20000,
            sha1: "bb5e8a5a3b766b5dc4285ecba330094caf8a71e6",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-18751.10",
            size: 0x200000,
            sha1: "4d1601dc08a08b724e33e7cd90a4f22e18cfed9c",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18752.11",
            size: 0x200000,
            sha1: "24007e4ae3ba1a06321328d14e2bd6002fa1936e",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18749.8",
            size: 0x200000,
            sha1: "ada21405a136935ac4da1a3535c25fccf903f2d1",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18750.9",
            size: 0x200000,
            sha1: "d98ae5518c5d31b155b1a7c4f7d9d67f44d7beae",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18747.6",
            size: 0x200000,
            sha1: "92636bdff0ae4cdb43dfc2986fad2d1b59469323",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18748.7",
            size: 0x200000,
            sha1: "6c2e903dd073b130bcabb347631b876dc868b494",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18862.4",
            size: 0x80000,
            sha1: "029db946338f8e0eccace8590082cc96bdf13e31",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18863.5",
            size: 0x80000,
            sha1: "f78a5f7e582d313880c734158bb0fa68b256a58a",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18753.16",
            size: 0x200000,
            sha1: "26f643d6b9cecf08bd249290a670a0edea1b5be4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18756.20",
            size: 0x200000,
            sha1: "fc2a6258387a1bc3fae2109b2dae6dd2a1984ab5",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18754.17",
            size: 0x200000,
            sha1: "e50646efb2ca59792833ce91398c4efa861ad6d1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18757.21",
            size: 0x200000,
            sha1: "226c4c7289b3b6009c1ffea4a171e3fb4e31a67c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18755.18",
            size: 0x200000,
            sha1: "2cd7130b226a28098191a6caf6fd761bb0bfac7b",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18758.22",
            size: 0x200000,
            sha1: "658e33503f6990f4d9a954c63efad5f53d15f3a4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18760.25",
            size: 0x200000,
            sha1: "bba6cd2a15b3f963388a3a87880da86b10f6e0a2",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18759.24",
            size: 0x200000,
            sha1: "5f285fc8cfe88c00ba2bbe1b509b49abd38e00ec",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18761.28",
            size: 0x200000,
            sha1: "50696cd320f1a6492e0c193713acbce085d959cd",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-18762.29",
            size: 0x200000,
            sha1: "7ff42a4c7236fec76f94f2d0c5537e503bcc98e5",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-18863.31",
            size: 0x200000,
            sha1: "a4a8a2f8f0901bfb57778351210ccfc421cacbd4",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18764.32",
            size: 0x200000,
            sha1: "cb2ada0f8a592940de11ee781ad4beb5095c3b37",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18765.36",
            size: 0x200000,
            sha1: "70b59da8f2532a02e980caba5bb86ec13a4d7ab5",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18766.37",
            size: 0x200000,
            sha1: "9ef5e26db4abf0ed36df63fc246b568e1c5d6cfa",
            role: Role::Samples,
        },
        RomFile {
            name: "opr-14742a.45",
            size: 0x20000,
            sha1: "f46429fffcee17d056f56d5fe035a33f1fd6c27e",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14743a.46",
            size: 0x20000,
            sha1: "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14744.58",
            size: 0x20000,
            sha1: "651f1db4089a400d073b19ada299b4b08b08f372",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14745.59",
            size: 0x20000,
            sha1: "e3349ece0e47f684d61ad11bfea4a90602287350",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14746.62",
            size: 0x20000,
            sha1: "34e047a93459406c22acf4c25089d1a4955f94ca",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14747.63",
            size: 0x20000,
            sha1: "7d7ec300eeb9a8de1590011e37108688c092f329",
            role: Role::CoproData,
        },
        RomFile {
            name: "epr-18824a.30",
            size: 0x40000,
            sha1: "27aa0477dc325162766d459ffe95b61ee65dd28f",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "epr-18742.7",
            size: 0x20000,
            sha1: "939b0f2413ae3c11fac11b49ab8b0de2c5e35e61",
            role: Role::Unknown,
        },
        RomFile {
            name: "mpr-18743.32",
            size: 0x200000,
            sha1: "8437cddc4c4d729e886a5ab076885a54bb7a30d0",
            role: Role::Unknown,
        },
    ],
};

/// The cabinet's controls, from the game's own INPUT TEST (test menu:
/// `test` held, `service` to move, `test` to choose), each switch found by
/// holding one port bit at a time on that screen (docs/games/manxtt.md):
/// port 0 `0x01` COIN CHUTE #1, `0x02` COIN CHUTE #2, `0x04` TEST, `0x08`
/// SERVICE, `0x40` START / VR (one button: start, and the view in a race);
/// port 1 `0x10` SHIFT UP, `0x20` SHIFT DOWN (the gear pedal). No other bit
/// of ports 0-2 shows. THROTTLE, BRAKE and BANK (the bike's lean) are
/// analog channels 0, 1 and 2; the test shows each with a range of `20H`
/// to `E0H`, BANK centred at `80H`. Ports read active low.
pub(super) const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter", "Digit1", "KeyV"], &[PadButton::Start, PadButton::North])
            .labelled("Start / View"),
        button("shiftup", 1, 0x10, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Shift up"),
        button("shiftdown", 1, 0x20, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Shift down"),
    ],
    analogs: &[
        AnalogInput {
            name: "throttle",
            channel: 0,
            centre: 0x20,
            span: 0xc0,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        AnalogInput {
            name: "brake",
            channel: 1,
            centre: 0x20,
            span: 0xc0,
            axis: PadAxis::LeftTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        AnalogInput {
            name: "bank",
            channel: 2,
            centre: 0x80,
            // Below 0x80 the bike leans and turns right, above it left
            // (*confirmed* by riding): the stick's right is the low side.
            span: -0x60,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            // The game leans the bike straight from the bank's position (a
            // full bank rolls the view 35 degrees within 4 frames), so a
            // key held would throw it over at once: the keys lean it over
            // 12 frames (0.2 s) and let it back in 8 (docs/games/manxtt.md).
            key_ramp: Some(KeyRamp { attack: 12, release: 8 }),
        },
    ],
    guns: &[],
    shifter: None,
    // One coin is a credit, and the game goes straight to the slider's
    // instructions; START skips them. The course and gearbox selects time
    // out to LAXEY COAST and AT.
    coin_start: "coin1@1100-1104,start@1300-1304",
};
