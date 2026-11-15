//! The hot-block list (docs/jit.md, "Hot-block list"): the block starts
//! each JIT translated, by chip and program hash, kept between runs so
//! that the workers translate them at boot, before the game reaches
//! them. Addresses only: no game code and no machine code, and it lives
//! only on the user's machine (the front end says where).
//!
//! Every entry is a hint. One that is stale (another version of the
//! program, code that moved in RAM) or bad (it doesn't decode) costs a
//! wasted translation at most: the JIT plans every block from the code
//! as it is, checks code in RAM as for any block, and a block the game
//! never reaches is never run.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

/// The list's file name in its folder (the front end picks the folder,
/// one per game set: docs/jit.md, "Hot-block list").
pub const FILE: &str = "hot.txt";

/// The file's first line; the number is the format's version (another
/// is ignored, so the list starts again).
pub const HEADER: &str = "reModel JIT hot blocks 1";

/// The chips whose JITs keep lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Chip {
    I960,
    Mb86234,
    Mb86235,
    /// Model 2B's SHARCs, copro and geometrizer (their programs differ).
    Adsp2106x,
}

impl Chip {
    pub fn name(self) -> &'static str {
        match self {
            Chip::I960 => "i960",
            Chip::Mb86234 => "mb86234",
            Chip::Mb86235 => "mb86235",
            Chip::Adsp2106x => "adsp2106x",
        }
    }

    fn parse(s: &str) -> Option<Chip> {
        [Chip::I960, Chip::Mb86234, Chip::Mb86235, Chip::Adsp2106x].into_iter().find(|c| c.name() == s)
    }
}

#[derive(Default)]
struct Inner {
    starts: BTreeMap<(Chip, u64), BTreeSet<u32>>,
    /// Changed since loaded or saved.
    dirty: bool,
}

/// The list, shared by a machine's JITs (and run-ahead's copy of it).
#[derive(Default)]
pub struct HotList {
    inner: Mutex<Inner>,
}

impl HotList {
    pub fn new() -> Self {
        HotList::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// From the text of a list file. An error (a version this build
    /// doesn't read, a malformed line) gives no list: the caller starts
    /// an empty one, which the next save replaces.
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut lines = text.lines();
        match lines.next() {
            Some(h) if h.trim() == HEADER => {}
            Some(h) if h.starts_with("reModel JIT hot blocks ") => {
                return Err(format!("format {} (this build reads {HEADER:?})", h.trim()))
            }
            _ => return Err("not a reModel JIT hot-block list".into()),
        }
        let mut starts: BTreeMap<(Chip, u64), BTreeSet<u32>> = BTreeMap::new();
        // The section the address lines belong to (`None`: a chip this
        // build doesn't know, skipped).
        let mut at: Option<Option<(Chip, u64)>> = None;
        for (n, line) in lines.enumerate() {
            let bad = || format!("line {}: {line:?}", n + 2);
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with(' ') {
                let Some(section) = at else { return Err(bad()) };
                for a in line.split_whitespace() {
                    let a = u32::from_str_radix(a, 16).map_err(|_| bad())?;
                    if let Some(key) = section {
                        starts.entry(key).or_default().insert(a);
                    }
                }
                continue;
            }
            let mut f = line.split_whitespace();
            let (Some(chip), Some(hash)) = (f.next(), f.next()) else { return Err(bad()) };
            let hash = u64::from_str_radix(hash, 16).map_err(|_| bad())?;
            at = Some(Chip::parse(chip).map(|c| (c, hash)));
        }
        Ok(HotList { inner: Mutex::new(Inner { starts, dirty: false }) })
    }

    /// The file's text.
    pub fn text(&self) -> String {
        let g = self.lock();
        let mut s = format!(
            "{HEADER}\n# Block entry points reModel's JITs translated, by chip and program hash (docs/jit.md,\n\
             # \"Hot-block list\"): addresses only, translated again at boot. Safe to delete.\n"
        );
        for (&(chip, hash), set) in &g.starts {
            s += &format!("{} {hash:016x} {}\n", chip.name(), set.len());
            for row in set.iter().copied().collect::<Vec<_>>().chunks(10) {
                let row: Vec<String> = row.iter().map(|a| format!("{a:x}")).collect();
                s += &format!(" {}\n", row.join(" "));
            }
        }
        s
    }

