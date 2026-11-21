use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, DriveBoard, Game, PadAxis, PadButton, Role};

/// Indianapolis 500 (Rev A, Twin, the newer revision), Model 2B-CRX: Sega's
/// own and the earliest 2B game here, uploading its own two SHARC programs.
/// Its copro program reads the copro data ROMs `.29/.30` on its external
/// bus (`model2::sharc`), which dynabb97, doa and the Jaleco games don't
/// have.
/// Pairs are listed low half-word first: program `.15/.16`; main data
/// `.11/.12` then `.9/.10`, and the 1 MB `.7/.8` in the second data bank
/// at `0x6000000`. Polygons `.17/.21` then `.18/.22` and textures
/// `.27/.25` draw correctly. The clones `indy500d` (Deluxe) and
/// `indy500to` (the older Twin revision) differ in the program, `.7/.8`
/// and sound program chips and aren't supported. The game commands a wheel
/// drive board and never checks its reply; with no link board it runs
/// stand-alone (docs/games/indy500.md).
pub static INDY500: Game = Game {
    id: "indy500",
    title: "Indianapolis 500",
    year: 1995,
    genre: "Racing",
    kind: crate::Kind::Racing,
    board: Board::Model2B,
    // The attract demo: the car under the flyover on the street course.
    attract_frame: 1500,
    scenes: &[],
    input_lag: 1,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: Some(DriveBoard::Wheel),
    link_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended { depth: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/indy500.md).
    idle_loops: &[crate::IdleLoop { start: 0x34914, end: 0x34928 }],
    packed: &[],
    // A coin-start run's `make sound` averages -35 dBFS RMS, with
    // dynabb97's and hotd's (docs/hw/sound.md §6).
    volume_db: 0.0,
    files: &[
        rom("epr-18598a.15", 0x80000, "2f616e363f4d246fece309e81325e5e3c4e9d9f8", Role::Program),
        rom("epr-18599a.16", 0x80000, "0982952ab3c5b035f37beb9304ac950c0e78aea8", Role::Program),
        rom("mpr-18237.11", 0x400000, "3ee69a5b9364048dfab242773d97f3af430845b7", Role::MainData),
        rom("mpr-18238.12", 0x400000, "6624417b65f15f20427bc42c27283f10342c76b5", Role::MainData),
        rom("mpr-18239.9", 0x400000, "0b81f6037657af7d96ed5e9bfef407d87cbcc294", Role::MainData),
        rom("mpr-18240.10", 0x400000, "67da857db7155a858a1fa575b6c50f4be3c9ab7c", Role::MainData),
        rom("epr-18596.7", 0x80000, "56ed21234c6494d95b4efda6c3374199c5ac65db", Role::MainDataHigh),
        rom("epr-18597.8", 0x80000, "f0fb0d73c1e72ce77c3931d436c54c034d2107a8", Role::MainDataHigh),
        rom("epr-18249.29", 0x80000, "8b453313c16d935701ed7dbf71c1607c40aede63", Role::CoproData),
        rom("epr-18250.30", 0x80000, "d453e25709cd5970cd21bdc8b4785bc8eb5a50d7", Role::CoproData),
        rom("mpr-18233.17", 0x400000, "501c6ab969713187025331942f922cb0e8efa69a", Role::Polygons),
        rom("mpr-18234.21", 0x400000, "4a9982fdce08f9d375371763dd5287e8485c24b1", Role::Polygons),
        rom("mpr-18235.18", 0x400000, "6081739c15a634d5cc7680a4fc7decead93540ed", Role::Polygons),
        rom("mpr-18236.22", 0x400000, "5de8b569d2a91047836f4a251c21db82fd7841c9", Role::Polygons),
        rom("mpr-18232.27", 0x400000, "79f07ee6b821724294ca9e7a079cb33249102508", Role::Textures),
        rom("mpr-18231.25", 0x400000, "ce592857496ccc0a51efb377cf7cccc000b4296b", Role::Textures),
        rom("epr-18600.31", 0x40000, "8ebabfcd27d634ff0bead0a1f138efc8708575e7", Role::AudioCpu),
        rom("mpr-18241.32", 0x200000, "114113325e9e5262af8750c05089f24818943cde", Role::Samples),
        rom("mpr-18242.33", 0x200000, "5c9cb8ce43a909b25b4e734c6a4ffd786f4dde31", Role::Samples),
        rom("mpr-18243.34", 0x200000, "9c24fbcd0318c7e195dd153d6ba05e8c1e052968", Role::Samples),
        rom("mpr-18244.35", 0x200000, "fec89260d887e90ee9c2803e2eaf937cf9bfa10b", Role::Samples),
    ],
};

/// The Twin cabinet: wheel, gas and brake pedals, a sequential shifter
/// (up and down switches) and two view buttons. Ports read active low;
/// which bit and channel is which comes from the game's input test, the
/// pedals' and wheel's directions from scripted races (docs/games/indy500.md). Two coins are one credit by default.
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
            key_ramp: None,
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
            key_ramp: None,
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
            key_ramp: None,
        },
    ],
    guns: &[],
    // Two coins, start, the gas pedal to confirm the course (the oval),
    // the gearbox (AT) and skip the intro, then the gas held down through
    // the race.
    shifter: None,
    coin_start: "coin1@1100-1104,coin1@1120-1124,start@1250-1254,gas=0xff@1400-1410,gas=0xff@1600-1610,gas=0xff@1800-1810,gas=0xff@2000-99999",
};
