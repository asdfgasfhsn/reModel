//! High-level model of the Model 2A TGP's *programs*: what the functions
//! of `srallyc`'s program do, run natively instead of on the chip.
//!
//! **This is a stepping stone, not the chip.** The MB86234's instruction
//! set is not known (`docs/hw/mb86234.md`), so the program cannot be run;
//! this module reads the program's *structure and data flow* instead
//! (function entry points, how many words each takes and gives, which
//! memory cells each touches) and does what each function was found to
//! do. Every function says how sure it is (`docs/hw/model2a.md` §6):
//!
//! - **structure** (confirmed from the program): the matrix stack and its
//!   depth rule, set/read/reset/identity, the command counter, the error
//!   counter, the vector registers;
//! - **maths read from the data flow, conventions assumed** (row vectors,
//!   right-handed rotations, local-first order): translate, scale,
//!   rotate, transform;
//! - **guessed**: the two 6-in-1-out functions 0x58 and 0x5C.
//!
//! Nothing here depends on the game; the program is recognised by
//! [`recognise`], so a different 2A program (other function numbers) gets
//! no model and keeps the old stand-in behaviour.

use std::collections::VecDeque;

/// FNV-1a of the program words, the key a program is recognised by.
pub fn program_hash(program: &[u32]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for &w in program {
        for b in w.to_le_bytes() {
            h = (h ^ b as u64).wrapping_mul(0x100_0000_01b3);
        }
    }
    h
}

/// Programs with a model, by [`program_hash`]: `srallyc`'s (1,665 words,
/// uploaded from its program ROM at `0x5AE94`).
const PROGRAMS: &[u64] = &[0xb911_6fa5_18de_9e86];

/// The model for a program, if it is one of the known ones.
pub fn recognise(program: &[u32]) -> Option<Hle> {
    PROGRAMS.contains(&program_hash(program)).then(Hle::default)
}

/// Words a function takes after its header, or `None` for a function the
/// model doesn't know (the game never calls one). The counts are those of
/// the calls in a 4,800-frame run; the program's code reads the same
/// number of words from the input FIFO for the ones that were checked.
pub fn args(f: u32) -> Option<usize> {
    Some(match f {
        0x02 | 0x04 | 0x05 | 0x20 | 0x21 | 0x23 | 0x25 | 0x26 => 0,
        0x03 | 0x29 | 0x2a | 0x2b | 0x32 | 0x33 | 0x34 | 0x43 | 0x48 => 1,
        0x22 => 12,
        0x27 | 0x28 | 0x2c | 0x2f | 0x54 => 3,
        0x42 | 0x44 => 4,
        0x5f => 5,
        0x58 | 0x5c => 6,
        0x5d | 0x5e => 7,
        _ => return None,
    })
}

/// How deep the matrix stack goes before a push stops copying (the
/// program compares the depth with 7; deeper pushes only count).
pub const STACK_LIMIT: u32 = 7;

#[derive(Clone, Debug, Default)]
pub struct Hle {
    /// The matrix stack: 12 floats each (a 3x3 rotation, row by row, then
    /// the translation), as bit patterns. Slot `slot` is the current one.
    pub mats: [[u32; 12]; 8],
    pub slot: u32,
    /// Pushes minus pops (it keeps counting past the limit).
    pub depth: u32,
    /// The four vector registers (`0x42` sets, `0x43` reads, `0x44` adds).
    pub vecs: [[u32; 3]; 4],
    /// Commands since the host last set it (`0x03`); `0x04` reads it.
    pub counter: u32,
    /// Unknown-function errors (`0x02` reads it): the program counts
    /// words it can't make a command of; the model never errs.
    pub errors: u32,
    /// The command being received: function and the words so far.
    pending: Option<(u32, Vec<u32>)>,
}

impl Hle {
    /// A command starts (a header word, or a store to a command port).
    pub fn begin(&mut self, f: u32, out: &mut VecDeque<u32>) {
        match args(f) {
            // A function the model doesn't know is a command with no words.
            None => {
                self.pending = None;
                self.counter = self.counter.wrapping_add(1);
            }
            Some(n) => {
                self.pending = Some((f, Vec::new()));
                if n == 0 {
                    self.finish(out);
                }
            }
        }
    }

    /// Is the next word a command header (no call waiting for words)?
    pub fn expects_header(&self) -> bool {
        self.pending.is_none()
    }

    /// An argument word arrives.
    pub fn data(&mut self, w: u32, out: &mut VecDeque<u32>) {
        let Some((f, a)) = &mut self.pending else { return };
        a.push(w);
        if Some(a.len()) == args(*f) {
            self.finish(out);
        }
    }

    fn finish(&mut self, out: &mut VecDeque<u32>) {
        let Some((f, a)) = self.pending.take() else { return };
        // The program's main loop adds one to the counter after every
        // command, the one that set it included.
        let counter_before = self.counter;
        self.run(f, &a, out, counter_before);
        self.counter = self.counter.wrapping_add(1);
    }

    fn cur(&mut self) -> &mut [u32; 12] {
        &mut self.mats[self.slot as usize]
    }

