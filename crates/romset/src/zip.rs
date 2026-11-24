//! Reading ROM sets from zip archives (as MAME keeps them): the central
//! directory, and stored or deflated members. Std only; inflate comes from
//! the PNG crate. An archive opened from a file is not read whole: its
//! central directory is, and a member is read from the file when asked for.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// One central directory record.
#[derive(Clone, Debug)]
struct Entry {
    name: String,
    method: u16,
    packed: usize,
    size: usize,
    crc: u32,
    /// Local header offset.
    local: usize,
}

/// Where an archive's bytes are: in memory, or in a file read a member at a time.
enum Backing {
    Memory(Vec<u8>),
    File(PathBuf),
}

pub struct Zip {
    backing: Backing,
    entries: Vec<Entry>,
    /// The file's size and modification time, so a cache key notices a changed file.
    stamp: String,
}

/// A member as the central directory lists it, with nothing read: its stored name, size and CRC-32.
#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub size: u64,
    pub crc: u32,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

/// The end of central directory record: the last "PK\x05\x06" (a comment may follow it).
fn find_eocd(data: &[u8]) -> Result<usize, String> {
    (0..data.len().saturating_sub(21))
        .rev()
        .find(|&o| data[o..o + 4] == *b"PK\x05\x06")
        .ok_or_else(|| "not a zip archive".to_string())
}

/// The records of a central directory (`cd` starts at its first record).
fn parse_entries(cd: &[u8], count: usize) -> Result<Vec<Entry>, String> {
    let mut o = 0;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        if cd.get(o..o + 4) != Some(b"PK\x01\x02") {
            return Err("bad central directory".into());
        }
        let field = |k: usize| u32_at(cd, o + k).ok_or("truncated");
        let method = u16_at(cd, o + 10).ok_or("truncated")?;
        let crc = field(16)?;
        let packed = field(20)? as usize;
        let size = field(24)? as usize;
        let name_len = u16_at(cd, o + 28).ok_or("truncated")? as usize;
        let extra = u16_at(cd, o + 30).ok_or("truncated")? as usize;
        let comment = u16_at(cd, o + 32).ok_or("truncated")? as usize;
        let local = field(42)? as usize;
        let name = String::from_utf8_lossy(cd.get(o + 46..o + 46 + name_len).ok_or("truncated")?).into_owned();
        entries.push(Entry { name, method, packed, size, crc, local });
        o += 46 + name_len + extra + comment;
    }
    Ok(entries)
}

