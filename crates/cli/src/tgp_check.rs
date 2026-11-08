//! `tgp-check`: a Model 2A TGP program's functions run on the MB86234
//! interpreter with random arguments and compared with the mathematics
//! they compute and with `model2::tgp2a_hle` (milestone 2's reading of
//! them). For `srallyc`'s program (`docs/hw/mb86234.md`, "How the
//! encodings were pinned"); a function another program doesn't have shows
//! as failing.

use mb86234::{Bus, Cpu, Stop};
use model2::tgp2a_hle::Hle;
use std::collections::VecDeque;

/// The shared RAM the program writes matrices to, and a data ROM from
/// [`mb86234::ROM_BASE`] (made up by the checks).
struct Ram(Vec<u32>, Vec<u32>);

impl Bus for Ram {
    fn read(&mut self, a: u32) -> u32 {
        match a.checked_sub(mb86234::ROM_BASE) {
            Some(o) => self.1.get(o as usize).copied().unwrap_or(0),
            None => self.0[(a & 0x7fff) as usize],
        }
    }
    fn write(&mut self, a: u32, v: u32) {
        if a < mb86234::ROM_BASE {
            self.0[(a & 0x7fff) as usize] = v;
        }
    }
}

struct Chip {
    cpu: Cpu,
    ram: Ram,
}

fn header(f: u32) -> u32 {
    f << 23 | f << 8 | f
}

impl Chip {
    fn new(program: &[u32], tables: Vec<u32>) -> Result<Self, String> {
        let mut c = Chip { cpu: Cpu::new(program.to_vec(), tables), ram: Ram(vec![0; 0x8000], Vec::new()) };
        // The load check: a word on command port 8 is answered with 8.
        c.cpu.fifo_in.push_back(8 << 23);
        if c.cpu.run(&mut c.ram, 100_000) != Stop::Input || c.cpu.fifo_out.pop_front() != Some(8) {
            return Err("the program doesn't answer the load check with 8".into());
        }
        c.cpu.fifo_out.clear();
        Ok(c)
    }

    fn call(&mut self, f: u32, args: &[u32]) -> Vec<u32> {
        self.cpu.fifo_in.push_back(header(f));
        self.cpu.fifo_in.extend(args);
        self.cpu.run(&mut self.ram, 1_000_000);
        self.cpu.fifo_out.drain(..).collect()
    }

    fn callf(&mut self, f: u32, args: &[f32]) -> Vec<f32> {
        let a: Vec<u32> = args.iter().map(|x| x.to_bits()).collect();
        self.call(f, &a).into_iter().map(f32::from_bits).collect()
    }
}

fn hle_call(h: &mut Hle, f: u32, args: &[f32]) -> Vec<f32> {
    let mut out = VecDeque::new();
    h.begin(f, &mut out);
    for a in args {
        h.data(a.to_bits(), &mut out);
    }
    out.into_iter().map(f32::from_bits).collect()
}

/// xorshift64*: reproducible arguments.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    /// Uniform in `[-r, r)`.
    fn f(&mut self, r: f64) -> f32 {
        ((self.next() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 * r - r) as f32
    }
    fn v(&mut self, n: usize, r: f64) -> Vec<f32> {
        (0..n).map(|_| self.f(r)).collect()
    }
}

/// Worst error over a check's cases, relative to the larger of the value
/// and 1 (so small results are judged absolutely).
#[derive(Default)]
struct Worst(f64);

impl Worst {
    fn see(&mut self, got: &[f32], want: &[f64]) {
        if got.len() != want.len() {
            self.0 = f64::INFINITY;
            return;
        }
        for (&g, &w) in got.iter().zip(want) {
            let e = (g as f64 - w).abs() / w.abs().max(1.0);
            self.0 = if e.is_nan() { f64::INFINITY } else { self.0.max(e) };
        }
    }
}

type M = [f64; 12];

fn mat(v: &[f32]) -> M {
    let mut m = [0.0; 12];
    for (o, &x) in m.iter_mut().zip(v) {
        *o = x as f64;
    }
    m
}

/// `v M + t` (row vector; element `3i + j` of the rotation, then t).
fn xform(m: &M, v: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|j| m[9 + j] + v[0] * m[j] + v[1] * m[3 + j] + v[2] * m[6 + j])
}

