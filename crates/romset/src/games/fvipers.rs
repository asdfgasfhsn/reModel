use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Role, RomFile};

/// Fighting Vipers (Japan), Model 2B-CRX: Sega's, uploading `indy500`'s
/// geometrizer program and a copro program of its own, which writes parts
/// of the display list into buffer RAM. Pairs are listed low half-word
/// first: program `.15/.16` then `.13/.14` (128 KB chips); main data
/// `.11/.12`, `.9/.10`; the 1 MB pairs `.7/.8` and `.5/.6` in the second
/// data bank (the SHARC programs are in `.7/.8`); polygons `.17/.21`,
/// `.18/.22`, `.19/.23`, 2 MB chips packed every 4 MB; textures `.27/.25`,
/// `.28/.26` in 8 MB slots; copro data `.29/.30` (`schamp`'s chips). Its
/// textures go to the upper half of each texture RAM bank (README, "What's
/// known about fvipers").
pub static FVIPERS: Game = Game {
    id: "fvipers",
    title: "Fighting Vipers",
    year: 1995,
    board: Board::Model2B,
    // The attract demo: Bahn squaring up to Honey on the rooftop.
    attract_frame: 3400,
    input_lag: 1,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    protection: None,
    dropouts: &[],
    // It uploads its textures to 0x11100000 and 0x11300000 only.
    texture_ram_mirrored: true,
    // In 8 MB slots the second and third polygon pairs' fighters draw as
    // shards; packed textures draw Bahn's coat wrong.
    packed: &[Role::Polygons],
    // A coin-start run's `make sound` averages -32 dBFS RMS (peak -12), with
    // the games here that play at 0 dB (docs/hw/sound.md §6).
    volume_db: 0.0,
    files: &[
        RomFile {
            name: "epr-18606d.15",
            size: 0x20000,
            sha1: "d10355198a3f62b503701f44dc49bfe018c787d1",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18607d.16",
            size: 0x20000,
            sha1: "656e25a6389f04f7fb9099f0b41fb03fa645a2f0",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18604d.13",
            size: 0x20000,
            sha1: "52b6ae90231d40a3ece133debaeb210fc36c6fcb",
            role: Role::Program,
        },
        RomFile {
            name: "epr-18605d.14",
            size: 0x20000,
            sha1: "3e0da0eaf1f98dbbd4ca5f78c04052b347b234b2",
            role: Role::Program,
        },
        RomFile {
            name: "mpr-18614.11",
            size: 0x400000,
            sha1: "49c80b11b207cba4ec10fbb7cc140f3a5b039e82",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18615.12",
            size: 0x400000,
            sha1: "59e5b6378404e10ace4f3675428d61d3ae9d1963",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18612.9",
            size: 0x400000,
            sha1: "89b56dd2f350edd093dc06f4cc258652c26b1d45",
            role: Role::MainData,
        },
        RomFile {
            name: "mpr-18613.10",
            size: 0x400000,
            sha1: "e16d5de2a00670aba4fbe0dc88ccf317de9842be",
            role: Role::MainData,
        },
        RomFile {
            name: "epr-18610d.7",
            size: 0x80000,
            sha1: "8d7b362a8fd9d63f5cea2f3fab97e5fe3fa30d87",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-18611d.8",
            size: 0x80000,
            sha1: "c962805f03e2503dd1671ba3e906c6e306a92e48",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-18608d.5",
            size: 0x80000,
            sha1: "97ce5faf9719cb02dd3a15d47245cc4634f08fcb",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "epr-18609d.6",
            size: 0x80000,
            sha1: "94c85a656c86bc4880db6bff2ef795ec30f62f39",
            role: Role::MainDataHigh,
        },
        RomFile {
            name: "mpr-18622.29",
            size: 0x200000,
            sha1: "9914be9925b86af6af670745b5eba3a9e4f24af9",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-18623.30",
            size: 0x200000,
            sha1: "a6f0f589ad174a34493ee24dc0cb509ead3aed70",
            role: Role::CoproData,
        },
        RomFile {
            name: "mpr-18616.17",
            size: 0x200000,
            sha1: "1a33c48f99eed20da4b622219d21ec5995acc9aa",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18619.21",
            size: 0x200000,
            sha1: "f79ae0a7b966ddb0948b464d233845d4f362a2e7",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18617.18",
            size: 0x200000,
            sha1: "f20a545148f2a1d6f4f1c897f1ed82ad17429dce",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18620.22",
            size: 0x200000,
            sha1: "30a1ef1e309a163b2d8756810fc33debf069141c",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18618.19",
            size: 0x200000,
            sha1: "67818ec4185da17f1549fb3a125cade267a46a48",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18621.23",
            size: 0x200000,
            sha1: "38d7019afcef6dbe292354d717fd49da511cbc2b",
            role: Role::Polygons,
        },
        RomFile {
            name: "mpr-18626.27",
            size: 0x200000,
            sha1: "d8fb4bbbdc00303330047be380a79da7838d4fd5",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18624.25",
            size: 0x200000,
            sha1: "5b6d2d17609ae741546d99d40f575bb24d62b5d3",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18627.28",
            size: 0x200000,
            sha1: "8b6e5e1342f98c9c6f2f7d61e843275d244f331a",
            role: Role::Textures,
        },
        RomFile {
            name: "mpr-18625.26",
            size: 0x200000,
            sha1: "b09a682eff7e835ff8c33aaece12f3727a91dd5e",
            role: Role::Textures,
        },
        RomFile {
            name: "epr-18628.31",
            size: 0x80000,
            sha1: "d8bd1485273652d7c2a303bbdcdf607d3b530283",
            role: Role::AudioCpu,
        },
        RomFile {
            name: "mpr-18629.32",
            size: 0x200000,
            sha1: "f6d2552ffc5473836aafb06735b62f65ef8f5ef5",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18630.33",
            size: 0x200000,
            sha1: "7e7ffbb4ec080a0815c6ca49b9d8efe1f676203b",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18631.34",
            size: 0x200000,
            sha1: "055ac989eafb81749326520d0be264f7a984c627",
            role: Role::Samples,
        },
        RomFile {
            name: "mpr-18632.35",
            size: 0x200000,
            sha1: "9e9523b7c2bc50f869d062f80955da1281951299",
            role: Role::Samples,
        },
    ],
};

/// Two players, each with an 8-way stick and three buttons (punch, kick,
/// guard). Ports read active low; which bit is which comes from the game's
/// input test (README, "What's known about fvipers"). One coin is one
/// credit.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("punch", 1, 0x01, &["KeyX"], &[PadButton::South]).labelled("Punch"),
        button("kick", 1, 0x02, &["KeyV"], &[PadButton::East]).labelled("Kick"),
        button("guard", 1, 0x04, &["KeyZ"], &[PadButton::West]).labelled("Guard"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("punch2", 2, 0x01, &[], &[]),
        button("kick2", 2, 0x02, &[], &[]),
        button("guard2", 2, 0x04, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    // One coin, start, then punch and kick every half second: that picks
    // the fighter under the cursor (Grace) and fights.
    shifter: None,
    coin_start: "coin1@1100-1104,start@1250-1254,punch@1400-99999/30,kick@1415-99999/30",
};
