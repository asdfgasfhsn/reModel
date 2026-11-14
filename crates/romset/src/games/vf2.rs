use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role};

/// Virtua Fighter 2 (2.1), Model 2A-CRX (`docs/games/vf2.md`, `docs/hw/model2a.md`).
///
/// Chip roles are by analogy with `srallyc`, not yet checked against what
/// the code reads: program `18385.12`/`18386.13` then `18387.14`/`18388.15`
/// (older revisions `17568/17569.12/13` with `17562/17563.14/15`, in
/// letter variants, are in the zip and unused); main data pairs
/// `.10/.11 .8/.9 .6/.7 .4/.5`, polygons `.16/.20 .17/.21 .18/.22`,
/// textures `.25/.24 .27/.26` (the second texture pair in its own 8 MB
/// slot, at 16-bit word `0x400000`: `Role::Textures` is not packed). The
/// samples, `.31 .32 .36 .37` back to back from the sound 68000's
/// `0x800000` as in `srallyc`, are checked against
/// the sound program's own sample table the same way (docs/games/vf2.md),
/// not yet by ear.
pub static VF2: Game = Game {
    id: "vf2",
    title: "Virtua Fighter 2",
    year: 1995,
    board: Board::Model2A,
    attract_frame: 1500,
    input_lag: 1,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[1],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended { shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls a work RAM byte (0x500000) until an
    // interrupt handler changes it (docs/games/vf2.md).
    idle_loops: &[crate::IdleLoop { start: 0x10f98, end: 0x10fa4 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        rom("epr-18385.12", 0x20000, "471c19389ceeec6138107dd81863320bd4825327", Role::Program),
        rom("epr-18386.13", 0x20000, "0f51e389e13efc172a26471331a60c459ad43c38", Role::Program),
        rom("epr-18387.14", 0x20000, "26fb787451824fc6060724e37fe0ba6bb66796cb", Role::Program),
        rom("epr-18388.15", 0x20000, "da79e51ad501b9560c4ed7cf1ec768daad93efe0", Role::Program),
        rom("mpr-17560.10", 0x200000, "88e9a8b6b0f58c96957015179e7ff10f837040e6", Role::MainData),
        rom("mpr-17561.11", 0x200000, "e154877380b9250d8119dd4c14ba306c7b337dcd", Role::MainData),
        rom("mpr-17558.8", 0x200000, "9a34724958fef9b49eae39c6ea136e0cf532154b", Role::MainData),
        rom("mpr-17559.9", 0x200000, "2f094ff0b95bf1cd5c283414634ea9597204d374", Role::MainData),
        rom("mpr-17566.6", 0x200000, "ad4d1ba5e5b39b2d87105ae80750284867aa4ed3", Role::MainData),
        rom("mpr-17567.7", 0x200000, "7e0700ded530e4eb58e9a68cdb92791284c91431", Role::MainData),
        rom("mpr-17564.4", 0x200000, "57666b6937f79bb65c43ed02b04a454882d01e61", Role::MainData),
        rom("mpr-17565.5", 0x200000, "d9ba93998286713758385033119416714674c8d8", Role::MainData),
        rom("mpr-17554.16", 0x200000, "c0624e58de2e427465daaa10dbb02ea2a1fd0f1b", Role::Polygons),
        rom("mpr-17548.20", 0x200000, "09d19abe5d75a335df7510df8abb2d4425159cdf", Role::Polygons),
        rom("mpr-17555.17", 0x200000, "720c4628d7783f0323b5723b441e13741556241e", Role::Polygons),
        rom("mpr-17549.21", 0x200000, "0570604dc2007288795a3125ffd480bc4b3b0802", Role::Polygons),
        rom("mpr-17556.18", 0x200000, "55b909d2bc2079d0dfed5036c78c9e09bce09843", Role::Polygons),
        rom("mpr-17550.22", 0x200000, "f14fdf275905a90a0d4cc534d90b0302f26676d8", Role::Polygons),
        rom("mpr-17553.25", 0x200000, "c627b25a1f61a9fe9182e2199f70f6e485503c7b", Role::Textures),
        rom("mpr-17552.24", 0x200000, "0ac1111f2ecb4f924b5119eaaac8fa7bc87ab9d1", Role::Textures),
        rom("mpr-17547.27", 0x200000, "5c1196a6454a4fead79a930979f2e69639ec2bb9", Role::Textures),
        rom("mpr-17546.26", 0x200000, "c6d8524dc0a879394f1234b7bb04836081bb3830", Role::Textures),
        rom("mpr-17573.31", 0x200000, "4c61a135819862df02347c118dc4d88a0adac273", Role::Samples),
        rom("mpr-17572.32", 0x200000, "9683ea9bedfc5cd7b4a28e9a68792c0dc549d911", Role::Samples),
        rom("mpr-17571.36", 0x200000, "cbbde1c55eddbeeefd283bb5afd79a670a282e3a", Role::Samples),
        rom("mpr-17570.37", 0x200000, "4c7ebdea08b2dedf621f121785ed1c40ebae4236", Role::Samples),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-17574.30", 0x80000, "b6c0c3f0473bd7fc3ef4f5146110dfcc899a5af9", Role::AudioCpu),
    ],
};

/// Stick and punch, kick, guard, as `fvipers`: confirmed on the game's own
/// input test (docs/games/vf2.md), every switch of both players.
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
    shifter: None,
    // Two coins (the default setting is two a credit: "CREDIT 1/2" after
    // one), start, then punch and kick every half second: that picks the
    // fighter under the cursor (Akira) and fights.
    coin_start: "coin1@1100-1104,coin1@1130-1134,start@1250-1254,punch@1400-99999/30,kick@1415-99999/30",
};