/// `R M` for 3x3 `r`, the translation kept.
fn pre(r: [[f64; 3]; 3], m: &M) -> M {
    let mut o = *m;
    for i in 0..3 {
        for j in 0..3 {
            o[3 * i + j] = (0..3).map(|k| r[i][k] * m[3 * k + j]).sum();
        }
    }
    o
}

/// The rotations as the program applies them (found by running it): about
/// x, y, z by `a`, `M' = R M`.
fn rot(axis: u32, a: f64) -> [[f64; 3]; 3] {
    let (s, c) = a.sin_cos();
    match axis {
        0 => [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]],
        1 => [[c, 0.0, s], [0.0, 1.0, 0.0], [-s, 0.0, c]],
        _ => [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]],
    }
}

/// A made-up course in the data ROM's format (`docs/hw/model2a.md` §9):
/// a block's table of cells, each four words (a polygon count, the byte
/// offsets from the ROM's start of its polygons and of their attribute
/// words, 0). A polygon is 16 words: four corners (x, y, z), then four
/// more words; its attribute word's bit 2 says it has four corners (else
/// three). Cell k is the 10 x 10 square at (10 (k mod 4), 10 (k div 4)) in
/// x and z, cut into strips, each a quad or two triangles. Right after
/// each cell's polygons (and attribute words) lies a quad covering the
/// whole course, which a search that ran past the count would find.
struct Course {
    rom: Vec<u32>,
    cells: Vec<Vec<Polygon>>,
}

/// A polygon of [`Course`]: its corners in x and z, and the ROM word
/// offsets of its 16 words and of its attribute word.
struct Polygon {
    corners: Vec<[f64; 2]>,
    at: usize,
    attr: usize,
}

const BLOCK: u32 = 0x100;

fn course(rng: &mut Rng) -> Course {
    let mut rom = vec![0u32; 0x200];
    let mut cells = Vec::new();
    for k in 0..16u32 {
        let (x0, z0) = ((k % 4) as f64 * 10.0, (k / 4) as f64 * 10.0);
        let n = 1 + rng.next() % 4;
        let mut polys: Vec<Vec<[f64; 2]>> = Vec::new();
        for j in 0..n {
            let xa = x0 + 10.0 * j as f64 / n as f64;
            let xb = x0 + 10.0 * (j + 1) as f64 / n as f64;
            let (a, b, c, d) = ([xa, z0], [xb, z0], [xb, z0 + 10.0], [xa, z0 + 10.0]);
            if rng.next() & 1 == 0 {
                polys.push(vec![a, b, c, d]);
            } else {
                polys.push(vec![a, b, c]);
                polys.push(vec![a, c, d]);
            }
        }
        polys.push(vec![[-1e3, -1e3], [1e3, -1e3], [1e3, 1e3], [-1e3, 1e3]]);
        let at = rom.len();
        let attrs = at + 16 * polys.len();
        let entry = (BLOCK + 4 * k) as usize;
        rom[entry..entry + 4].copy_from_slice(&[polys.len() as u32 - 1, 4 * at as u32, 4 * attrs as u32, 0]);
        let mut list = Vec::new();
        for (i, p) in polys.iter().enumerate() {
            let mut rec: Vec<u32> = (0..16).map(|_| rng.f(100.0).to_bits()).collect();
            for (c, xz) in p.iter().enumerate() {
                rec[3 * c] = (xz[0] as f32).to_bits();
                rec[3 * c + 2] = (xz[1] as f32).to_bits();
            }
            rom.extend(rec);
            if i + 1 < polys.len() {
                list.push(Polygon { corners: p.clone(), at: at + 16 * i, attr: attrs + i });
            }
        }
        for p in &polys {
            rom.push((rng.next() as u32 & !4) | if p.len() == 4 { 4 } else { 0 });
        }
        cells.push(list);
    }
    Course { rom, cells }
}

/// The cross products the search tests, `(c_i - p) x (c_i+1 - p)` in x and
/// z: all positive inside a polygon.
fn edges(p: [f64; 2], poly: &[[f64; 2]]) -> Vec<f64> {
    (0..poly.len())
        .map(|i| {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            (a[0] - p[0]) * (b[1] - p[1]) - (b[0] - p[0]) * (a[1] - p[1])
        })
        .collect()
}

