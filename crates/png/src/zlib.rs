//! zlib streams (RFC 1950) around deflate (RFC 1951): a full inflater
//! (stored, fixed and dynamic Huffman blocks) and a compact deflater
//! (LZ77 with hash chains, fixed Huffman codes).

/// Length code bases and extra bits for symbols 257-285.
const LEN_BASE: [u16; 29] =
    [3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131, 163, 195, 227, 258];
const LEN_EXTRA: [u8; 29] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0];
/// Distance code bases and extra bits for symbols 0-29.
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537, 2049, 3073, 4097, 6145,
    8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] =
    [0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13];
/// Order of code length code lengths in a dynamic block header.
const CL_ORDER: [usize; 19] = [16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15];

pub fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    // 5552 is the most bytes that can be summed before b may overflow.
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// LSB-first bit reader over a byte slice.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u64,
    cnt: u32,
    /// Bits consumed so far, to detect reading past the end.
    used: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Bits { data, pos: 0, buf: 0, cnt: 0, used: 0 }
    }

    /// Make at least 32 bits available (zeros past the end).
    fn refill(&mut self) {
        while self.cnt <= 56 {
            let b = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.buf |= (b as u64) << self.cnt;
            self.cnt += 8;
        }
    }

    fn peek(&mut self, n: u32) -> u32 {
        if self.cnt < n {
            self.refill();
        }
        (self.buf & ((1u64 << n) - 1)) as u32
    }

    fn consume(&mut self, n: u32) -> Result<(), String> {
        self.buf >>= n;
        self.cnt -= n;
        self.used += n as usize;
        if self.used > self.data.len() * 8 {
            return Err("deflate stream is truncated".into());
        }
        Ok(())
    }

    fn bits(&mut self, n: u32) -> Result<u32, String> {
        if n == 0 {
            return Ok(0);
        }
        let v = self.peek(n);
        self.consume(n)?;
        Ok(v)
    }

    fn align(&mut self) -> Result<(), String> {
        let n = (8 - (self.used % 8) as u32) % 8;
        self.bits(n).map(|_| ())
    }
}

const FAST_BITS: u32 = 10;

/// A canonical Huffman code: per-length counts and symbols in code order
/// (for the bit-by-bit path), plus a lookup table for short codes.
struct Huffman {
    count: [u16; 16],
    symbol: Vec<u16>,
    /// Indexed by the next FAST_BITS input bits: symbol << 4 | length, or 0.
    fast: Vec<u16>,
}

impl Huffman {
    fn new(lengths: &[u8]) -> Result<Self, String> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        // Over-subscribed codes are invalid; incomplete ones are allowed
        // (a single distance code is common) and fail when an unused code
        // shows up.
        let mut left = 1i32;
        for &c in &count[1..] {
            left = left * 2 - c as i32;
            if left < 0 {
                return Err("over-subscribed Huffman code".into());
            }
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + count[l];
        }
        let mut symbol = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbol[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        // Canonical codes, then the reversed-bit lookup table.
        let mut fast = vec![0u16; 1 << FAST_BITS];
        let mut code = 0u32;
        let mut next = [0u32; 16];
        for l in 1..16 {
            code = (code + count[l - 1] as u32) << 1;
            next[l] = code;
        }
        for (s, &l) in lengths.iter().enumerate() {
            let l = l as u32;
            if l == 0 || l > FAST_BITS {
                continue;
            }
            let c = next[l as usize];
            next[l as usize] += 1;
            let rev = c.reverse_bits() >> (32 - l);
            let mut k = rev;
            while k < 1 << FAST_BITS {
                fast[k as usize] = (s as u16) << 4 | l as u16;
                k += 1 << l;
            }
        }
        Ok(Huffman { count, symbol, fast })
    }