    fn run(&mut self, f: u32, a: &[u32], out: &mut VecDeque<u32>, counter: u32) {
        let fl = |w: u32| f32::from_bits(w);
        match f {
            0x02 => out.push_back(self.errors),
            0x03 => self.counter = a[0],
            0x04 => out.push_back(counter),
            // Push: copy the current matrix to the next slot and move on,
            // unless the stack is full; the depth counts either way.
            0x20 => {
                if self.depth < STACK_LIMIT {
                    let m = *self.cur();
                    self.slot += 1;
                    *self.cur() = m;
                }
                self.depth += 1;
            }
            // Pop: nothing at depth 0; a pop of a push that didn't copy
            // (depth 8 and over) only counts.
            0x21 => {
                if self.depth > 0 {
                    if self.depth < STACK_LIMIT + 1 {
                        self.slot -= 1;
                    }
                    self.depth -= 1;
                }
            }
            0x22 => self.cur().copy_from_slice(a),
            // Reset: back to the bottom slot, depth 0. The matrix itself
            // is left as it was.
            0x23 => {
                self.slot = 0;
                self.depth = 0;
            }
            // Identity (the program fills 12 words with 0 and the three
            // diagonal ones, step 4, with 1.0).
            0x25 => {
                let m = self.cur();
                *m = [0; 12];
                for d in [0, 4, 8] {
                    m[d] = 1.0f32.to_bits();
                }
            }
            0x26 => out.extend(*self.cur()),
            0x27 => {
                let (v, m) = ([fl(a[0]), fl(a[1]), fl(a[2])], self.cur());
                for j in 0..3 {
                    let t = fl(m[9 + j]) + v[0] * fl(m[j]) + v[1] * fl(m[3 + j]) + v[2] * fl(m[6 + j]);
                    m[9 + j] = t.to_bits();
                }
            }
            // Scale: each row of the rotation part by its factor.
            0x28 => {
                let m = self.cur();
                for i in 0..3 {
                    for j in 0..3 {
                        m[3 * i + j] = (fl(m[3 * i + j]) * fl(a[i])).to_bits();
                    }
                }
            }
            0x29..=0x2b => self.rotate(f - 0x29, fl(a[0])),
            0x2c => {
                let (v, m) = ([fl(a[0]), fl(a[1]), fl(a[2])], *self.cur());
                for j in 0..3 {
                    let r = v[0] * fl(m[j]) + v[1] * fl(m[3 + j]) + v[2] * fl(m[6 + j]) + fl(m[9 + j]);
                    out.push_back(r.to_bits());
                }
            }
            0x2f => {
                let v = [fl(a[0]), fl(a[1]), fl(a[2])];
                let len2 = v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
                let inv = if len2 > 0.0 { 1.0 / len2.sqrt() } else { 0.0 };
                out.extend(v.map(|x| (x * inv).to_bits()));
            }
            // Vector registers.
            0x42 => {
                if let Some(v) = self.vecs.get_mut(a[0] as usize) {
                    v.copy_from_slice(&a[1..4]);
                }
            }
            0x43 => out.extend(self.vecs.get(a[0] as usize).copied().unwrap_or([0; 3])),
            0x44 => {
                if let Some(v) = self.vecs.get_mut(a[0] as usize) {
                    for i in 0..3 {
                        v[i] = (fl(v[i]) + fl(a[1 + i])).to_bits();
                    }
                }
            }
            // The program pairs its six words (0, 1), (2, 3), (4, 5),
            // subtracts within each pair and takes the root of the sum of
            // squares. *Guessed* for both.
            0x58 | 0x5c => {
                let d = |i: usize| fl(a[2 * i + 1]) - fl(a[2 * i]);
                out.push_back((d(0) * d(0) + d(1) * d(1) + d(2) * d(2)).sqrt().to_bits());
            }
            // The rest take their words and give nothing the host reads:
            // output to the geometrizer, state the model doesn't keep.
            _ => {}
        }
    }

    /// Rotate the current matrix about axis `axis` (0 x, 1 y, 2 z) by `a`
    /// radians, applied before the matrix (`M' = R M`, row vectors).
    fn rotate(&mut self, axis: u32, a: f32) {
        let (s, c) = (f64::from(a).sin() as f32, f64::from(a).cos() as f32);
        let r = match axis {
            0 => [[1.0, 0.0, 0.0], [0.0, c, s], [0.0, -s, c]],
            1 => [[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]],
            _ => [[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]],
        };
        let m = self.cur();
        let old: Vec<f32> = m[..9].iter().map(|&w| f32::from_bits(w)).collect();
        for i in 0..3 {
            for j in 0..3 {
                let v = r[i][0] * old[j] + r[i][1] * old[3 + j] + r[i][2] * old[6 + j];
                m[3 * i + j] = v.to_bits();
            }
        }
    }
}

