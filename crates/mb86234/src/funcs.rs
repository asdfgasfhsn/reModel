//! The function unit on the TGP's external bus: lookup ports in front of
//! the board's table ROM (on Model 2A the `CoproData` pair
//! `opr-14742a.45`/`.46`, 64K 32-bit words).
//!
//! What the ROM holds is confirmed numerically (`docs/hw/mb86234.md`): a
//! quarter-wave sine of 16,384 floats (65,536 steps a turn) at word 0, an
//! arctangent table at 0x4000, and pairs of floats (slope, intercept) of
//! linear approximations to `1/m` (8,192 segments of `[1, 2)`, at 0x8000)
//! and to `1/sqrt(m)` (4,096 segments each for even and odd exponents, at
//! 0xC000). How the ports index it and scale the result is **this model's
//! reconstruction**: it is what makes the program's reciprocal, inverse
//! square root and table rotations come out right (they use no other
//! correction), not something read from hardware.
//!
//! - write `0x20`, an angle (low 16 bits, 65,536 a turn); read `0x20` its
//!   sine and `0x21` its cosine;
//! - write `0x28`, a float `x`; read `0x28` the slope's magnitude and `0x29`
//!   the intercept, both scaled by the power of two of `1/x` (and the
//!   intercept signed as `x`), so that `intercept - m * |slope|` is `1/x`
//!   for `m` = `x` with its exponent set to 0x7F;
//! - write `0x2A`, a positive float; read `0x2A` and `0x2B` the same for
//!   `1/sqrt(x)`;
//! - write `0x24` x and `0x25` y (floats); then `0x27` the ratio of the
//!   smaller to the larger (y/x, or x/y when |x| < |y|, which the jump
//!   condition `A0` tells the program); read `0x27` atan2(y, x), 65,536 a
//!   turn, from the ROM's arctangent table (16,384 entries over [0, 1]).

/// Where the tables are in the ROM, in words.
const SINE: usize = 0x0000;
const RECIP: usize = 0x8000;
const RSQRT: usize = 0xc000;
const ATAN: usize = 0x4000;

#[derive(Clone, Default)]
pub struct Funcs {
    /// The ROM (not state: it comes with the board).
    pub rom: Vec<u32>,
    /// The words last written to the ports `0x20`, `0x28` and `0x2A`.
    pub angle: u32,
    pub recip: u32,
    pub rsqrt: u32,
    /// The arctangent's x, y and ratio (ports `0x24`, `0x25`, `0x27`).
    pub atan_x: u32,
    pub atan_y: u32,
    pub atan_t: u32,
}

impl Funcs {
    pub fn new(rom: Vec<u32>) -> Self {
        Funcs { rom, ..Funcs::default() }
    }

    fn rom(&self, i: usize) -> u32 {
        self.rom.get(i).copied().unwrap_or(0)
    }

    /// The sine of `angle` (65,536 a turn) from the quarter wave.
    pub fn sine(&self, angle: u32) -> u32 {
        let a = angle & 0xffff;
        let i = (a & 0x3fff) as usize;
        let quarter = |i: usize| if i == 0x4000 { 1.0f32.to_bits() } else { self.rom(SINE + i) };
        let v = match a >> 14 {
            0 => quarter(i),
            1 => quarter(0x4000 - i),
            2 => quarter(i) ^ 0x8000_0000,
            _ => quarter(0x4000 - i) ^ 0x8000_0000,
        };
        if v == 0x8000_0000 {
            0
        } else {
            v
        }
    }

    /// The (slope magnitude, intercept) of the linear approximation the
    /// ROM has at `base` for the segment `k`, scaled by `scale` and the
    /// intercept negated when `negative`.
    fn linear(&self, base: usize, k: usize, scale: f64, negative: bool) -> (u32, u32) {
        let slope = f32::from_bits(self.rom(base + 2 * k)) as f64;
        let icpt = f32::from_bits(self.rom(base + 2 * k + 1)) as f64;
        let i = icpt * scale;
        (crate::word(slope.abs() * scale), crate::word(if negative { -i } else { i }))
    }

    fn recip_pair(&self) -> (u32, u32) {
        let x = self.recip;
        let e = ((x >> 23) & 0xff) as i32;
        let k = ((x & 0x7f_ffff) >> 10) as usize;
        self.linear(RECIP, k, 2f64.powi(127 - e), x >> 31 != 0)
    }

    fn rsqrt_pair(&self) -> (u32, u32) {
        let x = self.rsqrt;
        let e = ((x >> 23) & 0xff) as i32 - 127;
        let odd = e & 1;
        let k = (((x & 0x7f_ffff) >> 11) | ((odd as u32) << 12)) as usize;
        // Even exponents use 1/sqrt(m); odd ones sqrt(2)/sqrt(m), for 2m.
        let scale = if odd == 0 { 2f64.powf(-(e as f64) / 2.0) } else { 2f64.powf(-((e - 1) as f64) / 2.0 - 1.0) };
        self.linear(RSQRT, k, scale, false)
    }

    /// Is |x| < |y| for the arctangent (the jump condition `A0`)?
    pub fn atan_steep(&self) -> bool {
        crate::f(self.atan_x).abs() < crate::f(self.atan_y).abs()
    }

    /// atan2(y, x) in 65,536ths of a turn (signed), the octant's angle
    /// looked up for the ratio written to `0x27`.
    pub fn atan(&self) -> u32 {
        let (x, y) = (f32::from_bits(self.atan_x), f32::from_bits(self.atan_y));
        let t = f32::from_bits(self.atan_t).abs().min(1.0);
        let i = ((t * 16384.0).round() as usize).min(0x3fff);
        let n = (self.rom(ATAN + i) & 0xffff) as i32;
        let a = match (self.atan_steep(), x < 0.0) {
            (false, false) => n,
            (false, true) => 0x8000 - n,
            (true, false) => 0x4000 - n,
            (true, true) => 0x4000 + n,
        };
        let a = if y < 0.0 { -a } else { a };
        (a as i16) as i32 as u32
    }

    /// A read of the external bus at `a`, if a port is there.
    pub fn read(&self, a: u32) -> Option<u32> {
        Some(match a {
            0x20 => self.sine(self.angle),
            0x21 => self.sine(self.angle.wrapping_add(0x4000)),
            // The sine a half turn on, -sin (*hypothesis*: `daytona` reads
            // it where a rotation needs the negated sine;
            // docs/hw/mb86234.md).
            0x22 => self.sine(self.angle.wrapping_add(0x8000)),
            0x28 => self.recip_pair().0,
            0x29 => self.recip_pair().1,
            0x2a => self.rsqrt_pair().0,
            0x2b => self.rsqrt_pair().1,
            0x27 => self.atan(),
            _ => return None,
        })
    }

    /// A write; false if no port is there.
    pub fn write(&mut self, a: u32, v: u32) -> bool {
        match a {
            0x20 => self.angle = v,
            0x28 => self.recip = v,
            0x2a => self.rsqrt = v,
            0x24 => self.atan_x = v,
            0x25 => self.atan_y = v,
            0x27 => self.atan_t = v,
            _ => return false,
        }
        true
    }
}

state::fields!(Funcs { angle, recip, rsqrt, atan_x, atan_y, atan_t });