    fn decode(&self, bits: &mut Bits) -> Result<u16, String> {
        let e = self.fast[bits.peek(FAST_BITS) as usize];
        if e != 0 {
            bits.consume((e & 15) as u32)?;
            return Ok(e >> 4);
        }
        // Long code: walk the canonical code one bit at a time.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= bits.bits(1)? as i32;
            let count = self.count[len] as i32;
            if code - count < first {
                return Ok(self.symbol[(index + (code - first)) as usize]);
            }
            index += count;
            first = (first + count) << 1;
            code <<= 1;
        }
        Err("invalid Huffman code".into())
    }
}

fn fixed_tables() -> (Huffman, Huffman) {
    let mut lit = [8u8; 288];
    lit[144..256].fill(9);
    lit[256..280].fill(7);
    (Huffman::new(&lit).unwrap(), Huffman::new(&[5u8; 30]).unwrap())
}

fn dynamic_tables(bits: &mut Bits) -> Result<(Huffman, Huffman), String> {
    let nlen = bits.bits(5)? as usize + 257;
    let ndist = bits.bits(5)? as usize + 1;
    let ncode = bits.bits(4)? as usize + 4;
    if nlen > 286 || ndist > 30 {
        return Err("bad dynamic block counts".into());
    }
    let mut cl = [0u8; 19];
    for &k in &CL_ORDER[..ncode] {
        cl[k] = bits.bits(3)? as u8;
    }
    let clh = Huffman::new(&cl)?;
    let mut lengths = vec![0u8; nlen + ndist];
    let mut i = 0;
    while i < nlen + ndist {
        let sym = clh.decode(bits)?;
        let (value, repeat) = match sym {
            0..=15 => (sym as u8, 1),
            16 => {
                let Some(&prev) = i.checked_sub(1).map(|p| &lengths[p]) else {
                    return Err("repeat with no previous length".into());
                };
                (prev, 3 + bits.bits(2)? as usize)
            }
            17 => (0, 3 + bits.bits(3)? as usize),
            _ => (0, 11 + bits.bits(7)? as usize),
        };
        if i + repeat > lengths.len() {
            return Err("code lengths overrun".into());
        }
        lengths[i..i + repeat].fill(value);
        i += repeat;
    }
    if lengths[256] == 0 {
        return Err("no end-of-block code".into());
    }
    Ok((Huffman::new(&lengths[..nlen])?, Huffman::new(&lengths[nlen..])?))
}

/// Decompress a raw deflate stream.
pub fn inflate(data: &[u8]) -> Result<Vec<u8>, String> {
    inflate_counted(data).map(|(out, _)| out)
}

/// `inflate`, also returning how many input bytes the stream used.
fn inflate_counted(data: &[u8]) -> Result<(Vec<u8>, usize), String> {
    let mut out = Vec::with_capacity(data.len() * 4);
    let mut bits = Bits::new(data);
    loop {
        let last = bits.bits(1)? == 1;
        match bits.bits(2)? {
            0 => {
                bits.align()?;
                let len = bits.bits(16)?;
                let nlen = bits.bits(16)?;
                if len != !nlen & 0xffff {
                    return Err("stored block length check failed".into());
                }
                for _ in 0..len {
                    out.push(bits.bits(8)? as u8);
                }
            }
            t @ (1 | 2) => {
                let (lit, dist) = if t == 1 { fixed_tables() } else { dynamic_tables(&mut bits)? };
                loop {
                    let sym = lit.decode(&mut bits)? as usize;
                    if sym < 256 {
                        out.push(sym as u8);
                        continue;
                    }
                    if sym == 256 {
                        break;
                    }
                    let k = sym - 257;
                    if k >= 29 {
                        return Err("bad length symbol".into());
                    }
                    let len = LEN_BASE[k] as usize + bits.bits(LEN_EXTRA[k] as u32)? as usize;
                    let d = dist.decode(&mut bits)? as usize;
                    if d >= 30 {
                        return Err("bad distance symbol".into());
                    }
                    let back = DIST_BASE[d] as usize + bits.bits(DIST_EXTRA[d] as u32)? as usize;
                    if back > out.len() {
                        return Err("distance too far back".into());
                    }
                    let from = out.len() - back;
                    for j in 0..len {
                        out.push(out[from + j]);
                    }
                }
            }
            _ => return Err("invalid deflate block type".into()),
        }
        if last {
            return Ok((out, bits.used.div_ceil(8)));
        }
    }
}