impl state::State for Hle {
    fn save(&self, w: &mut state::Writer) {
        for m in &self.mats {
            m.save(w);
        }
        self.slot.save(w);
        self.depth.save(w);
        for v in &self.vecs {
            v.save(w);
        }
        self.counter.save(w);
        self.errors.save(w);
        let (f, a) = self.pending.clone().unwrap_or((u32::MAX, Vec::new()));
        f.save(w);
        a.save(w);
    }
    fn load(&mut self, r: &mut state::Reader) -> Result<(), state::Error> {
        for m in &mut self.mats {
            m.load(r)?;
        }
        self.slot.load(r)?;
        self.depth.load(r)?;
        for v in &mut self.vecs {
            v.load(r)?;
        }
        self.counter.load(r)?;
        self.errors.load(r)?;
        let (mut f, mut a) = (0u32, Vec::new());
        f.load(r)?;
        a.load(r)?;
        self.pending = (f != u32::MAX).then_some((f, a));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(h: &mut Hle, f: u32, a: &[f32]) -> Vec<f32> {
        let mut out = VecDeque::new();
        h.begin(f, &mut out);
        for w in a {
            h.data(w.to_bits(), &mut out);
        }
        out.into_iter().map(f32::from_bits).collect()
    }

    #[test]
    fn matrix_round_trip_and_stack() {
        let mut h = Hle::default();
        run(&mut h, 0x23, &[]);
        run(&mut h, 0x25, &[]);
        let id = run(&mut h, 0x26, &[]);
        assert_eq!(id, [1., 0., 0., 0., 1., 0., 0., 0., 1., 0., 0., 0.]);
        let set: Vec<f32> = (1..=12).map(|i| i as f32).collect();
        run(&mut h, 0x22, &set);
        run(&mut h, 0x20, &[]);
        assert_eq!(run(&mut h, 0x26, &[]), set, "a push copies");
        run(&mut h, 0x27, &[0., 0., 0.]);
        let other: Vec<f32> = (21..=32).map(|i| i as f32).collect();
        run(&mut h, 0x22, &other);
        run(&mut h, 0x21, &[]);
        assert_eq!(run(&mut h, 0x26, &[]), set, "a pop restores");
    }

    #[test]
    fn deep_pushes_only_count() {
        let mut h = Hle::default();
        run(&mut h, 0x25, &[]);
        for _ in 0..10 {
            run(&mut h, 0x20, &[]);
        }
        assert_eq!((h.slot, h.depth), (STACK_LIMIT, 10));
        for _ in 0..10 {
            run(&mut h, 0x21, &[]);
        }
        assert_eq!((h.slot, h.depth), (0, 0));
        run(&mut h, 0x21, &[]);
        assert_eq!((h.slot, h.depth), (0, 0), "an empty stack ignores a pop");
    }

    #[test]
    fn command_counter_counts_the_setter_too() {
        let mut h = Hle::default();
        run(&mut h, 0x03, &[f32::from_bits(0)]);
        run(&mut h, 0x23, &[]);
        run(&mut h, 0x23, &[]);
        // fn 3 itself, then two others.
        assert_eq!(run(&mut h, 0x04, &[]), [f32::from_bits(3)]);
        assert_eq!(run(&mut h, 0x02, &[]), [0.0]);
    }

    #[test]
    fn transforms_follow_the_matrix() {
        let mut h = Hle::default();
        run(&mut h, 0x25, &[]);
        run(&mut h, 0x27, &[1., 2., 3.]);
        assert_eq!(run(&mut h, 0x2c, &[10., 20., 30.]), [11., 22., 33.]);
        run(&mut h, 0x25, &[]);
        run(&mut h, 0x28, &[2., 1., 3.]);
        assert_eq!(run(&mut h, 0x2c, &[1., 1., 1.]), [2., 1., 3.]);
        run(&mut h, 0x25, &[]);
        run(&mut h, 0x2b, &[std::f32::consts::FRAC_PI_2]);
        let r = run(&mut h, 0x2c, &[1., 0., 0.]);
        assert!(r[0].abs() < 1e-6 && (r[1] - 1.0).abs() < 1e-6 && r[2] == 0.0, "{r:?}");
        let n = run(&mut h, 0x2f, &[3., 0., 4.]);
        assert!((n[0] - 0.6).abs() < 1e-6 && (n[2] - 0.8).abs() < 1e-6);
    }

    #[test]
    fn vector_registers() {
        let mut h = Hle::default();
        run(&mut h, 0x42, &[f32::from_bits(2), 1., 2., 3.]);
        run(&mut h, 0x44, &[f32::from_bits(2), 1., 1., 1.]);
        assert_eq!(run(&mut h, 0x43, &[f32::from_bits(2)]), [2., 3., 4.]);
    }

    #[test]
    fn state_round_trips() {
        let mut h = Hle::default();
        run(&mut h, 0x25, &[]);
        run(&mut h, 0x20, &[]);
        let mut out = VecDeque::new();
        h.begin(0x22, &mut out);
        h.data(5, &mut out);
        let bytes = state::to_bytes(&h);
        let mut g = Hle::default();
        state::State::load(&mut g, &mut state::Reader::new(&bytes)).unwrap();
        assert_eq!((g.slot, g.depth, g.pending.clone()), (1, 1, Some((0x22, vec![5]))));
        assert_eq!(g.mats, h.mats);
    }
}
