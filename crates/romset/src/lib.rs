//! ROM set manifests and loading for Model 2 games.
//!
//! Game data is always user-supplied. A manifest only records the chip names,
//! sizes and SHA-1 hashes needed to recognise a good dump and assemble it into
//! the address spaces the hardware sees.
//!
//! A ROM directory holds sets as MAME does, by set name: `ROOT/<set>.zip`
//! or unpacked in `ROOT/<set>/`. Loose chips directly in `ROOT` also work.

pub mod bindings;
pub mod controls;
pub mod games;
mod sha1;
pub mod zip;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub use bindings::Bindings;
pub use controls::{AnalogInput, ButtonInput, Controls, Gun, GunSource, PadAxis, PadButton, Reload, Shifter};
pub use sha1::sha1_hex;

/// Cabinet boards beyond the Model 2 board itself that the game talks to
/// through the I/O chip, and which protocol they speak. The board model
/// (`model2::drive`) emulates each kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveBoard {
    /// Sega Ski Super G's platform drive board: one command byte a frame
    /// on I/O `0x1C00008`, one reply byte on `0x1C00006`; boots with a
    /// `0x00` -> `0xCE` handshake (docs/games/skisuprg.md).
    SkiPlatform,
    /// Sega Touring Car Championship's force feedback wheel: one command
    /// byte a frame on `0x1C00008`; the game stores the reply from
    /// `0x1C00006` but never checks it (docs/games/stcc.md).
    Wheel,
}

/// A device on the board's expansion bus at `0x1D80000-0x1D87FFF` that the
/// game checks at boot (`model2::protection`). The game sends it a block of
/// data and two tables and compares the stream it reads back with bytes in
/// its own program ROM. How the device turns one into the other isn't
/// known (the block isn't the plain code the tables describe), so the
/// board model answers each request with those bytes, read from the ROM at
/// run time. This answers the check; it doesn't model what the device
/// computes for anything else.
#[derive(Clone, Copy, Debug)]
pub struct Protection {
    pub replies: &'static [ProtectionReply],
}

/// One request the game makes of its [`Protection`] device, and the answer.
#[derive(Clone, Copy, Debug)]
pub struct ProtectionReply {
    /// The value the game writes to the command register (`0x1D84000`)
    /// before the request.
    pub command: u16,
    /// Where the answer is in the program ROM (i960 address) and how many
    /// bytes long: the bytes the game compares the device's stream with.
    pub rom: u32,
    pub len: u32,
}

/// A data line of a polygon ROM chip that reads some of its 1 bits as 0
/// in the only dump there is (a bad dump; the hash is still the one
/// [`RomFile::sha1`] checks). Loading restores what can be restored: the
/// low half's link bits 8 and 9 of a polygon's attribute word, from their
/// copy in bits 23 and 24 (the high chip of the pair), which every intact
/// polygon ROM keeps equal ([`repair_link_bits`]). A float word that lost
/// the bit stays off by that mantissa bit (at most 2^-14 relative for bit
/// 9 of the low half).
#[derive(Clone, Copy, Debug)]
pub struct Dropout {
    /// The chip: the low half of a [`Role::Polygons`] pair.
    pub chip: &'static str,
    /// The data line, 8 or 9.
    pub bit: u8,
}

/// Model 2 board revisions. They share the i960 main CPU and sound system;
/// the geometry processor differs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Board {
    Model2,
    Model2A,
    Model2B,
    Model2C,
    /// Not yet confirmed from the program code.
    Unconfirmed,
}

impl Board {
    /// The board's name as Sega wrote it ("Model 2C"); "Model 2" for an
    /// unconfirmed revision.
    pub fn name(self) -> &'static str {
        match self {
            Board::Model2 | Board::Unconfirmed => "Model 2",
            Board::Model2A => "Model 2A",
            Board::Model2B => "Model 2B",
            Board::Model2C => "Model 2C",
        }
    }

    /// The boards whose geometry processor is an MB86234 TGP with an
    /// uploaded program, Model 2A's video RAM at `0x12000000` and a
    /// fixed-function geometrizer: 2A and, as far as `daytona` shows, the
    /// original Model 2 (docs/hw/model2.md).
    pub fn has_tgp_2a(self) -> bool {
        matches!(self, Board::Model2 | Board::Model2A)
    }
}

