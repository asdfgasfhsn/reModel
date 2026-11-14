use crate::controls::button;
use crate::{Board, Controls, Game, Gun, GunSource, PadButton, Reload, Role, RomFile};

/// Virtua Cop 2, Model 2A-CRX (`docs/games/vcop2.md`, `docs/hw/model2a.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13` (the
/// code) then `.14/.15` (data); main data `.10/.11 .8/.9 .6/.7 .4/.5`
/// packed from `0x2000000`; one polygon pair `.16/.20`; one texture pair
/// `.25/.24`; the TGP's tables as every 2A set, no TGP data ROM; the sound
/// program `.30` and the samples `.31 .32 .36 .37` back to back (the order
/// the sound program's sample table gives). The ramps `16310`-`16312` are
/// unused. Two light guns on hotd's gun interface (docs/games/vcop2.md).
pub static VCOP2: Game = Game {
    id: "vcop2",
    title: "Virtua Cop 2",
    year: 1995,
    board: Board::Model2A,
    attract_frame: 1500,
    input_lag: 0,
    run_ahead_max: 0,
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
    // The wait for the next frame: polls a work RAM byte (0x500000) until an
    // interrupt handler changes it, as vf2's does (docs/games/vcop2.md).
    idle_loops: &[crate::IdleLoop { start: 0x36fc, end: 0x3708 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-18524.12",
            size: 0x80000,
            sha1: "2979f8470cc31e6c5c32c6fec1a87dbd29b52309",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18525.13",
            size: 0x80000,
            sha1: "6b4188f04aad80b89f1826e8ca47cff763980410",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18518.14",
            size: 0x80000,
            sha1: "bed4ec9a5e59807d17e5e602bdaf3c68fcba08b6",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18519.15",
            size: 0x80000,
            sha1: "caf3c2676508a2ed032d3657ac640a257f04bdd4",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-18516.10",
            size: 0x200000,
            sha1: "5a9695fb5eda394a1111a05ee5fb9cce29970e91",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18517.11",
            size: 0x200000,
            sha1: "a4434bce019729e2148a95e3a6dea38de7f789c1",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18514.8",
            size: 0x200000,
            sha1: "006fb22eefdd9205ede9a74fe53cbffe8c8fd45b",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18515.9",
            size: 0x200000,
            sha1: "70f493aa4eb93edce8dd5b7b532d1f50f81069ce",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18522.6",
            size: 0x200000,
            sha1: "cc467cb26a8fccc48837d000fe9e1c41b0c0f4f9",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18523.7",
            size: 0x200000,
            sha1: "40d8231d184582c0fc01ad874371aaec7dfcc337",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18520.4",
            size: 0x80000,
            sha1: "44c4b5560d150909342e4182496f136c8c5e2edb",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18521.5",
            size: 0x80000,
            sha1: "11956fe912c34d6a86a6b91d55987f6bead73473",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18513.16",
            size: 0x200000,
            sha1: "edc2798c4d88975ce67b54fc0db008e7d24db6ef",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18510.20",
            size: 0x200000,
            sha1: "8a8597aa31609663869e584cc5fad6e4b84f7dbe",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18512.25",
            size: 0x200000,
            sha1: "774eba886083b0dad9a47519c5801e44346312cf",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18511.24",
            size: 0x200000,
            sha1: "f21474486f0dc4092cbad4566deea8a952862ab7",
            role: Role::Textures,
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
            name: "epr-18530.30",
            size: 0x80000,
            sha1: "ad297c7fecaa9b877f0dd31e859983816947e437",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-18529.31",
            size: 0x200000,
            sha1: "258418c1cb37338a694e48f3b48fadfae5f40239",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18528.32",
            size: 0x200000,
            sha1: "78ba93ab90322152efc37f7130073b0dc516ef5d",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18527.36",
            size: 0x200000,
            sha1: "26563f425f2f0906ae9278fe5de02955653d49fe",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18526.37",
            size: 0x200000,
            sha1: "8f13cb02c76f7b7cd11f3c3772ff13302d55e9c3",
            role: Role::Samples,
        },
    ],
};

/// Two light guns and two start buttons, from the game's input test and
/// GUN MARK CHECK (docs/games/vcop2.md). Port 0 holds the coins, test,
/// service and the starts, port 1 bits 0 and 1 the triggers. The guns are
/// read through hotd's gun interface (Y in counters 0/1, X in 2/3 for
/// player 1, then player 2's; a "sees no screen" bit a player in 8); the
/// ranges are the game's default calibration, which it maps onto the whole
/// view.
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
    guns: &[
        Gun {
            name: "gun1",
            source: GunSource::Counters { x: 2, y: 0, offscreen_bit: 0 },
            x_range: (137, 630),
            y_range: (36, 425),
            trigger: "trigger1",
            reload: Reload::Offscreen,
            keys_reload: &["KeyR"],
        },
        Gun {
            name: "gun2",
            source: GunSource::Counters { x: 6, y: 4, offscreen_bit: 1 },
            x_range: (134, 627),
            y_range: (36, 425),
            trigger: "trigger2",
            reload: Reload::Offscreen,
            keys_reload: &[],
        },
    ],
    shifter: None,
    // One coin a credit, start, then shoot BEGINNER on the stage select; in
    // play aim at the middle and either side in turn, fire every 20 frames
    // and shoot off screen (reload) every seventh shot. The game takes a
    // shot's aim two frames after the trigger, so the aims and the
    // off-screen pulses come two frames after their trigger pulses.
    coin_start: "coin1@1100-1104,start@1300-1304,gun1=85:150@1350-1420,trigger1@1400-1404,gun1=248:170@1500-6400,gun1=140:190@1582-6400/120,gun1=360:190@1642-6400/120,gun1=off@1562-6400/140,trigger1@1560-6400/20",
};
