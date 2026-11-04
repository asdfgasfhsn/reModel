//! The SCSP's effect DSP: 128 steps per sample over a ring buffer in sound
//! RAM (SCSP manual p04_14, p05_10; field layout in `docs/hw/sound.md` §2.9).
//!
//! Choices where the sources differ or are silent:
//! - NOFL is bit 8 of the low word (Mednafen); MAME reads bit 15.
//! - Memory reads and writes are allowed on every step; MAME performs them
//!   only on odd steps.
//! - A memory read lands in a latch that IWT copies to MEMS on any later
//!   step (MAME's model); Mednafen delays it by two steps.

/// Register block layout, byte offsets in the SCSP register space.
const COEF: u32 = 0x700;
const MADRS: u32 = 0x780;
const MPRO: u32 = 0x800;
const TEMP: u32 = 0xc00;
const MEMS: u32 = 0xe00;
const MIXS: u32 = 0xe80;
const EFREG: u32 = 0xec0;
const EXTS: u32 = 0xee0;

fn sext(v: i32, bits: u32) -> i32 {
    v << (32 - bits) >> (32 - bits)
}

/// 24-bit value to the DSP's 16-bit memory float: sign, 4-bit count of
/// leading sign-equal bits (0-12), 11 mantissa bits.
pub fn pack(v: i32) -> u16 {
    let s = (v >> 23) & 1;
    let mut e = 0;
    while e < 12 && (v >> (22 - e)) & 1 == s {
        e += 1;
    }
    let m = if e < 12 { (v << e) >> 11 & 0x7ff } else { v & 0x7ff };
    (s << 15 | e << 11 | m) as u16
}

/// Inverse of [`pack`]: back to a sign-extended 24-bit value.
pub fn unpack(w: u16) -> i32 {
    let s = (w >> 15) as i32 & 1;
    let e = (w >> 11 & 0xf) as i32;
    let m = (w & 0x7ff) as i32;
    if e < 12 {
        sext(s << 23 | (s ^ 1) << 22 | m << 11, 24) >> e
    } else {
        sext(s << 23 | s << 22 | m << 11, 24) >> 11
    }
}

pub struct Dsp {
    pub coef: [u16; 64],
    pub madrs: [u16; 32],
    pub mpro: [u64; 128],
    temp: [i32; 128],
    mems: [i32; 32],
    mixs: [i32; 16],
    /// Effect outputs for this sample, to the EFSDL/EFPAN of slots 0-15.
    pub efreg: [i32; 16],
    exts: [i16; 2],
    /// Steps up to and including the last non-zero one.
    len: usize,
    acc: i32,
    y_reg: i32,
    frc_reg: i32,
    adrs_reg: i32,
    memval: i32,
    dec: u32,
}

impl Default for Dsp {
    fn default() -> Self {
        Self::new()
    }
}

impl Dsp {
    pub fn new() -> Self {
        Dsp {
            coef: [0; 64],
            madrs: [0; 32],
            mpro: [0; 128],
            temp: [0; 128],
            mems: [0; 32],
            mixs: [0; 16],
            efreg: [0; 16],
            exts: [0; 2],
            len: 0,
            acc: 0,
            y_reg: 0,
            frc_reg: 0,
            adrs_reg: 0,
            memval: 0,
            dec: 0,
        }
    }

    /// True once a program has been loaded.
    pub fn enabled(&self) -> bool {
        self.len != 0
    }

    /// Steps up to and including the last non-zero instruction.
    pub fn program_len(&self) -> usize {
        self.len
    }

    pub fn clear_inputs(&mut self) {
        self.mixs = [0; 16];
    }

    /// Add a slot's 20-bit send to input mixer bus `n`.
    pub fn mix(&mut self, n: usize, v: i32) {
        self.mixs[n] += v;
    }