/// One chip in a set.
pub struct RomFile {
    pub name: &'static str,
    pub size: usize,
    pub sha1: &'static str,
    pub role: Role,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    /// i960 program ROM. Chips appear in pairs, 16-bit interleaved
    /// (first = low half-word).
    Program,
    /// i960-visible data ROM, 16-bit interleaved pairs, in 8 MB slots
    /// from `0x2000000`.
    MainData,
    /// The second bank of i960-visible data ROM, 16-bit interleaved pairs
    /// in 8 MB slots from `0x6000000` (a smaller pair repeats in its slot).
    /// segawski and waverunr fill it; their program pointers land on the
    /// pairs' data there (docs/games/<set>.md for each). How the board
    /// decodes the rest of `0x4000000-0x7ffffff` is unconfirmed.
    MainDataHigh,
    /// Geometry coprocessor data ROM, 16-bit interleaved pairs.
    CoproData,
    /// Polygon model data read by the geometry engine, interleaved pairs.
    Polygons,
    /// Data ROM on the external bus of Model 2A's TGP (MB86234), 16-bit
    /// pairs (low half first) packed from address `0x800000` of that bus:
    /// the course data its collision functions read.
    TgpData,
    /// Texture ROM, interleaved pairs.
    Textures,
    /// 68000 sound program (byte-swapped 16-bit).
    AudioCpu,
    /// SCSP sample ROMs (byte-swapped 16-bit, concatenated).
    Samples,
    /// MPEG sound board CPU program (byte-swapped 16-bit).
    MpegCpu,
    /// MPEG sound board stream data, concatenated.
    MpegData,
    /// DSB1 music board Z80 program (plain bytes). The MPEG ROMs beside
    /// it are `MpegData`.
    Dsb1Cpu,
    /// Not yet identified.
    Unknown,
}

/// Model 2C's ROM banks give each chip pair an 8 MB slot.
pub const ROM_SLOT: usize = 0x80_0000;

pub struct Game {
    /// Short set name, matching the usual dump naming (e.g. `topskatr`).
    pub id: &'static str,
    pub title: &'static str,
    /// Year of release, as on the title screen's copyright line.
    pub year: u16,
    pub board: Board,
    /// A frame of the attract mode (counted from boot, no inputs) that shows
    /// the game well: the browser build's game card screenshot
    /// (`remodel shot`).
    pub attract_frame: u64,
    /// The game's own input lag in play, measured: frames after the first
    /// that reads a control before the picture shows it (`make input-lag`,
    /// docs/games/<set>.md; the least seen, for the control
    /// measured). Run-ahead of more frames than this only judders, so the
    /// player suggests up to this much (`docs/play.md`, "Run-ahead").
    pub input_lag: u8,
    /// The most frames of run-ahead worth offering: [`Game::input_lag`]
    /// at most 3, or more where one control has lag and the main one has
    /// none (`schamp`, `lastbrnx`: punches 1, the stick 0, which judders).
    /// 0: nothing to hide, so the player offers no run-ahead
    /// (`docs/play.md`, "Run-ahead").
    pub run_ahead_max: u8,
    pub files: &'static [RomFile],
    /// Banks whose chip pairs follow each other with no gap, rather than
    /// each in its own [`ROM_SLOT`]: bel's main data and polygon ROMs, of
    /// 16 Mbit chips, sit every 4 MB (its textures still take 8 MB slots).
    pub packed: &'static [Role],
    /// Output level in dB on top of the player's mix, standing in for the
    /// cabinet amplifier's volume pot so games play at similar loudness.
    /// A listening choice: the ROMs don't set the analog gain.
    pub volume_db: f32,
    /// 3D viewports (window numbers) that hold HUD elements rather than the
    /// scene. Only enhancements use this: with the HUD at the edges of a
    /// wide view, each polygon in one moves by the third of the screen it
    /// is in, like the 2D HUD elements.
    pub hud_windows: &'static [usize],
    /// 3D viewports that hold HUD parts among other 3D (a rev counter's
    /// needle in a window that elsewhere holds cars in panels). Only
    /// enhancements use this: with the HUD at the edges of a wide view, a
    /// polygon in one moves with the 2D HUD element whose box contains the
    /// polygon's centre (a needle with its dial), and stays put if none
    /// does.
    pub hud_element_windows: &'static [usize],
    /// The cabinet's controls, where the board reads them, and default
    /// bindings.
    pub controls: Controls,
    /// A drive (motion or force feedback) board the game needs, if any.
    pub drive_board: Option<DriveBoard>,
    /// The set this one is a clone of (another revision of the same game):
    /// its chips are also looked for in the parent's folder or zip, as a
    /// merged MAME set keeps them, and it shares the parent's poster and
    /// bezel.
    pub parent: Option<&'static str>,
    /// Settings the game keeps in its battery-backed SRAM (`0x1D00000`)
    /// and never initialises itself, as (offset, bytes): what an operator
    /// would have chosen in the test menu. Written over the SRAM's power-on
    /// contents (all ones) at boot. sgt24h reads its I/O board type from
    /// there and indexes a table with it unchecked.
    pub backup_ram: &'static [(usize, &'static [u8])],
    /// The protection device the game checks for at `0x1D80000`, if any.
    pub protection: Option<Protection>,
    /// Known bad data lines in the set's dump, repaired at load.
    pub dropouts: &'static [Dropout],
    /// Texture RAM banks of 1 MB, each repeating through its 2 MB window
    /// at `0x11000000` and `0x11200000`: fvipers, schamp and vstriker
    /// upload their texels to the upper half of each window, which the
    /// rasteriser reads as the lower. Other boards keep 2 MB there (overrevb and sgt24h fill
    /// both halves with different data), of which the rasteriser reads the
    /// lower (docs/games/fvipers.md).
    pub texture_ram_mirrored: bool,
    /// Rules for smooth shading (an enhancement, off by default:
    /// `model2::light`); [`Shading::DEFAULT`] unless the game's pictures
    /// asked for something else.
    pub shading: Shading,
    /// What the player's "Enhanced" picture preset turns on for this game
    /// ([`Recommended::DEFAULT`] unless the game's pictures said
    /// otherwise; the evidence in `docs/games/<set>.md`).
    pub recommended: Recommended,
    /// Poll loops the main program waits in, which the board model may
    /// fast-forward through exactly (`model2::idle`, docs/idle-skip.md),
    /// each found and checked in the game's code (docs/games/<set>.md).
    /// Empty: nothing is skipped.
    pub idle_loops: &'static [IdleLoop],
}

