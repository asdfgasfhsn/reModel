use crate::controls::button;
use crate::{AnalogInput, Board, Controls, DriveBoard, Game, PadAxis, PadButton, Role, RomFile};

/// Indianapolis 500 (Rev A, Twin, the newer revision), Model 2B-CRX: Sega's
/// own and the earliest 2B game here, uploading its own two SHARC programs.
/// Its copro program reads the copro data ROMs `.29/.30` on its external
/// bus (`model2::sharc::ExtMem`), which the other 2B games don't have.
/// Pairs are listed low half-word first: program `.15/.16`; main data
/// `.11/.12` then `.9/.10`, and the 1 MB `.7/.8` in the second data bank
/// at `0x6000000`. Polygons `.17/.21` then `.18/.22` and textures
/// `.27/.25` draw correctly. The clones `indy500d` (Deluxe) and
/// `indy500to` (the older Twin revision) differ in the program, `.7/.8`
/// and sound program chips and aren't supported. The game commands a wheel
/// drive board and never checks its reply; with no link board it runs
/// stand-alone (README, "What's known about indy500").
pub static INDY500: Game = Game {
    id: "indy500",
    title: "Indianapolis 500",
    year: 1995,
    board: Board::Model2B,
    // The attract demo: the car under the flyover on the street course.
    attract_frame: 1500,
    hud_windows: &[],
    controls: CONTROLS,
    drive_board: Some(DriveBoard::Wheel),
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    packed: &[],
    // A coin-start run's `make sound` averages -35 dBFS RMS, with
    // dynabb97's and hotd's (docs/hw/sound.md §6).
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-18598a.15",
            size: 0x80000,
            sha1: "2f616e363f4d246fece309e81325e5e3c4e9d9f8",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18599a.16",
            size: 0x80000,
            sha1: "0982952ab3c5b035f37beb9304ac950c0e78aea8",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-18237.11",
            size: 0x400000,
            sha1: "3ee69a5b9364048dfab242773d97f3af430845b7",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18238.12",
            size: 0x400000,
            sha1: "6624417b65f15f20427bc42c27283f10342c76b5",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18239.9",
            size: 0x400000,
            sha1: "0b81f6037657af7d96ed5e9bfef407d87cbcc294",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18240.10",
            size: 0x400000,
            sha1: "67da857db7155a858a1fa575b6c50f4be3c9ab7c",
            role: Role::MainData,
        },
        RomFile {
            name: "epr-18596.7",
            size: 0x80000,
            sha1: "56ed21234c6494d95b4efda6c3374199c5ac65db",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-18597.8",
            size: 0x80000,
            sha1: "f0fb0d73c1e72ce77c3931d436c54c034d2107a8",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-18249.29",
            size: 0x80000,
            sha1: "8b453313c16d935701ed7dbf71c1607c40aede63",
            role: Role::CoproData,
        },
        RomFile {
            name: "epr-18250.30",
            size: 0x80000,
            sha1: "d453e25709cd5970cd21bdc8b4785bc8eb5a50d7",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-18233.17",
            size: 0x400000,
            sha1: "501c6ab969713187025331942f922cb0e8efa69a",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18234.21",
            size: 0x400000,
            sha1: "4a9982fdce08f9d375371763dd5287e8485c24b1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18235.18",
            size: 0x400000,
            sha1: "6081739c15a634d5cc7680a4fc7decead93540ed",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18236.22",
            size: 0x400000,
            sha1: "5de8b569d2a91047836f4a251c21db82fd7841c9",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18232.27",
            size: 0x400000,
            sha1: "79f07ee6b821724294ca9e7a079cb33249102508",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18231.25",
            size: 0x400000,
            sha1: "ce592857496ccc0a51efb377cf7cccc000b4296b",
            role: Role::Textures,
        },
        RomFile {
            name: "epr-18600.31",
            size: 0x40000,
            sha1: "8ebabfcd27d634ff0bead0a1f138efc8708575e7",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-18241.32",
            size: 0x200000,
            sha1: "114113325e9e5262af8750c05089f24818943cde",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18242.33",
            size: 0x200000,
            sha1: "5c9cb8ce43a909b25b4e734c6a4ffd786f4dde31",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18243.34",
            size: 0x200000,
            sha1: "9c24fbcd0318c7e195dd153d6ba05e8c1e052968",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18244.35",
            size: 0x200000,
            sha1: "fec89260d887e90ee9c2803e2eaf937cf9bfa10b",
            role: Role::Samples,
        },
    ],
};

/// The Twin cabinet: wheel, gas and brake pedals, a sequential shifter
/// (up and down switches) and two view buttons. Ports read active low;
/// which bit and channel is which comes from the game's input test, the
/// pedals' and wheel's directions from scripted races (README, "What's
/// known about indy500"). Two coins are one credit by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter"], &[PadButton::Start]),
        // The shifter, used with the manual gearbox (MT on the
        // transmission select screen).
        button("shiftup", 1, 0x10, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Gear +"),
        button("shiftdown", 1, 0x20, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Gear −"),
        // VIEW1 (zoom in) and VIEW2 (zoom out) step through the four views.
        button("view1", 1, 0x01, &["KeyV"], &[PadButton::North]).labelled("View in"),
        button("view2", 1, 0x02, &["KeyB"], &[PadButton::South]).labelled("View out"),
    ],
    analogs: &[
        // The wheel: lower values steer left. It also picks the course
        // and gearbox on the menus.
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
        // The pedals read 0x00 released and 0xFF fully down. Pressing the
        // gas confirms on the menus.
        AnalogInput {
            name: "gas",
            channel: 1,
            centre: 0x00,
            span: 255,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
        },
        AnalogInput {
            name: "brake",
            channel: 2,
            centre: 0x00,
            span: 255,
            axis: PadAxis::LeftTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown"],
            pad_neg: &[],
            pad_pos: &[],
        },
    ],
    guns: &[],
    // Two coins, start, the gas pedal to confirm the course (the oval),
    // the gearbox (AT) and skip the intro, then the gas held down through
    // the race.
    coin_start: "coin1@1100-1104,coin1@1120-1124,start@1250-1254,gas=0xff@1400-1410,gas=0xff@1600-1610,gas=0xff@1800-1810,gas=0xff@2000-99999",
};
