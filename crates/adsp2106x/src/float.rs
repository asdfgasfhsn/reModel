//! Floating point as the ALU and multiplier do it (UM §2.2, §2.4,
//! Appendix C): the 40-bit extended format (IEEE single precision with
//! eight more fraction bits, in register bits 39-0), denormal inputs
//! flushed to zero, denormal results flushed to zero with underflow,
//! rounding to nearest even or toward zero (MODE1 TRUNC), to a 32-bit or
//! 40-bit boundary (MODE1 RND32). NaN results are "all 1s".
//!
//! Arithmetic is exact integer arithmetic on the significands followed by
//! one rounding, so results are correctly rounded in both formats.

/// A 40-bit register value with all bits set: the NaN result.
pub const NAN: u64 = 0xff_ffff_ffff;
const MASK40: u64 = 0xff_ffff_ffff;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Mode {
    /// MODE1 TRUNC: round toward zero instead of to nearest.
    pub trunc: bool,
    /// MODE1 RND32: inputs lose their 8 LSBs, results are 32-bit IEEE.
    pub rnd32: bool,
}

impl Mode {
    fn frac_bits(self) -> u32 {
        if self.rnd32 {
            23
        } else {
            31
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Zero,
    Normal,
    Inf,
    Nan,
}

/// An unpacked value: for `Normal`, `sig` (hidden bit at bit 31) times
/// 2^(`exp` - 31).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unpacked {
    pub sign: bool,
    pub class: Class,
    pub exp: i32,
    pub sig: u64,
}

impl Unpacked {
    fn zero(sign: bool) -> Self {
        Unpacked { sign, class: Class::Zero, exp: 0, sig: 0 }
    }

    pub fn to_f64(self) -> f64 {
        let v = match self.class {
            Class::Zero => 0.0,
            Class::Normal => self.sig as f64 * 2f64.powi(self.exp - 31),
            Class::Inf => f64::INFINITY,
            Class::Nan => f64::NAN,
        };
        if self.sign {
            -v
        } else {
            v
        }
    }
}

/// Unpack a 40-bit register value; denormals are zeros (UM §2.2).
pub fn unpack(v: u64) -> Unpacked {
    let v = v & MASK40;
    let sign = v >> 39 != 0;
    let e = ((v >> 31) & 0xff) as i32;
    let frac = v & 0x7fff_ffff;
    match e {
        0 => Unpacked::zero(sign),
        255 if frac == 0 => Unpacked { sign, class: Class::Inf, exp: 128, sig: 0 },
        255 => Unpacked { sign, class: Class::Nan, exp: 128, sig: frac },
        _ => Unpacked { sign, class: Class::Normal, exp: e - 127, sig: 1 << 31 | frac },
    }
}

/// An operand as the ALU or multiplier sees it: with RND32 its eight LSBs
/// are zeros (UM §2.5.2.3, §2.6.4.2).
pub fn input(v: u64, m: Mode) -> Unpacked {
    unpack(if m.rnd32 { v & !0xff } else { v })
}

/// Status of a floating-point result, in ALU terms; the multiplier maps
/// them onto MN/MV/MU/MI.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    /// AZ: zero, or underflowed to zero.
    pub zero: bool,
    pub underflow: bool,
    pub overflow: bool,
    pub neg: bool,
    pub invalid: bool,
}

pub fn pack_zero(sign: bool) -> u64 {
    (sign as u64) << 39
}

pub fn pack_inf(sign: bool) -> u64 {
    (sign as u64) << 39 | 0xff << 31
}

/// The largest finite value with `frac_bits` fraction bits.
fn norm_max(sign: bool, frac_bits: u32) -> u64 {
    (sign as u64) << 39 | 0xfe << 31 | (((1u64 << frac_bits) - 1) << (31 - frac_bits))
}

pub fn nan() -> (u64, Flags) {
    (NAN, Flags { invalid: true, ..Flags::default() })
}

fn zero_result(sign: bool) -> (u64, Flags) {
    (pack_zero(sign), Flags { zero: true, neg: sign, ..Flags::default() })
}