/// A loop the i960 polls in until something it reads changes: its
/// instructions from `start` (where the loop branches back to) up to
/// `end`, just past the backward branch. The board model checks every
/// pass before it skips more (no writes, reads that can't change before
/// the next event, the CPU back in the same state but for counters), so
/// a wrong entry costs speed, not correctness.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IdleLoop {
    pub start: u32,
    pub end: u32,
}

impl Game {
    /// Frames of run-ahead the "Enhanced" preset sets: the game's own
    /// input lag, within [`Game::run_ahead_max`]. A game whose main
    /// control has no lag gets none, however much a button has.
    pub fn run_ahead_recommended(&self) -> u8 {
        self.input_lag.min(self.run_ahead_max)
    }
}

/// Per-game choices of the player's "Enhanced" picture preset (smooth
/// motion and run-ahead aside: those follow [`Game::input_lag`]). Each is
/// on only where the game's own documentation says it is right for it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Recommended {
    /// The depth buffer: seen fixing pictures in the game's frames and
    /// breaking none (`docs/enhancements.md`, "Depth buffer").
    pub depth: bool,
    /// Smooth shading (vertex) with the game's crease: curved bodies and
    /// fighters look better (`docs/enhancements.md`, "Smooth shading").
    pub shading: bool,
    /// Temporal anti-aliasing at full size; false: 4x multisampling
    /// instead, where temporal misbehaves (`vstriker`).
    pub temporal: bool,
}

impl Recommended {
    /// What a game without evidence gets: temporal anti-aliasing, nothing
    /// else that changes what the game draws.
    pub const DEFAULT: Recommended = Recommended { depth: false, shading: false, temporal: true };
}

