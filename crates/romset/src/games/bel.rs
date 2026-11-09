use crate::controls::button;
use crate::{Board, Controls, Game, Gun, GunSource, PadButton, Reload, Role, RomFile};

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
    board: Board::Model2C,
    // The attract demo: the vehicle driving up a forest track.
    attract_frame: 3900,
    input_lag: 3,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    packed: &[Role::MainData, Role::Polygons],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-20225.15",
            size: 0x20000,
            sha1: "cc6da75aafcbbc86720435182a66e8de065c8e99",
            role: Role::Program,
        },
        RomFile {
            name: "epr-20226.16",
            size: 0x20000,
            sha1: "204b3cc6bbfdc92b4871c45fe4abff4ab4a66317",
            role: Role::Program,
        },
        RomFile {
            name: "epr-20223.13",
            size: 0x20000,
            sha1: "03c308c58a72bf3b78f41d5a9c0adaa7aad631c2",
            role: Role::Program,
        },
        RomFile {
            name: "epr-20224.14",
            size: 0x20000,
            sha1: "f3b126e2fcef1cf673b239696ed8018241b1170e",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-20233.11",
            size: 0x200000,
            sha1: "a3f15cd68a514cf80f0a40dbbb08e8b0489a0e4b",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20234.12",
            size: 0x200000,
            sha1: "386d0d07738f579cb23e4168aceb26f56bcca1c1",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20231.9",
            size: 0x200000,
            sha1: "aa52ae307aa37faaaf86c326642af1946c5f4056",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20232.10",
            size: 0x200000,
            sha1: "f9138813f6d1ca2126f5de10d8d69dcbb533aa0e",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20229.7",
            size: 0x200000,
            sha1: "510b6d41f1d32a9929379ba76037db137164cd43",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20230.8",
            size: 0x200000,
            sha1: "d4f6d4fba7f43b21f0bf9d948ec93b372425bf7c",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20227.5",
            size: 0x200000,
            sha1: "fff27006659458300001425261b944e690f1d494",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20228.6",
            size: 0x200000,
            sha1: "ee3273302830f3499c7d4e548b629c51e0369e8a",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-20247.27",
            size: 0x200000,
            sha1: "5e9d38509c1e5273079a342a64ca2c956cd47e6d",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-20245.25",
            size: 0x200000,
            sha1: "a462e094c9a9ec4743e4bf2c4ce23357257a2a54",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-20248.28",
            size: 0x200000,
            sha1: "a231aeb7b984f5b927144f0eec4ef2282429494f",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-20246.26",
            size: 0x200000,
            sha1: "cd1d4bc0fcf89e47884b87863a09bb263bce72cc",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-20244.17",
            size: 0x200000,
            sha1: "59302e7119c9ff779ce0c871713fe3688c29cccb",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20240.21",
            size: 0x200000,
            sha1: "c70252b0b6f17aa0cd9b5264d4166df8ab7d1784",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20243.18",
            size: 0x200000,
            sha1: "b0bdc7f42450c8d9cebbcf43cf858f7399e378e4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20239.22",
            size: 0x200000,
            sha1: "1c634fbbcbafb1c3825117682901a3264599b246",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20242.19",
            size: 0x200000,
            sha1: "7b7724127b27834eaaa228050ceb779d8a027882",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20238.23",
            size: 0x200000,
            sha1: "239d2c9c49cb8ddc0d6aa956a497b494217f38d7",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20241.20",
            size: 0x200000,
            sha1: "7d6ab9c0ccec77676222611bf200d2e067e20520",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20237.24",
            size: 0x200000,
            sha1: "6e0a0323d6a804f1f1e4404694cc1ea7dfbf2d95",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-20236.29",
            size: 0x200000,
            sha1: "e7fde1fd509531e1002ff813163067dc0d134536",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-20235.30",
            size: 0x200000,
            sha1: "a60deabb662e9c09f5d6342dc1a1c6045744d93f",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-20250.32",
            size: 0x200000,
            sha1: "b0e7e493fb20ebf30c17378199e49d529ffb3f20",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-20251.33",
            size: 0x200000,
            sha1: "95b8d3dc29e87e6537b288d8e946728e0b345dd0",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-20252.34",
            size: 0x200000,
            sha1: "9e511e89e99c77f06a5fba033ca8f9b98bd86f91",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-20253.35",
            size: 0x200000,
            sha1: "f6df2483ca75573449ba36638dbbed4be7843a44",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-20249.31",
            size: 0x20000,
            sha1: "66ab8e843319d07663ef13f3d2299c6c7414071f",
            role: Role::AudioCpu,
        },
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
