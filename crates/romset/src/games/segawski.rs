use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role};

/// Sega Water Ski (Japan), Model 2C-CRX. Pairs are listed low half-word
/// first. Program `.15/.16` only; main data `.11/.12` at `0x2000000` and
/// `.9/.10` at `0x2800000` as in skisuprg, plus a second data bank: `.7/.8`
/// at `0x6000000` and the 1 MB pair `.5/.6` at `0x6800000`, repeating in its
/// slot (the game reads it at `0x6C0xxxx`). The memory test checks the ROMs
/// in that order (IC15, 16, 11, 12, 9, 10, 7, 8, 5, 6) and passes them all
/// (docs/games/segawski.md). No drive board or music board.
pub static SEGAWSKI: Game = Game {
    id: "segawski",
    title: "Sega Water Ski",
    year: 1997,
    genre: "Water skiing",
    kind: crate::Kind::Sports,
    board: Board::Model2C,
    hud_windows: &[],
    hud_element_windows: &[],
    attract_frame: 2550,
    input_lag: 0,
    run_ahead_max: 0,
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended { depth: true, shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/segawski.md).
    idle_loops: &[crate::IdleLoop { start: 0x4fd24, end: 0x4fd38 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-19963a.15", 0x80000, "7f1f600222447effb28cf2d56193ea9f45fd0646", Role::Program),
        rom("epr-19964a.16", 0x80000, "c0ccee4eb19d9626dee0f77f08060f1d9708b39d", Role::Program),
        rom("mpr-19980.11", 0x400000, "6633eeb2bc0ddd5eb5994cb7cc39ed4ab2094a7e", Role::MainData),
        rom("mpr-19981.12", 0x400000, "53abaf71e85f874d28a79c8775b1f0ac919fbe22", Role::MainData),
        rom("mpr-19982.9", 0x400000, "e6652111a494165c93d5e9dc7d1c3df46f1a8961", Role::MainData),
        rom("mpr-19983.10", 0x400000, "a32010648046738f64cbc7e1a074b9443bbde865", Role::MainData),
        rom("mpr-19984.7", 0x400000, "8e0fd13f5838218224ce162a55558b75dcda4cbf", Role::MainDataHigh),
        rom("mpr-19985.8", 0x400000, "0d764ec9e136e4d027aefb10e9f025aa2e081573", Role::MainDataHigh),
        rom("epr-19961.5", 0x80000, "8e540a429826d40acef5a34a0b6b1b89e059961b", Role::MainDataHigh),
        rom("epr-19962.6", 0x80000, "2d7290186aaca28f3aaf6656b090e1cbe509aa48", Role::MainDataHigh),
        rom("mpr-19969.27", 0x400000, "f61a6266556d5687cd03031065baa2dd73112666", Role::Textures),
        rom("mpr-19968.25", 0x400000, "f0f4d2dd06cdf745ed07fe428eaa3e3ad030bff6", Role::Textures),
        rom("mpr-19971.28", 0x400000, "c27e0a90dc1183b0cf7f32e324afa6c126f61d37", Role::Textures),
        rom("mpr-19970.26", 0x400000, "24232390f0cac5ffbb17a0093a602363c686fbf8", Role::Textures),
        rom("mpr-19972.17", 0x400000, "c68f662d53fbcec3d2dbd9ebdfd37cbf1ff46408", Role::Polygons),
        rom("mpr-19973.21", 0x400000, "be1f1017ff5f43218de2d99090e4e7db64f7a483", Role::Polygons),
        rom("mpr-19974.18", 0x400000, "7f16e668db5a4673cd909bdfa1b9dc665827b392", Role::Polygons),
        rom("mpr-19975.22", 0x400000, "35bbea4cd42ab39f976cd2ec964c32d5d3982c4a", Role::Polygons),
        rom("mpr-19976.19", 0x400000, "e6dee764301c0ed34e5134a5232898bb42563c20", Role::Polygons),
        rom("mpr-19977.23", 0x400000, "d61448394b7b2036e27bdbf7a062d63f076db9da", Role::Polygons),
        rom("mpr-19978.20", 0x400000, "e7ea6dfb57ab1fe924a80c244b8acfe4aad2b76d", Role::Polygons),
        rom("mpr-19979.24", 0x400000, "65a81f4503c6b9c26650befd183db821545c53e2", Role::Polygons),
        rom("mpr-19986.29", 0x400000, "859e3788c75599295a8b57ed7852f2cbb6a2a738", Role::CoproData),
        rom("mpr-19987.30", 0x400000, "35f41c474af3754152aecefe81e912120823e0ff", Role::CoproData),
        rom("mpr-19988.32", 0x400000, "63efbd9f67b38fddeeed25de660514867e03486a", Role::Samples),
        rom("mpr-19989.34", 0x400000, "98dc1d122ffb9b5c52994dea2b5d8c4f004a5f8e", Role::Samples),
        rom("epr-19967.31", 0x80000, "9f86d6e365a5535d354ff6b0614f3a19c0790d0f", Role::AudioCpu),
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
        key_ramp: None,
    }],
    guns: &[],
    shifter: None,
    coin_start: "coin1@1100-1104,coin1@1120-1124,set@1250-1254",
};