/// Functions 7 (a word of the data ROM at the offset the list gives) and
/// 0x52/0x53 (find the polygon under a point in three cells, then give
/// its 16 words and attribute word) on [`course`].
fn course_search(c: &mut Chip, rng: &mut Rng) -> (Worst, Worst) {
    let course = course(rng);
    c.ram.1 = course.rom.clone();
    let (mut w7, mut w) = (Worst::default(), Worst::default());
    let bad = |w: &mut Worst| w.0 = f64::INFINITY;
    for _ in 0..200 {
        let o = (rng.next() % course.rom.len() as u64) as u32;
        c.call(0x03, &[0]);
        c.ram.0[0x6000] = o;
        if c.call(0x07, &[]) != [course.rom[o as usize]] {
            bad(&mut w7);
        }
    }
    let mut found = 0;
    for i in 0..500 {
        let p = [rng.f(20.0) as f64 + 20.0, rng.f(20.0) as f64 + 20.0];
        let k = (p[0] / 10.0).floor() as usize % 4 + 4 * ((p[1] / 10.0).floor() as usize % 4);
        let hit = course.cells[k].iter().find(|q| edges(p, &q.corners).iter().all(|&e| e > 0.0));
        if course.cells[k].iter().any(|q| edges(p, &q.corners).iter().any(|e| e.abs() < 1e-3)) {
            continue;
        }
        // Three cells to search, the one under the point among them in
        // two cases out of three.
        let mut cand: Vec<usize> = (0..3).map(|_| (k + 1 + (rng.next() % 15) as usize) % 16).collect();
        if i % 3 != 2 {
            cand[(rng.next() % 3) as usize] = k;
        }
        let slot = 2 * (rng.next() % 4) as u32;
        c.call(0x03, &[0]);
        let words = [
            (p[0] as f32).to_bits(),
            rng.f(10.0).to_bits(),
            (p[1] as f32).to_bits(),
            BLOCK,
            0,
            4 * cand[2] as u32,
            4 * cand[0] as u32,
            4 * cand[1] as u32,
            slot,
        ];
        c.ram.0[0x6000..0x6009].copy_from_slice(&words);
        c.call(0x52, &[]);
        let table = [c.cpu.ram[104 + slot as usize], c.cpu.ram[105 + slot as usize]];
        match (cand.contains(&k), hit) {
            (true, Some(&Polygon { at, attr, .. })) => {
                found += 1;
                let mut want = course.rom[at..at + 16].to_vec();
                want.push(course.rom[attr]);
                if c.call(0x53, &[slot]) != want {
                    bad(&mut w);
                }
            }
            (true, None) => bad(&mut w),
            // No polygon in the three cells: the program marks the slot
            // with 104 (its table's own address) in both words.
            (false, _) => {
                if table != [104, 104] {
                    bad(&mut w);
                }
            }
        }
    }
    if found < 100 {
        bad(&mut w);
    }
    (w7, w)
}