impl Zip {
    /// Open the archive at `path`: only its central directory is read; members are read on demand.
    pub fn open(path: &Path) -> Result<Zip, String> {
        let err = |e: String| format!("{}: {e}", path.display());
        let mut f = File::open(path).map_err(|e| err(e.to_string()))?;
        let meta = f.metadata().map_err(|e| err(e.to_string()))?;
        let len = meta.len();
        let tail_len = len.min(65_535 + 22);
        let mut tail = vec![0; tail_len as usize];
        f.seek(SeekFrom::Start(len - tail_len))
            .and_then(|_| f.read_exact(&mut tail))
            .map_err(|e| err(e.to_string()))?;
        let eocd = find_eocd(&tail).map_err(err)?;
        let trunc = || err("truncated".into());
        let count = u16_at(&tail, eocd + 10).ok_or_else(trunc)? as usize;
        let cd_len = u32_at(&tail, eocd + 12).ok_or_else(trunc)? as u64;
        let cd_at = u32_at(&tail, eocd + 16).ok_or_else(trunc)? as u64;
        if cd_at.checked_add(cd_len).is_none_or(|end| end > len) {
            return Err(err("bad central directory".into()));
        }
        let mut cd = vec![0; cd_len as usize];
        f.seek(SeekFrom::Start(cd_at)).and_then(|_| f.read_exact(&mut cd)).map_err(|e| err(e.to_string()))?;
        let entries = parse_entries(&cd, count).map_err(err)?;
        let mtime =
            meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos());
        Ok(Zip { backing: Backing::File(path.to_path_buf()), entries, stamp: format!("{len}:{mtime}") })
    }

    /// An archive held in memory.
    pub fn parse(data: Vec<u8>) -> Result<Zip, String> {
        let eocd = find_eocd(&data)?;
        let count = u16_at(&data, eocd + 10).ok_or("truncated")? as usize;
        let at = u32_at(&data, eocd + 16).ok_or("truncated")? as usize;
        let entries = parse_entries(data.get(at..).ok_or("bad central directory")?, count)?;
        Ok(Zip { backing: Backing::Memory(data), entries, stamp: String::new() })
    }

    /// The archive's file, if it came from one.
    pub fn path(&self) -> Option<&Path> {
        match &self.backing {
            Backing::File(p) => Some(p),
            Backing::Memory(_) => None,
        }
    }

    /// The file's size and modification time as text: changes when the file does.
    pub fn stamp(&self) -> &str {
        &self.stamp
    }

    /// The members (not folders) the central directory lists, in order.
    pub fn members(&self) -> impl Iterator<Item = Member> + '_ {
        self.entries.iter().filter(|e| !e.name.ends_with('/')).map(Entry::member)
    }

    fn bytes(&self, off: usize, len: usize) -> Option<Vec<u8>> {
        match &self.backing {
            Backing::Memory(d) => d.get(off..off.checked_add(len)?).map(<[u8]>::to_vec),
            Backing::File(p) => {
                let mut f = File::open(p).ok()?;
                f.seek(SeekFrom::Start(off as u64)).ok()?;
                let mut b = vec![0; len];
                f.read_exact(&mut b).ok()?;
                Some(b)
            }
        }
    }

    /// Member names, as stored.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// The entry `name` names: a name with a folder (`topskatrj/epr-19753.15`)
    /// matches the stored path (or its tail); a bare name matches a member
    /// at the top of the archive, else the first one of that name in any
    /// folder. Case-insensitive.
    fn entry(&self, name: &str) -> Option<&Entry> {
        let want = name.to_ascii_lowercase();
        if want.contains('/') {
            let tail = format!("/{want}");
            return self.entries.iter().find(|e| {
                let n = e.name.to_ascii_lowercase();
                n == want || n.ends_with(&tail)
            });
        }
        let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
        self.entries
            .iter()
            .find(|e| e.name.to_ascii_lowercase() == want)
            .or_else(|| self.entries.iter().find(|e| base(&e.name) == want))
    }

    /// Whether the archive has a member `name` (as [`Zip::read`] finds it).
    pub fn has(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    /// The member `name` as the central directory lists it (as [`Zip::read`] finds it).
    pub fn member(&self, name: &str) -> Option<Member> {
        self.entry(name).map(Entry::member)
    }

    /// A member's contents by name (case-insensitive; a name with a folder
    /// names that folder's member, a bare name ignores folders), checked
    /// against its CRC-32.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
        let Entry { name: ref stored, method, packed, size, crc, local } = *self.entry(name)?;
        let read = || -> Result<Vec<u8>, String> {
            let head = self.bytes(local, 30).ok_or_else(|| format!("{stored}: truncated"))?;
            if head.get(..4) != Some(b"PK\x03\x04") {
                return Err(format!("{stored}: bad local header"));
            }
            let skip =
                30 + u16_at(&head, 26).ok_or("truncated")? as usize + u16_at(&head, 28).ok_or("truncated")? as usize;
            let raw = self.bytes(local + skip, packed).ok_or_else(|| format!("{stored}: truncated"))?;
            let out = match method {
                0 => raw,
                8 => png::zlib::inflate(&raw).map_err(|e| format!("{stored}: {e}"))?,
                m => return Err(format!("{stored}: unsupported compression method {m}")),
            };
            if out.len() != size || png::crc32(&out) != crc {
                return Err(format!("{stored}: CRC or size mismatch"));
            }
            Ok(out)
        };
        Some(read())
    }
}

