use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, RomFile};

/// Dynamite Baseball '97 (Rev A, Japan), Model 2B-CRX: the first of Sega's
/// own 2B games here, uploading its own two SHARC programs. Pairs are
/// listed low half-word first, each in an 8 MB slot: program `.15/.16`
/// then `.13/.14`; main data `.11/.12` at `0x2000000` and `.9/.10` at
/// `0x2800000`, a second bank `.7/.8` and `.5/.6` at `0x6000000` (as in
/// hotd). The memory test passes all twelve in this layout. Polygons
/// `.17/.21` then `.18/.22` and textures `.27/.25` draw correctly. Four
/// sample ROMs, no copro data, no music board (README, "What's known about
/// dynabb97").
pub static DYNABB97: Game = Game {
    id: "dynabb97",
    title: "Dynamite Baseball '97",
    year: 1997,
    board: Board::Model2B,
    // The attract demo: a batter at the plate facing the pitcher.
    attract_frame: 1850,
    hud_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    packed: &[],
    // CRX_drv 3.2.8, as hotd and skisuprg: a coin-start run's `make sound`
    // averages -34 dBFS RMS, between theirs (docs/hw/sound.md §6).
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-19833a.15",
            size: 0x80000,
            sha1: "b04613d564c04c35feafccad56ed85810d894185",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19834a.16",
            size: 0x80000,
            sha1: "c535ab4b38ffd42f03eed6a5a1706e867eaccd67",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19831a.13",
            size: 0x80000,
            sha1: "8e80e2627aafe395d8ced4a97ba50cd9a781fb45",
            role: Role::Program,
        },
        RomFile {
            name: "epr-19832a.14",
            size: 0x80000,
            sha1: "d770dfd70aa14dcc716aa47e6cbf26f32649f294",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-19841.11",
            size: 0x400000,
            sha1: "d527f46865d00a91d5b38a93dc38baf62f372cb1",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19842.12",
            size: 0x400000,
            sha1: "29ed4a005b52f6e16492998183ec4e5f7475022b",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19839.9",
            size: 0x400000,
            sha1: "ddea9cfc0a14461448acae2eed2092829ef3b418",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19840.10",
            size: 0x400000,
            sha1: "2a325ee39f9d719399040ed2a41123bcf0c6f385",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-19837.7",
            size: 0x400000,
            sha1: "1da108a2ec00e3fc472b1a819655aff8c679051d",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19838.8",
            size: 0x400000,
            sha1: "0cc0edd0a9c288143168d63a7d48d0fbfa64d8bf",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19835.5",
            size: 0x400000,
            sha1: "dcde1946008ab86c7fca212ec57c1cc468f30c58",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19836.6",
            size: 0x400000,
            sha1: "fd56bb284eb66e6c078b386a0db1c2b10dc1dd4a",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-19843.17",
            size: 0x400000,
            sha1: "8889a9438d8f3ea50058372ad03ebd4653f23313",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19845.21",
            size: 0x400000,
            sha1: "63e5859518172f88a5ba98b69309d4162c233cf0",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19844.18",
            size: 0x400000,
            sha1: "3ea5c3e41eb95e715860619f771bc580c91b095f",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19846.22",
            size: 0x400000,
            sha1: "58eab07976972917c345a8d3a50ff1e96e5fa798",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-19848.27",
            size: 0x400000,
            sha1: "e8db7125be8a052e41a00c69cc08ca0d75b3b96f",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-19847.25",
            size: 0x400000,
            sha1: "b0b6135b23349d7d6ae007002d8df83748cab7b1",
            role: Role::Textures,
        },
        RomFile {
            name: "epr-19849.31",
            size: 0x80000,
            sha1: "1fb824adaf3ed330a8039be726a87eb85c00abd7",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-19850.32",
            size: 0x200000,
            sha1: "a7189ad398138a91f96b192cb7c112c0301dcda4",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19851.33",
            size: 0x200000,
            sha1: "8765bdb1d471dbeea065a97ae131f2d8f78aa13d",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19852.34",
            size: 0x200000,
            sha1: "14d55f127da6673c538c2ef9be34a4e02ca449f3",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-19853.35",
            size: 0x200000,
            sha1: "cf51fafb3d45bf799b9ccb407bee862e15c95981",
            role: Role::Samples,
        },
    ],
};

/// Two players, each with an 8-way stick, two buttons and the bat: a lever
/// on an analog channel that is pulled back and let go to swing. Ports
/// read active low; which bit and channel is which comes from the game's
/// input test (README, "What's known about dynabb97"). One coin is one
/// credit by default.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        // SW #1 confirms on the menus (the team and pitcher select); what
        // SW #2 does is unconfirmed.
        button("sw1", 1, 0x01, &["KeyZ"], &[PadButton::South]).labelled("Button 1"),
        button("sw2", 1, 0x02, &["KeyX"], &[PadButton::East]).labelled("Button 2"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("sw1_2", 2, 0x01, &[], &[]),
        button("sw2_2", 2, 0x02, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    // The bat levers, channel 0 for player 1 and 1 for player 2. The input
    // test's BAT VOLUME reads 0 at 0x50 and 0xFF from 0xC0 up; pulling to
    // 0xC0 and letting go back to 0x50 swings (holding it doesn't). That
    // 0x50 is where the lever rests is inferred from that.
    analogs: &[
        AnalogInput {
            name: "bat",
            channel: 0,
            centre: 0x50,
            span: 0x70,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["Space"],
            pad_neg: &[],
            pad_pos: &[],
        },
        AnalogInput {
            name: "bat2",
            channel: 1,
            centre: 0x50,
            span: 0x70,
            axis: PadAxis::None,
            keys_neg: &[],
            keys_pos: &[],
            pad_neg: &[],
            pad_pos: &[],
        },
    ],
    guns: &[],
    // One coin, start, then button 1 every second (it picks the team and
    // pitcher and moves on), and the bat pulled and let go every 40 frames.
    coin_start: "coin1@1100-1104,start@1250-1254,sw1@1300-99999/60,bat=0xc0@2300-99999/40",
};