pub fn check(program: &[u32], tables: Vec<u32>) -> Result<bool, String> {
    let mut c = Chip::new(program, tables)?;
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    const N: usize = 500;
    let mut rows: Vec<(String, f64, f64, Option<f64>)> = Vec::new();
    let mut row =
        |name: &str, w: Worst, tol: f64, hle: Option<Worst>| rows.push((name.into(), w.0, tol, hle.map(|h| h.0)));

    // The library: two-word and one-word functions.
    type Lib = (u32, &'static str, usize, f64, fn(&[f64]) -> f64);
    let lib: [Lib; 9] = [
        (0x08, "08 add", 2, 1e-7, |a| a[0] + a[1]),
        (0x09, "09 subtract", 2, 1e-7, |a| a[0] - a[1]),
        (0x0a, "0A multiply", 2, 1e-7, |a| a[0] * a[1]),
        (0x0b, "0B divide", 2, 1e-7, |a| a[0] / a[1]),
        (0x0d, "0D abs", 1, 0.0, |a| a[0].abs()),
        (0x0e, "0E negate", 1, 0.0, |a| -a[0]),
        (0x11, "11 square root", 1, 2e-7, |a| a[0].abs().sqrt()),
        (0x15, "15 sine", 1, 1e-5, |a| a[0].sin()),
        (0x16, "16 cosine", 1, 1e-5, |a| a[0].cos()),
    ];
    for (f, name, n, tol, want) in lib {
        let mut w = Worst::default();
        for _ in 0..N {
            let a = rng.v(n, 100.0);
            let a64: Vec<f64> = a.iter().map(|&x| x as f64).collect();
            w.see(&c.callf(f, &a), &[want(&a64)]);
        }
        row(name, w, tol, None);
    }
    let mut w = Worst::default();
    for _ in 0..N {
        let a = rng.f(1.4);
        w.see(&c.callf(0x17, &[a]), &[(a as f64).tan()]);
    }
    row("17 tangent", w, 1e-5, None);
    let (mut w, mut wi) = (Worst::default(), Worst::default());
    for _ in 0..N {
        let i = (rng.next() as i32) >> 8;
        w.see(&c.call(0x13, &[i as u32]).into_iter().map(f32::from_bits).collect::<Vec<_>>(), &[i as f64]);
        let x = rng.f(1e6);
        let got = c.callf(0x14, &[x]);
        let n = got.first().map(|v| v.to_bits() as i32 as f32);
        wi.see(&n.into_iter().collect::<Vec<_>>(), &[(x as f64).round_ties_even()]);
    }
    row("13 integer to float", w, 0.0, None);
    row("14 float to integer (nearest)", wi, 0.0, None);
    let mut w = Worst::default();
    for _ in 0..N {
        let a = (rng.next() & 0xffff) as u32;
        let t = a as f64 * std::f64::consts::TAU / 65536.0;
        let s: Vec<f32> = [0x35, 0x36].iter().map(|&f| f32::from_bits(c.call(f, &[a])[0])).collect();
        w.see(&s, &[t.sin(), t.cos()]);
    }
    row("35/36 sine, cosine (ROM)", w, 1e-7, None);

    // The matrix functions, against the maths and the function model.
    let mut h = Hle::default();
    let both = |c: &mut Chip, h: &mut Hle, f: u32, a: &[f32]| (c.callf(f, a), hle_call(h, f, a));
    let get = |c: &mut Chip, h: &mut Hle| both(c, h, 0x26, &[]);
    let (mut w, mut wh) = (Worst::default(), Worst::default());
    for _ in 0..N / 10 {
        let m = rng.v(12, 10.0);
        let m64 = mat(&m);
        both(&mut c, &mut h, 0x23, &[]);
        both(&mut c, &mut h, 0x22, &m);
        let (g, gh) = get(&mut c, &mut h);
        w.see(&g, &m64);
        wh.see(&gh, &m64);
        for d in 0..9 {
            both(&mut c, &mut h, 0x20, &[]);
            let other = rng.v(12, 10.0);
            both(&mut c, &mut h, 0x22, &other);
            let _ = d;
        }
        for _ in 0..9 {
            both(&mut c, &mut h, 0x21, &[]);
        }
        let (g, gh) = get(&mut c, &mut h);
        w.see(&g, &m64);
        wh.see(&gh, &m64);
        both(&mut c, &mut h, 0x25, &[]);
        let (g, gh) = get(&mut c, &mut h);
        let id = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0];
        w.see(&g, &id);
        wh.see(&gh, &id);
    }
    row("20-23,25,26 stack, set, get, identity", w, 0.0, Some(wh));

    type Case = (u32, &'static str, usize, f64, fn(&M, &[f64]) -> M);
    let cases: [Case; 5] = [
        (0x27, "27 translate", 3, 1e-6, |m, a| {
            let mut o = *m;
            let t = xform(m, [a[0], a[1], a[2]]);
            o[9..].copy_from_slice(&t);
            o
        }),
        (0x28, "28 scale (rows)", 3, 1e-6, |m, a| {
            let mut o = *m;
            for i in 0..9 {
                o[i] *= a[i / 3];
            }
            o
        }),
        (0x29, "29 rotate x", 1, 2e-5, |m, a| pre(rot(0, a[0]), m)),
        (0x2a, "2A rotate y", 1, 2e-5, |m, a| pre(rot(1, a[0]), m)),
        (0x2b, "2B rotate z", 1, 2e-5, |m, a| pre(rot(2, a[0]), m)),
    ];
    for (f, name, n, tol, want) in cases {
        let (mut w, mut wh) = (Worst::default(), Worst::default());
        for _ in 0..N / 5 {
            let m = rng.v(12, 2.0);
            let a = rng.v(n, 4.0);
            let a64: Vec<f64> = a.iter().map(|&x| x as f64).collect();
            both(&mut c, &mut h, 0x22, &m);
            both(&mut c, &mut h, f, &a);
            let (g, gh) = get(&mut c, &mut h);
            let e = want(&mat(&m), &a64);
            w.see(&g, &e);
            wh.see(&gh, &e);
        }
        row(name, w, tol, Some(wh));
    }
    // The table rotations take a 16-bit angle; the same rotation.
    for (f, axis) in [(0x32, 0), (0x33, 1), (0x34, 2)] {
        let mut w = Worst::default();
        for _ in 0..N / 5 {
            let m = rng.v(12, 2.0);
            let a = (rng.next() & 0xffff) as u32;
            c.callf(0x22, &m);
            c.call(f, &[a]);
            let g = c.callf(0x26, &[]);
            w.see(&g, &pre(rot(axis, a as f64 * std::f64::consts::TAU / 65536.0), &mat(&m)));
        }
        row(&format!("{f:02X} rotate {} (ROM angle)", ["x", "y", "z"][axis as usize]), w, 1e-6, None);
    }
    let (mut w, mut wh) = (Worst::default(), Worst::default());
    for _ in 0..N / 5 {
        let m = rng.v(12, 2.0);
        let v = rng.v(3, 50.0);
        both(&mut c, &mut h, 0x22, &m);
        let (g, gh) = both(&mut c, &mut h, 0x2c, &v);
        let e = xform(&mat(&m), [v[0] as f64, v[1] as f64, v[2] as f64]);
        w.see(&g, &e);
        wh.see(&gh, &e);
    }
    row("2C transform a point", w, 3e-6, Some(wh));
    let (mut w, mut wh) = (Worst::default(), Worst::default());
    for _ in 0..N {
        let v = rng.v(3, 50.0);
        let l = v.iter().map(|&x| x as f64 * x as f64).sum::<f64>().sqrt();
        let e: Vec<f64> = v.iter().map(|&x| x as f64 / l).collect();
        let (g, gh) = both(&mut c, &mut h, 0x2f, &v);
        w.see(&g, &e);
        wh.see(&gh, &e);
    }
    row("2F normalise", w, 3e-7, Some(wh));
    let (mut w, mut wh) = (Worst::default(), Worst::default());
    for _ in 0..N / 5 {
        let i = (rng.next() & 3) as u32;
        let (a, b) = (rng.v(3, 10.0), rng.v(3, 10.0));
        let fi = f32::from_bits(i);
        both(&mut c, &mut h, 0x42, &[fi, a[0], a[1], a[2]]);
        both(&mut c, &mut h, 0x44, &[fi, b[0], b[1], b[2]]);
        let (g, gh) = both(&mut c, &mut h, 0x43, &[fi]);
        let e: Vec<f64> = (0..3).map(|k| (a[k] + b[k]) as f64).collect();
        w.see(&g, &e);
        wh.see(&gh, &e);
        // 0x48 transforms register i by the current matrix in place.
        let m = rng.v(12, 2.0);
        c.callf(0x22, &m);
        c.callf(0x48, &[fi]);
        let g = c.callf(0x43, &[fi]);
        w.see(&g, &xform(&mat(&m), [e[0], e[1], e[2]]));
    }
    row("42-44,48 vector registers", w, 1e-6, Some(wh));
    let (mut w, mut wh) = (Worst::default(), Worst::default());
    let (mut wc, mut wch) = (Worst::default(), Worst::default());
    for _ in 0..N {
        let a = rng.v(6, 100.0);
        let d = |i: usize| (a[2 * i + 1] - a[2 * i]) as f64;
        let e = [(d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt()];
        let (g, gh) = both(&mut c, &mut h, 0x58, &a);
        w.see(&g, &e);
        wh.see(&gh, &e);
        // 0x5C: a 2D cross product of differences (the order as the
        // program takes its words).
        let a64: Vec<f64> = a.iter().map(|&x| x as f64).collect();
        let e = [(a64[5] - a64[0]) * (a64[4] - a64[3]) - (a64[2] - a64[3]) * (a64[1] - a64[0])];
        let (g, gh) = both(&mut c, &mut h, 0x5c, &a);
        wc.see(&g, &e);
        wch.see(&gh, &e);
    }
    row("58 distance", w, 3e-7, Some(wh));
    // 0x50: the distance from (w0, w1, w2) to (w3, w4, w5), the second
    // point kept in RAM1 and read through type 0x10's op1 below 0x40.
    let mut w = Worst::default();
    for _ in 0..N {
        let a = rng.v(6, 100.0);
        let d = |i: usize| (a[i + 3] - a[i]) as f64;
        w.see(&c.callf(0x50, &a), &[(d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt()]);
    }
    row("50 distance (two points)", w, 3e-7, None);
    row("5C cross product", wc, 1e-5, Some(wch));
    // 0x54: turn the matrix so that the direction maps to +z.
    let mut w = Worst::default();
    for _ in 0..N / 5 {
        let v = rng.v(3, 10.0);
        c.callf(0x25, &[]);
        c.callf(0x54, &v);
        let m = mat(&c.callf(0x26, &[]));
        let l = v.iter().map(|&x| x as f64 * x as f64).sum::<f64>().sqrt();
        let got = xform(&m, [v[0] as f64, v[1] as f64, v[2] as f64]);
        w.see(&got.map(|x| x as f32), &[0.0, 0.0, l]);
    }
    row("54 point along a direction", w, 1e-5, None);
    // 0x51: the same turn with the other sign convention: the matrix's
    // third row becomes the direction (normalised).
    let mut w = Worst::default();
    for _ in 0..N / 5 {
        let v = rng.v(3, 10.0);
        c.callf(0x25, &[]);
        c.callf(0x51, &v);
        let m = c.callf(0x26, &[]);
        let l = v.iter().map(|&x| x as f64 * x as f64).sum::<f64>().sqrt();
        let e: Vec<f64> = v.iter().map(|&x| x as f64 / l).collect();
        w.see(&m[6..9], &e);
    }
    row("51 point along a direction (z row)", w, 1e-5, None);
    // 0x59, 0x5B: a dot product and a cross product, of vectors whose
    // components the game interleaves.
    let (mut wd, mut wx) = (Worst::default(), Worst::default());
    for _ in 0..N {
        let a = rng.v(6, 10.0);
        let a: Vec<f64> = a.iter().map(|&x| x as f64).collect();
        let a32: Vec<f32> = a.iter().map(|&x| x as f32).collect();
        wd.see(&c.callf(0x59, &a32), &[a[0] * a[1] + a[2] * a[3] + a[4] * a[5]]);
        let (u, v) = ([a[5], a[0], a[2]], [a[4], a[3], a[1]]);
        let x = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        wx.see(&c.callf(0x5b, &a32), &x);
    }
    row("59 dot product (interleaved)", wd, 1e-5, None);
    row("5B cross product (interleaved)", wx, 1e-5, None);
    // The composites against the functions they call, and 0x05's matrix
    // in shared RAM at the address the list at 0x6000 gives.
    let mut w = Worst::default();
    for k in 0..N / 10 {
        let a = rng.v(10, 100.0);
        let m = rng.v(12, 2.0);
        c.call(0x03, &[0]);
        c.ram.0[0x6000] = 0x100 + 4 * 13 * k as u32;
        c.callf(0x22, &m);
        c.callf(0x5d, &a[..7]);
        let at = 0x40 + 13 * k;
        let out: Vec<f32> = c.ram.0[at..at + 13].iter().map(|&x| f32::from_bits(x)).collect();
        let cmd = c.ram.0[at];
        c.callf(0x22, &m);
        c.callf(0x20, &[]);
        c.callf(0x5e, &a[..7]);
        let e = mat(&c.callf(0x26, &[]));
        c.callf(0x21, &[]);
        w.see(&out[1..], &e);
        if (cmd >> 23) & 0xff != 0x0b {
            w.0 = f64::INFINITY;
        }
    }
    row("05/5D/5E matrix to the display list", w, 0.0, None);
    let (w7, w52) = course_search(&mut c, &mut rng);
    row("07 data ROM word", w7, 0.0, None);
    row("52/53 course search (polygon under)", w52, 0.0, None);

    println!("{:<40} {:>12} {:>9}  {:>12}", "function", "worst error", "tolerance", "tgp2a_hle");
    let mut ok = true;
    for (name, e, tol, hle) in rows {
        let pass = e <= tol;
        ok &= pass;
        let hle = hle.map_or(String::new(), |h| format!("{h:12.3e}"));
        println!("{name:<40} {e:>12.3e} {tol:>9.0e}  {hle:>12}  {}", if pass { "ok" } else { "FAIL" });
    }
    println!(
        "{} words run; unknown words {}, unknown external addresses {}",
        c.cpu.steps,
        c.cpu.unknown.len(),
        c.cpu.unknown_ext.len()
    );
    Ok(ok && c.cpu.unknown.is_empty())
}
