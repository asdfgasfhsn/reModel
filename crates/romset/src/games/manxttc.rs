use crate::{Board, Game, Role, RomFile};

/// Manx TT Superbike (revision C, twin cabinet), Model 2A-CRX: the
/// twin-cabinet release of `manxtt` (docs/games/manxtt.md, "`manxttc`").
/// Its program (`epr-18822c..18825c`), sound program (`epr-18826.30`) and
/// first sample ROM (`mpr-18827.31`) differ; every other chip is
/// `manxtt`'s, in the same places, and so are the controls. The deluxe
/// set's `epr-18742.7` and `mpr-18743.32` are not in it.
pub static MANXTTC: Game = Game {
    id: "manxttc",
    title: "Manx TT Superbike (Twin)",
    year: 1995,
    board: Board::Model2A,
    attract_frame: 2700,
    input_lag: 2,
    run_ahead_max: 2,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: super::manxtt::CONTROLS,
    drive_board: Some(crate::DriveBoard::MotionSlider),
    parent: Some("manxtt"),
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0, as
    // srallyc's does (docs/games/manxtt.md).
    idle_loops: &[crate::IdleLoop { start: 0x3f1c, end: 0x3f34 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-18822c.12",
            size: 0x20000,
            sha1: "d3a6910bf6efc138e0e40332219b90dea7d6ea56",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18823c.13",
            size: 0x20000,
            sha1: "6da5c071e3ce842a99f928f473d4ccf7165785ac",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18824c.14",
            size: 0x20000,
            sha1: "389cbf951ba606acb9ab7bff5cda85d9166e64ff",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18825c.15",
            size: 0x20000,
            sha1: "f6196e8da5e6579fe3fa5c24ab9538964c98e267",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-18751.10",
            size: 0x200000,
            sha1: "4d1601dc08a08b724e33e7cd90a4f22e18cfed9c",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18752.11",
            size: 0x200000,
            sha1: "24007e4ae3ba1a06321328d14e2bd6002fa1936e",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18749.8",
            size: 0x200000,
            sha1: "ada21405a136935ac4da1a3535c25fccf903f2d1",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18750.9",
            size: 0x200000,
            sha1: "d98ae5518c5d31b155b1a7c4f7d9d67f44d7beae",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18747.6",
            size: 0x200000,
            sha1: "92636bdff0ae4cdb43dfc2986fad2d1b59469323",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18748.7",
            size: 0x200000,
            sha1: "6c2e903dd073b130bcabb347631b876dc868b494",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18862.4",
            size: 0x80000,
            sha1: "029db946338f8e0eccace8590082cc96bdf13e31",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18863.5",
            size: 0x80000,
            sha1: "f78a5f7e582d313880c734158bb0fa68b256a58a",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18753.16",
            size: 0x200000,
            sha1: "26f643d6b9cecf08bd249290a670a0edea1b5be4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18756.20",
            size: 0x200000,
            sha1: "fc2a6258387a1bc3fae2109b2dae6dd2a1984ab5",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18754.17",
            size: 0x200000,
            sha1: "e50646efb2ca59792833ce91398c4efa861ad6d1",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18757.21",
            size: 0x200000,
            sha1: "226c4c7289b3b6009c1ffea4a171e3fb4e31a67c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18755.18",
            size: 0x200000,
            sha1: "2cd7130b226a28098191a6caf6fd761bb0bfac7b",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18758.22",
            size: 0x200000,
            sha1: "658e33503f6990f4d9a954c63efad5f53d15f3a4",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18760.25",
            size: 0x200000,
            sha1: "bba6cd2a15b3f963388a3a87880da86b10f6e0a2",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18759.24",
            size: 0x200000,
            sha1: "5f285fc8cfe88c00ba2bbe1b509b49abd38e00ec",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18761.28",
            size: 0x200000,
            sha1: "50696cd320f1a6492e0c193713acbce085d959cd",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-18762.29",
            size: 0x200000,
            sha1: "7ff42a4c7236fec76f94f2d0c5537e503bcc98e5",
            role: Role::TgpData,
        },
        RomFile {
            name: "mpr-18827.31",
            size: 0x200000,
            sha1: "95275ed8315c044bfde2f23c10416f22627b34df",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18764.32",
            size: 0x200000,
            sha1: "cb2ada0f8a592940de11ee781ad4beb5095c3b37",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18765.36",
            size: 0x200000,
            sha1: "70b59da8f2532a02e980caba5bb86ec13a4d7ab5",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18766.37",
            size: 0x200000,
            sha1: "9ef5e26db4abf0ed36df63fc246b568e1c5d6cfa",
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
            name: "epr-18826.30",
            size: 0x40000,
            sha1: "c3dd8a1324a4dc9b012bd9bf21d1f48578870f72",
            role: Role::AudioCpu,
        },
    ],
};
