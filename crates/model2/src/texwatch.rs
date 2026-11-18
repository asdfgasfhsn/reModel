//! Texture RAM upload watch (observation only; the catalogue's loading
//! side, docs/uplift-studio.md "Catalogue").
//!
//! The game fills texture RAM with the CPU (and, on some boards, through a
//! mirror), so there is no single upload call to hook, and a hook on every
//! write would take the JIT's fast path away. Instead the bus keeps a
//! version counter per RAM (`Mem::writes`, bumped by every write, fast
//! path included): at each vblank this watch looks at the counter and, only
//! when it moved, compares the texture banks with its shadow copy page by
//! page and logs the pages that changed as [`Load`]s. One counter compare
//! per frame when nothing was written; a 4 MB `memcmp` (about 0.3 ms) when
//! something was. It is not part of the machine's state: saving, loading,
//! hashing and run-ahead never see it.

use crate::enhance::Fnv;

/// The comparison granularity: bytes.
pub const PAGE: usize = 4096;
/// The part of render RAM watched: the two texture banks (luma tables, at
/// 0x400000, are not texels).
pub const WATCHED: usize = 0x40_0000;
/// Most loads kept when nobody drains them.
const KEEP: usize = 200_000;

/// A run of changed pages seen at one frame's end: texels the game wrote
/// since the frame before.
#[derive(Clone, Debug, PartialEq)]
pub struct Load {
    /// The frame that ended with the change.
    pub frame: u64,
    /// Offset in render RAM (bank 0 at 0, bank 1 at 0x200000) and length.
    pub offset: u32,
    pub len: u32,
    /// Hash of the new bytes.
    pub hash: u64,
    /// The first and a middle 64 bytes of the new data, to look for the
    /// source (ROM) it was copied from.
    pub head: Vec<u8>,
    pub mid: Vec<u8>,
}

/// The watch: its shadow of the texture banks and what it found.
#[derive(Default)]
pub struct TexWatch {
    shadow: Vec<u8>,
    writes: u64,
    last: u64,
    primed: bool,
    loads: Vec<Load>,
    resyncs: Vec<u64>,
    /// Frames that were scanned, and host nanoseconds that took.
    pub scans: u64,
    pub scan_ns: u64,
}

/// Hash of a byte run, a word at a time.
pub fn hash_bytes(b: &[u8]) -> u64 {
    let mut f = Fnv::default();
    let (words, rest) = b.as_chunks::<8>();
    for w in words {
        f.0 = (f.0 ^ u64::from_le_bytes(*w)).wrapping_mul(0x0100_0000_01b3).rotate_left(23);
    }
    f.bytes(rest);
    f.0
}

impl TexWatch {
    pub fn new() -> Self {
        Self::default()
    }

    /// Called as frame `frame` ends, with render RAM and the sum of the
    /// version counters of the RAMs that hold texels.
    pub fn frame_end(&mut self, frame: u64, ram: &[u8], writes: u64) {
        let n = WATCHED.min(ram.len());
        if !self.primed || frame != self.last + 1 || writes < self.writes {
            // The first frame, or a jump (a state loaded, a rewind): what
            // changed in between is unknown, so start from what is there.
            if self.primed {
                self.resyncs.push(frame);
            }
            self.shadow.clear();
            self.shadow.extend_from_slice(&ram[..n]);
            self.primed = true;
        } else if writes != self.writes {
            let t = std::time::Instant::now();
            self.scan(frame, &ram[..n]);
            self.scans += 1;
            self.scan_ns += t.elapsed().as_nanos() as u64;
        }
        self.writes = writes;
        self.last = frame;
    }

    fn scan(&mut self, frame: u64, ram: &[u8]) {
        let mut run: Option<usize> = None;
        let pages = ram.len().div_ceil(PAGE);
        for p in 0..=pages {
            let (a, b) = (p * PAGE, ((p + 1) * PAGE).min(ram.len()));
            let changed = p < pages && ram[a..b] != self.shadow[a..b];
            if changed {
                self.shadow[a..b].copy_from_slice(&ram[a..b]);
                run.get_or_insert(a);
            } else if let Some(start) = run.take() {
                let data = &ram[start..a];
                let mid = (data.len() / 2) & !63;
                self.loads.push(Load {
                    frame,
                    offset: start as u32,
                    len: data.len() as u32,
                    hash: hash_bytes(data),
                    head: data[..data.len().min(64)].to_vec(),
                    mid: data[mid..(mid + 64).min(data.len())].to_vec(),
                });
            }
        }
        if self.loads.len() > KEEP {
            self.loads.drain(..KEEP / 2);
        }
    }

    /// The loads and resync frames found since the last call.
    pub fn take(&mut self) -> (Vec<Load>, Vec<u64>) {
        (std::mem::take(&mut self.loads), std::mem::take(&mut self.resyncs))
    }
}

/// The render RAM byte offset of the 16-bit word holding texel (x, y) of
/// `bank`'s sheet (`raster::Sources::texel`'s addressing).
pub fn texel_offset(bank: u32, x: u32, y: u32) -> u32 {
    let (x, y) = if x >= 1024 { (x - 1024, y ^ 1024) } else { (x, y) };
    let word = ((y & 2047) / 2) * 512 + (x & 1023) / 2;
    (bank & 1) * 0x20_0000 + word * 2
}