/// Round `sign` × `m` × 2^`e2` (`m` nonzero) to `frac_bits` fraction bits,
/// to nearest even or toward zero, and pack it. Overflow gives ±Infinity
/// (nearest) or ±NORM.MAX (toward zero); a result below the normal range
/// is ±0 with underflow (UM B-24). `m` has at most 64 bits: every
/// operation's exact result (or one with a sticky bit standing for what was
/// shifted out, below the rounding position) fits.
pub fn round_pack(sign: bool, m: u64, e2: i32, frac_bits: u32, trunc: bool) -> (u64, Flags) {
    debug_assert!(m != 0);
    let top = 63 - m.leading_zeros() as i32;
    let mut exp = top + e2;
    let shift = top - frac_bits as i32;
    let mut kept = if shift > 0 {
        let s = shift as u32;
        let kept = m >> s;
        let rem = m & ((1u64 << s) - 1);
        let half = 1u64 << (s - 1);
        if !trunc && (rem > half || rem == half && kept & 1 == 1) {
            kept + 1
        } else {
            kept
        }
    } else {
        m << (-shift) as u32
    };
    if kept >> (frac_bits + 1) != 0 {
        kept >>= 1;
        exp += 1;
    }
    let flags = Flags { neg: sign, ..Flags::default() };
    if exp > 127 {
        let v = if trunc { norm_max(sign, frac_bits) } else { pack_inf(sign) };
        return (v, Flags { overflow: true, ..flags });
    }
    if exp < -126 {
        return (pack_zero(sign), Flags { zero: true, underflow: true, ..flags });
    }
    let frac = kept & ((1 << frac_bits) - 1);
    ((sign as u64) << 39 | ((exp + 127) as u64) << 31 | frac << (31 - frac_bits), flags)
}

/// Repack a value that is already representable (an input that was
/// masked to the current precision, or a sign change of one).
pub fn pack(u: Unpacked) -> (u64, Flags) {
    match u.class {
        Class::Zero => zero_result(u.sign),
        Class::Inf => (pack_inf(u.sign), Flags { neg: u.sign, ..Flags::default() }),
        Class::Nan => nan(),
        Class::Normal => {
            let v = (u.sign as u64) << 39 | ((u.exp + 127) as u64) << 31 | (u.sig & 0x7fff_ffff);
            (v, Flags { neg: u.sign, ..Flags::default() })
        }
    }
}

/// The 32-bit (RND32) fast path shared by `add` and `mul`: `r` is the
/// exact result of an operation on two normal single-precision inputs,
/// computed in f64 (which holds it exactly). Rounds it to 23 fraction bits
/// as `round_pack` would; `None` if the result is zero, or would overflow
/// or underflow, for the exact path to handle.
#[inline(always)]
fn single(r: f64, trunc: bool) -> Option<(u64, Flags)> {
    let bits = r.to_bits();
    let sign = bits >> 63 != 0;
    let mag = bits & !(1 << 63);
    if mag == 0 {
        return None;
    }
    // Drop 29 of f64's 52 fraction bits, to nearest even or toward zero;
    // a carry out of the fraction lands in the exponent, as it should.
    let rounded = if trunc { mag >> 29 } else { (mag + ((1 << 28) - 1) + ((mag >> 29) & 1)) >> 29 };
    let e = (rounded >> 23) as i32 - 1023;
    if !(-126..=127).contains(&e) {
        return None;
    }
    let v32 = (sign as u64) << 31 | ((e + 127) as u64) << 23 | (rounded & 0x7f_ffff);
    Some((v32 << 8, Flags { neg: sign, ..Flags::default() }))
}

/// A normal single-precision value's f32 bits from a 40-bit register (RND32
/// inputs ignore the low 8 bits), or None for zero, denormal, Inf, NaN.
#[inline(always)]
fn normal32(v: u64) -> Option<u32> {
    let b = (v >> 8) as u32;
    let e = (b >> 23) & 0xff;
    (e != 0 && e != 0xff).then_some(b)
}

/// `x + y` (`sub`: `x - y`), halved if `half` ((Fx+Fy)/2: the exponent is
/// decremented before rounding, UM B-28).
#[inline]
pub fn add(x: u64, y: u64, sub: bool, half: bool, m: Mode) -> (u64, Flags) {
    if m.rnd32 && !half {
        if let (Some(a), Some(b)) = (normal32(x), normal32(y)) {
            // Exact in f64 while the exponents are at most 28 apart: the
            // aligned sum then has at most 53 significant bits.
            let d = ((a >> 23) & 0xff) as i32 - ((b >> 23) & 0xff) as i32;
            if d.abs() <= 28 {
                let (fa, fb) = (f32::from_bits(a) as f64, f32::from_bits(b) as f64);
                let r = if sub { fa - fb } else { fa + fb };
                if let Some(v) = single(r, m.trunc) {
                    return v;
                }
            }
        }
    }
    add_exact(x, y, sub, half, m)
}

