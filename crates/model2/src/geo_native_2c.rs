//! A native geometrizer for the Model 2C: the display-list programs the 2C
//! games upload to their geometrizer MB86235 (TGP), implemented in Rust
//! from their behaviour (docs/hw/geometrizer-2c.md). It takes the same
//! inputs as the TGP (the list from buffer RAM, polygon memory, its own
//! state from earlier commands) and gives the same rasteriser stream, word
//! for word: every floating-point step is the TGP's, in the program's
//! order, through the `mb86235::float` functions the interpreter and the
//! recompiled code use.
//!
//! The programs are one family: a shared version (`skisuprg`, `segawski`,
//! `waverunr`, `hotd`, `bel`, and `stcc`'s, which differs only in a status
//! output), `overrev`'s (different lighting and back-face test, a command
//! log) and `topskatr`'s (another memory layout, a skinned compact model
//! format and a second matrix). Each is picked by its program hash (the
//! name of its `tgp_<hash>` recompiled crate), never by game.
//!
//! Its state is kept where the program keeps it: the TGP's A and B RAMs,
//! its PR register queue (the matrices) and the few registers that live
//! across commands, so a TGP's state can be taken over ([`Native::import`])
//! and handed back ([`Native::export`]).

use crate::geo_native::Io;
use crate::smooth::Kind;
use mb86235::cpu::{External, Tgp};
use mb86235::float::{abs, add, addz, cfib, cif, clamp_negative, mul, negative, rsq, sub, subz};

/// A program this module implements, by `mb86235::cpu::program_hash` of
/// the uploaded words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// `skisuprg`'s, `segawski`'s, `waverunr`'s, `hotd`'s and `bel`'s
    /// program, and `stcc`'s (one more status-port write in its test
    /// command, which nothing sees).
    Shared,
    /// `overrev`'s: lighting from the clamped diffuse term, quads facing
    /// the eye only when strictly in front, a log of the commands in B RAM.
    Overrev,
    /// `topskatr`'s: the shared program's maths with its state elsewhere in
    /// memory, plus a skinned compact model format (fn 0x11) and a second
    /// matrix (fn 0x1B, 0x1C).
    TopSkater,
    /// The Model 2A's fixed-function geometrizer (docs/hw/model2a.md): the
    /// shared program's maths (*unconfirmed*: the chip is not a program
    /// here, but its lists parse with the same commands and the lit
    /// pictures look right), and no command check, since the hardware takes
    /// the command from the port the word was written to and the TGP's
    /// matrix words (function 5) carry whatever its register held in the
    /// low bits.
    Fixed,
}

/// Program hashes of the programs implemented here.
pub const PROGRAMS: &[(u64, Program)] = &[
    (0x2917_7553_5904_96aa, Program::Shared),
    (0xb850_fb34_8400_5f7b, Program::Shared),
    (0xa49b_bffc_5f8b_1d2a, Program::Overrev),
    (0xc655_9431_3775_3c1d, Program::TopSkater),
];

/// The program a TGP holds (by its `program_hash`), if this module
/// implements it.
pub fn recognise(program_hash: u64) -> Option<Program> {
    PROGRAMS.iter().find(|p| p.0 == program_hash).map(|p| p.1)
}

const ZERO: u32 = 0;
const ONE: u32 = 0x3f80_0000;
const MINUS_ONE: u32 = 0xbf80_0000;
const TWO: u32 = 0x4000_0000;
const HALF: u32 = 0x3f00_0000;
const THREE: u32 = 0x4040_0000;

/// The luma word of a back face drawn unlit: the back-face bit alone.
const BACK_FACE: u32 = 0x80_0000;

/// Most words a data loop may move, or polygons an object may have: far
/// more than the TGP gets through in a frame (a count of 0 is 2^32), so
/// only a broken list reaches it.
const MAX_LOOP: u64 = 1 << 22;

/// Words of A RAM, then B RAM, in [`Native::ram`].
const RAM_WORDS: usize = 1024;

/// The data RAMs of the TGP.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Ram {
    A,
    B,
}

/// A word of data RAM the program keeps something in.
#[derive(Clone, Copy)]
struct At(Ram, u32);

impl At {
    #[inline(always)]
    fn index(self, k: u32) -> usize {
        let i = ((self.1 + k) & 0x3ff) as usize;
        match self.0 {
            Ram::A => i,
            Ram::B => RAM_WORDS + i,
        }
    }
}

/// Where a program keeps its state.
struct Layout {
    /// Translation (3 words); `topskatr`'s second one follows it.
    translation: At,
    /// Focal lengths x, y.
    focus: At,
    light: At,
    /// The previous edge's vertices (3 words each), computed-normal modes.
    edge0: At,
    edge1: At,
    /// Set by the test command, cleared by object data: only a status
    /// output reads it.
    tested: Option<At>,
    lod: At,
    /// The object's polygon count, counted down.
    count: At,
    /// The polygon's attribute bits 17-0, as sent.
    attr: At,
    /// Mode bit 1 and up (bit 0 of it: normals computed).
    computed: At,
    /// 0xF with specular lighting on (mode bit 0), else 0.
    specular: At,
    /// Lighting parameters (4 words an entry) and distance coefficients.
    params: At,
    coef: At,
    /// A RAM address of the table of squarings per specular control value.
    squarings: u32,
    /// The word fn 0x05 takes the external base from.
    eb: At,
    /// `topskatr`'s fn 0x11: the attribute word and coefficient × LOD.
    attr_word: At,
    coef_lod: At,
    /// `topskatr`'s fn 0x1B: where it starts writing the queue.
    second_matrix: At,
}

