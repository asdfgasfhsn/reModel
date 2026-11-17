use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role};

/// Wave Runner, Model 2C-CRX. Pairs are listed low half-word first, and
/// each bank's pairs fill 8 MB slots, smaller pairs repeating (as stcc).
/// Beyond topskatr's layout: a third polygon pair (`.19/.23`, 1 MB), a
/// second texture pair (`.28/.26`, 1 MB), a 1 MB copro data pair
/// (`.29/.30`), and `.7/.8`, data the program reads through the
/// `0x06000000` window (docs/games/waverunr.md). No music
/// board: all sound is the SCSP's. The ride-on cabinet's own board is
/// optional (the game plays without it).
pub static WAVERUNR: Game = Game {
    id: "waverunr",
    title: "Wave Runner",
    year: 1996,
    genre: "Jet-ski racing",
    kind: crate::Kind::Racing,
    board: Board::Model2C,
    // The attract demo's race, three craft side by side.
    attract_frame: 1800,
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
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
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/waverunr.md).
    idle_loops: &[crate::IdleLoop { start: 0x353c4, end: 0x353d8 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-19282a.15", 0x80000, "a136bb80746f37450be51f98ca60791b4022035d", Role::Program),
        rom("epr-19283a.16", 0x80000, "428f156f60e61ef314b7b50474abddf6d4dc2aca", Role::Program),
        rom("mpr-19291.11", 0x400000, "25e25d9ecf3b3b1c8f5e017243cc5e02f7a13015", Role::MainData),
        rom("mpr-19292.12", 0x400000, "0e86e21e1c88263c548e03bce48ed4ce75643596", Role::MainData),
        rom("mpr-19293.9", 0x400000, "0497e886ccd5e5ef0cd8670200bf4cf64d9bfc2b", Role::MainData),
        rom("mpr-19294.10", 0x400000, "a898b03d66973a49deb9799102ab1faf4384c376", Role::MainData),
        rom("epr-19278.7", 0x80000, "c91eb2d68acd6ded394e0bd9f504cbb8f421c3ed", Role::MainDataHigh),
        rom("epr-19279.8", 0x80000, "da46379cabe5f151160b2558e255e9b64eaca22e", Role::MainDataHigh),
        rom("mpr-19286.27", 0x400000, "1fe53306e370da724df5caace720107798fb24b9", Role::Textures),
        rom("mpr-19285.25", 0x400000, "2b5441b9d7d613b0a90dfea243e9de44980d219d", Role::Textures),
        rom("epr-19303.28", 0x80000, "f4b776028f581329effa583022d8d65e889b6b0a", Role::Textures),
        rom("epr-19302.26", 0x80000, "572b49a4d0189d0513c27753e3563909d1977f03", Role::Textures),
        rom("mpr-19287.17", 0x400000, "8e677a106ceb9355632fadbdb837dc4a23c83684", Role::Polygons),
        rom("mpr-19288.21", 0x400000, "1e680680354d873c515c955256b0e4e72451326a", Role::Polygons),
        rom("mpr-19289.18", 0x400000, "f36033765c93274a1bcdd311fb2026aa2a8a0e42", Role::Polygons),
        rom("mpr-19290.22", 0x400000, "6e6bf7c709202221e03a06d9e53147d67ab4404f", Role::Polygons),
        rom("epr-19304.19", 0x80000, "2fdb9344441c625c3841dfc62e424ddaf16416d5", Role::Polygons),
        rom("epr-19305.23", 0x80000, "93db2c6d672c5c4bf1623edad8d4237c65c4f4e1", Role::Polygons),
        rom("epr-19280.29", 0x80000, "909663f440d19a34591d1f9707972c313e34f909", Role::CoproData),
        rom("epr-19281.30", 0x80000, "39ba8a35fdcfdd6c88b44ab392ca0e958da44767", Role::CoproData),
        rom("mpr-19295.32", 0x400000, "2a6d1b14ea3c031cad9905e4b9b6973755689ee1", Role::Samples),
        rom("mpr-19296.34", 0x400000, "3a258e0f7c642d043cbab5f94dfe69fac8561e93", Role::Samples),
        rom("epr-19284.31", 0x40000, "5e36fc7cca92e2eab7d65434cb39597505a2f8cf", Role::AudioCpu),
    ],
};

/// The ride-on (personal watercraft) cabinet. Ports read active low. Which
/// bit and channel is which comes from the game's input test; the effects
/// were confirmed by scripted runs except where noted (docs/games/waverunr.md). One credit is three coins by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        // Not in this cabinet's input test (one coin chute); the common
        // Model 2 layout, unconfirmed.
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter"], &[PadButton::Start]),
        button("view", 0, 0x80, &["KeyV"], &[PadButton::North]).labelled("View"),
    ],
    analogs: &[
        // Handlebar: 0x30 steers left, 0xD0 right. Also moves the course
        // selection.
        AnalogInput {
            name: "handlebar",
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
        // Throttle lever: 0xFF released, 0x00 fully open (about 100 km/h);
        // it also confirms the course choice.
        AnalogInput {
            name: "throttle",
            channel: 2,
            centre: 0xff,
            span: -255,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp", "KeyZ"],
            pad_neg: &[],
            pad_pos: &[PadButton::South],
            key_ramp: None,
        },
        // The body's roll sensor. Calibrated like the handlebar and read
        // in play (a lean past +-0x20 from centre triggers something);
        // no effect seen in scripted runs yet. Unconfirmed.
        AnalogInput {
            name: "roll",
            channel: 1,
            centre: 0x80,
            span: 112,
            axis: PadAxis::RightStickX,
            keys_neg: &["KeyA"],
            keys_pos: &["KeyD"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        // The body's pitch sensor: shown in the input test, never read in
        // play.
        AnalogInput {
            name: "pitch",
            channel: 3,
            centre: 0x80,
            span: 112,
            axis: PadAxis::None,
            keys_neg: &[],
            keys_pos: &[],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
    ],
    // Three coins, start, the throttle to pick the first course, then the
    // throttle held open through the race.
    guns: &[],
    shifter: None,
    coin_start: "coin1@1100-1104,coin1@1120-1124,coin1@1140-1144,start@1250-1254,throttle=0x00@1300-1700,throttle=0x00@1900-99999",
};