/// Per-game rules for smooth shading (`model2::light`, an enhancement):
/// which polygons it may smooth and how sharp an edge stays sharp.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shading {
    /// Faces meeting at more than this many degrees keep a hard edge.
    pub crease: u8,
    /// 3D windows never smooth-shaded (besides [`Game::hud_windows`]).
    pub skip_windows: &'static [usize],
    /// Models never smooth-shaded: address ranges (first, last). Needs the
    /// lighting side channel (not built yet): ignored without it.
    pub skip_models: &'static [(u32, u32)],
    /// Models with a crease angle of their own: (first, last, degrees).
    /// Needs the side channel, as `skip_models`.
    pub model_crease: &'static [(u32, u32, u8)],
    /// Checkerboard (screen door) polygons stay flat.
    pub skip_checker: bool,
    /// Welding tolerance, relative to the position (0: exact bits).
    /// Reserved: only exact welding is implemented.
    pub weld_eps: f32,
    /// Reserved: models whose consecutive object blocks are welded
    /// together. Not implemented.
    pub weld_models: &'static [(u32, u32)],
}

impl Shading {
    /// What every game uses until its pictures say otherwise: a 40 degree
    /// crease (boxes, 8-sided wheels and limbs stay faceted; bodies and
    /// 10+-sided cylinders smooth), nothing else excluded. Checked on all
    /// 18 working games' frames (docs/enhancements.md, "Smooth shading");
    /// a new game starts with it and needs nothing else: the generic code
    /// reads only these fields, never a game's identity. To give a game
    /// its own, write `shading: crate::Shading { crease: N,
    /// ..crate::Shading::DEFAULT }` in its manifest with a one-line reason,
    /// and the pictures behind it in `docs/games/<set>.md`.
    pub const DEFAULT: Shading = Shading {
        crease: 40,
        skip_windows: &[],
        skip_models: &[],
        model_crease: &[],
        skip_checker: false,
        weld_eps: 0.0,
        weld_models: &[],
    };
}

