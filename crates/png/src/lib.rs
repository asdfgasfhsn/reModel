//! A small PNG codec, std only: enough for debug snapshots, texture dumps
//! and texture packs without pulling in an image crate.
//!
//! Decoding handles every non-interlaced PNG: grey, grey + alpha, RGB, RGBA
//! and palette images at all legal bit depths (16-bit samples keep their
//! high byte), with tRNS transparency, all five row filters, and stored,
//! fixed and dynamic Huffman deflate blocks. The result is always 8-bit
//! RGBA. Encoding writes 8-bit RGB or RGBA, picking a filter per row and
//! compressing with LZ77 and fixed Huffman codes.

pub mod zlib;

/// An 8-bit RGBA image, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    pub width: usize,
    pub height: usize,
    /// width * height * 4 bytes.
    pub rgba: Vec<u8>,
}

const SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    for (n, e) in t.iter_mut().enumerate() {
        let mut c = n as u32;
        for _ in 0..8 {
            c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *e = c;
    }
    t
}

pub fn crc32(data: &[u8]) -> u32 {
    let t = crc_table();
    !data.iter().fold(!0u32, |c, &b| t[((c ^ b as u32) & 0xff) as usize] ^ (c >> 8))
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = ((p - a as i16).abs(), (p - b as i16).abs(), (p - c as i16).abs());
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Filter `row` (with `prev` the unfiltered row above, zeros for the
/// first) by filter type `kind` into `out`.
fn filter_row(kind: u8, row: &[u8], prev: &[u8], bpp: usize, out: &mut Vec<u8>) {
    out.push(kind);
    for i in 0..row.len() {
        let a = if i >= bpp { row[i - bpp] } else { 0 };
        let c = if i >= bpp { prev[i - bpp] } else { 0 };
        let b = prev[i];
        let pred = match kind {
            0 => 0,
            1 => a,
            2 => b,
            3 => ((a as u16 + b as u16) / 2) as u8,
            _ => paeth(a, b, c),
        };
        out.push(row[i].wrapping_sub(pred));
    }
}

/// Undo filter `kind` on `row` in place.
fn unfilter_row(kind: u8, row: &mut [u8], prev: &[u8], bpp: usize) -> Result<(), String> {
    match kind {
        0 => {}
        1 => {
            for i in bpp..row.len() {
                row[i] = row[i].wrapping_add(row[i - bpp]);
            }
        }
        2 => {
            for (r, &p) in row.iter_mut().zip(prev) {
                *r = r.wrapping_add(p);
            }
        }
        3 => {
            for i in 0..row.len() {
                let a = if i >= bpp { row[i - bpp] } else { 0 };
                row[i] = row[i].wrapping_add(((a as u16 + prev[i] as u16) / 2) as u8);
            }
        }
        4 => {
            for i in 0..row.len() {
                let (a, c) = if i >= bpp { (row[i - bpp], prev[i - bpp]) } else { (0, 0) };
                row[i] = row[i].wrapping_add(paeth(a, prev[i], c));
            }
        }
        _ => return Err(format!("unknown PNG filter type {kind}")),
    }
    Ok(())
}

/// Encode 8-bit pixels with `channels` 3 (RGB) or 4 (RGBA). `filter` forces
/// one filter type; None picks per row by the minimum-sum heuristic.
fn encode_with(width: usize, height: usize, channels: usize, data: &[u8], filter: Option<u8>) -> Vec<u8> {
    assert!(channels == 3 || channels == 4);
    assert_eq!(data.len(), width * height * channels, "pixel data does not match the size");
    let stride = width * channels;
    let mut raw = Vec::with_capacity((stride + 1) * height);
    let zero = vec![0u8; stride];
    let mut trial = Vec::with_capacity(stride + 1);
    for y in 0..height {
        let row = &data[y * stride..(y + 1) * stride];
        let prev = if y > 0 { &data[(y - 1) * stride..y * stride] } else { &zero[..] };
        let kind = filter.unwrap_or_else(|| {
            (0..5u8)
                .min_by_key(|&k| {
                    trial.clear();
                    filter_row(k, row, prev, channels, &mut trial);
                    trial[1..].iter().map(|&b| (b as i8).unsigned_abs() as u32).sum::<u32>()
                })
                .unwrap()
        });
        filter_row(kind, row, prev, channels, &mut raw);
    }
    let mut out = SIGNATURE.to_vec();
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&(width as u32).to_be_bytes());
    ihdr.extend_from_slice(&(height as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, if channels == 4 { 6 } else { 2 }, 0, 0, 0]);
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &zlib::compress(&raw));
    chunk(&mut out, b"IEND", &[]);
    out
}

