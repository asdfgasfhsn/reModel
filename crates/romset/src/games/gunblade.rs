use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, Gun, GunSource, PadButton, Reload, Role};

/// Gunblade NY (Rev A), Model 2B-CRX: the program writes both SHARCs' host
/// windows (`0x8C0000`, `0x840000`) and the 2B sound UART (`0x9C0000`).
/// Pairs are listed low half-word first: program `.15/.16`; main data
/// `.11/.12` then `.9/.10`; polygons `.17/.21` then `.18/.22`; textures
/// `.27/.25`; copro data `.29/.30` on the copro SHARC's bus; samples `.32`
/// then `.34`. The memory test passes every ROM in this layout. Its own
/// two SHARC programs; nothing new from the board model. Two mounted guns
/// on the ADC, as on bel (docs/games/gunblade.md).
pub static GUNBLADE: Game = Game {
    id: "gunblade",
    title: "Gunblade NY",
    year: 1995,
    genre: "Light gun",
    kind: crate::Kind::Shooting,
    board: Board::Model2B,
    // The attract demo: a New York street with a truck.
    attract_frame: 1500,
    scenes: &[],
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
    // (docs/games/gunblade.md).
    idle_loops: &[crate::IdleLoop { start: 0x6a65c, end: 0x6a674 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-18988a.15", 0x80000, "fcfb0a4691cd7d66168c421e4e1694ecaea56ab2", Role::Program),
        rom("epr-18989a.16", 0x80000, "92bffbf1250c53499c37a53f9e2a054fc7bf256f", Role::Program),
        rom("mpr-18974.11", 0x400000, "dcdfe9f59158cec2f02b213ee13f5e40cdb92e55", Role::MainData),
        rom("mpr-18975.12", 0x400000, "34a0b32eeed1a9f41bca8b9261851881b2ba79f2", Role::MainData),
        rom("mpr-18976.9", 0x400000, "892063e91b2ed20e0600d4b188da1e9f45a19692", Role::MainData),
        rom("mpr-18977.10", 0x400000, "c11d2c9e1e215aa7b2ebb777639c8cd651901f52", Role::MainData),
        rom("mpr-18986.29", 0x400000, "5eb6682399b358d77658d82e612b02b724e3f3e1", Role::CoproData),
        rom("mpr-18987.30", 0x400000, "0a04a1049d2da486dc9dbb97b383bd24259b78c8", Role::CoproData),
        rom("mpr-18980.17", 0x400000, "7b8bdcfe88953ce1b2d75af2ce4712ab6507e2cf", Role::Polygons),
        rom("mpr-18981.21", 0x400000, "a76193f70adb6abeba02328b290af5cca47d4e25", Role::Polygons),
        rom("mpr-18982.18", 0x400000, "95404baed88cc95b75ff9b9084d09622961d3e57", Role::Polygons),
        rom("mpr-18983.22", 0x400000, "550f8248699b9267da7d2e64002be56972381714", Role::Polygons),
        rom("mpr-18985.27", 0x400000, "2c487fb743730cacf92dbea952b1efada0f073df", Role::Textures),
        rom("mpr-18984.25", 0x400000, "095964de773f515d64d65dbc8f8ef9bae97e5ba9", Role::Textures),
        rom("epr-18990.31", 0x80000, "759b4683dc7149e04f41ddac7bd395e8d07ea858", Role::AudioCpu),
        rom("mpr-18978.32", 0x400000, "6c2cd6236cb001bb8d487a9b1e9907519dc43daa", Role::Samples),
        rom("mpr-18979.34", 0x400000, "a8165116b5e07e031ff960201dd8c9a441544961", Role::Samples),
    ],
};

/// Two mounted guns, from the game's input test. Ports read active low.
/// One coin a credit by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("trigger1", 1, 0x01, &["Space"], &[]).labelled("Trigger"),
        button("trigger2", 1, 0x02, &[], &[]).labelled("Trigger 2"),
    ],
    analogs: &[],
    // Each gun's yaw and pitch are potentiometers on the ADC (player 1 on
    // channels 0 and 2, player 2 on 1 and 3, from the input test): yaw grows
    // to the right, pitch downwards. The ranges put the crosshair on the
    // aimed pixel with a fresh EEPROM's calibration, to within 4 pixels
    // (the game's mapping of yaw bends slightly). There's no reloading:
    // the reload controls just fire.
    guns: &[
        Gun {
            name: "gun1",
            source: GunSource::Analog { x: 0, y: 2 },
            x_range: (113, 242),
            y_range: (30, 164),
            trigger: "trigger1",
            reload: Reload::Offscreen,
            keys_reload: &[],
        },
        Gun {
            name: "gun2",
            source: GunSource::Analog { x: 1, y: 3 },
            x_range: (113, 242),
            y_range: (30, 164),
            trigger: "trigger2",
            reload: Reload::Offscreen,
            keys_reload: &[],
        },
    ],
    // One coin and start, a shot at EASY on the mission select, then aim at
    // the middle and either side in turn and fire every 4 frames.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,gun1=140:180@1260-1500,trigger1@1300-1400/20,gun1=248:190@1500-6400,gun1=120:220@1580-6400/120,gun1=380:220@1640-6400/120,trigger1@1500-6400/4",
};