#[derive(Debug)]
pub enum Error {
    UnknownGame(String),
    Io(PathBuf, std::io::Error),
    Size {
        name: &'static str,
        expected: usize,
        actual: usize,
    },
    Hash {
        name: &'static str,
        expected: &'static str,
        actual: String,
    },
    /// A chip found in none of the places a set can be.
    Missing {
        name: &'static str,
        root: PathBuf,
        set: &'static str,
    },
    Zip(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::UnknownGame(id) => {
                let known: Vec<_> = games::ALL.iter().map(|g| g.id).collect();
                write!(f, "unknown game '{id}' (known: {})", known.join(", "))
            }
            Error::Io(p, e) => write!(f, "{}: {e}", p.display()),
            Error::Size { name, expected, actual } => {
                write!(f, "{name}: expected {expected:#x} bytes, found {actual:#x}")
            }
            Error::Hash { name, expected, actual } => {
                write!(f, "{name}: sha1 {actual} does not match known-good {expected}")
            }
            Error::Missing { name, root, set } => {
                let r = root.display();
                write!(f, "{name}: not in {r}/{set}/, {r}/{set}.zip or {r}/")
            }
            Error::Zip(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for Error {}

/// A loaded, assembled ROM set.
pub struct RomSet {
    pub game: &'static Game,
    /// i960 program space starting at address 0.
    pub program: Vec<u8>,
    /// Raw chip images, in manifest order.
    pub chips: Vec<Vec<u8>>,
}

impl RomSet {
    /// Load `game`'s chips from ROM directory `root`: each chip from
    /// `root/<set>/`, else `root/<set>.zip`, then (for a clone) the same
    /// for its parent set, else `root/` itself. With `verify`, sizes and
    /// hashes must match a known-good dump.
    pub fn load(game: &'static Game, root: &Path, verify: bool) -> Result<Self, Error> {
        let mut places = Vec::new();
        for set in std::iter::once(game.id).chain(game.parent) {
            let zip_path = root.join(format!("{set}.zip"));
            let archive = if zip_path.is_file() { Some(zip::Zip::open(&zip_path).map_err(Error::Zip)?) } else { None };
            places.push((root.join(set), archive));
        }
        let mut chips = Vec::with_capacity(game.files.len());
        'chip: for rom in game.files {
            for (dir, archive) in &places {
                if let Ok(d) = fs::read(dir.join(rom.name)) {
                    chips.push(d);
                    continue 'chip;
                }
                if let Some(member) = archive.as_ref().and_then(|z| z.read(rom.name)) {
                    chips.push(member.map_err(Error::Zip)?);
                    continue 'chip;
                }
            }
            let loose = fs::read(root.join(rom.name));
            chips.push(loose.map_err(|_| Error::Missing { name: rom.name, root: root.to_path_buf(), set: game.id })?);
        }
        Self::from_chips(game, chips, verify)
    }

    /// Assemble a set from chip images already in memory, in manifest
    /// order (e.g. fetched by a browser build).
    pub fn from_chips(game: &'static Game, chips: Vec<Vec<u8>>, verify: bool) -> Result<Self, Error> {
        for (rom, data) in game.files.iter().zip(&chips) {
            if data.len() != rom.size {
                return Err(Error::Size { name: rom.name, expected: rom.size, actual: data.len() });
            }
            if verify {
                let actual = sha1_hex(data);
                if actual != rom.sha1 {
                    return Err(Error::Hash { name: rom.name, expected: rom.sha1, actual });
                }
            }
        }
        let mut chips = chips;
        for d in game.dropouts {
            let (lo, hi) =
                polygon_pair(game, d.chip).unwrap_or_else(|| panic!("{}: {} is no polygon low half", game.id, d.chip));
            let [lo_chip, hi_chip] = chips.get_disjoint_mut([lo, hi]).expect("distinct chips");
            repair_link_bits(lo_chip, hi_chip, d.bit);
        }
        let program = assemble(game, &chips, Role::Program);
        Ok(RomSet { game, program, chips })
    }

    /// Assemble a 32-bit region from its interleaved chip pairs.
    pub fn region32(&self, role: Role) -> Vec<u8> {
        assemble(self.game, &self.chips, role)
    }

    /// `region32`, with each pair in its own `slot` bytes: a pair smaller
    /// than its slot repeats to fill it, as a board decoding fewer address
    /// lines than the slot has. Model 2C's ROM banks are 8 MB slots
    /// (stcc's 1 MB main data pair is read at `0x2C4B210`, 4 MB into the
    /// second slot).
    pub fn region32_slots(&self, role: Role, slot: usize) -> Vec<u8> {
        let mut out = Vec::new();
        let idx: Vec<usize> =
            self.game.files.iter().enumerate().filter(|(_, f)| f.role == role).map(|(i, _)| i).collect();
        for &[lo, hi] in idx.as_chunks::<2>().0 {
            let pair = interleave16(&self.chips[lo], &self.chips[hi]);
            if pair.is_empty() {
                continue;
            }
            let start = out.len();
            while out.len() - start < slot {
                let n = pair.len().min(slot - (out.len() - start));
                out.extend_from_slice(&pair[..n]);
            }
        }
        out
    }

    /// The bank of chips with `role` as the board maps it: pairs in
    /// [`ROM_SLOT`]s, or back to back for a bank the manifest lists as
    /// packed.
    pub fn bank(&self, role: Role) -> Vec<u8> {
        if self.game.packed.contains(&role) {
            self.region32(role)
        } else {
            self.region32_slots(role, ROM_SLOT)
        }
    }

    /// Concatenate the chips with `role` (16-bit, stored little-endian per
    /// word) and swap each byte pair, giving a 68000's big-endian view.
    pub fn region16be(&self, role: Role) -> Vec<u8> {
        let mut out = Vec::new();
        for (f, c) in self.game.files.iter().zip(&self.chips) {
            if f.role == role {
                out.extend(c.as_chunks::<2>().0.iter().flat_map(|&[a, b]| [b, a]));
            }
        }
        out
    }

    pub fn chip(&self, name: &str) -> Option<&[u8]> {
        self.game.files.iter().position(|r| r.name == name).map(|i| self.chips[i].as_slice())
    }
}

/// The set folder in ROM directory `root` that holds every chip of `game`
/// unpacked, at its size: `root/<set>/`, else (for a clone) its parent's,
/// as a merged MAME set keeps them. The browser build fetches chips from
/// there.
pub fn rom_folder(game: &Game, root: &Path) -> Option<&'static str> {
    std::iter::once(game.id).chain(game.parent).find(|set| {
        game.files.iter().all(|f| fs::metadata(root.join(set).join(f.name)).is_ok_and(|m| m.len() == f.size as u64))
    })
}

/// The sets whose texture pack serves `game`, in the order to look for one
/// (docs/texture-packs.md): its own, then for a clone its parent's. Pack
/// keys are the textures' content, so a parent's images serve every
/// version. The native player looks for `<set>/texpack/` directories in
/// this order, the browser for `/texpack/<set>/index.txt`.
pub fn pack_sets(game: &Game) -> impl Iterator<Item = &'static str> {
    std::iter::once(game.id).chain(game.parent)
}

