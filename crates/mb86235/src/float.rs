//! The MB86235's arithmetic on 32-bit register values, as the interpreter
//! (and so the recompiled code) computes it: IEEE single precision, rounded
//! to nearest, denormals kept (`docs/hw/mb86235.md` §1.4: the chip's own
//! rounding is unknown). Shared with the native 2C geometrizer
//! (`model2::geo_native_2c`), which must give the same bits.
//!
//! A NaN result is defined here rather than left to the host: the first
//! NaN operand, quieted, else the default NaN (what an x86 FPU gives with
//! its operands in this order). The compiler may swap the operands of an
//! addition or a product, and WebAssembly may give any NaN, so without
//! this two NaNs meeting would give different bits from build to build.
//! The chip's own NaN rules are unknown. Operands are in the order the
//! instruction takes them: `sub(a, b)` is `a - b`, so an `FSUB in1, in2`
//! is `sub(in2, in1)`.

#[inline(always)]
fn f(v: u32) -> f32 {
    f32::from_bits(v)
}

/// The quiet bit of a NaN.
const QUIET: u32 = 0x0040_0000;
/// The NaN an invalid operation (∞ − ∞, 0 × ∞) gives.
pub const DEFAULT_NAN: u32 = 0xffc0_0000;

/// `r`'s bits, or for a NaN the first NaN operand quieted, else the
/// default NaN.
#[inline(always)]
fn nan2(r: f32, a: u32, b: u32) -> u32 {
    if !r.is_nan() {
        r.to_bits()
    } else {
        nan_of(a, b)
    }
}

#[cold]
fn nan_of(a: u32, b: u32) -> u32 {
    if f(a).is_nan() {
        a | QUIET
    } else if f(b).is_nan() {
        b | QUIET
    } else {
        DEFAULT_NAN
    }
}

/// `a + b`.
#[inline(always)]
pub fn add(a: u32, b: u32) -> u32 {
    nan2(f(a) + f(b), a, b)
}

/// `a - b`.
#[inline(always)]
pub fn sub(a: u32, b: u32) -> u32 {
    nan2(f(a) - f(b), a, b)
}

/// `a * b`.
#[inline(always)]
pub fn mul(a: u32, b: u32) -> u32 {
    nan2(f(a) * f(b), a, b)
}

/// The `Z` forms' clamp: a negative result becomes +0 (and sets ZC, the
/// second value). A NaN or a negative zero is kept.
#[inline(always)]
pub fn clamp_negative(v: u32) -> (u32, bool) {
    if f(v) < 0.0 {
        (0, true)
    } else {
        (v, false)
    }
}

/// `FADDZ`: `a + b`, a negative sum clamped to +0.
#[inline(always)]
pub fn addz(a: u32, b: u32) -> u32 {
    clamp_negative(add(a, b)).0
}

/// `FSUBZ`: `a - b`, a negative difference clamped to +0.
#[inline(always)]
pub fn subz(a: u32, b: u32) -> u32 {
    clamp_negative(sub(a, b)).0
}

/// `FABS`: the sign bit cleared.
#[inline(always)]
pub fn abs(a: u32) -> u32 {
    f(a).abs().to_bits()
}

/// `FRCP`: `1 / a`.
#[inline(always)]
pub fn rcp(a: u32) -> u32 {
    nan2(1.0 / f(a), a, a)
}

/// `FRSQ`: `1 / sqrt(a)`, two roundings (the square root, then the
/// quotient).
#[inline(always)]
pub fn rsq(a: u32) -> u32 {
    nan2(1.0 / f(a).sqrt(), a, a)
}

/// `FLOG`: `log2(a)`.
#[inline(always)]
pub fn log2(a: u32) -> u32 {
    nan2(f(a).log2(), a, a)
}

/// `CIF`: a signed integer to float.
#[inline(always)]
pub fn cif(a: u32) -> u32 {
    (a as i32 as f32).to_bits()
}

/// `CFI`: float to signed integer, toward zero, saturating; NaN gives 0.
#[inline(always)]
pub fn cfi(a: u32) -> u32 {
    (f(a).trunc() as i32) as u32
}

/// `CFIB`: float to an unsigned byte, toward zero, clamped to 0-255; NaN
/// gives 0.
#[inline(always)]
pub fn cfib(a: u32) -> u32 {
    f(a).clamp(0.0, 255.0) as u32
}

/// The `AN` flag after a float result: negative (not -0, not a NaN).
#[inline(always)]
pub fn negative(v: u32) -> bool {
    f(v) < 0.0
}

/// The `AZ` flag after a float result: either zero.
#[inline(always)]
pub fn zero(v: u32) -> bool {
    f(v) == 0.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(v: f32) -> u32 {
        v.to_bits()
    }

    #[test]
    fn helpers() {
        assert_eq!(sub(b(3.0), b(1.0)), b(2.0));
        assert_eq!(addz(b(-1.0), b(0.5)), 0, "clamped to +0");
        assert_eq!(addz(b(-0.0), b(-0.0)), b(-0.0), "a negative zero is kept");
        assert_eq!(subz(b(1.0), b(3.0)), 0);
        assert_eq!(abs(b(-2.5)), b(2.5));
        assert_eq!(rsq(b(4.0)), b(0.5));
        assert_eq!(cfib(b(300.0)), 255);
        assert_eq!(cfib(b(-3.0)), 0);
        assert_eq!(cfib(0x7fc0_0000), 0, "NaN");
        assert_eq!(cfib(b(17.9)), 17);
        assert_eq!(cfi(b(-2.5)), (-2i32) as u32);
        assert_eq!(cif((-3i32) as u32), b(-3.0));
        assert!(negative(b(-1e-40)) && !negative(b(-0.0)) && !negative(0xffc0_0000));
    }

    #[test]
    fn nans() {
        let inf = b(f32::INFINITY);
        assert_eq!(sub(inf, inf), DEFAULT_NAN);
        assert_eq!(mul(0, inf), DEFAULT_NAN);
        // The first NaN operand wins, quieted, whichever order the host
        // computes in.
        assert_eq!(add(0xffff_ffff, 0x7fc0_0000), 0xffff_ffff);
        assert_eq!(add(0x7fc0_0000, 0xffff_ffff), 0x7fc0_0000);
        assert_eq!(mul(b(2.0), 0x7f80_0001), 0x7fc0_0001, "a signalling NaN is quieted");
        assert_eq!(rsq(b(-1.0)), DEFAULT_NAN);
        assert_eq!(rsq(0x7fc0_1234), 0x7fc0_1234);
    }
}
