//! ROM set manifests and loading for Model 2 games.
//!
//! Game data is always user-supplied. A manifest only records the chip names,
//! sizes and SHA-1 hashes needed to recognise a good dump and assemble it into
//! the address spaces the hardware sees.

pub mod games;
mod sha1;

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

pub use sha1::sha1_hex;

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
    /// Not yet identified.
    Unknown,
}

pub struct Game {
    /// Short set name, matching the usual dump naming (e.g. `topskatr`).
    pub id: &'static str,
    pub title: &'static str,
    pub board: Board,
    pub files: &'static [RomFile],
}

#[derive(Debug)]
pub enum Error {
    UnknownGame(String),
    Io(PathBuf, std::io::Error),
    Size { name: &'static str, expected: usize, actual: usize },
    Hash { name: &'static str, expected: &'static str, actual: String },
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
    /// Load the unpacked chips for `game` from `dir`. With `verify`, sizes
    /// and hashes must match a known-good dump.
    pub fn load(game: &'static Game, dir: &Path, verify: bool) -> Result<Self, Error> {
        let mut chips = Vec::with_capacity(game.files.len());
        for rom in game.files {
            let path = dir.join(rom.name);
            let data = fs::read(&path).map_err(|e| Error::Io(path.clone(), e))?;
            if data.len() != rom.size {
                return Err(Error::Size { name: rom.name, expected: rom.size, actual: data.len() });
            }
            if verify {
                let actual = sha1_hex(&data);
                if actual != rom.sha1 {
                    return Err(Error::Hash { name: rom.name, expected: rom.sha1, actual });
                }
            }
            chips.push(data);
        }
        let program = assemble(game, &chips, Role::Program);
        Ok(RomSet { game, program, chips })
    }

    pub fn chip(&self, name: &str) -> Option<&[u8]> {
        self.game.files.iter().position(|r| r.name == name).map(|i| self.chips[i].as_slice())
    }
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
            let n = g.files.iter().filter(|f| f.role == Role::Program).count();
            assert!(n > 0 && n % 2 == 0, "{}: program chips must come in pairs", g.id);
        }
    }
}
