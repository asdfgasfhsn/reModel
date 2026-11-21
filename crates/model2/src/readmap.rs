//! ROM read maps: which words of the polygon ROM and the texture ROM a run
//! has read (docs/uplift-studio.md, "Coverage").
//!
//! A [`ReadMap`] is a side bitmap, one bit per ROM word, set where the
//! geometrizer reads the polygon ROM (`geo::GeoExt::read`, the ROM half of
//! its bus, so models drawn from polygon RAM are never counted) and where
//! the rasteriser reads texture headers and UV lists
//! (`raster::BuildIn::tex_map`). It is off unless a front end attaches one,
//! and it is never part of the machine's state: nothing reads it back, so
//! emulation, state hashes and save states are the same with it on or off.
//!
//! [`Ranges`] is the stored form: sorted, merged, half-open word ranges,
//! with the set operations a report needs. Maps are merged across runs by
//! union; counts are in bytes.

use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

/// One bit per ROM word, set by any thread.
pub struct ReadMap {
    words: usize,
    bits: Vec<AtomicU64>,
}

impl ReadMap {
    /// A map for a ROM of `words` words, nothing read.
    pub fn new(words: usize) -> Self {
        ReadMap { words, bits: (0..words.div_ceil(64)).map(|_| AtomicU64::new(0)).collect() }
    }

    /// The ROM's size in words.
    pub fn len(&self) -> usize {
        self.words
    }

    pub fn is_empty(&self) -> bool {
        self.words == 0
    }

    /// Word `i` was read (ignored past the end of the ROM).
    #[inline]
    pub fn mark(&self, i: usize) {
        if let Some(b) = self.bits.get(i >> 6).filter(|_| i < self.words) {
            let bit = 1u64 << (i & 63);
            // Most reads hit words already marked: a plain load first
            // keeps the hot path off the atomic read-modify-write.
            if b.load(Relaxed) & bit == 0 {
                b.fetch_or(bit, Relaxed);
            }
        }
    }

    /// Words `start..end` were read.
    pub fn mark_range(&self, start: usize, end: usize) {
        for i in start..end.min(self.words) {
            self.mark(i);
        }
    }

    /// Whether word `i` has been read.
    pub fn get(&self, i: usize) -> bool {
        self.bits.get(i >> 6).is_some_and(|b| b.load(Relaxed) >> (i & 63) & 1 != 0)
    }

    /// Everything read so far, as merged ranges.
    pub fn ranges(&self) -> Ranges {
        let mut out = Vec::new();
        let mut i = 0;
        while i < self.words {
            let b = self.bits[i >> 6].load(Relaxed);
            if i & 63 == 0 && b == 0 {
                i += 64;
                continue;
            }
            if b >> (i & 63) & 1 == 0 {
                i += 1;
                continue;
            }
            let s = i;
            while i < self.words && self.bits[i >> 6].load(Relaxed) >> (i & 63) & 1 != 0 {
                i += 1;
            }
            out.push((s as u64, i as u64));
        }
        Ranges(out)
    }

    /// Add `r` to the map (a map merged from earlier runs).
    pub fn add(&self, r: &Ranges) {
        for &(s, e) in &r.0 {
            self.mark_range(s as usize, e as usize);
        }
    }
}

/// Sorted, non-overlapping, non-adjacent half-open ranges of word indices.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ranges(pub Vec<(u64, u64)>);

impl Ranges {
    /// Normalise any list of ranges: drop empty ones, sort, merge touching
    /// and overlapping ones.
    pub fn from_unsorted(mut v: Vec<(u64, u64)>) -> Self {
        v.retain(|&(s, e)| e > s);
        v.sort_unstable();
        let mut out: Vec<(u64, u64)> = Vec::with_capacity(v.len());
        for (s, e) in v {
            match out.last_mut() {
                Some(last) if s <= last.1 => last.1 = last.1.max(e),
                _ => out.push((s, e)),
            }
        }
        Ranges(out)
    }

    /// Everything in `self` or `other`.
    pub fn union(&self, other: &Ranges) -> Ranges {
        Ranges::from_unsorted(self.0.iter().chain(&other.0).copied().collect())
    }

    /// Everything in both.
    pub fn intersect(&self, other: &Ranges) -> Ranges {
        let (mut i, mut j, mut out) = (0, 0, Vec::new());
        while i < self.0.len() && j < other.0.len() {
            let (a, b) = (self.0[i], other.0[j]);
            let (s, e) = (a.0.max(b.0), a.1.min(b.1));
            if e > s {
                out.push((s, e));
            }
            if a.1 < b.1 {
                i += 1;
            } else {
                j += 1;
            }
        }
        Ranges(out)
    }