/// Encode `rgb` (width * height * 3 bytes) as a PNG file.
pub fn encode_rgb(width: usize, height: usize, rgb: &[u8]) -> Vec<u8> {
    encode_with(width, height, 3, rgb, None)
}

/// Encode `rgba` (width * height * 4 bytes) as a PNG file.
pub fn encode_rgba(width: usize, height: usize, rgba: &[u8]) -> Vec<u8> {
    encode_with(width, height, 4, rgba, None)
}

impl Image {
    pub fn encode(&self) -> Vec<u8> {
        encode_rgba(self.width, self.height, &self.rgba)
    }
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// Decode a PNG file to 8-bit RGBA.
pub fn decode(file: &[u8]) -> Result<Image, String> {
    if file.get(..8) != Some(&SIGNATURE[..]) {
        return Err("not a PNG file".into());
    }
    let mut pos = 8;
    let mut header = None;
    let (mut palette, mut trns, mut idat) = (Vec::new(), Vec::new(), Vec::new());
    loop {
        let Some(head) = file.get(pos..pos + 8) else {
            return Err("truncated PNG (no IEND)".into());
        };
        let len = be32(head) as usize;
        let kind: [u8; 4] = head[4..8].try_into().unwrap();
        let Some(body) = file.get(pos + 8..pos + 8 + len) else {
            return Err(format!("truncated {} chunk", String::from_utf8_lossy(&kind)));
        };
        let Some(crc) = file.get(pos + 8 + len..pos + 12 + len) else {
            return Err("truncated PNG chunk CRC".into());
        };
        if crc32(&file[pos + 4..pos + 8 + len]) != be32(crc) {
            return Err(format!("{} chunk CRC mismatch", String::from_utf8_lossy(&kind)));
        }
        pos += 12 + len;
        match &kind {
            b"IHDR" if len == 13 => header = Some(body.to_vec()),
            b"PLTE" => palette = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => break,
            _ if kind[0] & 0x20 == 0 && &kind != b"IHDR" => {
                return Err(format!("unsupported critical chunk {}", String::from_utf8_lossy(&kind)));
            }
            _ => {}
        }
    }
    let Some(h) = header else { return Err("PNG has no IHDR".into()) };
    let (width, height) = (be32(&h[0..4]) as usize, be32(&h[4..8]) as usize);
    let (depth, colour, interlace) = (h[8] as usize, h[9], h[12]);
    if width == 0 || height == 0 || width > 1 << 16 || height > 1 << 16 {
        return Err(format!("unsupported PNG size {width}x{height}"));
    }
    if interlace != 0 {
        return Err("interlaced (Adam7) PNGs are not supported; save it without interlacing".into());
    }
    let channels = match (colour, depth) {
        (0, 1 | 2 | 4 | 8 | 16) | (3, 1 | 2 | 4 | 8) => 1,
        (2, 8 | 16) => 3,
        (4, 8 | 16) => 2,
        (6, 8 | 16) => 4,
        _ => return Err(format!("invalid PNG colour type {colour} / bit depth {depth}")),
    };
    if colour == 3 && palette.is_empty() {
        return Err("palette PNG without PLTE".into());
    }
    let bits_pp = channels * depth;
    let stride = (width * bits_pp).div_ceil(8);
    let bpp = bits_pp.div_ceil(8);
    let mut raw = zlib::decompress(&idat)?;
    if raw.len() < (stride + 1) * height {
        return Err("PNG image data is too short".into());
    }
    let mut prev = vec![0u8; stride];
    let mut rgba = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        let line = &mut raw[y * (stride + 1)..(y + 1) * (stride + 1)];
        let (kind, row) = line.split_first_mut().unwrap();
        unfilter_row(*kind, row, &prev, bpp)?;
        prev.copy_from_slice(row);
        // Sample k of the row, at its full bit depth.
        let sample = |k: usize| -> u16 {
            match depth {
                8 => row[k] as u16,
                16 => u16::from_be_bytes([row[2 * k], row[2 * k + 1]]),
                d => {
                    let bit = k * d;
                    ((row[bit / 8] >> (8 - d - bit % 8)) & ((1 << d) - 1) as u8) as u16
                }
            }
        };
        let to8 = |v: u16| -> u8 {
            match depth {
                16 => (v >> 8) as u8,
                8 => v as u8,
                d => (v as u32 * 255 / ((1 << d) - 1)) as u8,
            }
        };
        let key = |i: usize| trns.get(2 * i..2 * i + 2).map(|b| u16::from_be_bytes([b[0], b[1]]));
        for x in 0..width {
            let px = match colour {
                0 => {
                    let v = sample(x);
                    let g = to8(v);
                    [g, g, g, if key(0) == Some(v) { 0 } else { 255 }]
                }
                2 => {
                    let v = [sample(3 * x), sample(3 * x + 1), sample(3 * x + 2)];
                    let clear = trns.len() >= 6 && (0..3).all(|c| key(c) == Some(v[c]));
                    [to8(v[0]), to8(v[1]), to8(v[2]), if clear { 0 } else { 255 }]
                }
                3 => {
                    let i = sample(x) as usize;
                    let Some(c) = palette.get(3 * i..3 * i + 3) else {
                        return Err(format!("palette index {i} out of range"));
                    };
                    [c[0], c[1], c[2], trns.get(i).copied().unwrap_or(255)]
                }
                4 => {
                    let g = to8(sample(2 * x));
                    [g, g, g, to8(sample(2 * x + 1))]
                }
                _ => [0, 1, 2, 3].map(|c| to8(sample(4 * x + c))),
            };
            rgba.extend_from_slice(&px);
        }
    }
    Ok(Image { width, height, rgba })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(n: usize, seed: u32) -> Vec<u8> {
        let mut s = seed;
        (0..n)
            .map(|i| {
                s = s.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // Smooth-ish data so every filter gets used.
                if i % 3 == 0 {
                    (i / 7) as u8
                } else {
                    (s >> 24) as u8
                }
            })
            .collect()
    }

