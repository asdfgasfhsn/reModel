//! A native geometrizer: the display-list program that the 2B games
//! `overrevb`, `sgt24h`, `dynabb97`, `doa` and `schamp` upload to their
//! geometrizer SHARC, implemented in Rust from its behaviour
//! (docs/hw/geometrizer-2b.md). It takes the same inputs as the SHARC (the
//! list from buffer RAM, the polygon memory, its own state from earlier
//! commands) and gives the same rasteriser stream, word for word: every
//! floating-point step is the SHARC's, in the program's order, through the
//! `adsp2106x` crate's arithmetic (32-bit results rounded toward zero, as
//! the program sets MODE1).
//!
//! It is picked by the code the game uploads ([`recognise`]: the
//! compiled-code hash the recompiler names that program's crate by),
//! never by game, and only on request (`GeoMode`, `--geo native`).
//!
//! Its state is kept where the program keeps it: the words of the SHARC's
//! data memory (block 1, `0x30000` up) and the few program-memory words it
//! uses as data, so a SHARC's state can be taken over ([`Native::import`])
//! and handed back ([`Native::export`]).

use crate::geo::{GeoExt, ListDma, LIST_END};
use adsp2106x::float::{self, Flags, Mode, Select, ToInt};
use mb86235::cpu::External;

/// The program's MODE1 arithmetic: RND32 and TRUNC (with ALUSAT for
/// integers).
const M: Mode = Mode { trunc: true, rnd32: true };

/// The value the boot writes to MSGR7 to switch the command check off.
const NO_CHECK_KEY: u32 = 0x7378_842e;

/// Data memory words (offsets from `0x30000`) the program keeps its state in.
const TRANSLATION: usize = 0x20;
const MATRIX: usize = 0x23;
const FOCUS: usize = 0x2c;
const MODE_WORD: usize = 0x2e;
const LOD: usize = 0x2f;
const LIGHT: usize = 0x30;
/// The last fast-RAM polygon's attribute bits (reset each frame).
const LAST_ATTR: usize = 0x33;
/// Set by the test command (fn 0x0E), cleared by object data.
const TESTED: usize = 0x34;
/// A quad's second vertex as read (computed-normal modes).
const PB_RAW: usize = 0x38;
/// The rotated normal (given-normal modes), or a ring of three
/// transformed vertices (computed-normal modes).
const RING: usize = 0x40;
/// Lighting parameters (4 words each) and distance coefficients.
const PARAMS: usize = 0x60;
const COEF: usize = 0xe0;
/// Program-memory data words, from `0x21000`: the first previous-edge
/// vertex of the computed-normal modes is at `+0x10`.
const PM_BASE: u32 = 0x2_1000;
const PM_WORDS: usize = 0x800;
const P0: usize = 0x10;

/// Data memory block 1 seen as 32-bit words: `0x28000-0x3FFFF`, the second
/// half aliasing the first as on the ADSP-21062.
const DM_WORDS: usize = 0x8000;

/// External bus addresses (as the SHARC program sees them, `model2b.md`).
const LIST_FIFO: u32 = 0x40_0000;
const RASTER_FIFO: u32 = 0x41_0000;
const POLY_RAM_WRITE: u32 = 0x42_0000;
const POLY_ADDRESS: u32 = 0x43_0000;
const POLY_DATA: u32 = 0x43_0001;

/// Most polygons an object may have when its count is 0 ("until the
/// terminator"): far more than the SHARC gets through in a frame's
/// instruction budget, so only a model without a terminator reaches it.
const MAX_POLYGONS: u64 = 1 << 20;
/// Most words a data loop may move (see [`Native::unbounded`]).
const MAX_LOOP: u64 = 1 << 22;

/// A program this module implements, by `adsp2106x::cpu::code_hash` of the
/// code that static discovery finds in the uploaded image.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Program {
    /// The geometrizer code `overrevb`, `sgt24h`, `dynabb97`, `doa` and
    /// `schamp` upload (identical up to the end of its code).
    Shared2B,
    /// `indy500`'s and `fvipers`' code, an earlier version of the same
    /// program: it differs only in scaling the first vertex's y by the x
    /// focal length in the computed-normal modes, and in what it sends at
    /// a frame's start (which the rasteriser never sees).
    Indy500,
}

/// Code hashes of the programs implemented here.
pub const PROGRAMS: &[(u64, Program)] =
    &[(0x1539_3321_f266_b9cf, Program::Shared2B), (0x1ee9_9ebc_8a45_8237, Program::Indy500)];

/// The code hash a program image is recognised by: [`adsp2106x::cpu::code_hash`]
/// of the words [`adsp2106x::codegen::discover`] finds as code.
pub fn image_hash(image: &[u64]) -> u64 {
    let words: Vec<u32> = adsp2106x::codegen::discover(image).code.into_iter().collect();
    adsp2106x::cpu::code_hash(image, &words)
}

/// The program an uploaded image holds, if this module implements it.
pub fn recognise(image: &[u64]) -> Option<Program> {
    let h = image_hash(image);
    PROGRAMS.iter().find(|p| p.0 == h).map(|p| p.1)
}