    pub fn read(&self, addr: u32) -> u16 {
        let w24 = |v: i32, a: u32| if a & 2 == 0 { v as u16 & 0xff } else { (v >> 8) as u16 };
        match addr {
            COEF..MADRS => self.coef[((addr - COEF) >> 1) as usize],
            MADRS..0x7c0 => self.madrs[((addr - MADRS) >> 1) as usize],
            MPRO..TEMP => {
                let i = ((addr - MPRO) >> 3) as usize;
                (self.mpro[i] >> (48 - 16 * ((addr >> 1) & 3))) as u16
            }
            TEMP..MEMS => w24(self.temp[((addr - TEMP) >> 2) as usize], addr),
            MEMS..MIXS => w24(self.mems[((addr - MEMS) >> 2) as usize], addr),
            MIXS..EFREG => {
                let v = self.mixs[((addr - MIXS) >> 2) as usize];
                if addr & 2 == 0 {
                    v as u16 & 0xf
                } else {
                    (v >> 4) as u16
                }
            }
            EFREG..EXTS => self.efreg[((addr - EFREG) >> 1) as usize].clamp(-32768, 32767) as u16,
            EXTS..0xee4 => self.exts[((addr - EXTS) >> 1) as usize] as u16,
            _ => 0,
        }
    }

    pub fn write(&mut self, addr: u32, v: u16, mask: u16) {
        let merge = |old: u16| old & !mask | v & mask;
        let w24 = |old: i32, a: u32| {
            if a & 2 == 0 {
                sext(old & !0xff | (merge(old as u16) & 0xff) as i32, 24)
            } else {
                sext(old & 0xff | (merge((old >> 8) as u16) as i32) << 8, 24)
            }
        };
        match addr {
            COEF..MADRS => {
                let i = ((addr - COEF) >> 1) as usize;
                self.coef[i] = merge(self.coef[i]);
            }
            MADRS..0x7c0 => {
                let i = ((addr - MADRS) >> 1) as usize;
                self.madrs[i] = merge(self.madrs[i]);
            }
            MPRO..TEMP => {
                let i = ((addr - MPRO) >> 3) as usize;
                let sh = 48 - 16 * ((addr >> 1) & 3);
                let w = merge((self.mpro[i] >> sh) as u16);
                self.mpro[i] = self.mpro[i] & !(0xffff << sh) | (w as u64) << sh;
                self.len = self.mpro.iter().rposition(|&x| x != 0).map_or(0, |p| p + 1);
            }
            TEMP..MEMS => {
                let i = ((addr - TEMP) >> 2) as usize;
                self.temp[i] = w24(self.temp[i], addr);
            }
            MEMS..MIXS => {
                let i = ((addr - MEMS) >> 2) as usize;
                self.mems[i] = w24(self.mems[i], addr);
            }
            EXTS..0xee4 => {
                let i = ((addr - EXTS) >> 1) as usize;
                self.exts[i] = merge(self.exts[i] as u16) as i16;
            }
            _ => {}
        }
    }