/// Decompress a zlib stream and check its Adler-32.
pub fn decompress(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 6 {
        return Err("zlib stream too short".into());
    }
    let (cmf, flg) = (data[0], data[1]);
    if cmf & 15 != 8 || !((cmf as u16) << 8 | flg as u16).is_multiple_of(31) {
        return Err("bad zlib header".into());
    }
    if flg & 0x20 != 0 {
        return Err("zlib preset dictionaries are not supported".into());
    }
    let (out, used) = inflate_counted(&data[2..])?;
    // The checksum follows the deflate data.
    let Some(tail) = data.get(2 + used..2 + used + 4) else {
        return Err("zlib checksum missing".into());
    };
    let want = u32::from_be_bytes([tail[0], tail[1], tail[2], tail[3]]);
    if adler32(&out) != want {
        return Err("zlib checksum mismatch".into());
    }
    Ok(out)
}

/// LSB-first bit writer.
struct Writer {
    out: Vec<u8>,
    buf: u64,
    cnt: u32,
}

impl Writer {
    fn put(&mut self, value: u32, n: u32) {
        self.buf |= (value as u64) << self.cnt;
        self.cnt += n;
        while self.cnt >= 8 {
            self.out.push(self.buf as u8);
            self.buf >>= 8;
            self.cnt -= 8;
        }
    }

    /// A Huffman code, which deflate stores most significant bit first.
    fn code(&mut self, code: u32, n: u32) {
        self.put(code.reverse_bits() >> (32 - n), n);
    }

    fn literal(&mut self, sym: u32) {
        match sym {
            0..=143 => self.code(0x30 + sym, 8),
            144..=255 => self.code(0x190 + sym - 144, 9),
            256..=279 => self.code(sym - 256, 7),
            _ => self.code(0xc0 + sym - 280, 8),
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.cnt > 0 {
            self.out.push(self.buf as u8);
        }
        self.out
    }
}

const WINDOW: usize = 32768;
const HASH_BITS: u32 = 15;
const MAX_CHAIN: usize = 48;
const NICE: usize = 128;

/// Raw deflate: one fixed-Huffman block, greedy LZ77 matching on 3-byte
/// hash chains.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut w = Writer { out: Vec::with_capacity(data.len() / 2 + 64), buf: 0, cnt: 0 };
    w.put(1, 1); // final block
    w.put(1, 2); // fixed Huffman
    let mut len_code = [0u8; 259];
    for (k, &base) in LEN_BASE.iter().enumerate() {
        let end = LEN_BASE.get(k + 1).map_or(259, |&b| b as usize);
        len_code[base as usize..end.max(base as usize + 1)].fill(k as u8);
    }
    let mut head = vec![u32::MAX; 1 << HASH_BITS];
    let mut prev = vec![u32::MAX; WINDOW];
    let hash = |i: usize| {
        let v = (data[i] as u32) | (data[i + 1] as u32) << 8 | (data[i + 2] as u32) << 16;
        (v.wrapping_mul(0x9e37_79b1) >> (32 - HASH_BITS)) as usize
    };
    let insert = |i: usize, head: &mut [u32], prev: &mut [u32]| {
        if i + 2 < data.len() {
            let h = hash(i);
            prev[i % WINDOW] = head[h];
            head[h] = i as u32;
        }
    };
    let mut i = 0;
    while i < data.len() {
        let (mut best_len, mut best_dist) = (0, 0);
        if i + 2 < data.len() {
            let max = (data.len() - i).min(258);
            let mut cand = head[hash(i)];
            let mut chain = 0;
            while cand != u32::MAX && chain < MAX_CHAIN {
                let c = cand as usize;
                if i - c > WINDOW - 1 {
                    break;
                }
                if data[c + best_len.min(max - 1)] == data[i + best_len.min(max - 1)] {
                    let mut l = 0;
                    while l < max && data[c + l] == data[i + l] {
                        l += 1;
                    }
                    if l > best_len {
                        (best_len, best_dist) = (l, i - c);
                        if l >= NICE.min(max) {
                            break;
                        }
                    }
                }
                let next = prev[c % WINDOW];
                if next == u32::MAX || next as usize >= c {
                    break;
                }
                cand = next;
                chain += 1;
            }
        }
        if best_len >= 3 {
            let k = len_code[best_len] as usize;
            w.literal(257 + k as u32);
            w.put((best_len - LEN_BASE[k] as usize) as u32, LEN_EXTRA[k] as u32);
            let d = DIST_BASE.partition_point(|&b| b as usize <= best_dist) - 1;
            w.code(d as u32, 5);
            w.put((best_dist - DIST_BASE[d] as usize) as u32, DIST_EXTRA[d] as u32);
            for j in i..i + best_len {
                insert(j, &mut head, &mut prev);
            }
            i += best_len;
        } else {
            w.literal(data[i] as u32);
            insert(i, &mut head, &mut prev);
            i += 1;
        }
    }
    w.literal(256);
    w.finish()
}

