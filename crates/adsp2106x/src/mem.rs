//! The ADSP-21062's internal memory (UM §5.2.2, §5.3): two blocks of
//! sixteen 16-bit columns, 4K rows each, seen as 48-bit words (three
//! columns, five groups), 32-bit words (two columns, eight groups) or
//! 16-bit short words. Modelling the columns rather than separate 48- and
//! 32-bit arrays makes mixed accesses alias exactly as the manual's
//! Figure 5.9a shows (a 32-bit read of a 48-bit word sees its upper 32
//! bits, for instance).

/// Rows per column on the ADSP-21062.
pub const ROWS: usize = 4096;
const COLS: usize = 16;

/// Normal-word addresses of the two blocks (UM Table 5.2a).
pub const BLOCK0: u32 = 0x2_0000;
pub const BLOCK1: u32 = 0x2_8000;

/// Number of 48-bit words in each block.
pub const WORDS48: u32 = 5 * ROWS as u32;

#[derive(Clone)]
pub struct Internal {
    /// [block][column * ROWS + row].
    pub cols: [Vec<u16>; 2],
}

impl Default for Internal {
    fn default() -> Self {
        Internal { cols: [vec![0; COLS * ROWS], vec![0; COLS * ROWS]] }
    }
}

/// Location of a normal-word address: block, group (address bits 14-12)
/// and row (bits 11-0). Addresses 0x30000-0x3FFFF alias block 1.
#[inline]
pub fn locate(addr: u32) -> Option<(usize, usize, usize)> {
    if !(0x2_0000..0x4_0000).contains(&addr) {
        return None;
    }
    let block = (addr >= BLOCK1) as usize;
    let off = (addr & 0x7fff) as usize;
    Some((block, off >> 12, off & (ROWS - 1)))
}

/// The columns holding the high, middle and low 16 bits of a 48-bit word
/// in group `g`: H M L in even groups, L H M in odd ones (Figure 5.9a).
#[inline]
fn cols48(g: usize) -> [usize; 3] {
    let c = 3 * g;
    if g.is_multiple_of(2) {
        [c, c + 1, c + 2]
    } else {
        [c + 1, c + 2, c]
    }
}

impl Internal {
    #[inline]
    fn get(&self, b: usize, col: usize, row: usize) -> u64 {
        self.cols[b][col * ROWS + row] as u64
    }

    #[inline]
    fn set(&mut self, b: usize, col: usize, row: usize, v: u64) {
        self.cols[b][col * ROWS + row] = v as u16;
    }

    /// A 48-bit word; groups 5-7 hold none (the "non-existent" addresses
    /// at the end of a block of instructions) and read as zero.
    #[inline]
    pub fn read48(&self, addr: u32) -> u64 {
        match locate(addr) {
            Some((b, g, r)) if g < 5 => {
                let [h, m, l] = cols48(g);
                self.get(b, h, r) << 32 | self.get(b, m, r) << 16 | self.get(b, l, r)
            }
            _ => 0,
        }
    }

    /// Returns false if the address holds no 48-bit word.
    #[inline]
    pub fn write48(&mut self, addr: u32, v: u64) -> bool {
        match locate(addr) {
            Some((b, g, r)) if g < 5 => {
                let [h, m, l] = cols48(g);
                self.set(b, h, r, v >> 32);
                self.set(b, m, r, v >> 16);
                self.set(b, l, r, v);
                true
            }
            _ => false,
        }
    }

    #[inline]
    pub fn read32(&self, addr: u32) -> u32 {
        match locate(addr) {
            Some((b, g, r)) => (self.get(b, 2 * g, r) << 16 | self.get(b, 2 * g + 1, r)) as u32,
            None => 0,
        }
    }

    #[inline]
    pub fn write32(&mut self, addr: u32, v: u32) {
        if let Some((b, g, r)) = locate(addr) {
            self.set(b, 2 * g, r, (v >> 16) as u64);
            self.set(b, 2 * g + 1, r, v as u64);
        }
    }