    /// Run the program for one sample. `rbp` is the ring base in words and
    /// `rbl` the ring length code (8K << rbl words).
    pub fn run(&mut self, ram: &mut [u8], rbp: u32, rbl: u32) {
        self.efreg = [0; 16];
        let ring_mask = (0x2000u32 << rbl) - 1;
        let ram_words = (ram.len() / 2) as u32;
        for step in 0..self.len {
            let op = self.mpro[step];
            let f = |hi: u32, n: u32| (op >> hi) as u32 & ((1 << n) - 1);
            let (tra, twt, twa) = (f(56, 7), f(55, 1) != 0, f(48, 7));
            let (xsel, ysel, ira) = (f(47, 1) != 0, f(45, 2), f(38, 6));
            let (iwt, iwa) = (f(37, 1) != 0, f(32, 5) as usize);
            let (table, mwt, mrd, ewt, ewa) = (f(31, 1) != 0, f(30, 1) != 0, f(29, 1) != 0, f(28, 1) != 0, f(24, 4));
            let (adrl, frcl, shift, yrl) = (f(23, 1) != 0, f(22, 1) != 0, f(20, 2), f(19, 1) != 0);
            let (negb, zero, bsel) = (f(18, 1) != 0, f(17, 1) != 0, f(16, 1) != 0);
            let (cra, nofl, masa) = (f(9, 6) as usize, f(8, 1) != 0, f(2, 5) as usize);
            let (adreb, nxadr) = (f(1, 1) != 0, f(0, 1) != 0);

            let mut inputs = match ira {
                0x00..=0x1f => self.mems[ira as usize],
                0x20..=0x2f => sext(self.mixs[ira as usize - 0x20] << 4, 24),
                0x30 | 0x31 => (self.exts[ira as usize - 0x30] as i32) << 8,
                _ => 0,
            };
            if iwt {
                self.mems[iwa] = self.memval;
                if ira as usize == iwa {
                    inputs = self.memval;
                }
            }
            let temp_r = self.temp[((tra + self.dec) & 0x7f) as usize];
            let b = if zero {
                0
            } else {
                let b = if bsel { sext(self.acc, 24) } else { temp_r };
                if negb {
                    -b
                } else {
                    b
                }
            };
            let x = if xsel { inputs } else { temp_r };
            let y = match ysel {
                0 => self.frc_reg,
                1 => sext((self.coef[cra] >> 3) as i32, 13),
                2 => sext(self.y_reg >> 11 & 0x1fff, 13),
                _ => sext(self.y_reg >> 4 & 0xfff, 13),
            };
            if yrl {
                self.y_reg = inputs;
            }
            let shifted = match shift {
                0 => self.acc.clamp(-0x80_0000, 0x7f_ffff),
                1 => (self.acc * 2).clamp(-0x80_0000, 0x7f_ffff),
                2 => sext(self.acc * 2, 24),
                _ => sext(self.acc, 24),
            };
            self.acc = sext((((x as i64) * (y as i64)) >> 12) as i32 + b, 26);
            if twt {
                self.temp[((twa + self.dec) & 0x7f) as usize] = shifted;
            }
            if frcl {
                self.frc_reg = if shift == 3 { shifted & 0xfff } else { shifted >> 11 & 0x1fff };
            }
            if mrd || mwt {
                let mut a = self.madrs[masa] as u32;
                if !table {
                    a = a.wrapping_add(self.dec);
                }
                if adreb {
                    a = a.wrapping_add(self.adrs_reg as u32 & 0xfff);
                }
                if nxadr {
                    a = a.wrapping_add(1);
                }
                a &= if table { 0xffff } else { ring_mask };
                let i = ((a + rbp) % ram_words) as usize * 2;
                if mrd {
                    let w = u16::from_be_bytes([ram[i], ram[i + 1]]);
                    self.memval = if nofl { (w as i16 as i32) << 8 } else { unpack(w) };
                }
                if mwt {
                    let w = if nofl { (shifted >> 8) as u16 } else { pack(shifted) };
                    ram[i..i + 2].copy_from_slice(&w.to_be_bytes());
                }
            }
            if adrl {
                self.adrs_reg = if shift == 3 { shifted >> 12 & 0xfff } else { inputs >> 16 };
            }
            if ewt {
                self.efreg[ewa as usize] += shifted >> 8;
            }
        }
        self.dec = self.dec.wrapping_sub(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn float_round_trip() {
        for v in [0, 1, -1, 0x7f_ffff, -0x80_0000, 0x12_3456, -0x12_3456, 0x7ff, -0x800, 0x40_0000] {
            let u = unpack(pack(v));
            // Precision: 12 significant bits.
            let err = (u - v).abs();
            assert!(err <= (v.abs() >> 11).max(0), "{v:#x} -> {:#06x} -> {u:#x}", pack(v));
        }
        assert_eq!(unpack(pack(0x7ff)), 0x7ff);
        assert_eq!(unpack(pack(-5)), -5);
    }

    #[test]
    fn passes_input_to_efreg() {
        // One step: X = MIXS[0], Y = COEF[0] = 0.5, B = 0, then a second
        // step writes the shifter (the first step's ACC) to EFREG[3].
        let mut d = Dsp::new();
        d.write(COEF, 0x0800 << 3, 0xffff);
        let s0: u64 = 1 << 47 | 1 << 45 | 0x20 << 38 | 1 << 17;
        let s1: u64 = 1 << 28 | 3 << 24 | 1 << 17;
        for (i, op) in [s0, s1].iter().enumerate() {
            for w in 0..4u32 {
                d.write(MPRO + 8 * i as u32 + 2 * w, (op >> (48 - 16 * w)) as u16, 0xffff);
            }
        }
        assert!(d.enabled());
        let mut ram = vec![0u8; 0x80000];
        d.mix(0, 0x1000 << 4);
        d.run(&mut ram, 0, 0);
        // MIXS << 4 = 0x100000, × 0.5 = 0x80000, >> 8 = 0x800.
        assert_eq!(d.efreg[3], 0x800);
    }
}
