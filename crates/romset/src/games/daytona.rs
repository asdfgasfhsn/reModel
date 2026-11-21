use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, KeyRamp, PadAxis, PadButton, Role, Shifter};

/// Daytona USA (revision A), the original Model 2 board (`docs/games/daytona.md`,
/// `docs/hw/model2.md`).
///
/// Chip pairs are listed low half-word first. Program `16722a.12`/`.13`
/// (its second 128 KB also at `0x220000`, where that code runs); main
/// data `.10/.11`, `.8/.9`, `.6/.7` packed from `0x2000000` (the boot
/// record's interrupt table is at `0x2802040`, in `.6/.7`, which also hold
/// the TGP program); polygons `.16/.20 .17/.21 .18/.22 .19/.23` and
/// textures `.25/.24 .27/.26` as on 2A (the pictures are right; not checked
/// chip by chip); the TGP's data ROM `.28/.29`, **`.28` the low half**
/// (the reverse of `srallyc`'s: so paired it begins with an identity
/// matrix of floats). The sound board is not the SCSP one
/// (`docs/hw/model2.md` §5): `epr-16720.7` is its 68000 program; the
/// first MultiPCM reads `.32` then `.33`, the second `.4` then `.5` (each
/// pair's first chip holds the sample header table; the banked samples
/// were matched to their MB by the loops, `docs/hw/multipcm.md`). `.8` is
/// not referenced by the sound program (unknown). The zip's other chips belong to the
/// clones and the ramps `16310`-`16312` (as on 2A) are unused.
pub static DAYTONA: Game = Game {
    id: "daytona",
    title: "Daytona USA",
    year: 1994,
    genre: "Racing",
    kind: crate::Kind::Racing,
    board: Board::Model2,
    attract_frame: 1500,
    scenes: &[],
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    link_board: None,
    parent: None,
    // The operator settings as the I/O board's store holds them (the board
    // model's 0x1C00200 buffer, docs/hw/model2.md): the program's own
    // defaults (a "SEGA" header and version, then the settings; the rest
    // of the 128 bytes are 0) with the link ID at byte 0x0B set to SINGLE
    // (0) instead of MASTER, and the CRC-16 (CCITT, over bytes
    // 0x0A-0x7F) at 0x08 to match: a stand-alone cabinet
    // (docs/games/daytona.md).
    backup_ram: &[(
        0,
        &[
            0x53, 0x45, 0x47, 0x41, 0x40, 0x82, 0x02, 0x00, 0x31, 0x3c, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01,
            0x01, 0x01, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x02, 0x14, 0x1c, 0x00, 0x01,
            0x00, 0x01, 0x04, 0x01,
        ],
    )],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The waits for the next frame: poll the frame counter byte at work
    // RAM 0x500000 until the vblank handler changes it (the main loop's,
    // and the boot screens') (docs/games/daytona.md).
    idle_loops: &[crate::IdleLoop { start: 0x1394, end: 0x13a0 }, crate::IdleLoop { start: 0x1b8b8, end: 0x1b8c4 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        rom("epr-16722a.12", 0x20000, "a476a9a3531beef760c88c9634ed4a7d270e8ee7", Role::Program),
        rom("epr-16723a.13", 0x20000, "2039ec1f8da524176fcf85473c10a8b6e49e139a", Role::Program),
        rom("mpr-16528.10", 0x200000, "e22fc8a70b533f7a6191f5952c581fb8f9627906", Role::MainData),
        rom("mpr-16529.11", 0x200000, "da3c922f950dd730ea348eae12aa1cb69cee9a58", Role::MainData),
        rom("mpr-16808.8", 0x200000, "343866a6e2187a8ebc17f6727080f9f2f9ac9200", Role::MainData),
        rom("mpr-16809.9", 0x200000, "8192d8698d6bd52ee11cc28917aff5840c447627", Role::MainData),
        rom("epr-16724a.6", 0x80000, "7fad3b8d03960e5e1f7a6cb36509238977e00fcc", Role::MainData),
        rom("epr-16725a.7", 0x80000, "d0c5581c56500b5266cab8e8151db24fcbdea0d7", Role::MainData),
        rom("mpr-16523.16", 0x200000, "0b83a3fc92b7d913a14cfb01d688c63555c17c41", Role::Polygons),
        rom("mpr-16518.20", 0x200000, "16afe5029591f3536b5b75d9cf50a34d0ea72c3d", Role::Polygons),
        rom("mpr-16524.17", 0x200000, "71b47626ffe5b26d1140afe1b830a9a2be86c88f", Role::Polygons),
        rom("mpr-16519.21", 0x200000, "dac8c281a5e9a6c4b60197e6676f3727264ee420", Role::Polygons),
        rom("mpr-16525.18", 0x200000, "33f5f37ea2e09fc73eed5388b46fdf1fa9e285e6", Role::Polygons),
        rom("mpr-16520.22", 0x200000, "660171674484375a27595630e5e2d2ad76a06d1a", Role::Polygons),
        rom("mpr-16772.19", 0x200000, "1789f35dd403f73f8be18495a0fe4ad1e6841417", Role::Polygons),
        rom("mpr-16771.23", 0x200000, "257a3675e4ef6adbf61285a5daa5954223c28cb2", Role::Polygons),
        rom("mpr-16522.25", 0x200000, "abf7b0fc0f111f90da42463d600db9fa32e95efe", Role::Textures),
        rom("mpr-16521.24", 0x200000, "a6a21a23cd34d0de6d3e6a5c3c2687f905d0dc2a", Role::Textures),
        rom("mpr-16770.27", 0x200000, "8aa933b74d4e05dc49987238705e50b00e5dae73", Role::Textures),
        rom("mpr-16769.26", 0x200000, "8c712ab09e61ef510741a55f29b3c4e497471372", Role::Textures),
        rom("mpr-16537.28", 0x200000, "b32fd1d3fc8983fb5f2a7b236b665a8c9b52769f", Role::TgpData),
        rom("mpr-16536.29", 0x200000, "2018468d7d849854b3d0cfbcd217317e2fc93555", Role::TgpData),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-16720.7", 0x20000, "9933ccc0757e8c86e0adb938d1c89210b26841ea", Role::AudioCpu),
        rom("epr-16721.8", 0x20000, "ee2fd1480e535fc37e9932e6fe4e31344559fc87", Role::Unknown),
        rom("mpr-16491.32", 0x200000, "06d1d55470ae99f8de0f8c88c694f34c4eb13668", Role::MultiPcmA),
        rom("mpr-16492.33", 0x200000, "2054f69cecad677eb00c6a3051f5b5d90885e19b", Role::MultiPcmA),
        rom("mpr-16493.4", 0x200000, "ea9a8b45a07dccaae62be7cf095532ce7596a70c", Role::MultiPcmB),
        rom("mpr-16494.5", 0x200000, "d4e246fc57a16ff562bbcbccf6a739b706f58696", Role::MultiPcmB),
    ],
};

/// A wheel, two pedals, a four-speed H gate and four view buttons, as on
/// the cabinet; every channel and bit read off the game's own input test
/// (docs/games/daytona.md). Register 8 holds coin 1 and 2, test, service,
/// start and three view buttons, register 9 bit 0 the fourth view button
/// and bits 4-6 the gear lever (as on srallyc). The wheel is analog
/// channel 0, an absolute position with `0x80` the centre, a dead zone of
/// +-5 and full lock at +-80 (`0x30`/`0xD0`; below `0x80` is left); the
/// accelerator is channel 1 and the brake channel 2, `0` released.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        // The four V.R. (view) buttons: the code tests these bits together
        // and picks a view by which is set.
        button("view1", 0, 0x20, &["KeyV"], &[PadButton::North]).labelled("View 1"),
        button("view2", 0, 0x40, &["KeyB"], &[]).labelled("View 2"),
        button("view3", 0, 0x80, &["KeyN"], &[]).labelled("View 3"),
        button("view4", 1, 0x01, &["KeyM"], &[]).labelled("View 4"),
        // The H gate (the Manual-4 mission), latched by the shifter below:
        // reg 9 bits 4-6, as on srallyc (docs/games/daytona.md).
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
            span: 80,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            // The game steers the front wheels straight from the wheel's
            // position (halving the gap each frame), so a key at full
            // lock turns a 200 km/h car 7 degrees in a frame and it swings
            // back when let go: the keys turn the wheel over 12 frames
            // (0.2 s) and let it back in 6 (docs/games/daytona.md).
            key_ramp: Some(KeyRamp { attack: 12, release: 6 }),
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
    // One coin a credit (the default), start, then the accelerator twice:
    // it picks the course under the wheel (Beginner) and the transmission
    // (Automatic) on the select screens (docs/games/daytona.md).
    coin_start: "coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710",
};
