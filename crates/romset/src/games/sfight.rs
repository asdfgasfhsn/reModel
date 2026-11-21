use super::rom;
use crate::{Board, Game, Role};

/// Sonic The Fighters, Model 2B-CRX (Sega): the Japanese release of
/// `schamp` (Sonic Championship). Only the program pair (`epr-19001.15/.16`)
/// and the 1 MB `.7/.8` pair (`epr-19003.7`, `epr-19004.8`) differ: every
/// other chip is `schamp`'s, in the same places, and so are the controls
/// (docs/games/schamp.md, "`sfight`").
pub static SFIGHT: Game = Game {
    id: "sfight",
    title: "Sonic The Fighters",
    year: 1996,
    genre: "Fighting",
    kind: crate::Kind::Fighting,
    board: Board::Model2B,
    // The attract: the title screen with the logo (schamp's attract is a
    // fight demo at this frame).
    attract_frame: 3500,
    scenes: &[],
    input_lag: 0,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: super::schamp::CONTROLS,
    drive_board: None,
    link_board: None,
    parent: Some("schamp"),
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    // It uploads its textures to 0x11100000 and 0x11300000 only.
    texture_ram_mirrored: true,
    // Sonic's quills and Knuckles' dreadlocks are faceted spikes whose
    // ridges meet at 25-40 degrees: at 40 they melt together, at 25 they
    // keep their ridges (docs/games/schamp.md, "Smooth shading").
    shading: crate::Shading { crease: 25, ..crate::Shading::DEFAULT },
    recommended: crate::Recommended { depth: true, shading: true, ..crate::Recommended::DEFAULT },
    // The wait for the next frame: polls a work RAM byte (0x500000) until an
    // interrupt handler changes it (docs/games/schamp.md).
    // The same wait as `schamp`'s (a work RAM byte at 0x500000 polled until
    // an interrupt changes it), 0x2c bytes lower in this program
    // (docs/games/schamp.md, "`sfight`").
    idle_loops: &[crate::IdleLoop { start: 0x11610, end: 0x1161c }],
    packed: &[],
    // A coin-start run's `make sound` averages -28 dBFS RMS (peak -8), louder
    // than the games here that play at 0 dB (about -31 to -38): brought
    // down towards their middle, as doa (docs/hw/sound.md §6).
    volume_db: -4.0,
    files: &[
        rom("epr-19001.15", 0x80000, "20718d985d14f4d2b1b8e982bfbebddd73cdb972", Role::Program),
        rom("epr-19002.16", 0x80000, "edcbf61122db568ccaa4c3106f507087c1740c9b", Role::Program),
        rom("mpr-19007.11", 0x400000, "5343a9a2502052e3587424c984bd48caa7564849", Role::MainData),
        rom("mpr-19008.12", 0x400000, "39ad2e9431543ea6cbc0307bc39933cf64956a74", Role::MainData),
        rom("mpr-19005.9", 0x400000, "300c9cdef199f31255bacb95399e9c75be73f817", Role::MainData),
        rom("mpr-19006.10", 0x400000, "37a4ff13cfccfda587ca59a9ef08b5914d2c28d4", Role::MainData),
        rom("epr-19003.7", 0x80000, "cbd55b7b7376ac2f67befaf4c43eef3727ba7b7f", Role::MainDataHigh),
        rom("epr-19004.8", 0x80000, "cf806501dbfa48d16cb7ed5f39a6146f734ba455", Role::MainDataHigh),
        rom("mpr-19015.29", 0x200000, "9914be9925b86af6af670745b5eba3a9e4f24af9", Role::CoproData),
        rom("mpr-19016.30", 0x200000, "a6f0f589ad174a34493ee24dc0cb509ead3aed70", Role::CoproData),
        rom("mpr-19009.17", 0x400000, "5af3a90c87ec8a90a8fc58ae469ef23ec6e6213c", Role::Polygons),
        rom("mpr-19012.21", 0x400000, "8e13a0bb34e187a340b38d76ab15ff6fe4bae764", Role::Polygons),
        rom("mpr-19010.18", 0x400000, "e3318ef0eb0168998e139e527339c7c667c17fb1", Role::Polygons),
        rom("mpr-19013.22", 0x400000, "a6c4b2b3bf8efc6f6263f73d6f4cacf9785010c1", Role::Polygons),
        rom("mpr-19019.27", 0x400000, "c29bedb41b14d63c6067ae12ad009deaafca2aa4", Role::Textures),
        rom("mpr-19017.25", 0x400000, "52fad61412040c90c7dd300c0fd7aa5b8d5af441", Role::Textures),
        rom("mpr-19020.28", 0x400000, "7b9a75caa8c5b12ba54c6f4f746d80b165ee97ab", Role::Textures),
        rom("mpr-19018.26", 0x400000, "9c707a7c2cffc5eff19f9919ddfae7300842fd19", Role::Textures),
        rom("epr-19021.31", 0x80000, "21290389cd8bd9e52ed438152cc6cb5793f809d3", Role::AudioCpu),
        rom("mpr-19022.32", 0x200000, "43a21609b49926a227558d4938088526acf1fe42", Role::Samples),
        rom("mpr-19023.33", 0x200000, "696dc85e066fb27c7618e52e0acd0d00451e4589", Role::Samples),
        rom("mpr-19024.34", 0x200000, "b431bd85c973aa0a4d6032ac98fb057139f142a2", Role::Samples),
        rom("mpr-19025.35", 0x200000, "b666d31f9be26eb0cdcb71041a3c3c08d5aa41e1", Role::Samples),
    ],
};
