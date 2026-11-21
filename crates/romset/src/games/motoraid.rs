use super::rom;
use crate::controls::button;
use crate::{AnalogInput, Board, Controls, Game, KeyRamp, PadAxis, PadButton, Role};

/// Motor Raid, Model 2A-CRX (`docs/games/motoraid.md`, `docs/hw/model2a.md`).
///
/// Chip pairs are listed low half-word first. Program `.12/.13` (1 MB);
/// main data `.10/.11` and `.8/.9` (8 MB each) from `0x2000000`, and
/// **`.6/.7` then `.4/.5` in the second data bank at `0x6000000`**
/// (`Role::MainDataHigh`, 8 MB slots, the 1 MB `.4/.5` repeating in
/// its slot): the program points into `0x6000000-0x6CFFFFF`, `.6/.7` is
/// model data full of pointers into `0x60-0x66` MB, and the texture copy
/// from `0x6700000` finds texels only there, in `.6/.7`'s last MB. Packed
/// after `.8/.9` instead, those reads were unmapped and the canyon walls,
/// tunnels and the tutorial's rider drew untextured or not at all
/// (docs/games/motoraid.md). Polygons `.16/.20 .17/.21 .18/.22` (paired
/// so, the halves make the most sane floats: 0.89, 0.90, 0.88), textures
/// `.25` (even words) / `.24`, the TGP's data ROM `.28` (low half) / `.29`
/// as `manxtt`'s; samples `.31 .32 .36 .37` back to back and the sound
/// program `epr-20029.30`, by analogy with `srallyc` (not checked against
/// the sound program or by ear).
pub static MOTORAID: Game = Game {
    id: "motoraid",
    title: "Motor Raid",
    year: 1997,
    genre: "Motorbike combat",
    kind: crate::Kind::Racing,
    board: Board::Model2A,
    attract_frame: 3900,
    scenes: &[],
    input_lag: 0,
    run_ahead_max: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    link_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in g0, as
    // srallyc's and manxtt's do (docs/games/motoraid.md).
    idle_loops: &[crate::IdleLoop { start: 0x30204, end: 0x30218 }],
    packed: &[Role::MainData, Role::CoproData, Role::Polygons, Role::TgpData],
    volume_db: 0.0,
    files: &[
        rom("epr-20007.12", 0x80000, "a6a0fa8fb9d62d0cc2ac84ea3ad457953952d980", Role::Program),
        rom("epr-20008.13", 0x80000, "fd15e8c81b3b2f3bdf3bb8d9414b9b8a6f1f000f", Role::Program),
        rom("mpr-20019.10", 0x400000, "0543d19d1d60b1d12b4409c1491782f2232da685", Role::MainData),
        rom("mpr-20020.11", 0x400000, "19e15e0e9ec1bb5d1b789876778fbb487cfea1ba", Role::MainData),
        rom("mpr-20017.8", 0x400000, "b48b5bd3a2f68c62d16516a037fbd45f49283d23", Role::MainData),
        rom("mpr-20018.9", 0x400000, "78a0c72095a664c4b6e529beea46a31ae0a99e5a", Role::MainData),
        rom("mpr-20015.6", 0x400000, "3e37cfcb4dcc8976805934faf8805cd83acde66e", Role::MainDataHigh),
        rom("mpr-20016.7", 0x400000, "b057d61478f7dc7a32ad233473f1a63498b3779e", Role::MainDataHigh),
        rom("epr-20013.4", 0x80000, "28f430319b34e715ca57ce4e01be23a786eab4bc", Role::MainDataHigh),
        rom("epr-20014.5", 0x80000, "c4cc61a42e89aaae075ad1b6e8df2907c5710d3e", Role::MainDataHigh),
        rom("mpr-20023.16", 0x400000, "804f69cd342e25cf1bed48e778981d67c4d1c9c7", Role::Polygons),
        rom("mpr-20026.20", 0x400000, "46be0cc2b8a4a3f530d081d11c6099d814977270", Role::Polygons),
        rom("mpr-20024.17", 0x400000, "6a386a666ae57da5e47364da7b97da9c913710ef", Role::Polygons),
        rom("mpr-20027.21", 0x400000, "17c23c64b1080ab6a8eb282cabcd7d7612193045", Role::Polygons),
        rom("mpr-20025.18", 0x400000, "9df0d1db32b818dad28f9eeab3bc19c56d27ec6d", Role::Polygons),
        rom("mpr-20028.22", 0x400000, "9aa0e13c8dc5073a603279a538cc7662531dfd19", Role::Polygons),
        rom("mpr-20022.25", 0x400000, "c73279e837f56c0417c07ba3c642af28fe9a24fa", Role::Textures),
        rom("mpr-20021.24", 0x400000, "059cea17f9d6f5960d9fd869c36ffb6fcf230c1a", Role::Textures),
        rom("epr-20011.28", 0x100000, "85abd667491fd019ee18ba256fd580356f4e1fe9", Role::TgpData),
        rom("epr-20012.29", 0x100000, "4474610eed52248e5e36be438eff5d39f076b134", Role::TgpData),
        rom("mpr-20030.31", 0x200000, "006911ce6332091d17808855c60a72fe928df778", Role::Samples),
        rom("mpr-20031.32", 0x200000, "77962afcac82589cc7bc852329335676ae3e23cf", Role::Samples),
        rom("mpr-20032.36", 0x200000, "bced0d30f9b6ab579a11ac069cbb9d6d91352246", Role::Samples),
        rom("mpr-20033.37", 0x200000, "a9e8e2d38b23716df2e211748c52b6b666f4c111", Role::Samples),
        rom("opr-14742a.45", 0x20000, "f46429fffcee17d056f56d5fe035a33f1fd6c27e", Role::CoproData),
        rom("opr-14743a.46", 0x20000, "5b9f1fc47caf21e061e930c0d72804e4ec8c7bca", Role::CoproData),
        rom("opr-14744.58", 0x20000, "651f1db4089a400d073b19ada299b4b08b08f372", Role::CoproData),
        rom("opr-14745.59", 0x20000, "e3349ece0e47f684d61ad11bfea4a90602287350", Role::CoproData),
        rom("opr-14746.62", 0x20000, "34e047a93459406c22acf4c25089d1a4955f94ca", Role::CoproData),
        rom("opr-14747.63", 0x20000, "7d7ec300eeb9a8de1590011e37108688c092f329", Role::CoproData),
        rom("epr-20029.30", 0x80000, "e7a18ccf5a0b9ebf18ae1d5518973fa3b4eb4653", Role::AudioCpu),
    ],
};