/// Why the native geometrizer stopped producing output.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stop {
    /// A command word failed the command check: the program then loops
    /// forever (as the SHARC does) and draws nothing more.
    BadCommand(u32),
    /// A command this model doesn't implement (code upload and call,
    /// fn 0x1D/0x1E), or a loop with no end.
    Unsupported(u32),
}

/// The native geometrizer's state (the program's own, see the module
/// comment) and diagnostics.
#[derive(Clone)]
pub struct Native {
    pub program: Program,
    dm: Vec<u32>,
    pm: [u32; PM_WORDS],
    /// Command words are checked (MSGR7 doesn't hold the key).
    check: bool,
    /// Frame handshakes the boot still waits for (two after an upload).
    boot_frames: u8,
    pub stop: Option<Stop>,
}

/// What a frame reads and writes: the list, polygon memory, the
/// rasteriser stream.
pub struct Io<'a, 'b> {
    pub list: &'a mut ListDma<'b>,
    pub poly: GeoExt<'a>,
    pub latch: &'a mut u32,
    pub out: &'a mut Vec<u32>,
}

impl Io<'_, '_> {
    #[inline]
    fn rd(&mut self) -> u32 {
        self.list.next().unwrap_or(LIST_END)
    }
    #[inline]
    fn emit(&mut self, w: u32) {
        self.out.push(w);
    }
}

// The SHARC's arithmetic on 32-bit values (a register's bits 39-8).

#[inline(always)]
fn r40(v: u32) -> u64 {
    (v as u64) << 8
}
#[inline(always)]
fn r32(v: u64) -> u32 {
    (v >> 8) as u32
}
#[inline(always)]
fn mul(a: u32, b: u32) -> u32 {
    r32(float::mul(r40(a), r40(b), M).0)
}
/// A product and whether it is negative (the multiplier's MN).
#[inline(always)]
fn mul_neg(a: u32, b: u32) -> (u32, bool) {
    let (v, f) = float::mul(r40(a), r40(b), M);
    (r32(v), f.neg)
}
#[inline(always)]
fn add(a: u32, b: u32) -> u32 {
    r32(float::add(r40(a), r40(b), false, false, M).0)
}
#[inline(always)]
fn add_flags(a: u32, b: u32) -> (u32, Flags) {
    let (v, f) = float::add(r40(a), r40(b), false, false, M);
    (r32(v), f)
}
#[inline(always)]
fn sub(a: u32, b: u32) -> u32 {
    r32(float::add(r40(a), r40(b), true, false, M).0)
}
#[inline(always)]
fn abs(a: u32) -> u32 {
    r32(float::sign_op(r40(a), false, true, M).0)
}
/// `PASS`'s flags.
#[inline(always)]
fn pass_flags(a: u32) -> Flags {
    float::sign_op(r40(a), false, false, M).1
}
/// The `LT` condition after a float ALU operation with these flags.
#[inline(always)]
fn lt(f: Flags) -> bool {
    f.neg && !f.zero
}
#[inline(always)]
fn max(a: u32, b: u32) -> u32 {
    r32(float::select(Select::Max, r40(a), r40(b), M).0)
}
#[inline(always)]
fn rsqrts(a: u32) -> u32 {
    r32(float::rsqrts(r40(a), M).0)
}
/// `FIX` under TRUNC with ALUSAT: toward minus infinity, saturating; a NaN
/// gives all ones.
#[inline(always)]
fn fix(a: u32) -> u32 {
    match float::to_int(r40(a), 0, ToInt::Floor, true, M).0 {
        Ok(v) => v,
        Err(nan) => r32(nan),
    }
}
/// Integer `CLIP x BY y`.
#[inline(always)]
fn clip(x: u32, y: u32) -> u32 {
    let (x, ay) = (x as i32 as i64, (y as i32 as i64).abs());
    (if x.abs() < ay {
        x
    } else if x >= 0 {
        ay
    } else {
        -ay
    }) as u32
}
/// Integer add with ALUSAT.
#[inline(always)]
fn sat_add(a: u32, b: u32) -> u32 {
    (a as i32).saturating_add(b as i32) as u32
}

const HALF: u32 = 0x3f00_0000;
const THREE: u32 = 0x4040_0000;

/// Where an object's words come from: polygon memory through the address
/// latch (ROM from `0x800000`, polygon RAM below), or the SHARC's own
/// memory ("fast" polygon RAM, model addresses with bit 24 set).
#[derive(Clone, Copy)]
enum Source {
    Poly(u32),
    Internal(u32),
}

/// The transform, lights and constants an object is drawn with.
#[derive(Clone, Copy)]
struct Ctx {
    t: [u32; 3],
    m: [u32; 9],
    fx: u32,
    fy: u32,
    l: [u32; 3],
    lod: u32,
}