#[inline(never)]
fn add_exact(x: u64, y: u64, sub: bool, half: bool, m: Mode) -> (u64, Flags) {
    let (a, mut b) = (input(x, m), input(y, m));
    if sub {
        b.sign = !b.sign;
    }
    if a.class == Class::Nan || b.class == Class::Nan {
        return nan();
    }
    match (a.class, b.class) {
        (Class::Inf, Class::Inf) if a.sign != b.sign => return nan(),
        (Class::Inf, _) => return pack(a),
        (_, Class::Inf) => return pack(b),
        (Class::Zero, Class::Zero) => return zero_result(a.sign && b.sign),
        _ => {}
    }
    // Align on the larger exponent with 30 guard bits; anything shifted
    // out sets a sticky bit. The larger term is at least 2^61, so the sum
    // keeps at least 29 bits below the 32-bit significand whenever bits
    // were shifted out: the sticky bit stays below the rounding position
    // and the rounding is that of the exact sum.
    let term = |u: Unpacked, emax: i32| -> i64 {
        if u.class == Class::Zero {
            return 0;
        }
        let d = (emax - u.exp) as u32;
        let wide = u.sig << 30;
        let v = if d >= 63 {
            1
        } else {
            let s = wide >> d;
            s | ((s << d != wide) as u64)
        };
        if u.sign {
            -(v as i64)
        } else {
            v as i64
        }
    };
    let emax = [a, b].iter().filter(|u| u.class == Class::Normal).map(|u| u.exp).max().unwrap();
    let sum = term(a, emax) + term(b, emax);
    if sum == 0 {
        return zero_result(false);
    }
    let e2 = emax - 31 - 30 - half as i32;
    round_pack(sum < 0, sum.unsigned_abs(), e2, m.frac_bits(), m.trunc)
}

/// `x * y` (UM B-53).
#[inline]
pub fn mul(x: u64, y: u64, m: Mode) -> (u64, Flags) {
    if m.rnd32 {
        if let (Some(a), Some(b)) = (normal32(x), normal32(y)) {
            // Two 24-bit significands: the product is exact in f64.
            if let Some(v) = single(f32::from_bits(a) as f64 * f32::from_bits(b) as f64, m.trunc) {
                return v;
            }
        }
    }
    mul_exact(x, y, m)
}

#[inline(never)]
fn mul_exact(x: u64, y: u64, m: Mode) -> (u64, Flags) {
    let (a, b) = (input(x, m), input(y, m));
    let sign = a.sign != b.sign;
    match (a.class, b.class) {
        (Class::Nan, _) | (_, Class::Nan) => nan(),
        (Class::Inf, Class::Zero) | (Class::Zero, Class::Inf) => nan(),
        (Class::Inf, _) | (_, Class::Inf) => (pack_inf(sign), Flags { neg: sign, ..Flags::default() }),
        (Class::Zero, _) | (_, Class::Zero) => zero_result(sign),
        _ => round_pack(sign, a.sig * b.sig, a.exp + b.exp - 62, m.frac_bits(), m.trunc),
    }
}

/// Order of two inputs for COMP/MIN/MAX: `None` if either is a NaN.
pub fn compare(x: u64, y: u64, m: Mode) -> Option<std::cmp::Ordering> {
    input(x, m).to_f64().partial_cmp(&input(y, m).to_f64())
}

/// `RND Fx`: round a 40-bit value to the 32-bit boundary; the input keeps
/// its LSBs whatever RND32 says (UM §2.5.2.3).
pub fn rnd(x: u64, m: Mode) -> (u64, Flags) {
    let u = unpack(x);
    match u.class {
        Class::Normal => round_pack(u.sign, u.sig, u.exp - 31, 23, m.trunc),
        _ => pack(u),
    }
}

/// `SCALB Fx BY Ry`: add `scale` to the exponent (UM B-34).
pub fn scalb(x: u64, scale: i32, m: Mode) -> (u64, Flags) {
    let u = input(x, m);
    match u.class {
        Class::Normal => {
            let e = (u.exp as i64 + scale as i64).clamp(-1000, 1000) as i32;
            round_pack(u.sign, u.sig, e - 31, m.frac_bits(), m.trunc)
        }
        _ => pack(u),
    }
}