/// Set names in ROM directory `root`: `<set>.zip` files and directories.
pub fn sets_in(root: &Path) -> Vec<String> {
    let mut sets: Vec<String> = fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let kind = e.file_type().ok()?;
            match name.strip_suffix(".zip") {
                Some(set) if kind.is_file() => Some(set.to_string()),
                None if kind.is_dir() => Some(name),
                _ => None,
            }
        })
        .collect();
    sets.sort();
    sets.dedup();
    sets
}

/// [`sets_in`], with each supported clone after its parent when the clone
/// has no zip or folder of its own (a merged MAME set keeps a clone's
/// chips in its parent's).
pub fn sets_with_clones(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let sets = sets_in(root);
    for set in &sets {
        out.push(set.clone());
        let clones = games::ALL.iter().filter(|g| g.parent == Some(set.as_str()) && !sets.iter().any(|s| s == g.id));
        out.extend(clones.map(|g| g.id.to_string()));
    }
    out
}

/// Concatenate the interleaved pairs of chips with `role`, in manifest order.
fn assemble(game: &Game, chips: &[Vec<u8>], role: Role) -> Vec<u8> {
    let idx: Vec<usize> = game.files.iter().enumerate().filter(|(_, f)| f.role == role).map(|(i, _)| i).collect();
    let mut out = Vec::new();
    for &[lo, hi] in idx.as_chunks::<2>().0 {
        out.extend(interleave16(&chips[lo], &chips[hi]));
    }
    out
}

/// Combine two 16-bit-wide chips into one 32-bit little-endian bus:
/// `lo` supplies bytes 0-1 of each word, `hi` bytes 2-3.
/// The manifest indices of `chip` and its partner, if `chip` is the low
/// half of a [`Role::Polygons`] pair.
fn polygon_pair(game: &Game, chip: &str) -> Option<(usize, usize)> {
    let idx: Vec<usize> =
        game.files.iter().enumerate().filter(|(_, f)| f.role == Role::Polygons).map(|(i, _)| i).collect();
    idx.as_chunks::<2>().0.iter().find(|&&[lo, _]| game.files[lo].name == chip).map(|&[lo, hi]| (lo, hi))
}

/// Restore link bit `bit` (8 or 9) of the polygon attribute words in a
/// pair's low chip `lo` from its copy 15 bits up, in the high chip `hi`.
/// An attribute word has a polygon type in bits 0-1 and bits 2-7 and 25-26
/// clear; its bits 23-24 repeat the link bits 8-9 (in every intact Model 2
/// polygon ROM here; `docs/hw/geometrizer.md` §3.3). The bit is set where
/// it reads 0, its copy 1, and the other link bit agrees with its own
/// copy. A float word that happens to look like that gets one mantissa bit
/// of its low half set, a change of at most 2^-14 relative, the size of
/// the error the dropout itself leaves in floats. Returns how many words
/// changed.
pub fn repair_link_bits(lo: &mut [u8], hi: &[u8], bit: u8) -> usize {
    assert!(bit == 8 || bit == 9, "only link bits 8 and 9 have a copy");
    let other = 17 - bit as u32;
    let mut n = 0;
    for (l, h) in lo.as_chunks_mut::<2>().0.iter_mut().zip(hi.as_chunks::<2>().0) {
        let w = u16::from_le_bytes(*l) as u32 | (u16::from_le_bytes(*h) as u32) << 16;
        let attr = w & 3 != 0 && w >> 2 & 0x3F == 0 && w >> 25 & 3 == 0;
        let lost = w >> bit & 1 == 0 && w >> (bit + 15) & 1 == 1;
        if attr && lost && w >> other & 1 == w >> (other + 15) & 1 {
            *l = (w as u16 | 1 << bit).to_le_bytes();
            n += 1;
        }
    }
    n
}

