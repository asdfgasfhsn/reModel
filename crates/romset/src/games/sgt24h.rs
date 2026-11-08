use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, RomFile};

/// Super GT 24h, Model 2B-CRX (Jaleco): the second 2B game, uploading the
/// same two SHARC programs as `overrevb`. Pairs are listed low half-word
/// first and fill 8 MB slots: main data `.11/.12` then `.9/.10` (the
/// program's pointers land on data in that order, filler in the other),
/// textures `.27/.25` and polygons `.17/.21` (models and textures draw
/// correctly). No copro data ROMs, no music board: Jaleco's sound driver
/// (`.31`, the same code as `overrev`'s) streams everything from the one
/// sample ROM. `epr-18643a.7` is the Z80 program of Sega's communication
/// (link) board, which the i960 never reads; with no board fitted the game
/// runs stand-alone (README, "What's known about sgt24h").
pub static SGT24H: Game = Game {
    id: "sgt24h",
    title: "Super GT 24h",
    year: 1996,
    board: Board::Model2B,
    // The attract demo: the AUTO MAX car on the grid before the grandstand.
    attract_frame: 1150,
    input_lag: 4,
    hud_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    // The I/O TYPE setting (game assignments, A to E), a 16-bit word at
    // SRAM 0x0A that the game's own reset of its settings leaves alone. The
    // game indexes its pedal scaling table with it unchecked, so the
    // power-on 0xFFFF scrambles the pedals; type A (0) is what the menu
    // shows for an unset value, and reads the pedals as they come.
    backup_ram: &[(0x0a, &[0, 0])],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    packed: &[],
    // Jaleco's driver plays its voices at DISDL 3 as on overrev: a race
    // averages about -38 dBFS RMS, as overrev's does (docs/hw/sound.md §6).
    volume_db: 6.0,
    files: &[
        RomFile {
            name: "epr-19155.15",
            size: 0x80000,
            sha1: "1fc4afc6e3910cc8adb0688542e61a9efb442e56",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19156.16",
            size: 0x80000,
            sha1: "a37611da0295f7d7e5d2411c3f9b73140d311f74",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19146.11",
            size: 0x400000,
            sha1: "c2a5b83c9041d8f46dfac4a3ff8cfdefb96d02b3",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19147.12",
            size: 0x400000,
            sha1: "c9f40328d6974b7767fa6ba719d0d2b7a173c210",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19148.9",
            size: 0x400000,
            sha1: "66cd8672c00e4e2572de7c5648de595674ffa8f8",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19149.10",
            size: 0x400000,
            sha1: "3e1748dc5e61c93eedbf0ca6b1946a30be722403",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19153.27",
            size: 0x200000,
            sha1: "70ce4e609c8b003ff04518044c18d29089e6a353",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19152.25",
            size: 0x200000,
            sha1: "51b2f11a01fb72e151025771f8a8496993e605c2",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19150.17",
            size: 0x400000,
            sha1: "3429d9f9434d75ddb5fa05d4b493828adfe826a4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19151.21",
            size: 0x400000,
            sha1: "cc5c2d9ab8a01f52e66969464f53ae3cefca6a09",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19154.32",
            size: 0x400000,
            sha1: "b9812c4f3042f95febc96bcdd46e3b0724ad4b4f",
            role: Role::Samples,
        },
        RomFile {
            name: "epr-19157.31",
            size: 0x80000,
            sha1: "439e784081329db2fe03419681150f3216f4ccff",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "epr-18643a.7",
            size: 0x20000,
            sha1: "8182e05a2ffebd590a936c1359c81e60caa79c2a",
            role: Role::Unknown,
        },
    ],
};

/// The driving cabinet with I/O type A. Ports read active low. Which bit
/// and channel is which comes from the game's input test; each one's effect
/// was confirmed by scripted runs (README, "What's known about sgt24h").
/// One credit is one coin by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter"], &[PadButton::Start]),
        // Sequential shifter, used with the manual gearbox (pick MT with the
        // wheel on the gear select screen). Bits 6 and 7 of the same port
        // are two more lever positions the input test doesn't show.
        button("shiftup", 1, 0x10, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Gear +"),
        button("shiftdown", 1, 0x20, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Gear −"),
        // Cycles the camera views. (VIEW 2, bit 1, is
        // listed only for I/O types B and D.)
        button("view", 1, 0x01, &["KeyV"], &[PadButton::North]).labelled("View"),
    ],
    analogs: &[
        // The wheel: 0x30 steers left, 0xD0 right. It also moves the
        // selection on the menus. The game calibrates the wheel and pedals
        // from the extremes it sees.
        AnalogInput {
            name: "wheel",
            channel: 0,
            centre: 0x80,
            span: 112,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
        },
        // The pedals read 0xFF released and 0x00 fully down with I/O type A
        // (types B-E invert them). Pressing the gas confirms on the menus.
        AnalogInput {
            name: "gas",
            channel: 1,
            centre: 0xff,
            span: -255,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
        },
        AnalogInput {
            name: "brake",
            channel: 2,
            centre: 0xff,
            span: -255,
            axis: PadAxis::LeftTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown"],
            pad_neg: &[],
            pad_pos: &[],
        },
    ],
    guns: &[],
    // One coin, start, the gas pedal to confirm course, car and gearbox
    // (AT) and skip the intro, then the gas held down through the race
    // until time runs out.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,gas=0x00@1500-1510,gas=0x00@1700-1710,gas=0x00@1900-1910,gas=0x00@2100-2110,gas=0x00@2300-2310,gas=0x00@2600-99999",
};
