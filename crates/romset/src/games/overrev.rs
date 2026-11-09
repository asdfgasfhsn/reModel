use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, RomFile};

/// Over Rev, Model 2C-CRX (Jaleco). The zip also holds the 2B clone's
/// program pair (`epr-19992b/19993b`); the 2C pair is `epr-20124a/20125a`
/// (it uploads the MB86235 programs other 2C games share). Pairs are
/// listed low half-word first and fill 8 MB slots as on topskatr: main data
/// `.11/.12` then `.9/.10` (the program's pointers land on data in that
/// order, filler in the other). No copro data ROMs, no music board: all
/// sound is the SCSP's, driven by Jaleco's own 68000 program (`.31`).
pub static OVERREV: Game = Game {
    id: "overrev",
    title: "Over Rev",
    year: 1997,
    board: Board::Model2C,
    // The attract demo: two cars drifting past the grandstand.
    attract_frame: 1700,
    input_lag: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    packed: &[],
    // Its voices mostly play at DISDL 3: the sound board averages about
    // 5.5 dB below the other SCSP-only games (docs/hw/sound.md §6).
    volume_db: 6.0,
    files: &[
        RomFile {
            name: "epr-20124a.15",
            size: 0x80000,
            sha1: "c65c641138ecd7312c4930702d1498b8a346175a",
            role: Role::Program,
        },
        RomFile {
            name: "epr-20125a.16",
            size: 0x80000,
            sha1: "cedb64d2d99a73301ef45c2f5f860a9b87faf6a7",
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

/// The driving cabinet. Ports read active low. Which bit and channel is
/// which comes from the game's input test; each one's effect was confirmed
/// by scripted runs (README, "What's known about overrev"). The input test
/// lists a VIEW2 button as unused ("---"). One credit is one coin by default.
pub(super) const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter"], &[PadButton::Start]),
        // Sequential shifter, used with the manual gearbox (pick MT with the
        // wheel on the gear select screen).
        button("shiftup", 1, 0x10, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Gear +"),
        button("shiftdown", 1, 0x20, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Gear −"),
        // Cycles the camera: chase, cockpit, bumper.
        button("view", 1, 0x01, &["KeyV"], &[PadButton::North]).labelled("View"),
    ],
    analogs: &[
        // The wheel: 0x30 steers left, 0xD0 right. It also moves the
        // selection on the menus.
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
        // The pedals read 0xFF released and 0x00 fully down (the input
        // test shows the raw value). Pressing the gas pedal confirms a
        // choice on the menus.
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
    // One coin, start, the gas pedal to confirm the mode, car and gearbox
    // (and skip the round intro), then the gas held down through the race.
    guns: &[],
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,gas=0x00@1400-1410,gas=0x00@1600-1610,gas=0x00@1800-1810,gas=0x00@2000-2010,gas=0x00@2200-2210,gas=0x00@2600-99999",
};
