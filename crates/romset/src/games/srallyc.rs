use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, RomFile};

/// Sega Rally Championship (revision C), Model 2A-CRX. Work in progress
/// (README, "What's known about `srallyc`", `docs/hw/model2a.md`).
///
/// Chip pairs are listed low half-word first. Program `17888c.12` and
/// `17889c.13`; main data pairs `.10/.11`, `.8/.9` and `.6/.7`, **packed** 4
/// MB apart from `0x2000000` (the game uploads texels from `0x2200000` and
/// reads model data at `0x2860000`, in the third pair; the 8 MB slots of
/// the 2B and 2C boards put the pairs elsewhere). The polygon ROM is two
/// 32-bit banks of 1M words, packed from model address `0x800000` and
/// **not paired as the numbers suggest**: bank 0 is `.16` (low half) with
/// `.20` (high), bank 1 is `.17` with `.21`. The texture ROM (texture point
/// lists and headers) is `.24` and `.25`, **`.25` first**: it supplies the
/// even 16-bit words. `.29/.28` (low half first) and `.31/.32` are the
/// TGP's data ROM, packed from address `0x800000` on its external bus (the
/// first holds, under a count of 300, the course data the collision
/// functions read; the second slowly varying bytes); the six `opr` chips
/// are the TGP's lookup tables (three pairs, `.45/.46` holding floats). The zip also holds the older program
/// revision `b` (`epr-17888b.12`, `epr-17889b.13`), `ic12.bin`/`ic13.bin`
/// (`b`'s program under other names) and `mpr-16310.15`, `-16311.16`,
/// `-16312.14`, ramps that no program here reads (the rasteriser's own
/// tables, unconfirmed); none is used.
pub static SRALLYC: Game = Game {
    id: "srallyc",
    title: "Sega Rally Championship",
    year: 1995,
    board: Board::Model2A,
    attract_frame: 0,
    input_lag: 0,
    hud_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::Textures, Role::TgpData],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-17888c.12",
            size: 0x80000,
            sha1: "33abf9cdcee9583dc600c94e1e29ce260e8c5d32",
            role: Role::Program,
        },
        RomFile {
            name: "epr-17889c.13",
            size: 0x80000,
            sha1: "4b1efb3d5644fed1753da1750bf5c300d3a15d2c",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-17746.10",
            size: 0x200000,
            sha1: "f4ada8e5c906fc384bed1b96f09cdf313f89e825",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17747.11",
            size: 0x200000,
            sha1: "5ba63a77e9fc70569af21d50b3171bc8ff4522b8",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17744.8",
            size: 0x200000,
            sha1: "1d187cad375121a45348d640edd3cc7dce658d28",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17745.9",
            size: 0x200000,
            sha1: "ed2b3298aad6f4e52dc672a0168183e457564b43",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17884.6",
            size: 0x200000,
            sha1: "81d927b8c4f9d0c4c5e29d676b30f30f83751fdc",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17885.7",
            size: 0x200000,
            sha1: "9449ac8f8f9ce8d8e536b05a91e46841fed7f2d0",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-17748.16",
            size: 0x200000,
            sha1: "283cc49bfb6c6381a7ead9273fd097dca5b981b6",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17750.20",
            size: 0x200000,
            sha1: "4d470e71df61298282c356814e2d151fda323fb6",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17749.17",
            size: 0x200000,
            sha1: "704175c8b29e4c989afcb7be42e7e0e096740eaf",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17751.21",
            size: 0x200000,
            sha1: "601542149d33ca52a47536b4b0af47bf1fd87eb2",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-17753.25",
            size: 0x200000,
            sha1: "dd5fd3c9592360d3e95623ac2491e6faabe9dbcb",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-17752.24",
            size: 0x200000,
            sha1: "1d342f87d1af1e5438d1ae818b1b14268e765897",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-17754.29",
            size: 0x200000,
            sha1: "c0a9b690523a529e4015e9af10dc3fb2a1726f08",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-17755.28",
            size: 0x200000,
            sha1: "e60803ae951489fe47d66731d15c32249ca547b4",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-17756.31",
            size: 0x200000,
            sha1: "1f1ee3f19a6bcf57bc5a1c7dd64ee83f8b81f084",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-17757.32",
            size: 0x200000,
            sha1: "1d3a0e441d150ada0535a9d50e2f69dd4b99c584",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-17886.36",
            size: 0x200000,
            sha1: "103c4838b27378c834c08d29d6fb6ba95e7f9d03",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-17887.37",
            size: 0x200000,
            sha1: "a85f05160b060d9d4a431aaa73cfc03f24214fb9",
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
            name: "epr-17890.30",
            size: 0x40000,
            sha1: "3635333d36463b6fab25560ed918e05138f964dc",
            role: Role::AudioCpu,
        },
    ],
};

/// The cabinet's controls. The buttons are the bits the game's input test
/// shows (port 0 bit 0 "CHUTE1", bit 1 "CHUTE2", bit 5 "VR", bit 6 "START",
/// and the test and service switches; port 1 bits 4 and 5 "SHIFT3" and
/// "SHIFT4"), each found by holding one bit at a time on that screen; the
/// input test has "SHIFT1" and "SHIFT2" too, which none of the other
/// bits of the four ports moves. The wheel, accelerator and brake are the
/// test's HANDLE, ACCEL and BRAKE: analog channels 0, 1 and 2 (each shows
/// the value it was given). The game wants two coins for a credit by
/// default ("CREDIT 1/2" after the first), so the coin and start script
/// pays twice. Ports read active low. Which way each analog value runs
/// is *unconfirmed* here.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("view", 0, 0x20, &["KeyV"], &[PadButton::North]).labelled("View"),
        button("shift3", 1, 0x10, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Gear 3"),
        button("shift4", 1, 0x20, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Gear 4"),
    ],
    analogs: &[
        AnalogInput {
            name: "wheel",
            channel: 0,
            centre: 0x80,
            span: 112,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
        },
        AnalogInput {
            name: "gas",
            channel: 1,
            centre: 0x00,
            span: 255,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
        },
        AnalogInput {
            name: "brake",
            channel: 2,
            centre: 0x00,
            span: 255,
            axis: PadAxis::LeftTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown"],
            pad_neg: &[],
            pad_pos: &[],
        },
    ],
    guns: &[],
    coin_start: "coin1@1100-1104,coin1@1150-1154,start@1300-1304",
};