const SHARED: Layout = Layout {
    translation: At(Ram::A, 0x20),
    focus: At(Ram::A, 0x23),
    light: At(Ram::A, 0x26),
    edge0: At(Ram::A, 0x29),
    edge1: At(Ram::A, 0x2c),
    tested: Some(At(Ram::A, 0x2f)),
    lod: At(Ram::A, 0xe6),
    count: At(Ram::A, 0xe7),
    attr: At(Ram::A, 0xe3),
    computed: At(Ram::A, 0xe4),
    specular: At(Ram::A, 0xe5),
    params: At(Ram::A, 0x30),
    coef: At(Ram::A, 0xb0),
    squarings: 0xd0,
    eb: At(Ram::A, 0xd8),
    attr_word: At(Ram::A, 0),
    coef_lod: At(Ram::A, 0),
    second_matrix: At(Ram::B, 0),
};

const TOP_SKATER: Layout = Layout {
    translation: At(Ram::B, 0x30),
    focus: At(Ram::A, 0x33),
    light: At(Ram::A, 0x36),
    edge0: At(Ram::A, 0x2c),
    edge1: At(Ram::B, 0x2c),
    tested: Some(At(Ram::A, 0x3f)),
    lod: At(Ram::A, 0x2f),
    count: At(Ram::A, 0x31),
    attr: At(Ram::A, 0x1b),
    computed: At(Ram::A, 0x1c),
    specular: At(Ram::A, 0x1d),
    params: At(Ram::A, 0x60),
    coef: At(Ram::B, 0x40),
    squarings: 0xe0,
    eb: At(Ram::A, 0x10),
    attr_word: At(Ram::A, 0x30),
    coef_lod: At(Ram::B, 0x2f),
    second_matrix: At(Ram::B, 0x0c),
};

/// `overrev`'s A RAM words of a frame counter its frame start counts down
/// (only a status output reads them; the counter is never started).
const OVERREV_COUNTER: usize = 0x3f0;
/// `overrev`'s command log: B RAM words from 0 until this, written through
/// AR3.
const OVERREV_LOG_END: u32 = 0x3ef;

/// Why the native geometrizer stopped producing output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A command word failed the command check: the TGP then loops
    /// forever, drawing nothing more until the next upload.
    BadCommand(u32),
    /// A command the program doesn't have, or one whose operands fail its
    /// checks (fn 0x05, 0x0D): the TGP loops forever too.
    Hang(u32),
    /// A loop with no end in practice (a count of 0 is 2^32).
    Unsupported(u32),
}

/// The native geometrizer's state (the program's own, see the module
/// comment).
#[derive(Clone)]
pub struct Native {
    pub program: Program,
    /// A RAM, then B RAM.
    ram: Vec<u32>,
    /// The PR register queue: the rotation matrices.
    pr: [u32; 24],
    /// The external base register (fn 0x05 sets it from memory, others to 0).
    eb: u32,
    /// `overrev`'s command log pointer.
    ar3: u32,
    /// The next frame is the first since the upload: it starts from the
    /// start-up code's wait, not the frame-start handshake.
    first: bool,
    pub stop: Option<Stop>,
}

/// The transform an object is drawn with.
struct Ctx {
    /// Translation and rotation for each matrix select (`topskatr`'s fn
    /// 0x11 picks one per vertex; everything else uses the first).
    t: [[u32; 3]; 2],
    m: [[u32; 9]; 2],
    fx: u32,
    fy: u32,
    l: [u32; 3],
    lod: u32,
}

impl Ctx {
    /// Model to view space: translation first, then the x, y, z terms.
    #[inline(always)]
    fn xform(&self, s: usize, p: [u32; 3]) -> [u32; 3] {
        let (t, m) = (&self.t[s], &self.m[s]);
        let row = |t: u32, a: u32, b: u32, c: u32| add(add(add(t, mul(p[0], a)), mul(p[1], b)), mul(p[2], c));
        [row(t[0], m[0], m[3], m[6]), row(t[1], m[1], m[4], m[7]), row(t[2], m[2], m[5], m[8])]
    }

    /// A normal into view space (rotation only).
    #[inline(always)]
    fn rotate(&self, n: [u32; 3]) -> [u32; 3] {
        let m = &self.m[0];
        let row = |a: u32, b: u32, c: u32| add(add(mul(n[0], a), mul(n[1], b)), mul(n[2], c));
        [row(m[0], m[3], m[6]), row(m[1], m[4], m[7]), row(m[2], m[5], m[8])]
    }

    /// A vertex as the rasteriser takes it: x and y times the focal
    /// lengths, z times 1, each a 24-bit float.
    #[inline(always)]
    fn emit(&self, io: &mut Io, v: [u32; 3]) {
        io.emit(mul(v[0], self.fx) >> 8);
        io.emit(mul(v[1], self.fy) >> 8);
        io.emit(mul(v[2], ONE) >> 8);
    }
}

#[inline(always)]
fn dot(a: [u32; 3], b: [u32; 3]) -> u32 {
    add(add(mul(a[0], b[0]), mul(a[1], b[1])), mul(a[2], b[2]))
}

/// The polygon memory port: `EB + EO`, then EO += 1.
struct Words {
    eb: u32,
    eo: u32,
}

impl Words {
    #[inline(always)]
    fn next(&mut self, io: &mut Io) -> u32 {
        let v = io.poly.read((self.eb + self.eo) & 0xff_ffff);
        self.eo = (self.eo + 1) & 0xff_ffff;
        v
    }
    #[inline(always)]
    fn vertex(&mut self, io: &mut Io) -> [u32; 3] {
        [self.next(io), self.next(io), self.next(io)]
    }
    #[inline(always)]
    fn skip(&mut self, n: u32) {
        self.eo = (self.eo + n) & 0xff_ffff;
    }
}

