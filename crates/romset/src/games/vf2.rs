use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role, RomFile};

/// Virtua Fighter 2 (2.1), Model 2A-CRX (`docs/games/vf2.md`, `docs/hw/model2a.md`).
///
/// Chip roles are by analogy with `srallyc`, not yet checked against what
/// the code reads: program `18385.12`/`18386.13` then `18387.14`/`18388.15`
/// (older revisions `17568/17569.12/13` with `17562/17563.14/15`, in
/// letter variants, are in the zip and unused); main data pairs
/// `.10/.11 .8/.9 .6/.7 .4/.5`, polygons `.16/.20 .17/.21 .18/.22`,
/// textures `.25/.24 .27/.26`. The samples, `.31 .32 .36 .37` back to back
/// from the sound 68000's `0x800000` as in `srallyc`, are checked against
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
    recommended: crate::Recommended::DEFAULT,
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::Textures, Role::TgpData],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-18385.12",
            size: 0x20000,
            sha1: "471c19389ceeec6138107dd81863320bd4825327",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18386.13",
            size: 0x20000,
            sha1: "0f51e389e13efc172a26471331a60c459ad43c38",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18387.14",
            size: 0x20000,
            sha1: "26fb787451824fc6060724e37fe0ba6bb66796cb",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18388.15",
            size: 0x20000,
            sha1: "da79e51ad501b9560c4ed7cf1ec768daad93efe0",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-17560.10",
            size: 0x200000,
            sha1: "88e9a8b6b0f58c96957015179e7ff10f837040e6",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17561.11",
            size: 0x200000,
            sha1: "e154877380b9250d8119dd4c14ba306c7b337dcd",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17558.8",
            size: 0x200000,
            sha1: "9a34724958fef9b49eae39c6ea136e0cf532154b",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17559.9",
            size: 0x200000,
            sha1: "2f094ff0b95bf1cd5c283414634ea9597204d374",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17566.6",
            size: 0x200000,
            sha1: "ad4d1ba5e5b39b2d87105ae80750284867aa4ed3",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17567.7",
            size: 0x200000,
            sha1: "7e0700ded530e4eb58e9a68cdb92791284c91431",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17564.4",
            size: 0x200000,
            sha1: "57666b6937f79bb65c43ed02b04a454882d01e61",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17565.5",
            size: 0x200000,
            sha1: "d9ba93998286713758385033119416714674c8d8",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17554.16",
            size: 0x200000,
            sha1: "c0624e58de2e427465daaa10dbb02ea2a1fd0f1b",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17548.20",
            size: 0x200000,
            sha1: "09d19abe5d75a335df7510df8abb2d4425159cdf",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17555.17",
            size: 0x200000,
            sha1: "720c4628d7783f0323b5723b441e13741556241e",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17549.21",
            size: 0x200000,
            sha1: "0570604dc2007288795a3125ffd480bc4b3b0802",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17556.18",
            size: 0x200000,
            sha1: "55b909d2bc2079d0dfed5036c78c9e09bce09843",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17550.22",
            size: 0x200000,
            sha1: "f14fdf275905a90a0d4cc534d90b0302f26676d8",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17553.25",
            size: 0x200000,
            sha1: "c627b25a1f61a9fe9182e2199f70f6e485503c7b",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-17552.24",
            size: 0x200000,
            sha1: "0ac1111f2ecb4f924b5119eaaac8fa7bc87ab9d1",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-17547.27",
            size: 0x200000,
            sha1: "5c1196a6454a4fead79a930979f2e69639ec2bb9",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-17546.26",
            size: 0x200000,
            sha1: "c6d8524dc0a879394f1234b7bb04836081bb3830",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-17573.31",
            size: 0x200000,
            sha1: "4c61a135819862df02347c118dc4d88a0adac273",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-17572.32",
            size: 0x200000,
            sha1: "9683ea9bedfc5cd7b4a28e9a68792c0dc549d911",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-17571.36",
            size: 0x200000,
            sha1: "cbbde1c55eddbeeefd283bb5afd79a670a282e3a",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-17570.37",
            size: 0x200000,
            sha1: "4c7ebdea08b2dedf621f121785ed1c40ebae4236",
            role: Role::Samples,
        },
        RomFile {
            name: "opr-14742a.45",
            size: 0x20000,
            sha1: "f46429fffcee17d056f56d5fe035a33f1fd6c27e",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14743a.46",
            size: 0x20000,
            sha1: "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14744.58",
            size: 0x20000,
            sha1: "651f1db4089a400d073b19ada299b4b08b08f372",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14745.59",
            size: 0x20000,
            sha1: "e3349ece0e47f684d61ad11bfea4a90602287350",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14746.62",
            size: 0x20000,
            sha1: "34e047a93459406c22acf4c25089d1a4955f94ca",
            role: Role::CoproData,
        },
        RomFile {
            name: "opr-14747.63",
            size: 0x20000,
            sha1: "7d7ec300eeb9a8de1590011e37108688c092f329",
            role: Role::CoproData,
        },
        RomFile {
            name: "epr-17574.30",
            size: 0x80000,
            sha1: "b6c0c3f0473bd7fc3ef4f5146110dfcc899a5af9",
            role: Role::AudioCpu,
        },
    ],
};

/// Stick and punch, kick, guard, as `fvipers`: unconfirmed for this game.
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
