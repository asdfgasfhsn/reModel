use super::rom;
use crate::controls::button;
use crate::{Board, Controls, Game, PadButton, Protection, ProtectionInterface, ProtectionReply, Role};

/// Pilot Kids (Psikyo), Model 2B-CRX: the program writes both SHARCs' host
/// windows and the 2B sound UART (`docs/games/pltkids.md`).
///
/// Chip pairs are listed low half-word first. Program `.15/.16`; main data
/// `.da0/.da1`; polygons `.tp0/.tp1` to `.tp6/.tp7` (each pairing makes
/// the most sane floats); textures `.tx1/.tx0` and `.tx3/.tx2`, the odd
/// chip low (the other way round the 3D draws in flat colours); the sound
/// program `.sd0` and the samples `.sd1` to `.sd4` in order (their data
/// runs on across each chip's end). The 2A clone's chips in the zip
/// (`.pr0/.pr1`, the TGP tables and the ramps) are unused.
pub static PLTKIDS: Game = Game {
    id: "pltkids",
    title: "Pilot Kids",
    year: 1998,
    board: Board::Model2B,
    // The title screen.
    attract_frame: 3010,
    input_lag: 0,
    run_ahead_max: 0,
    hud_windows: &[],
    hud_element_windows: &[],
    controls: CONTROLS,
    drive_board: None,
    parent: None,
    backup_ram: &[],
    // One boot check of the keyed device at 0x1D80000/0x1D90000, as
    // dynamcop's: a 12-byte block, key 0x7140, and the 8-byte answer
    // compared with ROM 0x518C8 (copied to RAM 0x5518C8, "98-PILOT"). A
    // mismatch sets 0x5092D4 (docs/games/pltkids.md).
    protection: Some(Protection {
        interface: ProtectionInterface::Keyed,
        replies: &[ProtectionReply { command: 0x7140, offset: None, rom: 0x518c8, len: 8 }],
    }),
    dropouts: &[],
    texture_ram_mirrored: false,
    shading: crate::Shading::DEFAULT,
    recommended: crate::Recommended::DEFAULT,
    // The wait for the next frame, in the code the boot copies to RAM:
    // polls the video status's frame bit (0x98000C bit 2) until vblank
    // toggles it, counting passes in r4 (docs/games/pltkids.md).
    idle_loops: &[crate::IdleLoop { start: 0x55cf3c, end: 0x55cf50 }],
    packed: &[],
    volume_db: 0.0,
    files: &[
        rom("epr-21285a.15", 0x80000, "14c3f5031f85c6756c00bc67765a967ebaf7eb7f", Role::Program),
        rom("epr-21286a.16", 0x80000, "01030621efa9c97eb43f4a5e3e029ec99a2363c5", Role::Program),
        rom("mpr-21262.da0", 0x400000, "6eb5e8284734f01beec1dbbee049b6b7672e2504", Role::MainData),
        rom("mpr-21263.da1", 0x400000, "641db6ec3e9266f8265a4b541bcd8c2f7d164cc3", Role::MainData),
        rom("mpr-21264.tp0", 0x400000, "3a07701b140eb3088fad29c8b2d9c1e1e7ef9471", Role::Polygons),
        rom("mpr-21268.tp1", 0x400000, "39cba6b4f1130a3da7e2d226c948425eec34090e", Role::Polygons),
        rom("mpr-21265.tp2", 0x400000, "a89b7a84192fcc1e9e0fe9adf7446f7b275d5a03", Role::Polygons),
        rom("mpr-21269.tp3", 0x400000, "9a8c302528e590be1b56ed301da30abf21f0be2e", Role::Polygons),
        rom("mpr-21266.tp4", 0x400000, "b21f8bf281bf2cfcdc7e5eb798cd633e905ab8b8", Role::Polygons),
        rom("mpr-21270.tp5", 0x400000, "7733f44e791974070df139958eb97e0585ee50f8", Role::Polygons),
        rom("mpr-21267.tp6", 0x400000, "6153f52add63295122e1215dd07d648d030a7306", Role::Polygons),
        rom("mpr-21271.tp7", 0x400000, "d52836760475c7d9fbb4e5b8147ac416ffd1fcd9", Role::Polygons),
        rom("mpr-21274.tx1", 0x400000, "548909d2da22ed98594e0ab6ecffebec4fca2f93", Role::Textures),
        rom("mpr-21272.tx0", 0x400000, "8363a082a666ceeb84df84929ff3fbaff49af821", Role::Textures),
        rom("mpr-21275.tx3", 0x400000, "feb8a34acb620a36ed5aea92d22622a76d7e1b29", Role::Textures),
        rom("mpr-21273.tx2", 0x400000, "1a1dc92488cde6284a96acce80e47a9cceccde76", Role::Textures),
        rom("epr-21276.sd0", 0x80000, "4e8e1ccbe025deca42fcf2582f3da46fa34780b7", Role::AudioCpu),
        rom("mpr-21277.sd1", 0x200000, "11081b3eabc33a42ecfc0b2b535ce16510496144", Role::Samples),
        rom("mpr-21278.sd2", 0x200000, "254c0ad4d6bd572ff0efc3ea80489e73716a31a7", Role::Samples),
        rom("mpr-21279.sd3", 0x200000, "312496b45b699051c8b4dd0e5d94e73fe5f3ad8d", Role::Samples),
        rom("mpr-21280.sd4", 0x200000, "a94adfe16b5c3236746451c181ccd3e1c27432f4", Role::Samples),
    ],
};

/// A stick and two buttons a player, from the game's Input Check
/// (docs/games/pltkids.md): port 0 the coins, test, service and starts,
/// port 1 player 1's SW-1 (shot), SW-2 (bomb) and directions, port 2
/// player 2's.
const CONTROLS: Controls = Controls {
    buttons: &[
        button("coin1", 0, 0x01, &["KeyC", "Digit5"], &[PadButton::Select]).labelled("Coin"),
        button("coin2", 0, 0x02, &[], &[]),
        button("test", 0, 0x04, &[], &[]),
        button("service", 0, 0x08, &[], &[]),
        button("start", 0, 0x10, &["Enter", "Digit1"], &[PadButton::Start]).labelled("Start"),
        button("start2", 0, 0x20, &["Digit2"], &[]).labelled("Start 2"),
        button("shot", 1, 0x01, &["KeyX"], &[PadButton::South]).labelled("Shot"),
        button("bomb", 1, 0x02, &["KeyZ"], &[PadButton::East]).labelled("Bomb"),
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
    // One coin a credit, start (stage 1 begins at once), then a shot every
    // 4 frames and a bomb every 500, standing still.
    coin_start: "coin1@1100-1104,start@1250-1254,shot@1400-99999/4,bomb@2000-99999/500",
};