    /// A short word (0x40000-0x7FFFF): the normal-word address is the
    /// short address shifted right; bit 0 picks the MSW (1) or LSW (0) of
    /// the 32-bit word (UM §5.3.4).
    #[inline]
    pub fn read16(&self, saddr: u32) -> u16 {
        match locate(short_to_normal(saddr)) {
            Some((b, g, r)) => self.get(b, 2 * g + (saddr & 1 == 0) as usize, r) as u16,
            None => 0,
        }
    }

    #[inline]
    pub fn write16(&mut self, saddr: u32, v: u16) {
        if let Some((b, g, r)) = locate(short_to_normal(saddr)) {
            self.set(b, 2 * g + (saddr & 1 == 0) as usize, r, v as u64);
        }
    }

    /// The 48-bit word addresses whose contents a write to `addr` of the
    /// given width can change, for instruction cache invalidation.
    #[inline]
    pub fn words48_touched(addr: u32, width48: bool) -> [Option<u32>; 2] {
        let Some((b, g, r)) = locate(addr) else { return [None, None] };
        let base = if b == 0 { BLOCK0 } else { BLOCK1 };
        let at = |g48: usize| (g48 < 5).then(|| base + (g48 * ROWS + r) as u32);
        if width48 {
            [at(g), None]
        } else {
            // Columns 2g and 2g+1 belong to 48-bit groups (2g)/3 and (2g+1)/3.
            let (a, c) = ((2 * g) / 3, (2 * g + 1) / 3);
            [at(a), if c != a { at(c) } else { None }]
        }
    }
}

/// Normal-word address of a short-word address (0x40000 → 0x20000).
#[inline]
pub fn short_to_normal(saddr: u32) -> u32 {
    let n = saddr >> 1;
    // Block 1 and its aliases: 0x50000-0x7FFFF map to 0x28000-0x3FFFF.
    if (0x2_0000..0x4_0000).contains(&n) {
        n
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_sizes_alias_as_in_figure_5_9a() {
        let mut m = Internal::default();
        // A 48-bit word in group 0 occupies columns 0-2 (H M L).
        assert!(m.write48(0x2_0005, 0x1234_5678_9abc));
        // Its upper 32 bits are the 32-bit word at the same address.
        assert_eq!(m.read32(0x2_0005), 0x1234_5678);
        // Group 1 is L H M: the 32-bit word in columns 2-3 is the low
        // halves of groups 0 and 1.
        assert!(m.write48(0x2_1005, 0xaaaa_bbbb_cccc));
        assert_eq!(m.read32(0x2_1005), 0x9abc_cccc);
        assert_eq!(m.read32(0x2_2005), 0xaaaa_bbbb);
        assert_eq!(m.read48(0x2_1005), 0xaaaa_bbbb_cccc);
        // Short words: LSW at the even address, MSW at the odd one.
        assert_eq!(m.read16(0x4_000a), 0x5678);
        assert_eq!(m.read16(0x4_000b), 0x1234);
        m.write16(0x4_000a, 0xbeef);
        assert_eq!(m.read48(0x2_0005), 0x1234_beef_9abc);
    }

    #[test]
    fn block_1_and_its_aliases() {
        let mut m = Internal::default();
        m.write32(0x2_8010, 7);
        assert_eq!(m.read32(0x3_0010), 7);
        assert_eq!(m.read32(0x3_8010), 7);
        assert_eq!(m.read16(0x5_0020), 7);
        assert_eq!(m.read16(0x7_0020), 7);
        assert_eq!(m.read32(0x2_0010), 0);
        // Groups 5-7 hold no 48-bit words.
        assert!(!m.write48(0x2_5000, 1));
        assert_eq!(m.read48(0x2_5000), 0);
    }

    #[test]
    fn cache_invalidation_covers_shared_columns() {
        assert_eq!(Internal::words48_touched(0x2_1003, false), [Some(0x2_0003), Some(0x2_1003)]);
        assert_eq!(Internal::words48_touched(0x2_0003, false), [Some(0x2_0003), None]);
        // Columns 14-15: column 14 is group 4's, column 15 is unused.
        assert_eq!(Internal::words48_touched(0x2_7003, false), [Some(0x2_4003), None]);
    }
}