    #[test]
    fn crc_known_value() {
        assert_eq!(crc32(b"IEND"), 0xae42_6082);
    }

    #[test]
    fn round_trip_rgb_and_rgba() {
        for (w, h) in [(1, 1), (7, 3), (64, 33)] {
            let rgb = noise(w * h * 3, 7);
            let img = decode(&encode_rgb(w, h, &rgb)).unwrap();
            let want: Vec<u8> = rgb.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect();
            assert_eq!((img.width, img.height), (w, h));
            assert_eq!(img.rgba, want);
            let rgba = noise(w * h * 4, 9);
            let img = Image { width: w, height: h, rgba };
            assert_eq!(decode(&img.encode()).unwrap(), img);
        }
    }

    #[test]
    fn every_filter_type() {
        let (w, h) = (19, 11);
        let rgba = noise(w * h * 4, 3);
        for f in 0..5 {
            let png = encode_with(w, h, 4, &rgba, Some(f));
            assert_eq!(decode(&png).unwrap().rgba, rgba, "filter {f}");
        }
        assert_eq!(paeth(10, 20, 15), 15);
    }

    /// Build a PNG from IHDR fields and already-filtered scanlines.
    fn handmade(w: u32, h: u32, depth: u8, colour: u8, extra: &[(&[u8; 4], &[u8])], raw: &[u8]) -> Vec<u8> {
        let mut out = SIGNATURE.to_vec();
        let mut ihdr = w.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[depth, colour, 0, 0, 0]);
        chunk(&mut out, b"IHDR", &ihdr);
        for (k, d) in extra {
            chunk(&mut out, k, d);
        }
        chunk(&mut out, b"IDAT", &zlib::compress(raw));
        chunk(&mut out, b"IEND", &[]);
        out
    }

    #[test]
    fn palette_grey_and_low_depths() {
        // 2-bit palette, 3 pixels: indices 0, 1, 2 (+ tRNS on index 1).
        let plte: &[u8] = &[255, 0, 0, 0, 255, 0, 0, 0, 255];
        let png = handmade(3, 1, 2, 3, &[(b"PLTE", plte), (b"tRNS", &[255, 0])], &[0, 0b00_01_10_00]);
        assert_eq!(decode(&png).unwrap().rgba, [255, 0, 0, 255, 0, 255, 0, 0, 0, 0, 255, 255]);
        // 4-bit grey: 0x0 and 0xf scale to 0 and 255.
        let png = handmade(2, 1, 4, 0, &[], &[0, 0x0f]);
        assert_eq!(decode(&png).unwrap().rgba, [0, 0, 0, 255, 255, 255, 255, 255]);
        // 16-bit grey + alpha keeps the high bytes.
        let png = handmade(1, 1, 16, 4, &[], &[0, 0x12, 0x34, 0x80, 0x00]);
        assert_eq!(decode(&png).unwrap().rgba, [0x12, 0x12, 0x12, 0x80]);
    }

    #[test]
    fn rejects_interlaced_and_garbage() {
        let mut png = handmade(1, 1, 8, 2, &[], &[0, 1, 2, 3]);
        assert!(decode(&png).is_ok());
        // Set the interlace byte and fix the IHDR CRC.
        png[8 + 8 + 12] = 1;
        let crc = crc32(&png[12..12 + 17]);
        png[29..33].copy_from_slice(&crc.to_be_bytes());
        assert!(decode(&png).unwrap_err().contains("interlaced"));
        assert!(decode(b"not a png").is_err());
    }

    /// Cross-check against ImageMagick when it is installed: decode its
    /// PNGs of every colour type, and let it read ours.
    #[test]
    fn imagemagick_interop() {
        use std::process::Command;
        if Command::new("magick").arg("-version").output().map(|o| !o.status.success()).unwrap_or(true) {
            eprintln!("magick not found; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("remodel-png-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let magick = |args: &[&str]| {
            let o = Command::new("magick").args(args).output().unwrap();
            assert!(o.status.success(), "magick {args:?}: {}", String::from_utf8_lossy(&o.stderr));
            o.stdout
        };
        let source = "( -size 37x23 gradient:red-blue ( -size 37x23 gradient:white-black -rotate 90 -resize 37x23! ) \
                      -compose multiply -composite ( -size 37x23 radial-gradient:white-none ) -compose copy-opacity \
                      -composite )";
        let cases: &[(&str, &[&str], u8)] = &[
            ("rgba8", &["-depth", "8", "-define", "png:color-type=6"], 0),
            ("rgb8", &["-alpha", "off", "-depth", "8", "-define", "png:color-type=2"], 0),
            ("rgba16", &["-depth", "16", "-define", "png:color-type=6"], 1),
            ("gray8", &["-colorspace", "gray", "-alpha", "off", "-depth", "8", "-define", "png:color-type=0"], 0),
            ("graya8", &["-colorspace", "gray", "-depth", "8", "-define", "png:color-type=4"], 0),
            ("pal8", &["-colors", "200", "-define", "png:color-type=3"], 0),
            ("pal4", &["-alpha", "off", "-colors", "12", "-depth", "4", "-define", "png:color-type=3"], 0),
        ];
        for (name, opts, tolerance) in cases {
            let path = dir.join(format!("{name}.png"));
            let p = path.to_str().unwrap();
            let mut args: Vec<&str> = source.split_whitespace().collect();
            args.extend_from_slice(opts);
            args.push(p);
            magick(&args);
            let img = decode(&std::fs::read(&path).unwrap()).unwrap_or_else(|e| panic!("{name}: {e}"));
            let want = magick(&[p, "-depth", "8", "rgba:-"]);
            assert_eq!(img.rgba.len(), want.len(), "{name}");
            for (i, (&a, &b)) in img.rgba.iter().zip(&want).enumerate() {
                // ImageMagick reports fully transparent pixels' colour as-is.
                assert!(a.abs_diff(b) <= *tolerance, "{name}: byte {i}: {a} vs {b}");
            }
        }
        let path = dir.join("interlaced.png");
        magick(&["-size", "8x8", "xc:red", "-interlace", "PNG", path.to_str().unwrap()]);
        assert!(decode(&std::fs::read(&path).unwrap()).unwrap_err().contains("interlaced"));
        // Ours, read back by ImageMagick.
        let rgba = noise(33 * 17 * 4, 5);
        let path = dir.join("ours.png");
        std::fs::write(&path, encode_rgba(33, 17, &rgba)).unwrap();
        assert_eq!(magick(&[path.to_str().unwrap(), "-depth", "8", "rgba:-"]), rgba);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
