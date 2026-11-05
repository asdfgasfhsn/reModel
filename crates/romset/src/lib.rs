//! ROM set manifests and loading for Model 2 games.
//!
//! Game data is always user-supplied. A manifest only records the chip names,
//! sizes and SHA-1 hashes needed to recognise a good dump and assemble it into
//! the address spaces the hardware sees.
//!
//! A ROM directory holds sets as MAME does, by set name: `ROOT/<set>.zip`
//! or unpacked in `ROOT/<set>/`. Loose chips directly in `ROOT` also work.

pub mod controls;
pub mod games;
mod sha1;
pub mod zip;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub use controls::{AnalogInput, ButtonInput, Controls, PadAxis, PadButton};
pub use sha1::sha1_hex;

/// Cabinet boards beyond the Model 2 board itself that the game talks to
/// through the I/O chip, and which protocol they speak. The board model
/// (`model2::drive`) emulates each kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DriveBoard {
    /// Sega Ski Super G's platform drive board: one command byte a frame
    /// on I/O `0x1C00008`, one reply byte on `0x1C00006`; boots with a
    /// `0x00` -> `0xCE` handshake (README, "What's known about skisuprg").
    SkiPlatform,
    /// Sega Touring Car Championship's force feedback wheel: one command
    /// byte a frame on `0x1C00008`; the game stores the reply from
    /// `0x1C00006` but never checks it (README, "What's known about stcc").
    Wheel,
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
    /// i960-visible data ROM, 16-bit interleaved pairs.
    MainData,
    /// Geometry coprocessor data ROM, 16-bit interleaved pairs.
    CoproData,
    /// Polygon model data read by the geometry engine, interleaved pairs.
    Polygons,
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

pub struct Game {
    /// Short set name, matching the usual dump naming (e.g. `topskatr`).
    pub id: &'static str,
    pub title: &'static str,
    pub board: Board,
    pub files: &'static [RomFile],
    /// 3D viewports (window numbers) that hold HUD elements rather than the
    /// scene. Only enhancements use this (wide views move them with the
    /// rest of the HUD).
    pub hud_windows: &'static [usize],
    /// The cabinet's controls, where the board reads them, and default
    /// bindings.
    pub controls: Controls,
    /// A drive (motion or force feedback) board the game needs, if any.
    pub drive_board: Option<DriveBoard>,
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
    /// `root/<set>/`, else `root/<set>.zip`, else `root/` itself. With
    /// `verify`, sizes and hashes must match a known-good dump.
    pub fn load(game: &'static Game, root: &Path, verify: bool) -> Result<Self, Error> {
        let dir = root.join(game.id);
        let zip_path = root.join(format!("{}.zip", game.id));
        let archive = if zip_path.is_file() { Some(zip::Zip::open(&zip_path).map_err(Error::Zip)?) } else { None };
        let mut chips = Vec::with_capacity(game.files.len());
        for rom in game.files {
            let data = match fs::read(dir.join(rom.name)) {
                Ok(d) => d,
                Err(_) => match archive.as_ref().and_then(|z| z.read(rom.name)) {
                    Some(member) => member.map_err(Error::Zip)?,
                    None => fs::read(root.join(rom.name)).map_err(|_| Error::Missing {
                        name: rom.name,
                        root: root.to_path_buf(),
                        set: game.id,
                    })?,
                },
            };
            chips.push(data);
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
    fn interleave_order() {
        assert_eq!(interleave16(&[1, 2, 5, 6], &[3, 4, 7, 8]), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn manifests_have_program_pairs() {
        for g in games::ALL {
            for role in [Role::Program, Role::MainData, Role::CoproData, Role::Polygons, Role::Textures] {
                let n = g.files.iter().filter(|f| f.role == role).count();
                assert!(n % 2 == 0, "{}: {role:?} chips must come in pairs", g.id);
            }
        }
    }
}
