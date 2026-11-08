use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, RomFile};

/// Sega Water Ski (Japan), Model 2C-CRX. Pairs are listed low half-word
/// first. Program `.15/.16` only; main data `.11/.12` at `0x2000000` and
/// `.9/.10` at `0x2800000` as in skisuprg, plus a second data bank: `.7/.8`
/// at `0x6000000` and the 1 MB pair `.5/.6` at `0x6800000`, repeating in its
/// slot (the game reads it at `0x6C0xxxx`). The memory test checks the ROMs
/// in that order (IC15, 16, 11, 12, 9, 10, 7, 8, 5, 6) and passes them all
/// (README, "What's known about segawski"). No drive board or music board.
pub static SEGAWSKI: Game = Game {
    id: "segawski",
    title: "Sega Water Ski",
    year: 1997,
    board: Board::Model2C,
    hud_windows: &[],
    attract_frame: 2550,
    input_lag: 0,
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    packed: &[],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-19963a.15",
            size: 0x80000,
            sha1: "7f1f600222447effb28cf2d56193ea9f45fd0646",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19964a.16",
            size: 0x80000,
            sha1: "c0ccee4eb19d9626dee0f77f08060f1d9708b39d",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19980.11",
            size: 0x400000,
            sha1: "6633eeb2bc0ddd5eb5994cb7cc39ed4ab2094a7e",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19981.12",
            size: 0x400000,
            sha1: "53abaf71e85f874d28a79c8775b1f0ac919fbe22",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19982.9",
            size: 0x400000,
            sha1: "e6652111a494165c93d5e9dc7d1c3df46f1a8961",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19983.10",
            size: 0x400000,
            sha1: "a32010648046738f64cbc7e1a074b9443bbde865",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19984.7",
            size: 0x400000,
            sha1: "8e0fd13f5838218224ce162a55558b75dcda4cbf",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19985.8",
            size: 0x400000,
            sha1: "0d764ec9e136e4d027aefb10e9f025aa2e081573",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-19961.5",
            size: 0x80000,
            sha1: "8e540a429826d40acef5a34a0b6b1b89e059961b",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-19962.6",
            size: 0x80000,
            sha1: "2d7290186aaca28f3aaf6656b090e1cbe509aa48",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19969.27",
            size: 0x400000,
            sha1: "f61a6266556d5687cd03031065baa2dd73112666",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19968.25",
            size: 0x400000,
            sha1: "f0f4d2dd06cdf745ed07fe428eaa3e3ad030bff6",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19971.28",
            size: 0x400000,
            sha1: "c27e0a90dc1183b0cf7f32e324afa6c126f61d37",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19970.26",
            size: 0x400000,
            sha1: "24232390f0cac5ffbb17a0093a602363c686fbf8",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19972.17",
            size: 0x400000,
            sha1: "c68f662d53fbcec3d2dbd9ebdfd37cbf1ff46408",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19973.21",
            size: 0x400000,
            sha1: "be1f1017ff5f43218de2d99090e4e7db64f7a483",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19974.18",
            size: 0x400000,
            sha1: "7f16e668db5a4673cd909bdfa1b9dc665827b392",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19975.22",
            size: 0x400000,
            sha1: "35bbea4cd42ab39f976cd2ec964c32d5d3982c4a",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19976.19",
            size: 0x400000,
            sha1: "e6dee764301c0ed34e5134a5232898bb42563c20",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19977.23",
            size: 0x400000,
            sha1: "d61448394b7b2036e27bdbf7a062d63f076db9da",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19978.20",
            size: 0x400000,
            sha1: "e7ea6dfb57ab1fe924a80c244b8acfe4aad2b76d",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19979.24",
            size: 0x400000,
            sha1: "65a81f4503c6b9c26650befd183db821545c53e2",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19986.29",
            size: 0x400000,
            sha1: "859e3788c75599295a8b57ed7852f2cbb6a2a738",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19987.30",
            size: 0x400000,
            sha1: "35f41c474af3754152aecefe81e912120823e0ff",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-19988.32",
            size: 0x400000,
            sha1: "63efbd9f67b38fddeeed25de660514867e03486a",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19989.34",
            size: 0x400000,
            sha1: "98dc1d122ffb9b5c52994dea2b5d8c4f004a5f8e",
            role: Role::Samples,
        },
        RomFile {
            name: "epr-19967.31",
            size: 0x80000,
            sha1: "9f86d6e365a5535d354ff6b0614f3a19c0790d0f",
            role: Role::AudioCpu,
        },
    ],
};

/// The water-ski cabinet, from the game's input test (labels in
/// comments). Ports read active low. One credit is two coins by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        // The panel: SELECT(UP), SET, SELECT(DOWN). SET starts a game and
        // confirms on the select screens.
        button("set", 1, 0x01, &["Enter", "KeyS"], &[PadButton::Start, PadButton::South]).labelled("Set"),
        button("selup", 1, 0x02, &["KeyW"], &[PadButton::DPadUp]).labelled("▲"),
        button("seldown", 0, 0x40, &["KeyX"], &[PadButton::DPadDown]).labelled("▼"),
        // PITCH LEFT / PITCH RIGHT: switches on the foot controller; the
        // game asks for pitch at jump ramps (what each does, unconfirmed).
        button("pitchleft", 1, 0x04, &["KeyQ"], &[PadButton::LeftShoulder]).labelled("Pitch L"),
        button("pitchright", 1, 0x08, &["KeyE"], &[PadButton::RightShoulder]).labelled("Pitch R"),
    ],
    // SLIDE: the platform sliding sideways; low steers left (confirmed
    // by scripted runs).
    analogs: &[AnalogInput {
        name: "slide",
        channel: 0,
        centre: 0x80,
        span: 112,
        axis: PadAxis::LeftStickX,
        keys_neg: &["ArrowLeft"],
        keys_pos: &["ArrowRight"],
        pad_neg: &[PadButton::DPadLeft],
        pad_pos: &[PadButton::DPadRight],
    }],
    guns: &[],
    coin_start: "coin1@1100-1104,coin1@1120-1124,set@1250-1254",
};