/// A count as the program's do-while loops take it: 0 runs 2^32 times.
fn count(n: u32) -> u64 {
    if n == 0 {
        1 << 32
    } else {
        n as u64
    }
}

impl Native {
    /// A native geometrizer for `program`; [`Native::import`] gives it the
    /// TGP's state.
    pub fn new(program: Program) -> Self {
        Native { program, ram: vec![0; 2 * RAM_WORDS], pr: [0; 24], eb: 0, ar3: 0, first: true, stop: None }
    }

    /// The 2A's geometrizer: no program, so the state a program's start-up
    /// code would set up (the table of squarings per specular control)
    /// is set here.
    pub fn fixed() -> Self {
        let mut n = Native::new(Program::Fixed);
        for (k, &v) in [1u32, 1, 1, 1, 2, 2, 2, 3].iter().enumerate() {
            n.ram[SHARED.squarings as usize + k] = v;
        }
        n.first = false;
        n
    }

    fn layout(&self) -> &'static Layout {
        match self.program {
            Program::TopSkater => &TOP_SKATER,
            _ => &SHARED,
        }
    }

    /// Take over a TGP's state between frames. `fresh`: it has just been
    /// uploaded and run through its start-up code only (the first frame
    /// then skips the frame-start handshake, as the TGP's does).
    pub fn import(&mut self, t: &Tgp, fresh: bool) {
        self.ram[..RAM_WORDS].copy_from_slice(&t.aram);
        self.ram[RAM_WORDS..].copy_from_slice(&t.bram);
        self.pr = t.pr;
        self.eb = t.eb;
        self.ar3 = t.ar[3];
        self.first = fresh;
        if matches!(self.stop, Some(Stop::Unsupported(_))) {
            self.stop = None;
        }
    }

    /// Write the state into a TGP, where the program keeps it.
    pub fn export(&self, t: &mut Tgp) {
        t.aram.copy_from_slice(&self.ram[..RAM_WORDS]);
        t.bram.copy_from_slice(&self.ram[RAM_WORDS..]);
        t.pr = self.pr;
        t.eb = self.eb;
        if self.program == Program::Overrev {
            t.ar[3] = self.ar3;
        }
    }

    /// The first state word that differs from a TGP's: (where, TGP's,
    /// native's), A RAM at its address, B RAM at 0x1000 + its address, PR
    /// at 0x2000 + its index, `overrev`'s AR3 at 0x3000.
    pub fn compare(&self, t: &Tgp) -> Option<(u32, u32, u32)> {
        let a = t.aram.iter().zip(&self.ram[..RAM_WORDS]).enumerate().map(|(i, (&x, &y))| (i as u32, x, y));
        let b = t.bram.iter().zip(&self.ram[RAM_WORDS..]).enumerate().map(|(i, (&x, &y))| (0x1000 + i as u32, x, y));
        let pr = t.pr.iter().zip(&self.pr).enumerate().map(|(i, (&x, &y))| (0x2000 + i as u32, x, y));
        let ar3 = (self.program == Program::Overrev).then_some((0x3000, t.ar[3], self.ar3));
        a.chain(b).chain(pr).chain(ar3).find(|(_, x, y)| x != y)
    }

    #[inline(always)]
    fn get(&self, at: At, k: u32) -> u32 {
        self.ram[at.index(k)]
    }
    #[inline(always)]
    fn set(&mut self, at: At, k: u32, v: u32) {
        self.ram[at.index(k)] = v;
    }

    /// A read through an address register (14 bits): B RAM (10 bits), A RAM
    /// below 0x400, external memory above (`EB`'s bank plus the address).
    fn read_at(&self, io: &mut Io, ram: Ram, addr: u32) -> u32 {
        let a = addr & 0x3fff;
        match ram {
            Ram::B => self.ram[RAM_WORDS + (a & 0x3ff) as usize],
            Ram::A if a < 0x400 => self.ram[a as usize],
            Ram::A => io.poly.read((self.eb & 0xff_c000) + a),
        }
    }

    fn write_at(&mut self, io: &mut Io, ram: Ram, addr: u32, v: u32) {
        let a = addr & 0x3fff;
        match ram {
            Ram::B => self.ram[RAM_WORDS + (a & 0x3ff) as usize] = v,
            Ram::A if a < 0x400 => self.ram[a as usize] = v,
            Ram::A => io.poly.write((self.eb & 0xff_c000) + a, v),
        }
    }

    fn unbounded(&mut self, n: u64, fnc: u32) -> bool {
        if n > MAX_LOOP {
            self.stop = Some(Stop::Unsupported(fnc));
            return true;
        }
        false
    }

    /// One frame: the frame-start handshake, then the list from the read
    /// pointer until END.
    pub fn run_frame(&mut self, io: &mut Io) {
        if self.stop.is_some() {
            return;
        }
        if std::mem::take(&mut self.first) {
            // The start-up code's own wait for the first frame.
        } else if self.program == Program::Overrev {
            self.overrev_frame_start();
        }
        loop {
            let w = io.rd();
            let fnc = (w >> 23) & 0x1f;
            if self.program != Program::Fixed && (fnc != (w >> 8) & 0x1f || fnc != w & 0x1f) {
                self.stop = Some(Stop::BadCommand(w));
                return;
            }
            if self.program == Program::Overrev {
                // The command log: fn, then an end marker.
                if self.ar3 >= OVERREV_LOG_END {
                    self.ar3 = 0;
                }
                self.ram[RAM_WORDS + (self.ar3 & 0x3ff) as usize] = fnc;
                self.ar3 = (self.ar3 + 1) & 0x3fff;
                self.ram[RAM_WORDS + (self.ar3 & 0x3ff) as usize] = u32::MAX;
            }
            let cmd = 0xff00_0000 | w >> 23;
            let l = self.layout();
            let skater = self.program == Program::TopSkater;
            match fnc {
                0x00 => io.emit(cmd),
                0x01 => self.object(io, cmd, false),
                0x11 if skater => self.object(io, cmd, true),
                0x02 => self.direct(io, cmd),
                0x03 => {
                    io.emit(cmd);
                    for _ in 0..6 {
                        let v = io.rd();
                        io.emit((v & 0x0fff_0000) >> 4 | (v & 0xfff));
                    }
                }
                0x04 => {
                    io.emit(cmd);
                    let addr = io.rd();
                    io.emit(addr);
                    let n = io.rd();
                    io.emit(n);
                    let n = count(n);
                    if self.unbounded(n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = io.rd();
                        io.emit(v);
                    }
                }
                0x05 => {
                    // Polygon RAM: an address below 2^24 and fewer than
                    // 0x8000 words (the loop takes 14 bits of it, 0 as 1).
                    let addr = io.rd();
                    self.eb = self.get(l.eb, 0) & 0xff_ffff;
                    if addr & 0xff00_0000 != 0 {
                        self.stop = Some(Stop::Hang(fnc));
                        return;
                    }
                    let n = io.rd();
                    if n & 0xffff_8000 != 0 {
                        self.stop = Some(Stop::Hang(fnc));
                        return;
                    }
                    let mut eo = addr & 0xff_ffff;
                    for _ in 0..(n & 0x3fff).max(1) {
                        let v = io.rd();
                        io.poly.write((self.eb + eo) & 0xff_ffff, v);
                        eo = (eo + 1) & 0xff_ffff;
                    }
                }
                0x06 => {
                    let index = io.rd();
                    let mut coef = l.coef.1.wrapping_add(index);
                    let mut param = l.params.1.wrapping_add(index << 2);
                    let n = count(io.rd());
                    if self.unbounded(n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = io.rd();
                        let c = io.rd();
                        self.write_at(io, l.coef.0, coef, c);
                        coef = (coef & 0x3fff) + 1;
                        // Ambient, diffuse and specular scale as floats; the
                        // specular control an integer.
                        let byte = |k: u32| cif((v >> (8 * k)) & 0xff);
                        for x in [byte(1), byte(0), byte(2), v >> 24] {
                            self.write_at(io, l.params.0, param, x);
                            param = (param & 0x3fff) + 1;
                        }
                    }
                }
                0x07 => {
                    let v = io.rd();
                    self.set(l.computed, 0, v >> 1);
                    self.set(l.specular, 0, if v & 1 != 0 { 0xf } else { 0 });
                }
                0x08 => {
                    io.emit(cmd);
                    let v = io.rd();
                    io.emit(v >> 8);
                }
                0x09 => {
                    let mut f = [io.rd(), io.rd()];
                    io.xform(Kind::Focus, &mut f);
                    for (k, &v) in f.iter().enumerate() {
                        self.set(l.focus, k as u32, v);
                    }
                }
                0x0a => {
                    for k in 0..3 {
                        let v = io.rd();
                        self.set(l.light, k, v);
                    }
                }
                0x0b => {
                    let mut m: [u32; 12] = std::array::from_fn(|_| io.rd());
                    io.xform(Kind::Matrix, &mut m);
                    self.pr[..9].copy_from_slice(&m[..9]);
                    for k in 0..3 {
                        self.set(l.translation, k as u32, m[9 + k]);
                    }
                }
                0x0c => {
                    let mut t = [io.rd(), io.rd(), io.rd()];
                    io.xform(Kind::Translation, &mut t);
                    for (k, &v) in t.iter().enumerate() {
                        self.set(l.translation, k as u32, v);
                    }
                }
                0x1b if skater => {
                    // The second matrix: the queue from the word B RAM holds
                    // (9), the translation after the first.
                    let at = (self.get(l.second_matrix, 0) % 24) as usize;
                    let mut m: [u32; 12] = std::array::from_fn(|_| io.rd());
                    io.xform(Kind::Matrix2, &mut m);
                    for (k, &v) in m[..9].iter().enumerate() {
                        self.pr[(at + k) % 24] = v;
                    }
                    for k in 0..3 {
                        self.set(l.translation, 3 + k as u32, m[9 + k]);
                    }
                }
                0x1c if skater => {
                    // The first matrix copied to the second.
                    let at = (self.get(l.second_matrix, 0) % 24) as usize;
                    for k in 0..9 {
                        self.pr[(at + k) % 24] = self.pr[k];
                    }
                    for k in 0..3 {
                        let v = self.get(l.translation, k);
                        self.set(l.translation, 3 + k, v);
                    }
                }
                0x0d => {
                    // Data push: an address up to 2^24 and a count up to
                    // 0x8000 (`overrev`: 2^24), compared as signed numbers.
                    self.eb = 0;
                    let addr = io.rd();
                    let n = io.rd();
                    let limit = if self.program == Program::Overrev { 0x100_0000u32 } else { 0x8000 };
                    if (0x100_0000u32.wrapping_sub(addr) as i32) < 0 || (limit.wrapping_sub(n) as i32) < 0 {
                        self.stop = Some(Stop::Hang(fnc));
                        return;
                    }
                    let mut eo = addr & 0xff_ffff;
                    for _ in 0..(n & 0x3fff).max(1) {
                        let v = io.poly.read(eo);
                        io.emit(v);
                        eo = (eo + 1) & 0xff_ffff;
                    }
                }
                0x0e => {
                    if self.test(io) {
                        return;
                    }
                }
                0x0f => return,
                0x10 => {
                    io.rd();
                }
                0x14 => {
                    io.emit(cmd);
                    let addr = io.rd();
                    io.emit(addr);
                    let n = io.rd();
                    io.emit(n << 2);
                    let n = count(n);
                    if self.unbounded(n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = io.rd();
                        for k in 0..4 {
                            io.emit((v >> (8 * k)) & 0xff);
                        }
                    }
                }
                0x16 => {
                    let v = io.rd();
                    self.set(l.lod, 0, v);
                }
                // The rest of the dispatch table points at a loop forever.
                _ => {
                    self.stop = Some(Stop::Hang(fnc));
                    return;
                }
            }
        }
    }

    /// `overrev`'s frame start: a counter at A RAM 0x3F0 (reload) and
    /// 0x3F1-0x3F2 (counting down), which nothing starts.
    fn overrev_frame_start(&mut self) {
        let c = OVERREV_COUNTER;
        if self.ram[c] == 0 {
            return;
        }
        if self.ram[c + 1] != 0 {
            self.ram[c + 1] = self.ram[c + 1].wrapping_sub(1);
        } else if self.ram[c + 2] != 0 {
            self.ram[c + 2] = self.ram[c + 2].wrapping_sub(1);
        } else {
            self.ram[c + 1] = self.ram[c];
            self.ram[c + 2] = self.ram[c];
        }
    }

    /// fn 0x0E, the board test: 32 words of a walking one, then blocks of
    /// polygon memory (address, count, checksum) to sum. The sums only set
    /// a status output. True if it can't finish.
    fn test(&mut self, io: &mut Io) -> bool {
        let l = self.layout();
        if let (Program::TopSkater, Some(t)) = (self.program, l.tested) {
            self.set(t, 0, 1);
        }
        match self.program {
            Program::Shared | Program::Fixed => {
                // Each word is compared with a walking one; a mismatch calls
                // a routine whose flags then keep the loop going whatever
                // its count.
                let (mut one, mut left, mut k) = (1u32, 32u32, 0u64);
                loop {
                    let v = io.rd();
                    let mismatch = v != one;
                    one <<= 1;
                    left = left.wrapping_sub(1);
                    k += 1;
                    if !(mismatch || left != 0) {
                        break;
                    }
                    if self.unbounded(k, 0x0e) {
                        return true;
                    }
                }
                let n = io.rd();
                if n != 0 {
                    // The blocks are kept in B RAM, then summed.
                    let blocks = count(n);
                    if self.unbounded(3 * blocks, 0x0e) {
                        return true;
                    }
                    let mut at = 0u32;
                    for _ in 0..3 * blocks {
                        let v = io.rd();
                        self.ram[RAM_WORDS + (at & 0x3ff) as usize] = v;
                        at = (at + 1) & 0x3fff;
                    }
                    let mut at = 0u32;
                    for _ in 0..blocks {
                        let words = self.ram[RAM_WORDS + ((at + 1) & 0x3ff) as usize];
                        at = (at + 3) & 0x3fff;
                        self.eb = 0;
                        if self.unbounded(count(words), 0x0e) {
                            return true;
                        }
                    }
                }
                if let Some(t) = l.tested {
                    self.set(t, 0, 1);
                }
            }
            Program::Overrev | Program::TopSkater => {
                // Each word is compared with 1; the blocks straight from the
                // list (`overrev`: at least one, as a do-while).
                for _ in 0..32 {
                    io.rd();
                }
                let n = io.rd();
                if n != 0 || self.program == Program::Overrev {
                    let blocks = count(n);
                    if self.unbounded(blocks, 0x0e) {
                        return true;
                    }
                    for _ in 0..blocks {
                        let _addr = io.rd();
                        let words = io.rd();
                        let _sum = io.rd();
                        self.eb = 0;
                        if self.unbounded(count(words), 0x0e) {
                            return true;
                        }
                    }
                }
            }
        }
        false
    }

    /// fn 2, direct data: polygons given in view space, passed through
    /// under raster command 1.
    fn direct(&mut self, io: &mut Io, cmd: u32) {
        io.emit((cmd & 0xff00_001c) | 1);
        let tpa = io.rd();
        let tha = io.rd();
        io.emit(tpa & 0xff_ffff);
        io.emit(tha & 0xff_ffff);
        for _ in 0..6 {
            let v = io.rd();
            io.emit(v >> 8);
        }
        loop {
            let attr = io.rd();
            if attr & 3 == 0 {
                return;
            }
            io.emit(attr & 0xff_ffff);
            for _ in 0..if attr & 1 != 0 { 8 } else { 5 } {
                let v = io.rd();
                io.emit(v >> 8);
            }
        }
    }

    /// A polygon's lighting parameters (attribute bits 22-18) and its
    /// distance coefficient (bits 31-27).
    #[inline(always)]
    fn params(&self, io: &mut Io, attr: u32) -> ([u32; 4], u32) {
        let l = self.layout();
        let at = l.params.1 + ((attr >> 16) & 0x7c);
        let p = [
            self.read_at(io, l.params.0, at),
            self.read_at(io, l.params.0, at + 1),
            self.read_at(io, l.params.0, at + 2),
            self.read_at(io, l.params.0, at + 3),
        ];
        (p, self.read_at(io, l.coef.0, l.coef.1 + (attr >> 27)))
    }

    /// The lighting routine: the diffuse term `s0` times the diffuse
    /// parameter plus the ambient one; with specular on, the specular term
    /// from `base` (`clamp(2·base·n.z − L.z)`, squared as the control's
    /// table entry says) times the specular scale. The result as the luma
    /// word (8 bits from bit 15).
    #[inline(always)]
    fn luma(&self, io: &mut Io, s0: u32, base: u32, nz: u32, lz: u32, p: [u32; 4]) -> u32 {
        let l = self.layout();
        let mut lum = add(mul(s0, p[1]), p[0]);
        let lz = mul(lz, ONE);
        let control = p[3];
        if control & self.get(l.specular, 0) != 0 {
            let mut s = subz(mul(mul(base, TWO), nz), lz);
            if control != 1 {
                let n = self.read_at(io, Ram::A, l.squarings + (control & 0x3fff)) & 0x3fff;
                for _ in 0..n.max(1) {
                    s = mul(s, s);
                }
            }
            lum = add(lum, mul(s, p[2]));
        }
        cfib(lum) << 15
    }

    /// The lit luma word of a polygon facing the eye (`front`) or of a
    /// double-sided one facing away, from the diffuse dot product.
    #[inline(always)]
    fn lit(&self, io: &mut Io, front: bool, dotl: u32, nz: u32, lz: u32, p: [u32; 4]) -> u32 {
        if self.program == Program::Overrev {
            // The term clamped at 0 (a back face's negated), and the
            // specular term from it.
            let s0 = if front { clamp_negative(dotl).0 } else { addz(mul(dotl, MINUS_ONE), ZERO) };
            self.luma(io, s0, s0, nz, lz, p)
        } else {
            // The routine clamps it, but takes the specular term from it
            // as it is.
            let d = if front { dotl } else { mul(dotl, MINUS_ONE) };
            self.luma(io, addz(d, ZERO), d, nz, lz, p)
        }
    }

    /// fn 1 (and `topskatr`'s fn 0x11), object data: a model from polygon
    /// memory, transformed, lit and sent as a polygon block.
    fn object(&mut self, io: &mut Io, cmd: u32, compact: bool) {
        let l = self.layout();
        if self.program != Program::Overrev {
            if let Some(t) = l.tested {
                self.set(t, 0, 0);
            }
        }
        io.emit(cmd);
        self.eb = 0;
        let tpa = io.rd() & 0xff_ffff;
        let tha = io.rd() & 0xff_ffff;
        let oba = io.rd();
        let obc = io.rd();
        io.object(oba);
        self.set(l.count, 0, obc);
        io.emit(tpa);
        io.emit(tha);
        let second = |s: &Self, k: usize| if compact { s.pr[(9 + k) % 24] } else { s.pr[k] };
        let c = Ctx {
            t: [
                [self.get(l.translation, 0), self.get(l.translation, 1), self.get(l.translation, 2)],
                [self.get(l.translation, 3), self.get(l.translation, 4), self.get(l.translation, 5)],
            ],
            m: [std::array::from_fn(|k| self.pr[k]), std::array::from_fn(|k| second(self, k))],
            fx: self.get(l.focus, 0),
            fy: self.get(l.focus, 1),
            l: [self.get(l.light, 0), self.get(l.light, 1), self.get(l.light, 2)],
            lod: self.get(l.lod, 0),
        };
        let mut w = Words { eb: 0, eo: oba & 0xff_ffff };
        // A compact model's vertices pick their matrix by bit 0 of x.
        let sel = |p: [u32; 3]| if compact { (p[0] & 1) as usize } else { 0 };
        let v0 = w.vertex(io);
        let p0 = c.xform(sel(v0), v0);
        c.emit(io, p0);
        let v1 = w.vertex(io);
        let p1 = c.xform(sel(v1), v1);
        c.emit(io, p1);
        if compact || self.get(l.computed, 0) & 1 != 0 {
            for k in 0..3 {
                self.set(l.edge0, k, p0[k as usize]);
                self.set(l.edge1, k, p1[k as usize]);
            }
            self.computed_normals(io, &c, &mut w, compact);
        } else {
            self.given_normals(io, &c, &mut w);
        }
    }

    /// Count a polygon off the object's count: true when it reaches 0.
    #[inline(always)]
    fn counted(&mut self) -> bool {
        let l = self.layout();
        let n = self.get(l.count, 0).wrapping_sub(1);
        self.set(l.count, 0, n);
        n == 0
    }

    /// A polygon's words after its attribute word: luma, distance, Pa, Pb.
    #[inline(always)]
    fn send(io: &mut Io, c: &Ctx, attr: u32, luma: u32, dist: u32, pa: [u32; 3], pb: Option<[u32; 3]>) {
        io.emit(attr & 0x3_ffff);
        io.emit(luma);
        io.emit(dist >> 8);
        c.emit(io, pa);
        if let Some(pb) = pb {
            c.emit(io, pb);
        }
    }

    /// Mode bit 1 clear: each polygon comes with its normal (10 words: attr,
    /// normal, Pa, Pb, a triangle's Pb unused).
    fn given_normals(&mut self, io: &mut Io, c: &Ctx, w: &mut Words) {
        let l = self.layout();
        for _ in 0..MAX_LOOP {
            let attr = w.next(io);
            if attr & 3 == 0 {
                return;
            }
            self.set(l.attr, 0, attr & 0x3_ffff);
            let quad = attr & 1 != 0;
            let n = c.rotate(w.vertex(io));
            let (p, coef) = self.params(io, attr);
            let pa = c.xform(0, w.vertex(io));
            let pb = if quad {
                Some(c.xform(0, w.vertex(io)))
            } else {
                w.skip(3);
                None
            };
            let dotp = dot(pa, n);
            let dotl = dot(n, c.l);
            let coef_lod = mul(coef, c.lod);
            let front = if self.program == Program::Overrev && quad {
                // FCMP against 0: only a positive dot product faces the eye.
                negative(sub(ZERO, dotp))
            } else {
                !negative(dotp)
            };
            let dist = if quad { mul(coef_lod, abs(dotp)) } else { mul(abs(dotp), coef_lod) };
            let luma = if front || attr & 0x2_0000 != 0 {
                let d = if front { mul(self.given_dotl(dotl, quad), ONE) } else { dotl };
                self.lit(io, front, d, n[2], c.l[2], p)
            } else {
                BACK_FACE
            };
            Self::send(io, c, attr, luma, dist, pa, pb);
            if self.counted() {
                return;
            }
        }
        self.stop = Some(Stop::Unsupported(0x01));
    }

    /// The diffuse dot product the given-normal paths hand the lighting
    /// routine for a front face: the shared program clamps a triangle's at
    /// 0, `overrev` both (and the routine itself clamps again).
    #[inline(always)]
    fn given_dotl(&self, dotl: u32, quad: bool) -> u32 {
        if self.program == Program::Overrev || !quad {
            clamp_negative(dotl).0
        } else {
            dotl
        }
    }

    /// Mode bit 1 set (and `topskatr`'s compact format): each polygon's
    /// normal is computed from its new vertex and the previous edge, which
    /// the program tracks as the rasteriser links them, normalised with
    /// FRSQ and two Newton steps.
    fn computed_normals(&mut self, io: &mut Io, c: &Ctx, w: &mut Words, compact: bool) {
        let l = self.layout();
        let sel = |p: [u32; 3]| if compact { (p[0] & 1) as usize } else { 0 };
        for _ in 0..MAX_LOOP {
            let attr = w.next(io);
            if !compact {
                // The model's normal.
                w.skip(3);
            }
            if attr & 3 == 0 {
                return;
            }
            self.set(l.attr, 0, attr & 0x3_ffff);
            let quad = attr & 1 != 0;
            let (p, coef) = self.params(io, attr);
            let coef_lod = if compact {
                self.set(l.attr_word, 0, attr);
                let v = mul(coef, c.lod);
                self.set(l.coef_lod, 0, v);
                v
            } else if quad {
                mul(coef, c.lod)
            } else {
                mul(c.lod, coef)
            };
            let va = w.vertex(io);
            let pa = c.xform(sel(va), va);
            let e0: [u32; 3] = std::array::from_fn(|k| self.get(l.edge0, k as u32));
            let e1: [u32; 3] = std::array::from_fn(|k| self.get(l.edge1, k as u32));
            let pb = if quad {
                let vb = w.vertex(io);
                Some(c.xform(sel(vb), vb))
            } else {
                if !compact {
                    w.skip(3);
                }
                None
            };
            // b × a, with a = Pa − P0 and b = P1 − P0.
            let (bx, by, bz) = (sub(e1[0], e0[0]), sub(e1[1], e0[1]), sub(e1[2], e0[2]));
            let (ax, ay, az) = (sub(pa[0], e0[0]), sub(pa[1], e0[1]), sub(pa[2], e0[2]));
            let cx = sub(mul(by, az), mul(ay, bz));
            let cy = sub(mul(bz, ax), mul(az, bx));
            let cz = sub(mul(bx, ay), mul(ax, by));
            let len2 = add(add(mul(cx, cx), mul(cy, cy)), mul(cz, cz));
            let toward = dot(pa, [cx, cy, cz]);
            let mut y = rsq(len2);
            for _ in 0..2 {
                y = mul(sub(THREE, mul(len2, mul(y, y))), mul(y, HALF));
            }
            // The next polygon's edge, by this one's link type (a
            // triangle's Pb is its Pa).
            let pb_or_pa = pb.unwrap_or(pa);
            match (attr >> 8) & 3 {
                1 => self.set_edge(l.edge1, pa),
                3 => self.set_edge(l.edge0, pb_or_pa),
                _ => {
                    self.set_edge(l.edge0, pa);
                    self.set_edge(l.edge1, pb_or_pa);
                }
            }
            let front = !negative(toward);
            let (luma, dist) = if front || attr & 0x2_0000 != 0 {
                let n = [mul(cx, y), mul(cy, y), mul(cz, y)];
                let dotp = dot(pa, n);
                let dotl = dot(n, c.l);
                (self.lit(io, front, dotl, n[2], c.l[2], p), mul(coef_lod, abs(dotp)))
            } else {
                (BACK_FACE, coef_lod)
            };
            Self::send(io, c, attr, luma, dist, pa, pb);
            if self.counted() {
                return;
            }
        }
        self.stop = Some(Stop::Unsupported(0x01));
    }

    #[inline(always)]
    fn set_edge(&mut self, at: At, v: [u32; 3]) {
        for (k, &x) in v.iter().enumerate() {
            self.set(at, k as u32, x);
        }
    }
}

