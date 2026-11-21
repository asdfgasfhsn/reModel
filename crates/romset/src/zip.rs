//! Reading ROM sets from zip archives (as MAME keeps them): the central
//! directory, and stored or deflated members. Std only; inflate comes from
//! the PNG crate.

use std::path::Path;

pub struct Zip {
    data: Vec<u8>,
    /// (name, method, compressed size, uncompressed size, CRC-32, local
    /// header offset) per member.
    entries: Vec<(String, u16, usize, usize, u32, usize)>,
}

fn u16_at(b: &[u8], o: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(o..o + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], o: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(o..o + 4)?.try_into().ok()?))
}

impl Zip {
    pub fn open(path: &Path) -> Result<Zip, String> {
        let data = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Zip::parse(data).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(data: Vec<u8>) -> Result<Zip, String> {
        // End of central directory: the last "PK\x05\x06" (a comment may
        // follow it).
        let eocd = (0..data.len().saturating_sub(21))
            .rev()
            .find(|&o| data[o..o + 4] == *b"PK\x05\x06")
            .ok_or("not a zip archive")?;
        let count = u16_at(&data, eocd + 10).ok_or("truncated")? as usize;
        let mut o = u32_at(&data, eocd + 16).ok_or("truncated")? as usize;
        let mut entries = Vec::with_capacity(count);
        for _ in 0..count {
            if data.get(o..o + 4) != Some(b"PK\x01\x02") {
                return Err("bad central directory".into());
            }
            let field = |k: usize| u32_at(&data, o + k).ok_or("truncated");
            let method = u16_at(&data, o + 10).ok_or("truncated")?;
            let crc = field(16)?;
            let packed = field(20)? as usize;
            let size = field(24)? as usize;
            let name_len = u16_at(&data, o + 28).ok_or("truncated")? as usize;
            let extra = u16_at(&data, o + 30).ok_or("truncated")? as usize;
            let comment = u16_at(&data, o + 32).ok_or("truncated")? as usize;
            let local = field(42)? as usize;
            let name = String::from_utf8_lossy(data.get(o + 46..o + 46 + name_len).ok_or("truncated")?).into_owned();
            entries.push((name, method, packed, size, crc, local));
            o += 46 + name_len + extra + comment;
        }
        Ok(Zip { data, entries })
    }

    /// Member names, as stored.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|e| e.0.as_str())
    }

    /// The entry `name` names: a name with a folder (`topskatrj/epr-19753.15`)
    /// matches the stored path (or its tail); a bare name matches a member
    /// at the top of the archive, else the first one of that name in any
    /// folder. Case-insensitive.
    fn entry(&self, name: &str) -> Option<&(String, u16, usize, usize, u32, usize)> {
        let want = name.to_ascii_lowercase();
        if want.contains('/') {
            let tail = format!("/{want}");
            return self.entries.iter().find(|e| {
                let n = e.0.to_ascii_lowercase();
                n == want || n.ends_with(&tail)
            });
        }
        let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
        self.entries
            .iter()
            .find(|e| e.0.to_ascii_lowercase() == want)
            .or_else(|| self.entries.iter().find(|e| base(&e.0) == want))
    }

    /// Whether the archive has a member `name` (as [`Zip::read`] finds it).
    pub fn has(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    /// A member's contents by name (case-insensitive; a name with a folder
    /// names that folder's member, a bare name ignores folders), checked
    /// against its CRC-32.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
        let &(ref stored, method, packed, size, crc, local) = self.entry(name)?;
        let read = || -> Result<Vec<u8>, String> {
            let d = &self.data;
            if d.get(local..local + 4) != Some(b"PK\x03\x04") {
                return Err(format!("{stored}: bad local header"));
            }
            let start = local
                + 30
                + u16_at(d, local + 26).ok_or("truncated")? as usize
                + u16_at(d, local + 28).ok_or("truncated")? as usize;
            let raw = d.get(start..start + packed).ok_or_else(|| format!("{stored}: truncated"))?;
            let out = match method {
                0 => raw.to_vec(),
                8 => png::zlib::inflate(raw).map_err(|e| format!("{stored}: {e}"))?,
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A one-member archive, stored (method 0) or deflated (method 8).
    fn archive(name: &str, body: &[u8], method: u16) -> Vec<u8> {
        let packed = if method == 8 { png::zlib::deflate(body) } else { body.to_vec() };
        let crc = png::crc32(body);
        let mut z = Vec::new();
        let header = |z: &mut Vec<u8>, sig: &[u8], central: bool| {
            z.extend(sig);
            if central {
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
            if central {
                // Comment length, disk, attributes; then the local header offset.
                z.extend([0; 10]);
                z.extend(0u32.to_le_bytes());
            }
            z.extend(name.as_bytes());
        };
        header(&mut z, b"PK\x03\x04", false);
        z.extend(&packed);
        let cd = z.len();
        header(&mut z, b"PK\x01\x02", true);
        let cd_len = z.len() - cd;
        z.extend(b"PK\x05\x06");
        z.extend([0; 4]);
        z.extend(1u16.to_le_bytes());
        z.extend(1u16.to_le_bytes());
        z.extend((cd_len as u32).to_le_bytes());
        z.extend((cd as u32).to_le_bytes());
        z.extend(0u16.to_le_bytes());
        z
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
    fn a_name_with_a_folder_picks_that_folders_member() {
        // A merged set keeps same-named chips of its variants in folders (`topskatrj/`, `topskatru/`).
        let entry = |n: &str| (n.to_string(), 0u16, 0usize, 0usize, 0u32, 0usize);
        let z = Zip {
            data: Vec::new(),
            entries: vec![
                entry("topskatrj/epr-1.15"),
                entry("topskatru/epr-1.15"),
                entry("Top.BIN"),
                entry("sub/top.bin"),
            ],
        };
        let at = |n: &str| z.entry(n).map(|e| e.0.as_str());
        assert_eq!(at("topskatru/epr-1.15"), Some("topskatru/epr-1.15"));
        assert_eq!(at("TOPSKATRJ/EPR-1.15"), Some("topskatrj/epr-1.15"), "case-insensitive");
        assert_eq!(at("other/epr-1.15"), None, "a folder that is not there is not another folder's member");
        // A bare name finds the top-level member before one in a folder, else the first of that name.
        assert_eq!(at("top.bin"), Some("Top.BIN"));
        assert_eq!(at("epr-1.15"), Some("topskatrj/epr-1.15"));
        assert!(z.has("topskatru/epr-1.15") && !z.has("nothing"));
    }
}
