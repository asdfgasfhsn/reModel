use super::rom;
use crate::{Board, Game, Role};

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
        rom("epr-18822c.12", 0x20000, "d3a6910bf6efc138e0e40332219b90dea7d6ea56", Role::Program),
        rom("epr-18823c.13", 0x20000, "6da5c071e3ce842a99f928f473d4ccf7165785ac", Role::Program),
        rom("epr-18824c.14", 0x20000, "389cbf951ba606acb9ab7bff5cda85d9166e64ff", Role::Program),
        rom("epr-18825c.15", 0x20000, "f6196e8da5e6579fe3fa5c24ab9538964c98e267", Role::Program),
        rom("mpr-18751.10", 0x200000, "4d1601dc08a08b724e33e7cd90a4f22e18cfed9c", Role::MainData),
        rom("mpr-18752.11", 0x200000, "24007e4ae3ba1a06321328d14e2bd6002fa1936e", Role::MainData),
        rom("mpr-18749.8", 0x200000, "ada21405a136935ac4da1a3535c25fccf903f2d1", Role::MainData),
        rom("mpr-18750.9", 0x200000, "d98ae5518c5d31b155b1a7c4f7d9d67f44d7beae", Role::MainData),
        rom("mpr-18747.6", 0x200000, "92636bdff0ae4cdb43dfc2986fad2d1b59469323", Role::MainData),
        rom("mpr-18748.7", 0x200000, "6c2e903dd073b130bcabb347631b876dc868b494", Role::MainData),
        rom("mpr-18862.4", 0x80000, "029db946338f8e0eccace8590082cc96bdf13e31", Role::MainData),
        rom("mpr-18863.5", 0x80000, "f78a5f7e582d313880c734158bb0fa68b256a58a", Role::MainData),
        rom("mpr-18753.16", 0x200000, "26f643d6b9cecf08bd249290a670a0edea1b5be4", Role::Polygons),
        rom("mpr-18756.20", 0x200000, "fc2a6258387a1bc3fae2109b2dae6dd2a1984ab5", Role::Polygons),
        rom("mpr-18754.17", 0x200000, "e50646efb2ca59792833ce91398c4efa861ad6d1", Role::Polygons),
        rom("mpr-18757.21", 0x200000, "226c4c7289b3b6009c1ffea4a171e3fb4e31a67c", Role::Polygons),
        rom("mpr-18755.18", 0x200000, "2cd7130b226a28098191a6caf6fd761bb0bfac7b", Role::Polygons),
        rom("mpr-18758.22", 0x200000, "658e33503f6990f4d9a954c63efad5f53d15f3a4", Role::Polygons),
        rom("mpr-18760.25", 0x200000, "bba6cd2a15b3f963388a3a87880da86b10f6e0a2", Role::Textures),
        rom("mpr-18759.24", 0x200000, "5f285fc8cfe88c00ba2bbe1b509b49abd38e00ec", Role::Textures),
        rom("mpr-18761.28", 0x200000, "50696cd320f1a6492e0c193713acbce085d959cd", Role::TgpData),
        rom("mpr-18762.29", 0x200000, "7ff42a4c7236fec76f94f2d0c5537e503bcc98e5", Role::TgpData),
        rom("mpr-18827.31", 0x200000, "95275ed8315c044bfde2f23c10416f22627b34df", Role::Samples),
        rom("mpr-18764.32", 0x200000, "cb2ada0f8a592940de11ee781ad4beb5095c3b37", Role::Samples),
        rom("mpr-18765.36", 0x200000, "70b59da8f2532a02e980caba5bb86ec13a4d7ab5", Role::Samples),
        rom("mpr-18766.37", 0x200000, "9ef5e26db4abf0ed36df63fc246b568e1c5d6cfa", Role::Samples),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-18826.30", 0x40000, "c3dd8a1324a4dc9b012bd9bf21d1f48578870f72", Role::AudioCpu),
    ],
};
