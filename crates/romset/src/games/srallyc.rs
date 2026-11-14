use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, Shifter};

/// Sega Rally Championship (revision C), Model 2A-CRX (docs/games/srallyc.md, `docs/hw/model2a.md`).
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
/// even 16-bit words. `.29/.28` (low half first) are the TGP's data ROM,
/// from address `0x800000` on its external bus (under a count of 300, the
/// course data the collision functions read). The sound board's 8 MB of
/// samples are `.31`, `.32`, `.36`, `.37` in that order, back to back from
/// the 68000's `0x800000`: the sound program's sample table puts samples
/// in all four 2 MB slots, and only this order makes each chip's data end
/// where its slot's last sample does and the loops repeat after their ends
/// (`.31/.32` were taken for TGP data, which the TGP never reads there;
/// docs/games/srallyc.md); the six `opr` chips
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
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0 (code
    // the game copies to work RAM) (docs/games/srallyc.md).
    idle_loops: &[crate::IdleLoop { start: 0x5a379c, end: 0x5a37b4 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::Textures, Role::TgpData],
    volume_db: 10.0,
    files: &[
        rom("epr-17888c.12", 0x80000, "33abf9cdcee9583dc600c94e1e29ce260e8c5d32", Role::Program),
        rom("epr-17889c.13", 0x80000, "4b1efb3d5644fed1753da1750bf5c300d3a15d2c", Role::Program),
        rom("mpr-17746.10", 0x200000, "f4ada8e5c906fc384bed1b96f09cdf313f89e825", Role::MainData),
        rom("mpr-17747.11", 0x200000, "5ba63a77e9fc70569af21d50b3171bc8ff4522b8", Role::MainData),
        rom("mpr-17744.8", 0x200000, "1d187cad375121a45348d640edd3cc7dce658d28", Role::MainData),
        rom("mpr-17745.9", 0x200000, "ed2b3298aad6f4e52dc672a0168183e457564b43", Role::MainData),
        rom("mpr-17884.6", 0x200000, "81d927b8c4f9d0c4c5e29d676b30f30f83751fdc", Role::MainData),
        rom("mpr-17885.7", 0x200000, "9449ac8f8f9ce8d8e536b05a91e46841fed7f2d0", Role::MainData),
        rom("mpr-17748.16", 0x200000, "283cc49bfb6c6381a7ead9273fd097dca5b981b6", Role::Polygons),
        rom("mpr-17750.20", 0x200000, "4d470e71df61298282c356814e2d151fda323fb6", Role::Polygons),
        rom("mpr-17749.17", 0x200000, "704175c8b29e4c989afcb7be42e7e0e096740eaf", Role::Polygons),
        rom("mpr-17751.21", 0x200000, "601542149d33ca52a47536b4b0af47bf1fd87eb2", Role::Polygons),
        rom("mpr-17753.25", 0x200000, "dd5fd3c9592360d3e95623ac2491e6faabe9dbcb", Role::Textures),
        rom("mpr-17752.24", 0x200000, "1d342f87d1af1e5438d1ae818b1b14268e765897", Role::Textures),
        rom("mpr-17754.29", 0x200000, "c0a9b690523a529e4015e9af10dc3fb2a1726f08", Role::TgpData),
        rom("mpr-17755.28", 0x200000, "e60803ae951489fe47d66731d15c32249ca547b4", Role::TgpData),
        rom("mpr-17756.31", 0x200000, "1f1ee3f19a6bcf57bc5a1c7dd64ee83f8b81f084", Role::Samples),
        rom("mpr-17757.32", 0x200000, "1d3a0e441d150ada0535a9d50e2f69dd4b99c584", Role::Samples),
        rom("mpr-17886.36", 0x200000, "103c4838b27378c834c08d29d6fb6ba95e7f9d03", Role::Samples),
        rom("mpr-17887.37", 0x200000, "a85f05160b060d9d4a431aaa73cfc03f24214fb9", Role::Samples),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-17890.30", 0x40000, "3635333d36463b6fab25560ed918e05138f964dc", Role::AudioCpu),
    ],
};

/// The cabinet's controls. The buttons are the bits the game's input test
/// shows (port 0 bit 0 "CHUTE1", bit 1 "CHUTE2", bit 5 "VR", bit 6 "START",
/// and the test and service switches), each found by holding one bit at a
/// time on that screen. The **shift lever** is an H gate read as three
/// bits of port 1, which the game's input polling turns into a gear with
/// an 8-entry table (*confirmed* from its code): bit 4 alone is 3rd, bit 5
/// alone 4th, bit 6 with bit 4 is 1st, bit 6 with bit 5 2nd, anything else
/// neutral; so "SHIFT1" and "SHIFT2" of the input test are two-bit
/// combinations, which is why holding single bits found only SHIFT3 and
/// SHIFT4. The lever only matters with a manual (MT) car: the wheel picks
/// one on the car select screen. The wheel, accelerator and brake are the
/// test's HANDLE, ACCEL and BRAKE, analog channels 0, 1 and 2: *confirmed*
/// by driving, the wheel is centred at 0x80 (the car runs straight) and
/// steers left below it and right above it; the pedals read 0x00 released
/// (the car stays still) and 0xFF fully down. The game wants two coins
/// for a credit by default ("CREDIT 1/2" after the first), so the coin and
/// start script pays twice. Ports read active low.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("view", 0, 0x20, &["KeyV"], &[PadButton::North]).labelled("View"),
        // The H gate (manual cars), latched by the shifter below: a gear
        // key puts the lever in that gear until another is chosen.
        button("gear1", 1, 0x50, &["KeyQ"], &[]).labelled("Gear 1"),
        button("gear2", 1, 0x60, &["KeyW"], &[]).labelled("Gear 2"),
        button("gear3", 1, 0x10, &["KeyE"], &[]).labelled("Gear 3"),
        button("gear4", 1, 0x20, &["KeyR"], &[]).labelled("Gear 4"),
        // Virtual buttons (no bits): neutral and stepping through the gate.
        button("neutral", 1, 0x00, &["KeyX"], &[]).labelled("Neutral"),
        button("shiftup", 1, 0x00, &["KeyA"], &[PadButton::RightShoulder, PadButton::East]).labelled("Gear +"),
        button("shiftdown", 1, 0x00, &["KeyZ"], &[PadButton::LeftShoulder, PadButton::West]).labelled("Gear −"),
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
            key_ramp: None,
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
            key_ramp: None,
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
            key_ramp: None,
        },
    ],
    guns: &[],
    shifter: Some(Shifter {
        gears: &["gear1", "gear2", "gear3", "gear4"],
        neutral: "neutral",
        up: "shiftup",
        down: "shiftdown",
    }),
    coin_start: "coin1@1100-1104,coin1@1150-1154,start@1300-1304",
};
