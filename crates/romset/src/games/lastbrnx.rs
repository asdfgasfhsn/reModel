use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role};

/// Last Bronx (Export, Rev A), Model 2B-CRX: Sega's, with the shared
/// geometrizer program plus three commands of its own and a copro program
/// of its own. Pairs are listed low half-word first: program `.15/.16`
/// (512 KB chips, the Export revision's `epr-19061a/19062a`: the zip also
/// holds the Japanese (`epr-19046a/19047a`) and US (`epr-19059a/19060a`)
/// revisions' program pairs, the clones `lastbrnxj` and `lastbrnxu`, which
/// aren't supported); main data `.11/.12` at `0x2000000` and `.9/.10` at
/// `0x2800000` (4 MB chips, 8 MB a pair, so packed or not is the same);
/// polygons `.17/.21`; textures `.27/.25`. The SHARC programs are in the
/// program ROM, so there is no `.7/.8` pair and no copro data ROMs. Its
/// textures go to the lower half of each texture RAM bank, as `indy500`'s.
pub static LASTBRNX: Game = Game {
    id: "lastbrnx",
    title: "Last Bronx",
    year: 1996,
    genre: "Weapons fighting",
    kind: crate::Kind::Fighting,
    board: Board::Model2B,
    // The attract mode's fight in the Cross Street stage.
    attract_frame: 700,
    input_lag: 0,
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
    recommended: crate::Recommended { shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0
    // (docs/games/lastbrnx.md).
    idle_loops: &[crate::IdleLoop { start: 0x73f4, end: 0x7408 }],
    packed: &[],
    // A coin-start run's audio averages -32.7 dBFS RMS (peak -13), with the
    // games here that play at 0 dB (docs/hw/sound.md §6).
    volume_db: 0.0,
    files: &[
        rom("epr-19061a.15", 0x80000, "fa63081b0aa6f02c3d197485865ee38e9c78b43d", Role::Program),
        rom("epr-19062a.16", 0x80000, "a85ca36a537ba21d11ef3cfdf914c2c93ac5e68f", Role::Program),
        rom("mpr-19050.11", 0x400000, "abdf7aa4c594f0916d4335c70fdd67dc6b1f4630", Role::MainData),
        rom("mpr-19051.12", 0x400000, "bec22f657c6d939c095b99ca9c6eb44b9683fd72", Role::MainData),
        rom("mpr-19048.9", 0x400000, "cc5f8e61fee07aa4fc5bfe2d011088ee523c77c2", Role::MainData),
        rom("mpr-19049.10", 0x400000, "5955885ad2bfd69d7a2c4e1d1df907aca41fbdd0", Role::MainData),
        rom("mpr-19052.17", 0x400000, "b393af96522306dc2e055aea1e837979f41940d4", Role::Polygons),
        rom("mpr-19053.21", 0x400000, "950a92209b7c24f66db62c31627a1f1d52721f1e", Role::Polygons),
        rom("mpr-19055.27", 0x200000, "99c49fe135dc46fa861337b5bac654ae8478778a", Role::Textures),
        rom("mpr-19054.25", 0x200000, "f618e2b9b26a1f7eccebfc8f8e17ef8ad9029be8", Role::Textures),
        rom("mpr-19056.31", 0x80000, "baa039cd86650b6cd81f295916c4d256e60cb29c", Role::AudioCpu),
        rom("mpr-19057.32", 0x400000, "aa008f83e1eff0daafe01944248ebae6054cee9f", Role::Samples),
        rom("mpr-19058.34", 0x400000, "7c89cba757bd58747ed0d633b2fe7ef559fcd15e", Role::Samples),
    ],
};

/// Two players, each with an 8-way stick and three buttons (punch, kick,
/// guard). Ports read active low; which bit is which comes from the game's
/// input test (docs/games/lastbrnx.md). One coin is one
/// credit.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("punch", 1, 0x01, &["KeyX"], &[PadButton::South]).labelled("Punch"),
        button("kick", 1, 0x02, &["KeyV"], &[PadButton::East]).labelled("Kick"),
        button("guard", 1, 0x04, &["KeyZ"], &[PadButton::West]).labelled("Guard"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("punch2", 2, 0x01, &[], &[]),
        button("kick2", 2, 0x02, &[], &[]),
        button("guard2", 2, 0x04, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    // One coin, start, then punch and kick every half second: that takes
    // the first fighter (Yusaku) and fights Tommy in stage 1.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,punch@1400-99999/30,kick@1415-99999/30",
};