/// `FLOAT Rx [BY Ry]`: always rounded to the 40-bit boundary (UM B-38).
pub fn float(x: u32, scale: i32, m: Mode) -> (u64, Flags) {
    let v = x as i32 as i64;
    if v == 0 {
        return zero_result(false);
    }
    round_pack(v < 0, v.unsigned_abs(), scale.clamp(-1000, 1000), 31, m.trunc)
}

/// How FIX/TRUNC round to an integer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToInt {
    Nearest,
    TowardZero,
    Floor,
}

/// Float to fixed (UM B-37): the result's 32-bit value, flags, and
/// whether it overflowed (AV). `sat`: MODE1 ALUSAT. An overflow or
/// Infinity without saturation, or a NaN, gives the NaN pattern instead
/// (returned as `Err`).
pub fn to_int(x: u64, scale: i32, round: ToInt, sat: bool, m: Mode) -> (Result<u32, u64>, Flags) {
    let u = input(x, m);
    let mut f = Flags::default();
    let overflow = |sign: bool, mut f: Flags| -> (Result<u32, u64>, Flags) {
        f.overflow = true;
        if sat {
            let v = if sign { 0x8000_0000 } else { 0x7fff_ffff };
            f.neg = sign;
            (Ok(v), f)
        } else {
            f.invalid = true;
            (Err(NAN), f)
        }
    };
    match u.class {
        Class::Nan => {
            f.invalid = true;
            return (Err(NAN), f);
        }
        Class::Inf => return overflow(u.sign, f),
        Class::Zero => {
            f.zero = true;
            return (Ok(0), f);
        }
        Class::Normal => {}
    }
    let e = u.exp as i64 + scale as i64;
    // AV when the biased exponent plus scale exceeds 157 (UM B-37).
    if e + 127 > 157 {
        return overflow(u.sign, f);
    }
    f.underflow = e < 0;
    // |value| = sig * 2^(e - 31), e <= 30.
    let s = (31 - e).min(100) as u32;
    let sig = u.sig as u128;
    let (q, rem) = (sig >> s, sig & ((1u128 << s) - 1));
    let half = 1u128 << (s - 1);
    let mag = match round {
        ToInt::TowardZero => q,
        ToInt::Floor if u.sign && rem != 0 => q + 1,
        ToInt::Floor => q,
        ToInt::Nearest if rem > half || rem == half && q & 1 == 1 => q + 1,
        ToInt::Nearest => q,
    };
    let v = if u.sign { -(mag as i128) } else { mag as i128 };
    if v > i32::MAX as i128 || v < i32::MIN as i128 {
        return overflow(u.sign, f);
    }
    f.zero = v == 0;
    f.neg = v < 0;
    (Ok(v as i32 as u32), f)
}

/// `MANT Fx`: the significand with its hidden bit, 1.31 unsigned (UM
/// B-35); `None` for a NaN or Infinity (all 1s, invalid).
pub fn mant(x: u64, m: Mode) -> Option<u32> {
    let u = input(x, m);
    match u.class {
        Class::Zero => Some(0),
        Class::Normal => Some(u.sig as u32),
        _ => None,
    }
}

/// The 8-bit-accurate reciprocal seed for a 7-bit mantissa index. The
/// manual says the seed comes from a ROM table but doesn't list it; this
/// is 2/(1.m) at the middle of the index's interval, rounded to 8
/// fraction bits (docs/hw/adsp2106x.md, open questions).
fn recips_seed(i: u32) -> u64 {
    let m = 1.0 + (i as f64 + 0.5) / 128.0;
    ((2.0 / m - 1.0) * 256.0).round().clamp(0.0, 255.0) as u64
}

/// The 4-bit-accurate reciprocal square root seed for the 7-bit index
/// (exponent LSB, 6 mantissa MSBs); same caveat as `recips_seed`.
fn rsqrts_seed(i: u32) -> u64 {
    let odd_exp = i & 0x40 == 0; // biased exponent even: unbiased odd
    let m = 1.0 + ((i & 0x3f) as f64 + 0.5) / 64.0;
    let r = if odd_exp { 2.0 / (2.0 * m).sqrt() } else { 2.0 / m.sqrt() };
    ((r - 1.0) * 16.0).round().clamp(0.0, 15.0) as u64
}