impl Entry {
    fn member(&self) -> Member {
        Member { name: self.name.clone(), size: self.size as u64, crc: self.crc }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// An archive of `(name, body)` members, each stored (method 0) or deflated (method 8).
    pub(crate) fn archive_of(members: &[(&str, &[u8])], method: u16) -> Vec<u8> {
        let mut z = Vec::new();
        let mut central = Vec::new();
        for &(name, body) in members {
            let packed = if method == 8 { png::zlib::deflate(body) } else { body.to_vec() };
            let crc = png::crc32(body);
            let local = z.len() as u32;
            let header = |z: &mut Vec<u8>, sig: &[u8], is_central: bool| {
                z.extend(sig);
                if is_central {
                    z.extend(20u16.to_le_bytes());
                }
                z.extend([20, 0, 0, 0]);
                z.extend(method.to_le_bytes());
                z.extend([0; 4]);
                z.extend(crc.to_le_bytes());
                z.extend((packed.len() as u32).to_le_bytes());
                z.extend((body.len() as u32).to_le_bytes());
                z.extend((name.len() as u16).to_le_bytes());
                z.extend(0u16.to_le_bytes());
                if is_central {
                    // Comment length, disk, attributes; then the local header offset.
                    z.extend([0; 10]);
                    z.extend(local.to_le_bytes());
                }
                z.extend(name.as_bytes());
            };
            header(&mut z, b"PK\x03\x04", false);
            z.extend(&packed);
            header(&mut central, b"PK\x01\x02", true);
        }
        let cd = z.len();
        z.extend(&central);
        z.extend(b"PK\x05\x06");
        z.extend([0; 4]);
        z.extend((members.len() as u16).to_le_bytes());
        z.extend((members.len() as u16).to_le_bytes());
        z.extend((central.len() as u32).to_le_bytes());
        z.extend((cd as u32).to_le_bytes());
        z.extend(0u16.to_le_bytes());
        z
    }

    fn archive(name: &str, body: &[u8], method: u16) -> Vec<u8> {
        archive_of(&[(name, body)], method)
    }

    #[test]
    fn reads_stored_and_deflated_members() {
        let body: Vec<u8> = (0..5000u32).map(|i| (i * 7 % 13) as u8).collect();
        for method in [0, 8] {
            let z = Zip::parse(archive("dir/EPR-1.15", &body, method)).unwrap();
            assert_eq!(z.names().collect::<Vec<_>>(), ["dir/EPR-1.15"]);
            assert_eq!(z.read("epr-1.15").unwrap().unwrap(), body);
            assert!(z.read("missing").is_none());
        }
    }

    #[test]
    fn a_zip_file_is_read_a_member_at_a_time() {
        let a: Vec<u8> = (0..3000u32).map(|i| (i * 5 % 11) as u8).collect();
        let b: Vec<u8> = (0..70_000u32).map(|i| (i * 3 % 17) as u8).collect();
        let dir = std::env::temp_dir().join(format!("remodel-zip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.zip");
        std::fs::write(&path, archive_of(&[("a.bin", &a), ("sub/b.bin", &b)], 8)).unwrap();
        let z = Zip::open(&path).unwrap();
        let listed: Vec<_> = z.members().map(|m| (m.name, m.size, m.crc)).collect();
        assert_eq!(listed, [("a.bin".into(), 3000, png::crc32(&a)), ("sub/b.bin".into(), 70_000, png::crc32(&b))]);
        assert_eq!(z.read("b.bin").unwrap().unwrap(), b);
        assert_eq!(z.read("a.bin").unwrap().unwrap(), a);
        assert!(!z.stamp().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_name_with_a_folder_picks_that_folders_member() {
        // A merged set keeps same-named chips of its variants in folders (`topskatrj/`, `topskatru/`).
        let entry = |n: &str| Entry { name: n.to_string(), method: 0, packed: 0, size: 0, crc: 0, local: 0 };
        let z = Zip {
            backing: Backing::Memory(Vec::new()),
            stamp: String::new(),
            entries: vec![
                entry("topskatrj/epr-1.15"),
                entry("topskatru/epr-1.15"),
                entry("Top.BIN"),
                entry("sub/top.bin"),
            ],
        };
        let at = |n: &str| z.entry(n).map(|e| e.name.as_str());
        assert_eq!(at("topskatru/epr-1.15"), Some("topskatru/epr-1.15"));
        assert_eq!(at("TOPSKATRJ/EPR-1.15"), Some("topskatrj/epr-1.15"), "case-insensitive");
        assert_eq!(at("other/epr-1.15"), None, "a folder that is not there is not another folder's member");
        // A bare name finds the top-level member before one in a folder, else the first of that name.
        assert_eq!(at("top.bin"), Some("Top.BIN"));
        assert_eq!(at("epr-1.15"), Some("topskatrj/epr-1.15"));
        assert!(z.has("topskatru/epr-1.15") && !z.has("nothing"));
    }
}