    /// Everything in `0..len` that is not in `self`.
    pub fn complement(&self, len: u64) -> Ranges {
        let (mut at, mut out) = (0, Vec::new());
        for &(s, e) in &self.0 {
            if s > at {
                out.push((at, s.min(len)));
            }
            at = at.max(e);
        }
        if at < len {
            out.push((at, len));
        }
        out.retain(|&(s, e)| e > s);
        Ranges(out)
    }

    /// Words in the ranges.
    pub fn count(&self) -> u64 {
        self.0.iter().map(|&(s, e)| e - s).sum()
    }

    /// The ranges clipped to `0..len`.
    pub fn clip(&self, len: u64) -> Ranges {
        Ranges(self.0.iter().filter(|r| r.0 < len).map(|&(s, e)| (s, e.min(len))).collect())
    }

    /// Runs of at least `min_words` identical-valued words that are all
    /// zero or all ones, in `rom` (what a report calls blank).
    pub fn blank<T: Copy + PartialEq + Default + std::ops::Not<Output = T>>(rom: &[T], min_words: usize) -> Ranges {
        let (zero, ones) = (T::default(), !T::default());
        let mut out = Vec::new();
        let mut i = 0;
        while i < rom.len() {
            let v = rom[i];
            if v != zero && v != ones {
                i += 1;
                continue;
            }
            let s = i;
            while i < rom.len() && rom[i] == v {
                i += 1;
            }
            if i - s >= min_words {
                out.push((s as u64, i as u64));
            }
        }
        Ranges(out)
    }

    /// The `start end` lines of the stored form, prefixed with `tag`.
    pub fn write(&self, tag: &str, out: &mut String) {
        for &(s, e) in &self.0 {
            out.push_str(&format!("{tag} {s} {e}\n"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_map_gives_back_merged_ranges() {
        let m = ReadMap::new(300);
        for i in [0, 1, 2, 5, 63, 64, 65, 200] {
            m.mark(i);
        }
        m.mark(300); // past the end: ignored
        m.mark_range(10, 13);
        assert_eq!(m.ranges(), Ranges(vec![(0, 3), (5, 6), (10, 13), (63, 66), (200, 201)]));
        assert!(m.get(64) && !m.get(66) && !m.get(300));
    }

    #[test]
    fn ranges_merge_by_union_and_intersect_and_complement() {
        let a = Ranges::from_unsorted(vec![(10, 20), (0, 5), (5, 8), (30, 30)]);
        assert_eq!(a, Ranges(vec![(0, 8), (10, 20)]));
        let b = Ranges(vec![(6, 12), (18, 40)]);
        assert_eq!(a.union(&b), Ranges(vec![(0, 40)]));
        assert_eq!(a.intersect(&b), Ranges(vec![(6, 8), (10, 12), (18, 20)]));
        assert_eq!(a.complement(25), Ranges(vec![(8, 10), (20, 25)]));
        assert_eq!(a.complement(15), Ranges(vec![(8, 10)]));
        assert_eq!(a.count(), 18);
        assert_eq!(a.clip(12), Ranges(vec![(0, 8), (10, 12)]));
        assert_eq!(a.union(&a), a);
        assert_eq!(a.union(&Ranges::default()), a);
    }

    /// A map merged from earlier runs, and the same reads again, are the
    /// same map.
    #[test]
    fn maps_merge_across_runs() {
        let first = ReadMap::new(1000);
        first.mark_range(0, 100);
        first.mark_range(500, 600);
        let second = ReadMap::new(1000);
        second.mark_range(50, 150);
        second.add(&first.ranges());
        second.add(&first.ranges());
        assert_eq!(second.ranges(), Ranges(vec![(0, 150), (500, 600)]));
    }

    #[test]
    fn blank_runs_need_a_length() {
        let mut rom = vec![7u32; 100];
        rom[10..30].fill(0);
        rom[50..52].fill(0);
        rom[60..90].fill(u32::MAX);
        assert_eq!(Ranges::blank(&rom, 16), Ranges(vec![(10, 30), (60, 90)]));
        let tex = vec![0u16; 40];
        assert_eq!(Ranges::blank(&tex, 16), Ranges(vec![(0, 40)]));
    }
}
