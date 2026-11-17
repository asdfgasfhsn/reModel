use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Protection, ProtectionInterface, ProtectionReply, Role};

/// Zero Gunner (Psikyo, 1997), Model 2B-CRX: the program writes both
/// SHARCs' host windows (`0x8C0000`, `0x840000`) and the 2B sound UART
/// (`0x9C0000`). Pairs are listed low half-word first: program `.15/.16`;
/// main data `.11/.12` then `.9/.10`; polygons `.17/.21`; textures
/// `.27/.25`; samples `.32` then `.33`; no copro data. It checks a keyed
/// protection device during play (docs/games/zerogun.md).
pub static ZEROGUN: Game = Game {
    id: "zerogun",
    title: "Zero Gunner",
    year: 1997,
    genre: "Shoot 'em up",
    kind: crate::Kind::Shooting,
    board: Board::Model2B,
    // The attract demo: a helicopter over the sea.
    attract_frame: 1500,
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    link_board: None,
    parent: None,
    backup_ram: &[],
    protection: Some(Protection {
        interface: ProtectionInterface::Keyed,
        // A 192-byte block sent at boot, then, now and then in play, one of
        // sixteen requests picked at random: a key and a 12-byte piece of
        // the block (two pieces share key 0x234D), answered with the 8
        // bytes the game compares the answer with (ROM 0x279A4 on).
        replies: &[
            ProtectionReply { command: 0xa1bc, offset: Some(0x00), rom: 0x279a4, len: 8 },
            ProtectionReply { command: 0xad23, offset: Some(0x06), rom: 0x279ac, len: 8 },
            ProtectionReply { command: 0x13cd, offset: Some(0x0c), rom: 0x279b4, len: 8 },
            ProtectionReply { command: 0x4d53, offset: Some(0x12), rom: 0x279bc, len: 8 },
            ProtectionReply { command: 0x234d, offset: Some(0x18), rom: 0x279c4, len: 8 },
            ProtectionReply { command: 0x113d, offset: Some(0x1e), rom: 0x279cc, len: 8 },
            ProtectionReply { command: 0x1049, offset: Some(0x24), rom: 0x279d4, len: 8 },
            ProtectionReply { command: 0x993d, offset: Some(0x2a), rom: 0x279dc, len: 8 },
            ProtectionReply { command: 0x983c, offset: Some(0x30), rom: 0x279e4, len: 8 },
            ProtectionReply { command: 0x0935, offset: Some(0x36), rom: 0x279ec, len: 8 },
            ProtectionReply { command: 0x9845, offset: Some(0x3c), rom: 0x279f4, len: 8 },
            ProtectionReply { command: 0x556d, offset: Some(0x42), rom: 0x279fc, len: 8 },
            ProtectionReply { command: 0x98cc, offset: Some(0x48), rom: 0x27a04, len: 8 },
            ProtectionReply { command: 0x234d, offset: Some(0x4e), rom: 0x27a0c, len: 8 },
            ProtectionReply { command: 0x3422, offset: Some(0x54), rom: 0x27a14, len: 8 },
            ProtectionReply { command: 0x0010, offset: Some(0x5a), rom: 0x27a1c, len: 8 },
        ],
    }),
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame: polls the video status's frame bit
    // (0x98000C bit 2) until vblank toggles it, counting passes in r4
    // (docs/games/zerogun.md).
    idle_loops: &[crate::IdleLoop { start: 0x549050, end: 0x549064 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-20439.15", 0x80000, "1e178e6bd2b1312cd6290f1be4b386f520465836", Role::Program),
        rom("epr-20440.16", 0x80000, "82bf138a42c659b675b14e41d526b1628fb46ae3", Role::Program),
        rom("mpr-20296.11", 0x400000, "7f69c90dd3c3e6e522d1065b3c4b09434cb4e634", Role::MainData),
        rom("mpr-20297.12", 0x400000, "670a86c3a1a78550c760cc66c0a6181928fb9054", Role::MainData),
        rom("mpr-20294.9", 0x400000, "c0c032adac69bd545e3aab481878b08f3c3edab8", Role::MainData),
        rom("mpr-20295.10", 0x400000, "d34f2fc9b4481c75a6824aa4bdd3f1884188d35b", Role::MainData),
        rom("mpr-20298.17", 0x400000, "595f6fc2e9c58ce9763d51798ceead8d470f0a33", Role::Polygons),
        rom("mpr-20299.21", 0x400000, "730d58286fb7e91aa4128dc208b0f60eb3becc78", Role::Polygons),
        rom("mpr-20301.27", 0x200000, "8dce67c6f9e48d749c64b11d4569df413dc40e07", Role::Textures),
        rom("mpr-20300.25", 0x200000, "75db68e57ec3fbc7af377342eef81f26fae4e1c4", Role::Textures),
        rom("epr-20302.31", 0x80000, "6ffec81042fd5708e8a5df47b63f9809f93bf0f8", Role::AudioCpu),
        rom("mpr-20303.32", 0x200000, "57a496c5dcc1a3931b6e41bf8d41e45d6dac0c31", Role::Samples),
        rom("mpr-20304.33", 0x200000, "d73adafceff2f1776c93e53bd5677d67f1c2c08f", Role::Samples),
    ],
};

/// A stick and two buttons a player, from the game's I/O CHECK: port 0
/// (`0x1C00002`) coins, test, service and the start buttons; port 1
/// (`0x1C00004`) player 1, port 2 (`0x1C00006`) player 2. Ports read
/// active low. The DIP switches are at `0x1C0000C`.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("shot", 1, 0x01, &["KeyZ"], &[PadButton::South]).labelled("Shot"),
        button("bomb", 1, 0x02, &["KeyX"], &[PadButton::East]).labelled("Bomb"),
        button("down", 1, 0x10, &["ArrowDown"], &[PadButton::DPadDown]).labelled("Down"),
        button("up", 1, 0x20, &["ArrowUp"], &[PadButton::DPadUp]).labelled("Up"),
        button("right", 1, 0x40, &["ArrowRight"], &[PadButton::DPadRight]).labelled("Right"),
        button("left", 1, 0x80, &["ArrowLeft"], &[PadButton::DPadLeft]).labelled("Left"),
        // Player 2: the same layout on the next port, unbound.
        button("shot2", 2, 0x01, &[], &[]),
        button("bomb2", 2, 0x02, &[], &[]),
        button("down2", 2, 0x10, &[], &[]),
        button("up2", 2, 0x20, &[], &[]),
        button("right2", 2, 0x40, &[], &[]),
        button("left2", 2, 0x80, &[], &[]),
    ],
    analogs: &[],
    guns: &[],
    shifter: None,
    // One coin and start (the fighter select's timer picks the gunship),
    // then shoot every 8 frames and bomb every 400.
    coin_start: "coin1@1100-1104,start@1250-1254,shot@1400-99999/8,bomb@1900-99999/400",
};
