//! Finding a chip by what it is (its SHA-1), wherever the ROM folder keeps it:
//! in a zip under any member name, loose under any file name, in any
//! subfolder. A manifest's chip name is where [`crate::RomSet`] looks first;
//! this is what it falls back to when the name finds nothing or finds a
//! different dump.
//!
//! The folder is walked once (zips by their central directory alone), the
//! files are narrowed to the ones of the chip's size, and those are hashed
//! until one matches. A hash costs a read and a decompression, so every one
//! computed goes in a [`HashCache`], on disk between runs, keyed by what
//! says the file is unchanged (path, size, modification time, and for a zip
//! member its CRC-32). Two members with the same CRC-32 and size are
//! taken to be the same bytes when only the cache is asked; a chip picked
//! by content is always read and hashed again before it is used.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::sha1::sha1_hex;
use crate::zip::Zip;
use crate::RomFile;

/// Archives larger than this are not ROM sets (no Model 2 set's zip is a
/// tenth of it): an emulator's download bundling whole sets is skipped
/// unread rather than searched.
const MAX_ZIP: u64 = 128 << 20;
/// How deep below the ROM folder files are looked for.
const MAX_DEPTH: usize = 4;

/// SHA-1s computed before, by the file's identity (see the module comment).
/// Lives in a text file of `key<TAB>sha1` lines; with no file it is a plain
/// in-memory memo.
pub struct HashCache {
    path: Option<PathBuf>,
    map: HashMap<String, String>,
    used: HashSet<String>,
    dirty: bool,
}

/// More entries than this on disk and a save keeps only what the run used.
const KEEP: usize = 50_000;

impl HashCache {
    /// No file: remembers for this run only.
    pub fn memory() -> Self {
        HashCache { path: None, map: HashMap::new(), used: HashSet::new(), dirty: false }
    }

    /// The cache kept in `path` (read now, written by [`HashCache::save`]).
    pub fn at(path: &Path) -> Self {
        let mut c = HashCache { path: Some(path.to_path_buf()), ..Self::memory() };
        if let Ok(text) = fs::read_to_string(path) {
            for line in text.lines() {
                if let Some((k, v)) = line.split_once('\t') {
                    c.map.insert(k.to_string(), v.to_string());
                }
            }
        }
        c
    }

    /// The user's cache: `$REMODEL_ROM_CACHE` (a file; `off` for none), else
    /// `remodel/rom-sha1.tsv` under `$XDG_CACHE_HOME` or `~/.cache`.
    pub fn user() -> Self {
        match std::env::var("REMODEL_ROM_CACHE") {
            Ok(p) if p == "off" => return Self::memory(),
            Ok(p) if !p.is_empty() => return Self::at(Path::new(&p)),
            _ => {}
        }
        let base = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| Path::new(&h).join(".cache")));
        match base {
            Some(b) => Self::at(&b.join("remodel").join("rom-sha1.tsv")),
            None => Self::memory(),
        }
    }

    /// How many hashes it holds.
    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    fn get(&mut self, key: &str) -> Option<String> {
        let v = self.map.get(key)?.clone();
        self.used.insert(key.to_string());
        Some(v)
    }

    fn put(&mut self, key: String, sha1: String) {
        if key.contains(['\t', '\n']) {
            return;
        }
        self.used.insert(key.clone());
        if self.map.insert(key, sha1.clone()).as_ref() != Some(&sha1) {
            self.dirty = true;
        }
    }

    /// Write the cache back if this run added to it. Failing to is harmless.
    pub fn save(&mut self) {
        let Some(path) = self.path.clone().filter(|_| self.dirty) else { return };
        if self.map.len() > KEEP {
            let used = &self.used;
            self.map.retain(|k, _| used.contains(k));
        }
        let mut lines: Vec<String> = self.map.iter().map(|(k, v)| format!("{k}\t{v}\n")).collect();
        lines.sort();
        if let Some(dir) = path.parent() {
            let _ = fs::create_dir_all(dir);
        }
        let tmp = path.with_extension(format!("tmp{}", std::process::id()));
        if fs::write(&tmp, lines.concat()).is_ok() && fs::rename(&tmp, &path).is_ok() {
            self.dirty = false;
        } else {
            let _ = fs::remove_file(&tmp);
        }
    }
}

#[derive(Clone)]
enum Loc {
    File { path: PathBuf, stamp: String },
    Member { zip: Arc<Zip>, name: String, crc: u32 },
}

/// Somewhere a chip's bytes could be: a file, or a member of a zip.
#[derive(Clone)]
pub(crate) struct Candidate {
    loc: Loc,
    pub size: u64,
}

fn file_stamp(m: &fs::Metadata) -> String {
    let mtime =
        m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
    format!("{}:{mtime}", m.len())
}

impl Candidate {
    pub fn file(path: PathBuf) -> Option<Candidate> {
        let m = fs::metadata(&path).ok().filter(|m| m.is_file())?;
        Some(Candidate { size: m.len(), loc: Loc::File { stamp: file_stamp(&m), path } })
    }