/// The 4 KB pages of render RAM a texture rectangle occupies.
pub fn rect_pages(bank: u32, x0: u32, y0: u32, w: u32, h: u32) -> Vec<u32> {
    let mut pages = Vec::new();
    for y in (y0..y0 + h.max(1)).step_by(2) {
        for x in [x0, x0 + w.max(1) - 1] {
            let p = texel_offset(bank, x, y) / PAGE as u32;
            if !pages.contains(&p) {
                pages.push(p);
            }
        }
    }
    pages
}

/// The byte ranges `[start, end)` of render RAM a texture rectangle's
/// words occupy: one per pair of rows.
pub fn rect_ranges(bank: u32, x0: u32, y0: u32, w: u32, h: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    for y in (y0..y0 + h.max(1)).step_by(2) {
        let a = texel_offset(bank, x0, y);
        let b = texel_offset(bank, x0 + w.max(1) - 1, y);
        out.push((a.min(b), a.max(b) + 2));
    }
    out
}

/// Where in the sheet a load went, for a human: `bank0 rows 64-127`
/// (rows of the left half of the sheet, which a row pair of 1 KB spans).
pub fn dest_text(offset: u32, len: u32) -> String {
    let bank = offset / 0x20_0000;
    let o = offset % 0x20_0000;
    let last = (o + len.max(1) - 1).min(0x1f_ffff);
    if o >= 0x10_0000 {
        return format!("bank{bank} beyond the sheet (+0x{o:x})");
    }
    format!("bank{bank} rows {}-{}", o / 1024 * 2, last / 1024 * 2 + 1)
}

/// An index of 32-byte windows of a ROM image, to find where uploaded data
/// was copied from.
pub struct RomIndex {
    data: std::sync::Arc<[u8]>,
    windows: std::collections::HashMap<u64, u32>,
}

impl RomIndex {
    pub fn new(data: std::sync::Arc<[u8]>) -> Self {
        let mut windows = std::collections::HashMap::new();
        for off in (0..data.len().saturating_sub(31)).step_by(32) {
            let w = &data[off..off + 32];
            if w.iter().all(|&b| b == w[0]) {
                continue; // a fill matches everywhere
            }
            windows.entry(hash_bytes(w)).or_insert(off as u32);
        }
        RomIndex { data, windows }
    }

    /// The ROM offset `sample`'s first byte came from, if the sample is a
    /// verbatim copy of ROM data (found at any 4-byte shift).
    pub fn find(&self, sample: &[u8]) -> Option<u32> {
        if sample.len() < 32 {
            return None;
        }
        for s in (0..=sample.len() - 32).step_by(4) {
            let w = &sample[s..s + 32];
            if let Some(&off) = self.windows.get(&hash_bytes(w)) {
                if self.data[off as usize..off as usize + 32] == *w && off as usize >= s {
                    return Some(off - s as u32);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logs_changed_pages_as_runs_and_resyncs_on_a_jump() {
        let mut ram = vec![0u8; WATCHED];
        let mut w = TexWatch::new();
        w.frame_end(1, &ram, 0);
        // Nothing written: nothing scanned.
        w.frame_end(2, &ram, 0);
        assert_eq!(w.scans, 0);
        // Pages 1 and 2 change, and page 5.
        ram[PAGE + 7] = 9;
        ram[2 * PAGE] = 1;
        ram[5 * PAGE + 100] = 3;
        w.frame_end(3, &ram, 3);
        let (loads, resyncs) = w.take();
        assert!(resyncs.is_empty());
        assert_eq!(loads.len(), 2);
        assert_eq!((loads[0].offset as usize, loads[0].len as usize), (PAGE, 2 * PAGE));
        assert_eq!((loads[1].offset as usize, loads[1].len as usize), (5 * PAGE, PAGE));
        assert_eq!(loads[1].frame, 3);
        assert_ne!(loads[0].hash, loads[1].hash);
        // Unchanged again: no loads. A frame jump resyncs and logs nothing.
        w.frame_end(4, &ram, 3);
        ram[0] = 1;
        w.frame_end(10, &ram, 4);
        let (loads, resyncs) = w.take();
        assert!(loads.is_empty());
        assert_eq!(resyncs, vec![10]);
        // After the resync the new baseline holds.
        ram[PAGE * 3] = 1;
        w.frame_end(11, &ram, 5);
        assert_eq!(w.take().0.len(), 1);
    }

    #[test]
    fn rect_addressing_and_rom_index() {
        // Texel (0,0) of bank 1 is at its bank's start; the next row pair
        // is 1 KB on; a 32x16 texture at (64, 32) spans 8 row pairs.
        assert_eq!(texel_offset(1, 0, 0), 0x20_0000);
        assert_eq!(texel_offset(0, 0, 2), 1024);
        assert_eq!(rect_ranges(0, 64, 32, 32, 16).len(), 8);
        assert_eq!(rect_pages(0, 64, 32, 32, 16), vec![4, 5]);
        assert_eq!(dest_text(0x20_0000 + 4096, 4096), "bank1 rows 8-15");
        let rom: Vec<u8> = (0..4096u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
        let index = RomIndex::new(rom.clone().into());
        // A copy that starts off the window grid is still found.
        assert_eq!(index.find(&rom[1000..1064]), Some(1000));
        assert_eq!(index.find(&[7u8; 64]), None);
    }
}