/// Compress into a zlib stream.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut z = vec![0x78, 0x9c];
    z.extend_from_slice(&deflate(data));
    z.extend_from_slice(&adler32(data).to_be_bytes());
    z
}

#[cfg(test)]
mod tests {
    use super::*;

    /// zlib.compress(data, 9) from CPython: one dynamic Huffman block.
    const DYNAMIC: [u8; 300] = [
        0x78, 0xda, 0x25, 0x92, 0x0b, 0x0a, 0xc4, 0x30, 0x08, 0x44, 0xcf, 0x2a, 0x64, 0x20, 0x81, 0x90, 0x80, 0xd1,
        0xfb, 0xef, 0x9b, 0xee, 0xc2, 0xb6, 0xf1, 0x13, 0x7d, 0xa3, 0x55, 0x85, 0xf8, 0x4d, 0x3f, 0xae, 0x8f, 0xa1,
        0x19, 0xad, 0xa3, 0xd2, 0xda, 0x59, 0x51, 0x98, 0x78, 0xf3, 0x46, 0xa8, 0x4a, 0x31, 0xf5, 0x86, 0x2a, 0x6e,
        0x49, 0xb5, 0xf0, 0x75, 0x6a, 0x2c, 0xdb, 0xef, 0x4d, 0xee, 0x84, 0x0b, 0x8c, 0xc3, 0x8d, 0x9a, 0xc5, 0x85,
        0xc6, 0x8c, 0x87, 0x77, 0x2c, 0x9d, 0x3a, 0xd4, 0x94, 0x56, 0x2e, 0x71, 0x3f, 0xae, 0x6e, 0x13, 0x25, 0xa7,
        0x92, 0xb4, 0xf3, 0xea, 0x03, 0xe9, 0x6d, 0x96, 0x5a, 0x3c, 0x38, 0x35, 0xaf, 0x03, 0x85, 0x63, 0xb6, 0x6b,
        0xf5, 0x8e, 0x77, 0x29, 0xa6, 0x97, 0x81, 0xe3, 0xa8, 0x81, 0x32, 0xe3, 0xc5, 0x77, 0xb9, 0xb0, 0x54, 0x8f,
        0x3f, 0x91, 0x0d, 0xc8, 0xb0, 0x22, 0x72, 0x5e, 0xd7, 0x56, 0x92, 0xf6, 0x08, 0xa0, 0xe6, 0x6c, 0xf3, 0x5e,
        0xe4, 0x8d, 0x41, 0x81, 0x03, 0xc8, 0xd0, 0xfc, 0x6c, 0xfa, 0x84, 0x0b, 0x72, 0x3b, 0xad, 0x69, 0x87, 0x6d,
        0x88, 0x8e, 0x95, 0xd5, 0xb4, 0x46, 0x25, 0x44, 0xb3, 0xa7, 0xc7, 0xf3, 0x27, 0x67, 0x1c, 0x0e, 0xc4, 0xaa,
        0x1c, 0xe1, 0xe6, 0xce, 0x2a, 0x17, 0x85, 0xc8, 0xb3, 0x43, 0x7e, 0xaa, 0x0b, 0xe9, 0xb6, 0x87, 0x3b, 0xac,
        0x31, 0xdd, 0x62, 0xf4, 0x89, 0x88, 0xfc, 0x3a, 0x12, 0x42, 0x59, 0xd7, 0xcc, 0x48, 0x46, 0xf8, 0xdf, 0x01,
        0x29, 0x26, 0x3f, 0xe7, 0x13, 0xfc, 0xa9, 0xad, 0x02, 0xd4, 0x63, 0xa1, 0xd8, 0x26, 0x67, 0x6a, 0x73, 0x5a,
        0x0d, 0x98, 0x41, 0x4c, 0xe9, 0xa6, 0xf3, 0x78, 0x14, 0xaa, 0xab, 0xff, 0xa2, 0xeb, 0x41, 0x36, 0x0f, 0x5b,
        0xc3, 0xb1, 0xf5, 0xed, 0xe8, 0x16, 0xa3, 0xe9, 0xc4, 0x4a, 0xb7, 0x61, 0xc9, 0x87, 0x1d, 0xc6, 0xf2, 0x87,
        0x80, 0x2f, 0x3d, 0xce, 0x74, 0xd1, 0x6d, 0xb5, 0xf7, 0x2a, 0x2c, 0x5e, 0xf7, 0x61, 0x7f, 0x4d, 0xfd, 0x81,
        0x40, 0xda, 0x1f, 0xd8, 0x5a, 0xec, 0x41, 0x3f, 0x88, 0xd1, 0xf7, 0x0c,
    ];