pub fn interleave16(lo: &[u8], hi: &[u8]) -> Vec<u8> {
    assert_eq!(lo.len(), hi.len());
    let mut out = Vec::with_capacity(lo.len() * 2);
    for (l, h) in lo.as_chunks::<2>().0.iter().zip(hi.as_chunks::<2>().0) {
        out.extend_from_slice(l);
        out.extend_from_slice(h);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_ahead_is_offered_up_to_three_frames() {
        for g in games::ALL {
            assert!(g.run_ahead_max <= 3, "{}", g.id);
            // Never past what the game's lag gives, except where a button
            // has some and the main control none.
            assert!(g.run_ahead_max <= g.input_lag.max(1) || g.run_ahead_max == 0, "{}", g.id);
            assert!(g.run_ahead_recommended() <= g.run_ahead_max);
        }
        let ra = |id| games::find(id).ok().map(|g| (g.run_ahead_max, g.run_ahead_recommended()));
        assert_eq!(ra("sgt24h"), Some((3, 3)));
        assert_eq!(ra("schamp"), Some((1, 0)));
        assert_eq!(ra("lastbrnx"), Some((1, 0)));
        assert_eq!(ra("topskatr"), Some((0, 0)));
        assert_eq!(ra("stcc"), Some((1, 1)));
    }

    #[test]
    fn clones_fall_back_to_the_parents_pack() {
        let sets = |id| pack_sets(games::find(id).unwrap()).collect::<Vec<_>>();
        assert_eq!(sets("overrevb"), ["overrevb", "overrev"]);
        assert_eq!(sets("topskatr"), ["topskatr"]);
    }

    #[test]
    fn interleave_order() {
        assert_eq!(interleave16(&[1, 2, 5, 6], &[3, 4, 7, 8]), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    /// Halves of a 32-bit word as two chips' little-endian 16-bit words.
    fn halves(words: &[u32]) -> (Vec<u8>, Vec<u8>) {
        let lo = words.iter().flat_map(|w| (*w as u16).to_le_bytes()).collect();
        let hi = words.iter().flat_map(|w| ((w >> 16) as u16).to_le_bytes()).collect();
        (lo, hi)
    }

    #[test]
    fn link_bit_repair() {
        let words = [
            0x7884_0902, // link 1, copy 1: intact
            0x7984_0902, // link 1, copy 3: bit 9 lost -> 0x7984_0b02
            0x7904_0801, // link 0, copy 2: bit 9 lost -> 0x7904_0a01
            0x7880_0902, // link 1, copy 1 with bit 23 clear: copies disagree on bit 8, left alone
            0x3f80_0001, // 1.0000001: bits 25-26 set, not an attribute word
            0x7984_09c2, // bits 2-7 not clear: not an attribute word
        ];
        let (mut lo, hi) = halves(&words);
        assert_eq!(repair_link_bits(&mut lo, &hi, 9), 2);
        let got: Vec<u32> = lo
            .as_chunks::<2>()
            .0
            .iter()
            .zip(hi.as_chunks::<2>().0)
            .map(|(l, h)| u16::from_le_bytes(*l) as u32 | (u16::from_le_bytes(*h) as u32) << 16)
            .collect();
        assert_eq!(got, [0x7884_0902, 0x7984_0b02, 0x7904_0a01, 0x7880_0902, 0x3f80_0001, 0x7984_09c2]);
    }

    #[test]
    fn dropouts_name_polygon_low_halves() {
        for g in games::ALL {
            for d in g.dropouts {
                assert!(polygon_pair(g, d.chip).is_some(), "{}: {} is no polygon low half", g.id, d.chip);
                assert!(d.bit == 8 || d.bit == 9, "{}: only link bits can be restored", g.id);
            }
        }
    }

    #[test]
    fn srallyc_samples_fill_the_sound_board_window() {
        // The sound programs' sample tables fill 0x800000-0xFFFFFF, `.31` first
        // and `.37` last (docs/games/srallyc.md, vf2.md).
        for g in games::ALL.iter().chain(games::IN_PROGRESS).filter(|g| ["srallyc", "vf2"].contains(&g.id)) {
            let s: Vec<_> = g.files.iter().filter(|f| f.role == Role::Samples).collect();
            assert_eq!(s.iter().map(|f| f.size).sum::<usize>(), 0x80_0000, "{}", g.id);
            assert!(s[0].name.ends_with(".31") && s[3].name.ends_with(".37"), "{}", g.id);
        }
    }

    #[test]
    fn manifests_have_program_pairs() {
        for g in games::ALL.iter().chain(games::IN_PROGRESS) {
            for role in [
                Role::Program,
                Role::MainData,
                Role::MainDataHigh,
                Role::CoproData,
                Role::Polygons,
                Role::Textures,
                Role::TgpData,
            ] {
                let n = g.files.iter().filter(|f| f.role == role).count();
                assert!(n % 2 == 0, "{}: {role:?} chips must come in pairs", g.id);
            }
        }
    }
}
