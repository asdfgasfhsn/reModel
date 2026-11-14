use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Protection, ProtectionInterface, ProtectionReply, Role};

/// Dynamite Cop (Export), Model 2A-CRX (`docs/games/dynamcop.md`,
/// `docs/hw/model2a.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13` then
/// `.14/.15`; main data `.10/.11` and `.8/.9` (8 MB each) from `0x2000000`
/// and `.6/.7` then `.4/.5` in the second data bank at `0x6000000`
/// (`Role::MainDataHigh`, as `motoraid`'s: the program's pointers land in
/// `0x2000000-0x2FFFFFF` and `0x6000000-0x6FFFFFF`); polygons `.16/.20
/// .17/.21 .18/.22 .19/.23` packed (each pairing makes the most sane
/// floats); textures `.25/.24` and `.27/.26`, the second in its own 8 MB
/// slot as `vf2`'s; the TGP's maths tables as every 2A set, no TGP data
/// ROM; the sound program `.30` and the samples `.31 .32 .36 .37` back to
/// back (the order the sound program's sample table gives). The ramps
/// `16310`-`16312` and the clones' program chips in the zip are unused
/// (docs/games/dynamcop.md).
pub static DYNAMCOP: Game = Game {
    id: "dynamcop",
    title: "Dynamite Cop",
    year: 1998,
    board: Board::Model2A,
    attract_frame: 3600,
    input_lag: 0,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    // Two boot checks of a keyed device at 0x1D80000/0x1D90000: a 20-byte
    // block from ROM 0x1FFFC0 with key 0x1326, then one from 0x1FFF80 with
    // key 0x7700, each answer's 16 bytes compared with ROM 0xBC7F0 and
    // 0xBC800. A mismatch fills 64 KB of work RAM with 0xFF and the game
    // crashes (docs/games/dynamcop.md).
    protection: Some(Protection {
        interface: ProtectionInterface::Keyed,
        replies: &[
            ProtectionReply { command: 0x1326, offset: None, rom: 0xbc7f0, len: 16 },
            ProtectionReply { command: 0x7700, offset: None, rom: 0xbc800, len: 16 },
        ],
    }),
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0, as
    // motoraid's and srallyc's do (docs/games/dynamcop.md).
    idle_loops: &[crate::IdleLoop { start: 0x71194, end: 0x711a8 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        rom("epr-20930.12", 0x80000, "53b0f9dc8494effa077170ddced2d95f43a5f134", Role::Program),
        rom("epr-20931.13", 0x80000, "5e266b5e153a0d9a57360cfd1af81e3a58a2fb7d", Role::Program),
        rom("epr-20932.14", 0x80000, "3022283dded4d08d790d034b6d543c0397b5bf5a", Role::Program),
        rom("epr-20933.15", 0x80000, "a741a0205c1b3664ab4d09d6d991a768269a79ea", Role::Program),
        rom("mpr-20797.10", 0x400000, "af2b2d82364621a1528d6ed59fbfbf15dc93ee72", Role::MainData),
        rom("mpr-20798.11", 0x400000, "8c2e210ac7c7b133ba9befc79a07c4ca6b4e3f18", Role::MainData),
        rom("mpr-20795.8", 0x400000, "97c657edd98cde6f0780a04a7711e5b370087a60", Role::MainData),
        rom("mpr-20796.9", 0x400000, "24fda2cd458cf7a3db485564c02ac61d30cbdf5e", Role::MainData),
        rom("mpr-20793.6", 0x400000, "e70b55709067628ea0bf3f5190a300100b61eed1", Role::MainDataHigh),
        rom("mpr-20794.7", 0x400000, "d90e86d38bda12f2d0f99e23a42928f05bde3ea8", Role::MainDataHigh),
        rom("mpr-20791.4", 0x400000, "b98af63e81f6c1b2766d7e96acbd1821bba000d4", Role::MainDataHigh),
        rom("mpr-20792.5", 0x400000, "a333885872a64b322f3cb464a70352d73654b1b3", Role::MainDataHigh),
        rom("mpr-20799.16", 0x400000, "18a4e8d0e968fff3b645b59a0023b0ef38d51924", Role::Polygons),
        rom("mpr-20803.20", 0x400000, "0215b5de6d10f0852ac0ca4e10475e10243e39c7", Role::Polygons),
        rom("mpr-20800.17", 0x400000, "dc0c470c6b410ab991ef0e09ce1cc0f63c8a204d", Role::Polygons),
        rom("mpr-20804.21", 0x400000, "7bd2ae89f9cc7c0570dbaffe5f54aea2dfa1b39e", Role::Polygons),
        rom("mpr-20801.18", 0x400000, "d0861366c4123c833a325df5345f951386a94d1a", Role::Polygons),
        rom("mpr-20805.22", 0x400000, "7c95bfe2e95bae3d59c3c9efe1f40b5bc292ad44", Role::Polygons),
        rom("mpr-20802.19", 0x400000, "b90909849fbe0f62d5ec7c38608c84e7fa845ebf", Role::Polygons),
        rom("mpr-20806.23", 0x400000, "5f32a56857e2213b110c32deea184dba882e34b8", Role::Polygons),
        rom("mpr-20809.25", 0x400000, "c6f1a1fd2684f352d3846b7f859b0405fa2d667a", Role::Textures),
        rom("mpr-20807.24", 0x400000, "3f7fa1d7d3d398bc8d5295bc1df6fe11405d20d9", Role::Textures),
        rom("mpr-20810.27", 0x400000, "a658f1864829058b1d419e7c001e47cd0ab06a20", Role::Textures),
        rom("mpr-20808.26", 0x400000, "f857b303afda6301b19d97dfe5c313126261716e", Role::Textures),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-20811.30", 0x80000, "2640c6b6966f4a888329e583b6b713bd0e779b6b", Role::AudioCpu),
        rom("mpr-20812.31", 0x200000, "09682d18144e60d740a9d7a3e19db6f76fa581f1", Role::Samples),
        rom("mpr-20813.32", 0x200000, "32913385f09da2e43af0c4a4612b955527bfe759", Role::Samples),
        rom("mpr-20814.36", 0x200000, "731ce721bb9e148f3a9f7fbe569522567a681c4e", Role::Samples),
        rom("mpr-20815.37", 0x200000, "32b4bf6c096fdccdd5d8f1ddb6c27d3389a52234", Role::Samples),
    ],
};

/// A stick and three buttons a player, from the game's input test
/// (docs/games/dynamcop.md): port 0 the coins, test, service and starts,
/// port 1 player 1's punch, kick, jump and directions, port 2 player 2's.
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
        button("jump", 1, 0x04, &["KeyZ"], &[PadButton::West]).labelled("Jump"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("punch2", 2, 0x01, &[], &[]),
        button("kick2", 2, 0x02, &[], &[]),
        button("jump2", 2, 0x04, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    shifter: None,
    // One coin a credit, start, then punch every 20 frames (which picks the
    // first fighter on PLAYER SELECT) and kick every 50, standing still.
    coin_start: "coin1@1100-1104,start@1250-1254,punch@1400-99999/20,kick@1410-99999/50",
};