    /// Member `name` (as [`Zip::read`] finds it) of `zip`.
    pub fn member(zip: &Arc<Zip>, name: &str) -> Option<Candidate> {
        let m = zip.member(name)?;
        Some(Candidate { size: m.size, loc: Loc::Member { zip: zip.clone(), name: m.name, crc: m.crc } })
    }

    /// Where it is, for messages: the file, or `zip:member`.
    pub fn label(&self) -> String {
        match &self.loc {
            Loc::File { path, .. } => path.display().to_string(),
            Loc::Member { zip, name, .. } => match zip.path() {
                Some(p) => format!("{}:{name}", p.display()),
                None => name.clone(),
            },
        }
    }

    fn key(&self) -> String {
        match &self.loc {
            Loc::File { path, stamp } => format!("f|{}|{stamp}", path.display()),
            Loc::Member { zip, name, crc } => {
                format!(
                    "z|{}|{}|{name}|{crc:08x}",
                    zip.path().map_or(String::new(), |p| p.display().to_string()),
                    zip.stamp()
                )
            }
        }
    }

    pub fn read(&self) -> Result<Vec<u8>, String> {
        match &self.loc {
            Loc::File { path, .. } => fs::read(path).map_err(|e| format!("{}: {e}", path.display())),
            Loc::Member { zip, name, .. } => {
                zip.read(name).unwrap_or_else(|| Err(format!("{name}: not in the archive")))
            }
        }
    }

    /// Its SHA-1: from the cache if it knows this file (or, for a member, one with the
    /// same CRC-32 and size), else computed and remembered.
    pub fn sha1(&self, cache: &mut HashCache) -> Result<String, String> {
        let key = self.key();
        if let Some(h) = cache.get(&key) {
            return Ok(h);
        }
        let same = match &self.loc {
            Loc::Member { crc, .. } => Some(format!("c|{crc:08x}|{}", self.size)),
            Loc::File { .. } => None,
        };
        if let Some(h) = same.as_deref().and_then(|k| cache.get(k)) {
            cache.put(key, h.clone());
            return Ok(h);
        }
        let h = sha1_hex(&self.read()?);
        cache.put(key, h.clone());
        if let Some(k) = same {
            cache.put(k, h.clone());
        }
        Ok(h)
    }
}

/// Every file under a ROM folder (and every member of its zips), by size.
struct Index {
    by_size: HashMap<u64, Vec<Candidate>>,
}

impl Index {
    fn scan(root: &Path) -> Index {
        let mut index = Index { by_size: HashMap::new() };
        index.walk(root, 0);
        index
    }

    fn walk(&mut self, dir: &Path, depth: usize) {
        let Ok(rd) = fs::read_dir(dir) else { return };
        let mut paths: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        paths.sort();
        for p in paths {
            // (Hidden files, and the m2emu emulator download: never read, whatever its size.)
            let hidden = p
                .file_name()
                .and_then(|n| n.to_str())
                .is_none_or(|n| n.starts_with('.') || n.to_ascii_lowercase().starts_with("m2emu"));
            let Ok(m) = fs::metadata(&p) else { continue };
            if hidden {
                continue;
            }
            if m.is_dir() {
                if depth < MAX_DEPTH {
                    self.walk(&p, depth + 1);
                }
            } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
                if m.len() <= MAX_ZIP {
                    if let Ok(z) = Zip::open(&p) {
                        let z = Arc::new(z);
                        for mem in z.members() {
                            let loc = Loc::Member { zip: z.clone(), name: mem.name, crc: mem.crc };
                            self.by_size.entry(mem.size).or_default().push(Candidate { size: mem.size, loc });
                        }
                    }
                }
            } else if m.is_file() {
                let c = Candidate { size: m.len(), loc: Loc::File { stamp: file_stamp(&m), path: p } };
                self.by_size.entry(c.size).or_default().push(c);
            }
        }
    }
}

/// Finds chips under one ROM folder by content. The folder is walked the first time it is asked.
pub(crate) struct Locator {
    root: PathBuf,
    index: Option<Index>,
}

impl Locator {
    pub fn new(root: &Path) -> Self {
        Locator { root: root.to_path_buf(), index: None }
    }

    fn index(&mut self) -> &Index {
        let root = &self.root;
        self.index.get_or_insert_with(|| Index::scan(root))
    }

    /// How many files and members have exactly `size` bytes.
    pub fn count(&mut self, size: usize) -> usize {
        self.index().by_size.get(&(size as u64)).map_or(0, Vec::len)
    }

    /// A file or member whose SHA-1 is `rom`'s. With `read`, its bytes too, read and hashed
    /// again (so a cached hash that was wrong cannot put a wrong chip in).
    pub fn find(&mut self, rom: &RomFile, cache: &mut HashCache, read: bool) -> Option<(Candidate, Vec<u8>)> {
        let cands = self.index().by_size.get(&(rom.size as u64))?.clone();
        for c in cands {
            if c.sha1(cache).is_ok_and(|h| h == rom.sha1) {
                if !read {
                    return Some((c, Vec::new()));
                }
                if let Ok(data) = c.read() {
                    if sha1_hex(&data) == rom.sha1 {
                        return Some((c, data));
                    }
                }
            }
        }
        None
    }
}
