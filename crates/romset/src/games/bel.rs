use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, Gun, GunSource, PadButton, Reload, Role};

/// Behind Enemy Lines, Model 2C-CRX (Sega / EPL). Pairs are listed low
/// half-word first. Program `.15/.16` then `.13/.14` (128 KB chips, back to
/// back). The main data and polygon pairs are 16 Mbit chips packed every
/// 4 MB: main data `.11/.12`, `.9/.10`, `.7/.8`, `.5/.6` from `0x2000000`
/// (the memory test passes every ROM so); textures keep 8 MB slots. Two
/// mounted guns on the ADC, EPL's own sound driver (`.31`), no music board
/// (docs/games/bel.md).
pub static BEL: Game = Game {
    id: "bel",
    title: "Behind Enemy Lines",
    year: 1997,
    genre: "Shooter",
    kind: crate::Kind::Shooting,
    board: Board::Model2C,
    // The attract demo: the vehicle driving up a forest track.
    attract_frame: 3900,
    scenes: &[],
    input_lag: 3,
    run_ahead_max: 3,
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
    recommended: crate::Recommended { depth: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls a work RAM byte (0x5B48A0) until an
    // interrupt handler changes it (docs/games/bel.md).
    idle_loops: &[crate::IdleLoop { start: 0xe28, end: 0xe34 }],
    packed: &[Role::MainData, Role::Polygons],
    volume_db: 0.0,
    files: &[
        rom("epr-20225.15", 0x20000, "cc6da75aafcbbc86720435182a66e8de065c8e99", Role::Program),
        rom("epr-20226.16", 0x20000, "204b3cc6bbfdc92b4871c45fe4abff4ab4a66317", Role::Program),
        rom("epr-20223.13", 0x20000, "03c308c58a72bf3b78f41d5a9c0adaa7aad631c2", Role::Program),
        rom("epr-20224.14", 0x20000, "f3b126e2fcef1cf673b239696ed8018241b1170e", Role::Program),
        rom("mpr-20233.11", 0x200000, "a3f15cd68a514cf80f0a40dbbb08e8b0489a0e4b", Role::MainData),
        rom("mpr-20234.12", 0x200000, "386d0d07738f579cb23e4168aceb26f56bcca1c1", Role::MainData),
        rom("mpr-20231.9", 0x200000, "aa52ae307aa37faaaf86c326642af1946c5f4056", Role::MainData),
        rom("mpr-20232.10", 0x200000, "f9138813f6d1ca2126f5de10d8d69dcbb533aa0e", Role::MainData),
        rom("mpr-20229.7", 0x200000, "510b6d41f1d32a9929379ba76037db137164cd43", Role::MainData),
        rom("mpr-20230.8", 0x200000, "d4f6d4fba7f43b21f0bf9d948ec93b372425bf7c", Role::MainData),
        rom("mpr-20227.5", 0x200000, "fff27006659458300001425261b944e690f1d494", Role::MainData),
        rom("mpr-20228.6", 0x200000, "ee3273302830f3499c7d4e548b629c51e0369e8a", Role::MainData),
        rom("mpr-20247.27", 0x200000, "5e9d38509c1e5273079a342a64ca2c956cd47e6d", Role::Textures),
        rom("mpr-20245.25", 0x200000, "a462e094c9a9ec4743e4bf2c4ce23357257a2a54", Role::Textures),
        rom("mpr-20248.28", 0x200000, "a231aeb7b984f5b927144f0eec4ef2282429494f", Role::Textures),
        rom("mpr-20246.26", 0x200000, "cd1d4bc0fcf89e47884b87863a09bb263bce72cc", Role::Textures),
        rom("mpr-20244.17", 0x200000, "59302e7119c9ff779ce0c871713fe3688c29cccb", Role::Polygons),
        rom("mpr-20240.21", 0x200000, "c70252b0b6f17aa0cd9b5264d4166df8ab7d1784", Role::Polygons),
        rom("mpr-20243.18", 0x200000, "b0bdc7f42450c8d9cebbcf43cf858f7399e378e4", Role::Polygons),
        rom("mpr-20239.22", 0x200000, "1c634fbbcbafb1c3825117682901a3264599b246", Role::Polygons),
        rom("mpr-20242.19", 0x200000, "7b7724127b27834eaaa228050ceb779d8a027882", Role::Polygons),
        rom("mpr-20238.23", 0x200000, "239d2c9c49cb8ddc0d6aa956a497b494217f38d7", Role::Polygons),
        rom("mpr-20241.20", 0x200000, "7d6ab9c0ccec77676222611bf200d2e067e20520", Role::Polygons),
        rom("mpr-20237.24", 0x200000, "6e0a0323d6a804f1f1e4404694cc1ea7dfbf2d95", Role::Polygons),
        rom("mpr-20236.29", 0x200000, "e7fde1fd509531e1002ff813163067dc0d134536", Role::CoproData),
        rom("mpr-20235.30", 0x200000, "a60deabb662e9c09f5d6342dc1a1c6045744d93f", Role::CoproData),
        rom("mpr-20250.32", 0x200000, "b0e7e493fb20ebf30c17378199e49d529ffb3f20", Role::Samples),
        rom("mpr-20251.33", 0x200000, "95b8d3dc29e87e6537b288d8e946728e0b345dd0", Role::Samples),
        rom("mpr-20252.34", 0x200000, "9e511e89e99c77f06a5fba033ca8f9b98bd86f91", Role::Samples),
        rom("mpr-20253.35", 0x200000, "f6df2483ca75573449ba36638dbbed4be7843a44", Role::Samples),
        rom("mpr-20249.31", 0x20000, "66ab8e843319d07663ef13f3d2299c6c7414071f", Role::AudioCpu),
    ],
};

/// Two mounted guns, from the game's input test and gun calibration.
/// Ports read active low; test and service are bits 3 and 2. One coin a
/// credit by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("service", 0, 0x04, &[], &[]),
        button("test", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("trigger1", 1, 0x01, &["Space"], &[]).labelled("Trigger"),
        button("trigger2", 1, 0x02, &[], &[]).labelled("Trigger 2"),
        button("missile1", 1, 0x10, &["KeyX"], &[]).labelled("Missile"),
        button("missile2", 1, 0x20, &[], &[]).labelled("Missile 2"),
    ],
    analogs: &[],
    // Each gun's yaw and pitch are potentiometers on the ADC: yaw grows to
    // the right, pitch upwards. The ranges put the crosshair on the aimed
    // pixel with a fresh EEPROM's calibration; it can't reach the view's
    // edges, so they run past 0-255 and clamp. The guns can't point off
    // the screen: the reload controls press the missile button instead.
    guns: &[
        Gun {
            name: "gun1",
            source: GunSource::Analog { x: 0, y: 2 },
            x_range: (-19, 273),
            y_range: (273, -18),
            trigger: "trigger1",
            reload: Reload::Button("missile1"),
            keys_reload: &[],
        },
        Gun {
            name: "gun2",
            source: GunSource::Analog { x: 1, y: 3 },
            x_range: (-19, 273),
            y_range: (273, -18),
            trigger: "trigger2",
            reload: Reload::Button("missile2"),
            keys_reload: &[],
        },
    ],
    // One coin and start, then aim at the middle and either side in turn,
    // fire every 8 frames and a missile every 300.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,gun1=248:190@2200-9000,gun1=120:220@2280-9000/120,gun1=380:220@2340-9000/120,trigger1@2260-9000/8,missile1@2500-9000/300",
};