impl Ctx {
    /// Model to view space: the translation first, then x, y and z terms.
    #[inline(always)]
    fn xform(&self, p: [u32; 3]) -> [u32; 3] {
        let m = &self.m;
        let row = |t: u32, a: u32, b: u32, c: u32| add(add(add(t, mul(p[0], a)), mul(p[1], b)), mul(p[2], c));
        [row(self.t[0], m[0], m[3], m[6]), row(self.t[1], m[1], m[4], m[7]), row(self.t[2], m[2], m[5], m[8])]
    }

    /// A normal into view space (rotation only).
    #[inline(always)]
    fn rotate(&self, n: [u32; 3]) -> [u32; 3] {
        let m = &self.m;
        let row = |a: u32, b: u32, c: u32| add(add(mul(n[0], a), mul(n[1], b)), mul(n[2], c));
        [row(m[0], m[3], m[6]), row(m[1], m[4], m[7]), row(m[2], m[5], m[8])]
    }

    /// A transformed vertex as the rasteriser takes it: x and y times the
    /// focal lengths, z as it is, each a 24-bit float.
    #[inline(always)]
    fn emit(&self, io: &mut Io, v: [u32; 3]) {
        io.emit(mul(self.fx, v[0]) >> 8);
        io.emit(mul(self.fy, v[1]) >> 8);
        io.emit(v[2] >> 8);
    }
}

#[inline(always)]
fn dot(a: [u32; 3], b: [u32; 3]) -> u32 {
    add(add(mul(a[0], b[0]), mul(a[1], b[1])), mul(a[2], b[2]))
}

/// The diffuse factor: |n·L|, or 0 when the light and the eye are on
/// different sides of the polygon (computed as a difference of the product
/// with itself, as the program does).
#[inline(always)]
fn diffuse_term(dotl: u32, dotp: u32) -> u32 {
    let (p, neg) = mul_neg(dotl, dotp);
    if neg {
        sub(p, p)
    } else {
        abs(dotl)
    }
}

/// The specular term raised by the control value's bits (0: none; then
/// squared once for 2-3, twice for 4-6, three times from 7).
#[inline(always)]
fn specular_power(mut s: u32, control: u32) -> u32 {
    if control == 0 {
        s = sub(s, s);
    }
    if control >> 1 != 0 {
        s = mul(s, s);
    }
    if control >> 2 != 0 {
        s = mul(s, s);
    }
    if sat_add(control, 1) >> 3 != 0 {
        s = mul(s, s);
    }
    s
}

impl Native {
    /// The state after an upload of `program` that leaves `msgr7` in MSGR7.
    pub fn new(program: Program, msgr7: u32) -> Self {
        let mut dm = vec![0; DM_WORDS];
        dm[LAST_ATTR] = 0x3_0100;
        Native { program, dm, pm: [0; PM_WORDS], check: msgr7 != NO_CHECK_KEY, boot_frames: 2, stop: None }
    }

    /// Take over a SHARC's state between frames: its data words, and the
    /// boot handshakes still to come after `frames` frames since the upload.
    pub fn import(&mut self, s: &adsp2106x::cpu::Sharc, frames: u64) {
        for (i, w) in self.dm.iter_mut().enumerate() {
            *w = s.mem.read32(0x3_0000 + i as u32);
        }
        for (i, w) in self.pm.iter_mut().enumerate() {
            *w = (s.mem.read48(PM_BASE + i as u32) >> 16) as u32;
        }
        self.check = s.iop.get(0xf).copied() != Some(NO_CHECK_KEY);
        self.boot_frames = 2u64.saturating_sub(frames) as u8;
        if matches!(self.stop, Some(Stop::Unsupported(_))) {
            self.stop = None;
        }
    }

    /// Write the state into a SHARC's memory, where the program keeps it.
    pub fn export(&self, s: &mut adsp2106x::cpu::Sharc) {
        for (i, &w) in self.dm.iter().enumerate() {
            s.mem.write32(0x3_0000 + i as u32, w);
        }
        for (i, &w) in self.pm.iter().enumerate() {
            let a = PM_BASE + i as u32;
            let low = s.mem.read48(a) & 0xffff;
            s.mem.write48(a, (w as u64) << 16 | low);
        }
    }

    /// The data words the state is kept in (for comparing with a SHARC's).
    pub fn data(&self) -> (&[u32], &[u32]) {
        (&self.dm, &self.pm)
    }

    #[inline(always)]
    fn dm_index(addr: u32) -> Option<usize> {
        (0x2_8000..0x4_0000).contains(&addr).then_some((addr & 0x7fff) as usize)
    }

    /// A data-memory read of the program's: internal memory or the bus.
    fn read(&mut self, io: &mut Io, addr: u32) -> u32 {
        if let Some(i) = Self::dm_index(addr) {
            return self.dm[i];
        }
        match addr {
            LIST_FIFO => io.rd(),
            POLY_DATA => io.poly.read(*io.latch),
            _ => 0,
        }
    }

