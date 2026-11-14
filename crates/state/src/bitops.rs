//! Bit-field helpers shared by the CPU cores' decoders.

/// Bits `hi..=lo` of `w`, right-aligned.
#[inline]
pub fn bits(w: impl Into<u64>, hi: u32, lo: u32) -> u32 {
    ((w.into() >> lo) & ((1u64 << (hi - lo + 1)) - 1)) as u32
}

/// `v` sign-extended from `width` bits.
#[inline]
pub fn sext(v: u32, width: u32) -> i32 {
    let s = 32 - width;
    ((v << s) as i32) >> s
}

/// [`sext`] as an unsigned value.
#[inline]
pub fn sext_u(v: u32, width: u32) -> u32 {
    sext(v, width) as u32
}

/// `v` sign-extended from `len` bits; unchanged when `len` is 0 or 64+.
#[inline]
pub fn sext64(v: u64, len: u32) -> u64 {
    if len == 0 || len >= 64 {
        v
    } else {
        (((v << (64 - len)) as i64) >> (64 - len)) as u64
    }
}

/// [`sext`] on a signed value.
#[inline]
pub fn sext_i(v: i32, width: u32) -> i32 {
    sext(v as u32, width)
}