/// `RECIPS Fx` (UM B-39).
pub fn recips(x: u64, m: Mode) -> (u64, Flags) {
    let u = input(x, m);
    let neg = u.sign;
    match u.class {
        Class::Nan => nan(),
        Class::Zero => (pack_inf(u.sign), Flags { overflow: true, neg, ..Flags::default() }),
        _ if u.exp > 125 => (pack_zero(u.sign), Flags { zero: true, neg, ..Flags::default() }),
        _ => {
            let idx = ((u.sig >> 24) & 0x7f) as u32;
            let exp = (-u.exp - 1 + 127) as u64;
            ((u.sign as u64) << 39 | exp << 31 | recips_seed(idx) << 23, Flags { neg, ..Flags::default() })
        }
    }
}

/// `RSQRTS Fx` (UM B-40).
pub fn rsqrts(x: u64, m: Mode) -> (u64, Flags) {
    let u = input(x, m);
    match u.class {
        Class::Nan => nan(),
        Class::Zero => (pack_inf(u.sign), Flags { overflow: true, neg: u.sign, ..Flags::default() }),
        _ if u.sign => nan(),
        Class::Inf => (pack_zero(false), Flags { zero: true, ..Flags::default() }),
        _ => {
            let biased = (u.exp + 127) as u32;
            let idx = (biased & 1) << 6 | ((u.sig >> 25) & 0x3f) as u32;
            let exp = (-(u.exp >> 1) - 1 + 127) as u64;
            (exp << 31 | rsqrts_seed(idx) << 27, Flags::default())
        }
    }
}

/// `PASS Fx`, `-Fx` (`neg`) or `ABS Fx` (`abs`): the input at the current
/// precision with its sign kept, flipped or cleared (UM B-29, B-30, B-31),
/// and whether it was negative (ABS's AS flag).
#[inline]
pub fn sign_op(x: u64, neg: bool, abs: bool, m: Mode) -> (u64, Flags, bool) {
    let mut u = input(x, m);
    let was = u.sign;
    if neg {
        u.sign = !u.sign;
    }
    if abs {
        u.sign = false;
    }
    let (v, f) = pack(u);
    (v, f, was)
}

/// What [`select`] computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Select {
    Copysign,
    Min,
    Max,
    Clip,
}

/// `COPYSIGN`, `MIN`, `MAX` or `CLIP` of two floats (UM B-41 to B-44).
pub fn select(op: Select, x: u64, y: u64, m: Mode) -> (u64, Flags) {
    let (a, b) = (input(x, m), input(y, m));
    if a.class == Class::Nan || b.class == Class::Nan {
        return nan();
    }
    let u = match op {
        Select::Copysign => Unpacked { sign: b.sign, ..a },
        Select::Min | Select::Max if a.class == Class::Zero && b.class == Class::Zero => {
            // MIN(+0,-0) is -0, MAX(+0,-0) is +0 (UM B-42, B-43).
            let sign = if op == Select::Min { a.sign || b.sign } else { a.sign && b.sign };
            Unpacked { sign, ..a }
        }
        Select::Min => {
            if b.to_f64() < a.to_f64() {
                b
            } else {
                a
            }
        }
        Select::Max => {
            if b.to_f64() > a.to_f64() {
                b
            } else {
                a
            }
        }
        Select::Clip => {
            if a.to_f64().abs() < b.to_f64().abs() {
                a
            } else {
                Unpacked { sign: a.sign, ..b }
            }
        }
    };
    pack(u)
}

/// `FPACK Fx` (UM B-74): the 16-bit float, and whether it overflowed.
pub fn fpack(x: u64) -> (u32, bool) {
    let v = (x >> 8) as u32;
    let sign = v >> 31;
    let exp = (v >> 23) & 0xff;
    let frac = v & 0x7f_ffff;
    // Round `value` (with `drop` bits below the kept ones) to nearest even.
    let round = |value: u32, drop: u32| -> u32 {
        let q = value >> drop;
        let rem = value & ((1 << drop) - 1);
        let half = 1 << (drop - 1);
        if rem > half || rem == half && q & 1 == 1 {
            q + 1
        } else {
            q
        }
    };
    let (bits, overflow) = if exp > 135 {
        (0x7fff, true)
    } else if exp > 120 {
        let e4 = (exp >> 4 & 8) | (exp & 7);
        let f = round(frac, 12);
        // Rounding up past the 11 bits carries into the exponent.
        let packed = (e4 << 11) + f;
        if packed > 0x7fff {
            (0x7fff, true)
        } else {
            (packed, false)
        }
    } else if exp > 109 {
        // Hidden 1 and the upper (exp - 110) fraction bits, rounded.
        let n = exp - 110;
        (round(1 << 23 | frac, 23 - n), false)
    } else {
        (0, false)
    };
    (sign << 15 | bits, overflow)
}