    /// The list in `path`: empty if there is none, or (with a warning) if
    /// it can't be read.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => HotList::parse(&text).unwrap_or_else(|e| {
                eprintln!("jit: {}: {e}; starting a new hot-block list", path.display());
                HotList::new()
            }),
            Err(_) => HotList::new(),
        }
    }

    /// Write the list to `path` if it changed since it was loaded or last
    /// saved (through a temporary file, so a reader never sees half of
    /// it). True if it wrote.
    pub fn save(&self, path: &Path) -> std::io::Result<bool> {
        if !self.lock().dirty {
            return Ok(false);
        }
        let text = self.text();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        self.lock().dirty = false;
        Ok(true)
    }

    /// The starts listed for `chip` running the program `hash`.
    pub fn get(&self, chip: Chip, hash: u64) -> Vec<u32> {
        self.lock().starts.get(&(chip, hash)).map(|s| s.iter().copied().collect()).unwrap_or_default()
    }

    /// Note blocks translated at `starts`.
    pub fn add(&self, chip: Chip, hash: u64, starts: impl IntoIterator<Item = u32>) {
        let mut g = self.lock();
        let set = g.starts.entry((chip, hash)).or_default();
        let before = set.len();
        set.extend(starts);
        if set.len() != before {
            g.dirty = true;
        }
    }

    /// Starts listed in all.
    pub fn len(&self) -> usize {
        self.lock().starts.values().map(BTreeSet::len).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The starts listed, by chip.
    pub fn counts(&self) -> Vec<(Chip, usize)> {
        let mut m: BTreeMap<Chip, usize> = BTreeMap::new();
        for (&(c, _), s) in &self.lock().starts {
            *m.entry(c).or_default() += s.len();
        }
        m.into_iter().collect()
    }
}

/// A 64-bit FNV-1a hash of `words` (a program's, as a list key).
pub fn hash_words(words: impl IntoIterator<Item = u32>) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for w in words {
        for b in w.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_hints() {
        let l = HotList::new();
        l.add(Chip::I960, 0x1234, [0x100, 0x2000, 0x5a_0000]);
        l.add(Chip::Mb86235, 7, (0..25).map(|k| k * 3));
        let text = l.text();
        let m = HotList::parse(&text).unwrap();
        assert_eq!(m.get(Chip::I960, 0x1234), vec![0x100, 0x2000, 0x5a_0000]);
        assert_eq!(m.get(Chip::Mb86235, 7).len(), 25);
        assert!(m.get(Chip::Mb86234, 7).is_empty());
        assert_eq!(m.text(), text);
        // Another version, garbage: no list. A chip this build doesn't
        // know: skipped.
        assert!(HotList::parse(&text.replace(" 1\n", " 2\n")).is_err());
        assert!(HotList::parse("hello").is_err());
        assert!(HotList::parse(&format!("{HEADER}\ni960 zz 1\n 10\n")).is_err());
        let other = HotList::parse(&format!("{HEADER}\nsh4 0 2\n 10 20\ni960 1 1\n 30\n")).unwrap();
        assert_eq!(other.len(), 1);
    }

    #[test]
    fn saves_only_what_changed() {
        // Under target/ (on disk; /tmp may be memory).
        let dir =
            Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../target/hotlist-test-{}", std::process::id()));
        let path = dir.join("hot.txt");
        let l = HotList::new();
        assert!(!l.save(&path).unwrap(), "nothing to save");
        l.add(Chip::I960, 1, [4, 8]);
        assert!(l.save(&path).unwrap());
        assert!(!l.save(&path).unwrap());
        l.add(Chip::I960, 1, [8]);
        assert!(!l.save(&path).unwrap(), "nothing new");
        assert_eq!(HotList::load(&path).get(Chip::I960, 1), vec![4, 8]);
        let _ = std::fs::remove_dir_all(dir);
    }
}