/// The cabinet's controls, from the game's own INPUT TEST (test menu:
/// `test` held, `service` to move, `test` to choose), each switch found by
/// holding one port bit at a time on that screen (docs/games/motoraid.md):
/// port 0 `0x01` COIN CHUTE, `0x08` SERVICE, `0x40` START/VIEW; port 1
/// `0x10` PUNCH, `0x20` KICK (the handlebar's buttons). TEST is port 0
/// `0x04` as on the other boards (it leaves the screen); `0x02`, the second
/// chute elsewhere, shows nothing here. BANK (the bike's lean), ACCELE and
/// BRAKE are analog channels 2, 0 and 1, as `manxtt`'s. Ports read active
/// low.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x40, &["Enter", "Digit1", "KeyV"], &[PadButton::Start, PadButton::North])
            .labelled("Start / View"),
        button("punch", 1, 0x10, &["KeyX"], &[PadButton::South, PadButton::RightShoulder]).labelled("Punch"),
        button("kick", 1, 0x20, &["KeyZ"], &[PadButton::West, PadButton::LeftShoulder]).labelled("Kick"),
    ],
    analogs: &[
        AnalogInput {
            name: "throttle",
            channel: 0,
            centre: 0x20,
            span: 0xc0,
            axis: PadAxis::RightTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowUp"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        AnalogInput {
            name: "brake",
            channel: 1,
            centre: 0x20,
            span: 0xc0,
            axis: PadAxis::LeftTrigger,
            keys_neg: &[],
            keys_pos: &["ArrowDown"],
            pad_neg: &[],
            pad_pos: &[],
            key_ramp: None,
        },
        AnalogInput {
            name: "bank",
            channel: 2,
            centre: 0x80,
            span: -0x60,
            axis: PadAxis::LeftStickX,
            keys_neg: &["ArrowLeft"],
            keys_pos: &["ArrowRight"],
            pad_neg: &[PadButton::DPadLeft],
            pad_pos: &[PadButton::DPadRight],
            key_ramp: Some(KeyRamp { attack: 12, release: 8 }),
        },
    ],
    guns: &[],
    shifter: None,
    // Two coins a credit ("CREDIT 0/2").
    coin_start: "coin1@1100-1104,coin1@1150-1154,start@1300-1304",
};