impl state::State for Native {
    fn save(&self, w: &mut state::Writer) {
        self.ram.save(w);
        self.pr.save(w);
        [self.eb, self.ar3].save(w);
        self.first.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.ram.load(r)?;
        self.pr.load(r)?;
        let mut regs = [0u32; 2];
        regs.load(r)?;
        [self.eb, self.ar3] = regs;
        self.first.load(r)?;
        self.stop = None;
        Ok(())
    }
}

/// True if a TGP has been uploaded to and not run since: its program
/// starts at 0 and never comes back there.
pub fn fresh(t: &Tgp) -> bool {
    t.pc == 0
}

/// A copy of a freshly uploaded TGP run through its start-up code, to where
/// it waits for the first frame (F0).
pub fn started(t: &Tgp) -> Tgp {
    struct Quiet;
    impl External for Quiet {
        fn read(&mut self, _: u32) -> u32 {
            0
        }
        fn write(&mut self, _: u32, _: u32) {}
    }
    let mut copy = Tgp::new();
    let bytes = state::to_bytes(t);
    if state::State::load(&mut copy, &mut state::Reader::new(&bytes)).is_ok() {
        copy.ext_flags = 0;
        let _ = copy.run(&mut Quiet, 4096);
    }
    copy
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geo::{GeoExt, ListDma};

    fn f(v: f32) -> u32 {
        v.to_bits()
    }

    fn cmd(fnc: u32) -> u32 {
        fnc << 23 | fnc << 8 | fnc
    }

    /// Run one frame of `list` on `n` with polygon ROM `rom`.
    fn frame(n: &mut Native, list: &[u32], rom: &[u32]) -> Vec<u32> {
        let buffer: Vec<u8> =
            list.iter().flat_map(|w| w.to_le_bytes()).chain(std::iter::repeat(0)).take(0x2_0000).collect();
        let (mut words, mut fed, mut out, mut latch) = (0, Vec::new(), Vec::new(), 0);
        let mut ram = vec![0; 0x1_0000];
        let mut list = ListDma::new(&buffer, &[], 0, &mut words, &mut fed);
        let mut io =
            Io { list: &mut list, poly: GeoExt::new(&mut ram, rom), latch: &mut latch, out: &mut out, tap: None };
        n.run_frame(&mut io);
        out
    }

    #[test]
    fn unknown_programs_are_left_to_the_tgp() {
        assert_eq!(recognise(0), None);
        assert_eq!(recognise(0x2917_7553_5904_96aa), Some(Program::Shared));
    }

    /// An object with given normals: its block's layout, a lit quad facing
    /// the eye and its distance.
    #[test]
    fn object_block() {
        let mut n = Native::new(Program::Shared);
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 10.0];
        let mut list = vec![cmd(0x0b)];
        list.extend(identity.map(f));
        list.extend([cmd(0x09), f(2.0), f(3.0), cmd(0x0a), f(0.0), f(0.0), f(1.0), cmd(0x16), f(4.0)]);
        // Entry 1: diffuse 100, ambient 20, coefficient 0.5.
        list.extend([cmd(0x06), 1, 1, 20 << 8 | 100, f(0.5), cmd(0x07), 0]);
        list.extend([cmd(0x01), 0x12_3456, 0x0065_4321, 0x80_0000, 0, cmd(0x0f)]);
        let mut rom = vec![f(0.0), f(0.0), f(0.0), f(1.0), f(0.0), f(0.0)];
        rom.extend([1 | 1 << 18 | 1 << 27, f(0.0), f(0.0), f(1.0)]);
        rom.extend([f(1.0), f(1.0), f(0.0), f(0.0), f(1.0), f(0.0)]);
        rom.push(0);
        let out = frame(&mut n, &list, &rom);
        let v = |x: f32| f(x) >> 8;
        assert_eq!(out[..3], [0xff00_0001, 0x12_3456, 0x65_4321]);
        assert_eq!(out[3..9], [v(0.0), v(0.0), v(10.0), v(2.0), v(0.0), v(10.0)]);
        assert_eq!(out[9], 1, "attr & 0x3FFFF");
        // n·L = 1: 1·100 + 20.
        assert_eq!(out[10], 120 << 15);
        // 0.5 · LOD · |n·Pa|.
        assert_eq!(out[11], v(20.0));
        assert_eq!(out[12..18], [v(2.0), v(3.0), v(10.0), v(0.0), v(3.0), v(10.0)]);
        assert_eq!(out.len(), 18);
    }

    #[test]
    fn command_check() {
        let mut n = Native::new(Program::Shared);
        assert_eq!(frame(&mut n, &[cmd(0x08), f(1.0), cmd(0x0f)], &[]).len(), 2);
        assert!(frame(&mut n, &[0x0400_0000, cmd(0x0f)], &[]).is_empty());
        assert_eq!(n.stop, Some(Stop::BadCommand(0x0400_0000)));
        assert!(frame(&mut n, &[cmd(0x08), f(1.0), cmd(0x0f)], &[]).is_empty(), "for good");
    }

    /// The 2A's geometrizer has no command check: the command is the port
    /// the word went to, and the TGP's matrix words (and the i960's, when
    /// it writes zero) carry anything in their low bits.
    #[test]
    fn fixed_function_takes_any_low_bits() {
        let mut n = Native::fixed();
        let mut list = vec![4 << 23 | 0x1f, 0x10, 2, 0xaa, 0xbb, 0x0b << 23 | 0x6d00];
        list.extend([f(1.0), 0, 0, 0, f(1.0), 0, 0, 0, f(1.0), 0, 0, 0]);
        list.push(cmd(0x0f));
        let out = frame(&mut n, &list, &[]);
        assert_eq!(out, [0xff00_0004, 0x10, 2, 0xaa, 0xbb], "fn 4 passes its words through");
        assert_eq!(n.stop, None);
        assert_eq!(n.pr[0], f(1.0), "the matrix was taken");
        // The squarings a program's start-up code sets up.
        assert_eq!(n.ram[SHARED.squarings as usize + 7], 3);
    }

    #[test]
    fn state_round_trip() {
        let mut n = Native::new(Program::Overrev);
        frame(&mut n, &[cmd(0x0a), f(1.0), f(2.0), f(3.0), cmd(0x07), 3, cmd(0x0f)], &[]);
        let bytes = state::to_bytes(&n);
        let mut m = Native::new(Program::Overrev);
        state::State::load(&mut m, &mut state::Reader::new(&bytes)).unwrap();
        assert_eq!((m.ram.clone(), m.pr, m.ar3, m.first), (n.ram.clone(), n.pr, n.ar3, n.first));
        assert_eq!(m.ar3, 3, "the command log");
    }
}
