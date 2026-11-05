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

    /// A member's contents by name (case-insensitive, ignoring folders in
    /// the archive), checked against its CRC-32.
    pub fn read(&self, name: &str) -> Option<Result<Vec<u8>, String>> {
        let base = |n: &str| n.rsplit('/').next().unwrap_or(n).to_ascii_lowercase();
        let want = name.to_ascii_lowercase();
        let &(ref stored, method, packed, size, crc, local) = self.entries.iter().find(|e| base(&e.0) == want)?;
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
}
