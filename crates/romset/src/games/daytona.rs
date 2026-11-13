use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, PadAxis, PadButton, Role, RomFile};

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
/// (`docs/hw/model2.md` §5): `epr-16720.7` is its 68000 program, which
/// nothing runs yet; `.8` and the four 2 MB chips are presumably its
/// tables and samples (unconfirmed). The zip's other chips belong to the
/// clones and the ramps `16310`-`16312` (as on 2A) are unused.
pub static DAYTONA: Game = Game {
    id: "daytona",
    title: "Daytona USA",
    year: 1994,
    board: Board::Model2,
    attract_frame: 1500,
    input_lag: 1,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
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
        RomFile {
            name: "epr-16722a.12",
            size: 0x20000,
            sha1: "a476a9a3531beef760c88c9634ed4a7d270e8ee7",
            role: Role::Program,
        },
        RomFile {
            name: "epr-16723a.13",
            size: 0x20000,
            sha1: "2039ec1f8da524176fcf85473c10a8b6e49e139a",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-16528.10",
            size: 0x200000,
            sha1: "e22fc8a70b533f7a6191f5952c581fb8f9627906",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-16529.11",
            size: 0x200000,
            sha1: "da3c922f950dd730ea348eae12aa1cb69cee9a58",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-16808.8",
            size: 0x200000,
            sha1: "343866a6e2187a8ebc17f6727080f9f2f9ac9200",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-16809.9",
            size: 0x200000,
            sha1: "8192d8698d6bd52ee11cc28917aff5840c447627",
            role: Role::MainData,
        },
        RomFile {
            name: "epr-16724a.6",
            size: 0x80000,
            sha1: "7fad3b8d03960e5e1f7a6cb36509238977e00fcc",
            role: Role::MainData,
        },
        RomFile {
            name: "epr-16725a.7",
            size: 0x80000,
            sha1: "d0c5581c56500b5266cab8e8151db24fcbdea0d7",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-16523.16",
            size: 0x200000,
            sha1: "0b83a3fc92b7d913a14cfb01d688c63555c17c41",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16518.20",
            size: 0x200000,
            sha1: "16afe5029591f3536b5b75d9cf50a34d0ea72c3d",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16524.17",
            size: 0x200000,
            sha1: "71b47626ffe5b26d1140afe1b830a9a2be86c88f",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16519.21",
            size: 0x200000,
            sha1: "dac8c281a5e9a6c4b60197e6676f3727264ee420",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16525.18",
            size: 0x200000,
            sha1: "33f5f37ea2e09fc73eed5388b46fdf1fa9e285e6",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16520.22",
            size: 0x200000,
            sha1: "660171674484375a27595630e5e2d2ad76a06d1a",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16772.19",
            size: 0x200000,
            sha1: "1789f35dd403f73f8be18495a0fe4ad1e6841417",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16771.23",
            size: 0x200000,
            sha1: "257a3675e4ef6adbf61285a5daa5954223c28cb2",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-16522.25",
            size: 0x200000,
            sha1: "abf7b0fc0f111f90da42463d600db9fa32e95efe",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-16521.24",
            size: 0x200000,
            sha1: "a6a21a23cd34d0de6d3e6a5c3c2687f905d0dc2a",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-16770.27",
            size: 0x200000,
            sha1: "8aa933b74d4e05dc49987238705e50b00e5dae73",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-16769.26",
            size: 0x200000,
            sha1: "8c712ab09e61ef510741a55f29b3c4e497471372",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-16537.28",
            size: 0x200000,
            sha1: "b32fd1d3fc8983fb5f2a7b236b665a8c9b52769f",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-16536.29",
            size: 0x200000,
            sha1: "2018468d7d849854b3d0cfbcd217317e2fc93555",
            role: Role::TgpData,
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
            name: "epr-16720.7",
            size: 0x20000,
            sha1: "9933ccc0757e8c86e0adb938d1c89210b26841ea",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "epr-16721.8",
            size: 0x20000,
            sha1: "ee2fd1480e535fc37e9932e6fe4e31344559fc87",
            role: Role::Unknown,
        },
        RomFile {
            name: "mpr-16491.32",
            size: 0x200000,
            sha1: "06d1d55470ae99f8de0f8c88c694f34c4eb13668",
            role: Role::Unknown,
        },
        RomFile {
            name: "mpr-16492.33",
            size: 0x200000,
            sha1: "2054f69cecad677eb00c6a3051f5b5d90885e19b",
            role: Role::Unknown,
        },
        RomFile {
            name: "mpr-16493.4",
            size: 0x200000,
            sha1: "ea9a8b45a07dccaae62be7cf095532ce7596a70c",
            role: Role::Unknown,
        },
        RomFile {
            name: "mpr-16494.5",
            size: 0x200000,
            sha1: "d4e246fc57a16ff562bbcbccf6a739b706f58696",
            role: Role::Unknown,
        },
    ],
};

/// A wheel, two pedals, a four-speed H gate and four view buttons, as on
/// the cabinet. From the game's input polling: register 8 holds coin 1 and
/// 2, test, service, start and three view buttons, register 9 bit 0 the
/// fourth. *Confirmed by running it:* the accelerator is analog channel 1
/// (it confirms the select screens and drives the car) and the wheel
/// channel 0 (it turns the car); the brake on channel 2, the wheel's
/// direction and the shifter are still to be checked on the input test
/// (docs/games/daytona.md).
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
    shifter: None,
    // One coin a credit (the default), start, then the accelerator twice:
    // it picks the course under the wheel (Beginner) and the transmission
    // (Automatic) on the select screens (docs/games/daytona.md).
    coin_start: "coin1@1100-1104,start@1300-1304,gas=0xff@1500-1510,gas=0xff@1700-1710",
};