    /// A data-memory write of the program's.
    fn write(&mut self, io: &mut Io, addr: u32, v: u32) {
        if let Some(i) = Self::dm_index(addr) {
            self.dm[i] = v;
            return;
        }
        match addr {
            RASTER_FIFO => io.emit(v),
            POLY_RAM_WRITE..=0x42_ffff => io.poly.write(addr - POLY_RAM_WRITE, v),
            POLY_ADDRESS => *io.latch = v,
            // Program memory as data (block 0), the IOP registers and the
            // rest of the bus: nothing this model keeps.
            _ => {}
        }
    }

    /// A loop count as `LCNTR` takes it: 0 runs 2^32 times.
    fn count(n: u32) -> u64 {
        if n == 0 {
            1 << 32
        } else {
            n as u64
        }
    }

    /// A data loop far longer than the SHARC gets through in a frame (a
    /// count of 0 runs 2^32 times): stop here rather than spin.
    fn unbounded(&mut self, n: u64, fnc: u32) -> bool {
        if n > MAX_LOOP {
            self.stop = Some(Stop::Unsupported(fnc));
            return true;
        }
        false
    }

    /// One frame: the list from the read pointer until END (the SHARC's
    /// frame protocol: two handshakes after an upload, then a list a frame).
    pub fn run_frame(&mut self, io: &mut Io) {
        if self.boot_frames > 0 {
            // The first frame passes one handshake; the second the other,
            // and then the list.
            self.boot_frames -= 1;
            if self.boot_frames > 0 {
                return;
            }
            self.dm[TESTED] = 0;
        }
        if self.stop.is_some() {
            return;
        }
        self.dm[LAST_ATTR] = 0x3_0100;
        loop {
            let w = io.rd();
            let fnc = (w >> 23) & 0x1f;
            if self.check && (fnc != w & 0x1f || fnc != (w >> 8) & 0x1f) {
                self.stop = Some(Stop::BadCommand(w));
                return;
            }
            let cmd = 0xff00_0000 | w >> 23;
            match fnc {
                0x00 => io.emit(cmd),
                0x01 | 0x11 => self.object(io, cmd),
                0x02 | 0x12 => self.direct(io, cmd),
                0x03 | 0x13 => {
                    io.emit(cmd);
                    for _ in 0..6 {
                        let v = io.rd();
                        io.emit((v & 0xfff) | (v & 0x0fff_0000) >> 4);
                    }
                }
                0x04 => {
                    io.emit(cmd);
                    let addr = io.rd();
                    io.emit(addr);
                    let n = io.rd();
                    io.emit(n);
                    let n = Self::count(n);
                    if self.unbounded(n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = io.rd();
                        io.emit(v);
                    }
                }
                0x05 | 0x15 => {
                    let addr = io.rd();
                    let mut at = if addr & 1 << 24 != 0 {
                        addr.wrapping_add(0xff03_0100)
                    } else {
                        addr.wrapping_add(POLY_RAM_WRITE)
                    };
                    let n = Self::count(io.rd());
                    if self.unbounded(n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = io.rd();
                        self.write(io, at, v);
                        at = at.wrapping_add(1);
                    }
                }
                0x06 => {
                    let index = io.rd();
                    let mut coef = 0x3_00e0u32.wrapping_add(index);
                    let mut param = 0x3_0060u32.wrapping_add(index << 2);
                    let n = Self::count(io.rd());
                    if self.unbounded(2 * n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = io.rd();
                        let byte = |k: u32| ((v >> (8 * k)) & 0xff) as f32;
                        let c = io.rd();
                        // Diffuse, ambient, specular control (an integer),
                        // specular scale.
                        for x in [byte(0).to_bits(), byte(1).to_bits(), v >> 24, byte(2).to_bits()] {
                            self.write(io, param, x);
                            param = param.wrapping_add(1);
                        }
                        self.write(io, coef, c);
                        coef = coef.wrapping_add(1);
                    }
                }
                0x07 | 0x17 => self.dm[MODE_WORD] = io.rd(),
                0x08 | 0x18 => {
                    io.emit(cmd);
                    let v = io.rd();
                    io.emit(v >> 8);
                }
                0x09 | 0x19 => {
                    self.dm[FOCUS] = io.rd();
                    self.dm[FOCUS + 1] = io.rd();
                }
                0x0a | 0x1a => {
                    for k in 0..3 {
                        let v = io.rd();
                        self.dm[LIGHT + k] = v;
                        self.pm[8 + k] = v;
                    }
                }
                0x0b | 0x1b => {
                    for k in 0..12 {
                        let v = io.rd();
                        if k < 9 {
                            self.dm[MATRIX + k] = v;
                        } else {
                            self.dm[TRANSLATION + k - 9] = v;
                        }
                        if k < 3 {
                            self.pm[k] = v;
                        } else if k >= 9 {
                            self.pm[k - 6] = v;
                        }
                    }
                }
                0x0c | 0x1c => {
                    for k in 0..3 {
                        let v = io.rd();
                        self.dm[TRANSLATION + k] = v;
                        self.pm[3 + k] = v;
                    }
                }
                0x0d => {
                    let mut addr = io.rd();
                    let n = Self::count(io.rd());
                    if self.unbounded(n, fnc) {
                        return;
                    }
                    for _ in 0..n {
                        let v = self.read(io, addr);
                        io.emit(v);
                        addr = addr.wrapping_add(1);
                    }
                }
                0x0e => {
                    if self.test(io) {
                        return;
                    }
                }
                0x0f | 0x1f => return,
                0x10 => {
                    io.rd();
                }
                0x14 => {
                    io.emit(cmd);
                    let addr = io.rd();
                    io.emit(addr);
                    let n = io.rd();
                    io.emit(n << 2);
                    let n = Self::count(n);
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
                0x16 => self.dm[LOD] = io.rd(),
                // 0x1D uploads code into the program and calls it, 0x1E calls
                // uploaded code: not modelled.
                _ => {
                    self.stop = Some(Stop::Unsupported(fnc));
                    return;
                }
            }
        }
    }

    /// fn 0x0E, the board test: 32 words of a walking one, then blocks of
    /// polygon memory (address, count, checksum) to sum. Nothing is drawn;
    /// the blocks are kept in fast polygon RAM's place. True if it can't
    /// finish.
    fn test(&mut self, io: &mut Io) -> bool {
        for i in 0..32 {
            // A word that isn't the pattern calls a routine from the last
            // instructions of the loop, which on the SHARC tests the loop
            // end twice (UM §3.5.1.1): the count goes wrong, which this
            // model doesn't follow.
            if io.rd() != 1 << i {
                self.stop = Some(Stop::Unsupported(0x0e));
                return true;
            }
        }
        let n = io.rd();
        if n != 0 {
            let blocks = Self::count(n);
            if self.unbounded(3 * blocks, 0x0e) {
                return true;
            }
            let mut at = 0x3_0100u32;
            for _ in 0..3 * blocks {
                let v = io.rd();
                self.write(io, at, v);
                at = at.wrapping_add(1);
            }
            for k in 0..blocks {
                let base = 0x3_0100u32.wrapping_add(3 * k as u32);
                let mut addr = self.read(io, base);
                let words = Self::count(self.read(io, base.wrapping_add(1)));
                if words > MAX_LOOP {
                    self.stop = Some(Stop::Unsupported(0x0e));
                    return true;
                }
                // The sums only set FLAG outputs; the reads set the latch.
                for _ in 0..words {
                    *io.latch = addr;
                    addr = addr.wrapping_add(1);
                }
            }
        }
        self.dm[TESTED] = u32::MAX;
        false
    }

    /// fn 2, direct data: polygons given in view space, passed through
    /// with the raster command of object data.
    fn direct(&mut self, io: &mut Io, cmd: u32) {
        let tpa = io.rd();
        let tha = io.rd();
        io.emit(cmd.wrapping_sub(1));
        io.emit(tpa & 0xff_ffff);
        io.emit(tha & 0xff_ffff);
        for _ in 0..6 {
            let v = io.rd();
            io.emit(v >> 8);
        }
        let mut attr = io.rd();
        loop {
            let a = attr & 0xff_ffff;
            let w: [u32; 5] = std::array::from_fn(|_| io.rd() >> 8);
            io.emit(a);
            io.emit(w[0]);
            io.emit(w[1]);
            io.emit(w[2]);
            io.emit(w[3]);
            io.emit(w[4]);
            if a & 1 != 0 {
                for _ in 0..3 {
                    let v = io.rd();
                    io.emit(v >> 8);
                }
            }
            attr = io.rd();
            if attr & 3 == 0 {
                return;
            }
        }
    }

    #[inline(always)]
    fn word(&self, io: &mut Io, src: Source, k: u32) -> u32 {
        match src {
            Source::Poly(base) => io.poly.read(base.wrapping_add(k)),
            Source::Internal(base) => {
                let a = base.wrapping_add(k);
                Self::dm_index(a).map_or(0, |i| self.dm[i])
            }
        }
    }

    #[inline(always)]
    fn vertex(&self, io: &mut Io, src: Source, k: u32) -> [u32; 3] {
        [self.word(io, src, k), self.word(io, src, k + 1), self.word(io, src, k + 2)]
    }

    /// The lighting parameters for an attribute word: diffuse, ambient,
    /// specular control and scale, and the distance coefficient.
    #[inline(always)]
    fn params(&self, attr: u32) -> ([u32; 4], u32) {
        let p = PARAMS + ((attr >> 16) & 0x7c) as usize;
        ([self.dm[p], self.dm[p + 1], self.dm[p + 2], self.dm[p + 3]], self.dm[COEF + (attr >> 27) as usize])
    }

    /// fn 1, object data: a model from polygon memory, transformed, lit and
    /// sent as a polygon block (geometrizer.md §4.2).
    fn object(&mut self, io: &mut Io, cmd: u32) {
        self.dm[TESTED] = 0;
        let tpa = io.rd();
        let tha = io.rd();
        let oba = io.rd();
        let obc = io.rd();
        io.emit(cmd);
        io.emit(tpa & 0xff_ffff);
        io.emit(tha & 0xff_ffff);
        let src = if oba & 1 << 24 != 0 { Source::Internal(oba.wrapping_add(0xff03_0100)) } else { Source::Poly(oba) };
        let mode = self.dm[MODE_WORD];
        let c = Ctx {
            t: [self.dm[TRANSLATION], self.dm[TRANSLATION + 1], self.dm[TRANSLATION + 2]],
            m: std::array::from_fn(|k| self.dm[MATRIX + k]),
            fx: self.dm[FOCUS],
            fy: self.dm[FOCUS + 1],
            l: [self.dm[LIGHT], self.dm[LIGHT + 1], self.dm[LIGHT + 2]],
            lod: self.dm[LOD],
        };
        let polygons = if obc == 0 { MAX_POLYGONS } else { obc as u64 };
        let specular = mode & 1 != 0;
        let p0 = c.xform(self.vertex(io, src, 0));
        if self.program == Program::Indy500 && mode & 2 != 0 {
            Ctx { fy: c.fx, ..c }.emit(io, p0);
        } else {
            c.emit(io, p0);
        }
        let p1 = c.xform(self.vertex(io, src, 3));
        c.emit(io, p1);
        let (done, ahead) = if mode & 2 == 0 {
            (self.given_normals(io, &c, src, polygons, specular), 4)
        } else {
            self.pm[P0..P0 + 3].copy_from_slice(&p0);
            self.dm[RING..RING + 3].copy_from_slice(&p1);
            (self.computed_normals(io, &c, src, polygons, specular), 6)
        };
        // The program reads each polygon's first words while it finishes
        // the one before: the address latch is left a few words into the
        // polygon after the last one drawn.
        if let Source::Poly(oba) = src {
            *io.latch = oba.wrapping_add(6 + 10 * done as u32 + ahead);
        }
    }

    /// Mode bit 1 clear: each polygon's normal comes with it. Returns the
    /// number of polygons drawn.
    fn given_normals(&mut self, io: &mut Io, c: &Ctx, src: Source, polygons: u64, specular: bool) -> u64 {
        let fast = matches!(src, Source::Internal(_));
        let mut at = 6u32;
        for done in 0..polygons {
            let attr = self.word(io, src, at);
            if fast {
                self.dm[LAST_ATTR] = attr & 0x3_ffff;
            }
            if attr & 3 == 0 {
                return done;
            }
            io.emit(attr & 0x3_ffff);
            let n = c.rotate(self.vertex(io, src, at + 1));
            if !fast {
                self.dm[RING..RING + 3].copy_from_slice(&n);
            }
            let pa = c.xform(self.vertex(io, src, at + 4));
            let dotl = dot(n, c.l);
            let dotp = dot(n, pa);
            let back = if lt(pass_flags(dotp)) { 0x100 } else { 0 };
            let ([diffuse, ambient, control, scale], coef) = self.params(attr);
            let mut lum = add(mul(diffuse_term(dotl, dotp), diffuse), ambient);
            if specular {
                let s = max(sub(mul(add(dotl, dotl), n[2]), c.l[2]), 0);
                lum = add(lum, mul(specular_power(s, control), scale));
            }
            let luma = sat_add(clip(fix(lum), 0xff), back) << 15;
            let dist = mul(mul(coef, abs(dotp)), c.lod);
            io.emit(luma);
            io.emit(dist >> 8);
            c.emit(io, pa);
            if attr & 1 != 0 {
                let pb = c.xform(self.vertex(io, src, at + 7));
                c.emit(io, pb);
            }
            at = at.wrapping_add(10);
        }
        // The fast-RAM code sends the polygon's last word from the loop's
        // last instruction and again after the loop: an object that ends
        // by its count, not a terminator, ends with that word twice.
        if fast {
            if let Some(&w) = io.out.last() {
                io.emit(w);
            }
        }
        polygons
    }

    /// Mode bit 1 set: each polygon's normal is computed from its new
    /// vertex and the previous edge (whose vertices the program tracks as
    /// the rasteriser links them, geometrizer.md §3.4), normalised with
    /// RSQRTS and two Newton steps. The normals in the model are skipped.
    /// Returns the number of polygons drawn.
    fn computed_normals(&mut self, io: &mut Io, c: &Ctx, src: Source, polygons: u64, specular: bool) -> u64 {
        // The ring of three vertices, by word: the new vertex goes at `r`,
        // the previous edge's second vertex is the three words before it.
        // The edge's first vertex is at program-memory word `q`.
        let fast = matches!(src, Source::Internal(_));
        let (mut r, mut q) = (3usize, P0);
        let ring = |k: usize| RING + k % 9;
        let mut at = 6u32;
        for done in 0..polygons {
            let attr = self.word(io, src, at);
            if attr & 3 == 0 {
                return done;
            }
            if q + 3 > PM_WORDS {
                self.stop = Some(Stop::Unsupported(0x01));
                return done;
            }
            let quad = attr & 1 != 0;
            io.emit(attr & 0x3_ffff);
            let pa = c.xform(self.vertex(io, src, at + 4));
            for (k, &v) in pa.iter().enumerate() {
                self.dm[ring(r + k)] = v;
            }
            let p1: [u32; 3] = std::array::from_fn(|k| self.dm[ring(r + 6 + k)]);
            let p0: [u32; 3] = [self.pm[q], self.pm[q + 1], self.pm[q + 2]];
            let a: [u32; 3] = std::array::from_fn(|k| sub(pa[k], p0[k]));
            let b: [u32; 3] = std::array::from_fn(|k| sub(p1[k], p0[k]));
            let cz = sub(mul(a[1], b[0]), mul(a[0], b[1]));
            let cy = sub(mul(a[0], b[2]), mul(a[2], b[0]));
            let cx = sub(mul(a[2], b[1]), mul(a[1], b[2]));
            let len2 = add(mul(cx, cx), add(mul(cz, cz), mul(cy, cy)));
            let mut y = rsqrts(len2);
            for _ in 0..2 {
                y = mul(mul(HALF, y), sub(THREE, mul(mul(y, y), len2)));
            }
            let n = [mul(cx, y), mul(cy, y), mul(cz, y)];
            let raw_pb = quad.then(|| self.vertex(io, src, at + 7));
            if let (Some(raw), false) = (raw_pb, fast) {
                self.dm[PB_RAW..PB_RAW + 3].copy_from_slice(&raw);
            }
            let dotl = dot(n, c.l);
            let e = [add(pa[0], 0), pa[1], add(pa[2], 0)];
            let (dotp, f) = add_flags(add(mul(n[0], e[0]), mul(n[1], e[1])), mul(n[2], e[2]));
            let back = if lt(f) { 0x100 } else { 0 };
            let ([diffuse, ambient, control, scale], coef) = self.params(attr);
            let mut lum = add(mul(diffuse_term(dotl, dotp), diffuse), ambient);
            if specular {
                let s = max(sub(mul(n[2], add(dotl, dotl)), c.l[2]), 0);
                lum = add(mul(specular_power(s, control), scale), lum);
            }
            let luma = (sat_add(clip(fix(lum), 0xff), back) << 23) >> 8;
            let dist = mul(mul(coef, abs(dotp)), c.lod);
            io.emit(luma);
            io.emit(dist >> 8);
            c.emit(io, pa);
            if let Some(raw) = raw_pb {
                let pb = c.xform(raw);
                c.emit(io, pb);
                for (k, &v) in pb.iter().enumerate() {
                    self.dm[ring(r + 3 + k)] = v;
                }
            }
            // The next polygon's edge, by this one's link type.
            let link = (attr >> 8) & 3;
            let set_p0 = |s: &mut Self, from: usize, n: usize| {
                for k in 0..n {
                    s.pm[q + k] = s.dm[ring(from + k)];
                }
            };
            match (link, quad) {
                // (Pa, Pb): a triangle's Pb is its Pa.
                (0 | 2, true) => {
                    set_p0(self, r, 3);
                    r = (r + 6) % 9;
                }
                // The fast-RAM code leaves this case after moving only x
                // (its last two moves sit past a jump that isn't delayed):
                // the edge keeps the old y and z, and both pointers are
                // left one word on.
                (0 | 2, false) if fast => {
                    set_p0(self, r, 1);
                    r = (r + 2) % 9;
                    q += 1;
                }
                (0 | 2, false) => {
                    set_p0(self, r, 3);
                    r = (r + 3) % 9;
                }
                // (P0 kept, Pa).
                (1, _) => r = (r + 3) % 9,
                // (Pb, P1 kept).
                (_, true) => set_p0(self, r + 3, 3),
                (_, false) => set_p0(self, r, 3),
            }
            at = at.wrapping_add(10);
        }
        polygons
    }
}

impl state::State for Native {
    fn save(&self, w: &mut state::Writer) {
        self.dm.save(w);
        self.pm.to_vec().save(w);
        self.check.save(w);
        self.boot_frames.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        self.dm.load(r)?;
        let mut pm = Vec::new();
        pm.load(r)?;
        self.pm.copy_from_slice(&pm);
        self.check.load(r)?;
        self.boot_frames.load(r)?;
        self.stop = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(v: f32) -> u32 {
        v.to_bits()
    }

    /// Run one frame of `list` (words) on `n` with polygon ROM `rom`.
    fn frame(n: &mut Native, list: &[u32], rom: &[u32]) -> Vec<u32> {
        let buffer: Vec<u8> =
            list.iter().flat_map(|w| w.to_le_bytes()).chain(std::iter::repeat(0)).take(0x2_0000).collect();
        let (mut words, mut fed, mut out, mut latch) = (0, Vec::new(), Vec::new(), 0);
        let mut ram = vec![0; 0x1_0000];
        let mut list = ListDma::new(&buffer, &[], 0, &mut words, &mut fed);
        let mut io = Io { list: &mut list, poly: GeoExt::new(&mut ram, rom), latch: &mut latch, out: &mut out };
        n.run_frame(&mut io);
        out
    }

    fn cmd(fnc: u32) -> u32 {
        fnc << 23 | fnc << 8 | fnc
    }

    #[test]
    fn arithmetic_helpers() {
        assert_eq!(clip(300, 0xff), 255);
        assert_eq!(clip((-300i32) as u32, 0xff), (-255i32) as u32);
        assert_eq!(clip(17, 0xff), 17);
        assert_eq!(fix(0x7fc0_0000), u32::MAX, "NaN");
        assert_eq!(fix(f(2.75)), 2);
        assert_eq!(fix(f(-2.25)), (-3i32) as u32, "toward minus infinity");
        assert_eq!(sat_add(0x7fff_ffff, 1), 0x7fff_ffff);
        // Rounded toward zero: 1 - 2^-30 is just below 1.
        assert_eq!(sub(f(1.0), f(2f32.powi(-30))), 0x3f7f_ffff);
        let s = f(0.5);
        let p = |c| f32::from_bits(specular_power(s, c));
        assert_eq!([p(0), p(1), p(2), p(3), p(4), p(7)], [0.0, 0.5, 0.25, 0.25, 0.0625, 0.00390625]);
    }

    #[test]
    fn unknown_programs_are_left_to_the_sharc() {
        assert_eq!(recognise(&[0; 16]), None);
    }

    /// An object in the given-normal mode: its block's layout, the luma of
    /// a lit polygon facing the eye and its distance.
    #[test]
    fn object_block() {
        let mut n = Native::new(Program::Shared2B, NO_CHECK_KEY);
        // The boot's two handshakes: the first frame reads nothing.
        assert!(frame(&mut n, &[cmd(0x0f)], &[]).is_empty());
        let identity = [1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 10.0];
        let mut list = vec![cmd(0x0b)];
        list.extend(identity.map(f));
        list.extend([cmd(0x09), f(2.0), f(3.0), cmd(0x0a), f(0.0), f(0.0), f(1.0), cmd(0x16), f(4.0)]);
        // Entry 1: diffuse 100, ambient 20, coefficient 0.5.
        list.extend([cmd(0x06), 1, 1, 20 << 8 | 100, f(0.5), cmd(0x07), 0]);
        list.extend([cmd(0x01), 0x12_3456, 0x0065_4321, 0x80_0000, 0, cmd(0x0f)]);
        // P0, P1, a quad facing +z (lighting entry and coefficient 1), the
        // terminator.
        let mut rom = vec![f(0.0), f(0.0), f(0.0), f(1.0), f(0.0), f(0.0)];
        rom.extend([1 | 1 << 18 | 1 << 27, f(0.0), f(0.0), f(1.0)]);
        rom.extend([f(1.0), f(1.0), f(0.0), f(0.0), f(1.0), f(0.0)]);
        rom.push(0);
        let out = frame(&mut n, &list, &rom);
        let v = |x: f32| f(x) >> 8;
        assert_eq!(out[..3], [0xff00_0001, 0x12_3456, 0x65_4321]);
        assert_eq!(out[3..9], [v(0.0), v(0.0), v(10.0), v(2.0), v(0.0), v(10.0)]);
        assert_eq!(out[9], 1, "attr & 0x3FFFF");
        // n·L = 1, n·Pa = 10: lit, front: 1·100 + 20.
        assert_eq!(out[10], 120 << 15);
        // 0.5 · |n·Pa| · LOD.
        assert_eq!(out[11], v(20.0));
        assert_eq!(out[12..18], [v(2.0), v(3.0), v(10.0), v(0.0), v(3.0), v(10.0)]);
        assert_eq!(out.len(), 18);
    }

    /// With the check on (no MSGR7 key), a bad command word stops it.
    #[test]
    fn command_check() {
        let mut n = Native::new(Program::Shared2B, 0);
        frame(&mut n, &[], &[]);
        assert_eq!(frame(&mut n, &[cmd(0x08), f(1.0), cmd(0x0f)], &[]).len(), 2);
        assert!(frame(&mut n, &[0x0400_0000, cmd(0x0f)], &[]).is_empty());
        assert_eq!(n.stop, Some(Stop::BadCommand(0x0400_0000)));
        assert!(frame(&mut n, &[cmd(0x08), f(1.0), cmd(0x0f)], &[]).is_empty(), "for good");
    }

    #[test]
    fn state_round_trip() {
        let mut n = Native::new(Program::Shared2B, NO_CHECK_KEY);
        frame(&mut n, &[], &[]);
        frame(&mut n, &[cmd(0x0a), f(1.0), f(2.0), f(3.0), cmd(0x07), 3, cmd(0x0f)], &[]);
        let bytes = state::to_bytes(&n);
        let mut m = Native::new(Program::Shared2B, 0);
        state::State::load(&mut m, &mut state::Reader::new(&bytes)).unwrap();
        assert_eq!(m.data(), n.data());
        assert_eq!((m.check, m.boot_frames), (n.check, n.boot_frames));
    }
}
