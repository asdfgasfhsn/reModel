use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role};

/// Sky Target, Model 2A-CRX (`docs/games/skytargt.md`, `docs/hw/model2a.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13`; main
/// data `.10/.11 .8/.9` from `0x2000000`, and `.6/.7` as the second bank
/// at `0x6000000` (the 3D colours the game copies to palette RAM are read
/// at `0x640FB5C`); polygons `.16/.20 .17/.21`; textures `.25/.24`; the
/// TGP's data ROM `.28/.29`, **`.28` the low half** as `daytona`'s (so
/// paired its words are counts and floats); the TGP's tables as every 2A
/// set; the sound program `.30` and the samples `.31 .32 .36 .37` back to
/// back (the order the sound program's sample table gives). The ramps
/// `16310`-`16312` are unused (docs/games/skytargt.md).
pub static SKYTARGT: Game = Game {
    id: "skytargt",
    title: "Sky Target",
    year: 1995,
    board: Board::Model2A,
    attract_frame: 2000,
    input_lag: 1,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0, as
    // hotd's does (docs/games/skytargt.md).
    idle_loops: &[crate::IdleLoop { start: 0x379d4, end: 0x379e8 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        rom("epr-18406.12", 0x80000, "01cd519daaf6138d9df4940bf8bb5923a1f163df", Role::Program),
        rom("epr-18407.13", 0x80000, "faf6dcf8f345c1e7968823f2dba60afcd88f37c2", Role::Program),
        rom("mpr-18415.10", 0x400000, "3061cc68755ca36255f325135aa44659afc3498c", Role::MainData),
        rom("mpr-18416.11", 0x400000, "6ffeef418364df9e08398c7564142cbf5750beb2", Role::MainData),
        rom("mpr-18417.8", 0x400000, "88b97a76f0a85a3977915808eee4d64b69734e88", Role::MainData),
        rom("mpr-18418.9", 0x400000, "cf7c6887519e53d7fa321a2ad888b1673e16565b", Role::MainData),
        rom("epr-18404.6", 0x80000, "d6805faea657ea0f998fb2470d7d24aa78a02bd4", Role::MainDataHigh),
        rom("epr-18405.7", 0x80000, "21b6b390d8635349ba76899acea176954a24985e", Role::MainDataHigh),
        rom("mpr-18413.16", 0x400000, "2bd6eae4ab5751d485be105a06776fccd3c48d21", Role::Polygons),
        rom("mpr-18409.20", 0x400000, "6f622a82fd5ffd7a4692b5bf51b76755053a674b", Role::Polygons),
        rom("mpr-18414.17", 0x400000, "1729f6ff689a462a3d6e303ebc2dac323145a67c", Role::Polygons),
        rom("mpr-18410.21", 0x400000, "139cbba0ceffa83c0f9925258944ec8a414b3040", Role::Polygons),
        rom("mpr-18412.25", 0x400000, "66796f6c20e680a87e8939a70692680b1dd0b324", Role::Textures),
        rom("mpr-18411.24", 0x400000, "842a647a70ef29a8c775e88c0bcbc63782496bba", Role::Textures),
        rom("mpr-18420.28", 0x200000, "b6949b745d0bedeecd6d0240f8911cb345c16d8d", Role::TgpData),
        rom("mpr-18419.29", 0x200000, "37230e96dd526fb47fcbde5778e5466d8955a969", Role::TgpData),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-18408.30", 0x80000, "30e1894432a0765c64b93dd5ca7ca17ef58ac6c0", Role::AudioCpu),
        rom("mpr-18424.31", 0x200000, "826f167d7a4f5d30466b2f75f0123187c29c2d69", Role::Samples),
        rom("mpr-18423.32", 0x200000, "ae69c9d4e333579d826178d2863156dc784aedef", Role::Samples),
        rom("mpr-18422.36", 0x200000, "49669be09e10dfae7fddce0fc4e415466cb29566", Role::Samples),
        rom("mpr-18421.37", 0x200000, "5dbbf2ba008adad36929fcecb7c2c1e5ffd12618", Role::Samples),
    ],
};

/// A flight stick with a machine gun trigger and a missile button, from the
/// game's input test (docs/games/skytargt.md): port 0 holds the coins,
/// test, service, the view button and start (as `srallyc`'s), port 1 bits
/// 4 and 5 the machine gun and missile; the stick is analog channels 2
/// (left-right) and 0 (up-down).
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("view", 0, 0x20, &["KeyV"], &[PadButton::North]).labelled("View"),
        button("start", 0, 0x40, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("gun", 1, 0x10, &["KeyX", "Space"], &[PadButton::South, PadButton::RightTrigger])
            .labelled("Machine gun"),
        button("missile", 1, 0x20, &["KeyZ"], &[PadButton::East, PadButton::LeftTrigger]).labelled("Missile"),
    ],
    analogs: &[
        AnalogInput {
            name: "stickx",
            channel: 2,
            centre: 0x80,
            span: 127,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: None,
        },
        AnalogInput {
            name: "sticky",
            channel: 0,
            centre: 0x80,
            // High moves the plane up the screen.
            span: -127,
            axis: PadAxis::LeftStickY,
            keys_neg: &["ArrowUp"],
            keys_pos: &["ArrowDown"],
            pad_neg: &[PadButton::DPadUp],
            pad_pos: &[PadButton::DPadDown],
            key_ramp: None,
        },
    ],
    guns: &[],
    shifter: None,
    // Two coins (two credits a play by default), start, then hold the
    // machine gun (it fires while held) and fire a missile every second.
    coin_start: "coin1@1100-1104,coin1@1130-1134,start@1300-1304,gun@1400-99999,missile@1400-99999/60",
};