    #[test]
    fn dynamic_huffman_stream() {
        assert_eq!((DYNAMIC[2] >> 1) & 3, 2, "first block is dynamic");
        let alphabet = b"eeeeeeeetttaaoinshrdlu";
        let want: Vec<u8> =
            (0..600usize).map(|i| alphabet[(i * i * 7 + i * 3 + (i >> 3)) % 101 % alphabet.len()]).collect();
        assert_eq!(decompress(&DYNAMIC).unwrap(), want);
    }

    #[test]
    fn stored_block() {
        // Final stored block "abc" (LEN 3, NLEN !3), then Adler-32.
        let mut z = vec![0x78, 0x01, 0x01, 0x03, 0x00, 0xfc, 0xff, b'a', b'b', b'c'];
        z.extend_from_slice(&adler32(b"abc").to_be_bytes());
        assert_eq!(decompress(&z).unwrap(), b"abc");
    }

    #[test]
    fn deflate_round_trip() {
        let mut seed = 1u32;
        let mut data: Vec<u8> = (0..70000)
            .map(|i| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
                if i % 1000 < 600 {
                    (i % 7) as u8
                } else {
                    (seed >> 24) as u8
                }
            })
            .collect();
        data.extend(std::iter::repeat_n(9, 1000));
        let z = compress(&data);
        assert!(z.len() < data.len() / 2);
        assert_eq!(decompress(&z).unwrap(), data);
        assert_eq!(decompress(&compress(&[])).unwrap(), b"");
    }

    #[test]
    fn corrupt_streams_fail_cleanly() {
        assert!(decompress(&DYNAMIC[..100]).is_err());
        let mut bad = DYNAMIC;
        bad[299] ^= 1;
        assert!(decompress(&bad).is_err());
    }

    #[test]
    fn adler_known_value() {
        assert_eq!(adler32(b"Wikipedia"), 0x11e6_0398);
    }
}