/// `FUNPACK Rx` (UM B-75): the 16-bit float in bits 15-0 to IEEE single.
pub fn funpack(v: u32) -> u32 {
    let sign = (v >> 15) & 1;
    let e4 = (v >> 11) & 15;
    let f11 = v & 0x7ff;
    let body = if e4 != 0 {
        let msb = e4 >> 3;
        let exp = msb << 7 | (if msb == 0 { 0xf } else { 0 }) << 3 | (e4 & 7);
        exp << 23 | f11 << 12
    } else if f11 == 0 {
        0
    } else {
        let n = f11.leading_zeros() - 21;
        let exp = 120 - n;
        // Strip the leading 1 and left-align the rest of the 11 bits.
        let rest = (f11 << (n + 1)) & 0x7ff;
        exp << 23 | rest << 12
    };
    sign << 31 | body
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The f64 fast paths of `add` and `mul` agree with the exact ones bit
    /// for bit, flags included, over random and edge-case inputs.
    #[test]
    fn single_precision_fast_paths_match_exact() {
        let mut seed = 0x1234_5678_9abc_def1u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let edge = |r: u64, k: u64| -> u64 {
            // Exponents near the ends of the range, near each other, and
            // anything; low 8 bits random (RND32 ignores them).
            let e = match k % 4 {
                0 => r >> 40 & 0xff,
                1 => 1 + (r >> 40) % 8,
                2 => 0xfe - (r >> 40) % 8,
                _ => 0x7f + (r >> 40) % 30 - 15,
            };
            (r & 1 << 39) | e << 31 | (r & 0x7fff_ffff)
        };
        for mode in [Mode { trunc: true, rnd32: true }, Mode { trunc: false, rnd32: true }] {
            for i in 0..400_000u64 {
                let (x, y) = (edge(next(), i), edge(next(), i / 4));
                let y = if i % 7 == 0 { x ^ (next() & 0xffff) } else { y };
                for sub in [false, true] {
                    assert_eq!(
                        add(x, y, sub, false, mode),
                        add_exact(x, y, sub, false, mode),
                        "{x:#x} {y:#x} {sub} {mode:?}"
                    );
                }
                assert_eq!(mul(x, y, mode), mul_exact(x, y, mode), "{x:#x} * {y:#x} {mode:?}");
            }
        }
    }

    const RN40: Mode = Mode { trunc: false, rnd32: false };
    const RN32: Mode = Mode { trunc: false, rnd32: true };
    const TZ32: Mode = Mode { trunc: true, rnd32: true };

    /// A 40-bit register value from an f32.
    fn f(v: f32) -> u64 {
        (v.to_bits() as u64) << 8
    }
    fn g(v: u64) -> f32 {
        f32::from_bits((v >> 8) as u32)
    }

    #[test]
    fn add_rounds_like_ieee_single_in_32_bit_mode() {
        for (a, b) in [(1.0f32, 2.0f32), (0.1, 0.2), (1e30, -1e30), (3.0, 1e-8), (-7.5, 2.25), (16777216.0, 1.0)] {
            assert_eq!(g(add(f(a), f(b), false, false, RN32).0), a + b, "{a} + {b}");
            assert_eq!(g(add(f(a), f(b), true, false, RN32).0), a - b, "{a} - {b}");
            assert_eq!(g(mul(f(a), f(b), RN32).0), a * b, "{a} * {b}");
        }
    }

    #[test]
    fn extended_precision_keeps_eight_more_bits() {
        // 1 + 2^-30 is exact in 40 bits, rounds to 1.0 in 32.
        let one = f(1.0);
        let tiny = f(2f32.powi(-30));
        let (r40, _) = add(one, tiny, false, false, RN40);
        assert_eq!(r40, one | 2, "LSB of the 31-bit fraction is register bit 1");
        assert_eq!(add(one, tiny, false, false, RN32).0, one);
    }

    #[test]
    fn truncation_rounds_toward_zero_and_overflows_to_norm_max() {
        // 1 + 2^-23 + 2^-24 rounds up to nearest, truncates down.
        let x = f(1.0 + 2f32.powi(-23));
        let h = f(2f32.powi(-24));
        assert_eq!(g(add(x, h, false, false, RN32).0), 1.0 + 2.0 * 2f32.powi(-23));
        assert_eq!(g(add(x, h, false, false, TZ32).0), 1.0 + 2f32.powi(-23));
        let (v, fl) = mul(f(f32::MAX), f(2.0), TZ32);
        assert!(fl.overflow);
        assert_eq!(g(v), f32::MAX);
        let (v, fl) = mul(f(f32::MAX), f(2.0), RN32);
        assert!(fl.overflow && g(v).is_infinite());
    }

    #[test]
    fn denormals_flush_to_zero_with_underflow() {
        let (v, fl) = mul(f(1e-30), f(1e-30), RN32);
        assert_eq!(v, 0);
        assert!(fl.underflow && fl.zero);
        // A denormal input is a zero.
        assert_eq!(add(f(f32::from_bits(1)), f(0.0), false, false, RN32).0, 0);
    }

    #[test]
    fn nan_and_invalid() {
        assert_eq!(add(f(f32::INFINITY), f(f32::NEG_INFINITY), false, false, RN32), nan());
        assert_eq!(mul(f(f32::INFINITY), f(0.0), RN32), nan());
        assert_eq!(add(f(f32::NAN), f(1.0), false, false, RN32).0, NAN);
    }

    #[test]
    fn fix_rounding_modes() {
        let fix = |v: f32, r: ToInt| to_int(f(v), 0, r, false, RN32).0.unwrap() as i32;
        assert_eq!(fix(2.5, ToInt::Nearest), 2);
        assert_eq!(fix(3.5, ToInt::Nearest), 4);
        assert_eq!(fix(-2.5, ToInt::Floor), -3);
        assert_eq!(fix(-2.5, ToInt::TowardZero), -2);
        assert_eq!(fix(-0.25, ToInt::Floor), -1, "negative underflow truncated is -1");
        let (r, fl) = to_int(f(3e9), 0, ToInt::Nearest, true, RN32);
        assert_eq!((r, fl.overflow), (Ok(0x7fff_ffff), true));
        let (r, fl) = to_int(f(3e9), 0, ToInt::Nearest, false, RN32);
        assert_eq!((r, fl.invalid), (Err(NAN), true));
        assert_eq!(to_int(f(1.5), 2, ToInt::TowardZero, false, RN32).0, Ok(6));
    }

    #[test]
    fn float_is_exact_to_40_bits_even_in_32_bit_mode() {
        let (v, _) = float(0x7fff_ffff, 0, RN32);
        // 2^31 - 1 needs 31 significant bits: it survives in 40 bits.
        assert_eq!(unpack(v).sig, 0xffff_fffe);
        assert_eq!(g(float(-3i32 as u32, 1, RN32).0), -6.0);
    }

    #[test]
    fn short_float_round_trip() {
        for v in [1.0f32, -2.5, 0.0078125, 100.0] {
            let (p, ov) = fpack(f(v));
            assert!(!ov);
            assert_eq!(f32::from_bits(funpack(p)), v, "{v} packs to {p:#06x}");
        }
        // Gradual underflow: 2^-9 packs with exponent 0 and unpacks.
        let (p, _) = fpack(f(2f32.powi(-9)));
        assert_eq!(p >> 11 & 15, 0);
        assert_eq!(f32::from_bits(funpack(p)), 2f32.powi(-9));
        assert!(fpack(f(1e10)).1);
    }

    #[test]
    fn seeds_are_within_their_stated_accuracy() {
        for v in [1.0f32, 1.7, 3.3, 0.01, 123456.0] {
            let r = g(recips(f(v), RN32).0) as f64;
            assert!((r * v as f64 - 1.0).abs() <= 1.0 / 256.0, "RECIPS {v}: {r}");
            let s = g(rsqrts(f(v), RN32).0) as f64;
            assert!((s * (v as f64).sqrt() - 1.0).abs() <= 1.0 / 16.0, "RSQRTS {v}: {s}");
        }
        assert!(recips(f(0.0), RN32).1.overflow);
        assert_eq!(rsqrts(f(-1.0), RN32).0, NAN);
    }
}
