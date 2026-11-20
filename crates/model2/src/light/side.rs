//! The side channel ([`Frame`]): what a geometrizer knew about a frame's
//! lighting that the raster stream doesn't carry. Per object block (one
//! raster `0x1` command) its model address, the focal lengths and light
//! vector in effect and a few facts about the program (level L1); per
//! polygon record the normal the program lit it with, its facing and its
//! lighting entry's values (level L2).
//!
//! A native geometrizer fills it through a [`Tap`] (`geo_native::Io::light`,
//! two hooks per program: a block, a face); the chip path gets block facts
//! from a scan of the display list (`scan_list`). Every program difference
//! is turned into generic flags where it is recorded, so nothing here knows
//! a program: 2B lights single-sided back faces and rounds toward zero
//! ([`Block::TRUNCATE`]), 2C sends luma 0 for them ([`Block::BACK_UNLIT`]),
//! the specular term starts from the signed or the clamped diffuse dot
//! product ([`Base`]), and how many times it is squared is resolved per
//! face ([`Material::squarings`]).
//!
//! [`check_mesh`] recomputes every polygon's flat luma from the side
//! channel with [`Lamp::eval`] and compares it with the stream's: a block
//! that disagrees anywhere is demoted to L1 for the frame, so a wrong
//! formula or flag can never change a picture beyond what the stream alone
//! gives.

use super::{quantise, Mesh};
use std::collections::HashMap;

/// The first word of [`Frame::to_words`].
pub const MAGIC: u32 = u32::from_le_bytes(*b"LGT1");

/// One object block: a raster `0x1` command the geometrizer emitted, in
/// stream order (9 words in [`Frame::to_words`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Block {
    /// Model address (polygon ROM from `0x800000`, polygon RAM below); 0
    /// for direct data.
    pub oba: u32,
    /// [`Block::DIRECT`] and the rest.
    pub flags: u32,
    /// Focal lengths in effect: stream x, y are view space times these.
    pub fx: f32,
    pub fy: f32,
    /// The light vector as the list sent it (dotted with view-space
    /// normals as it is).
    pub light: [f32; 3],
    /// Its faces in [`Frame::faces`] (count 0 without [`Block::HAS_FACES`]).
    pub first: u32,
    pub count: u32,
    /// SPIKE: hash of the polygon words the geometrizer read for this
    /// object (the model's own data); 0 for direct data. Not in the
    /// capture's first 9 words per block: a trailing section.
    pub key: u64,
    /// SPIKE (tier 2): the transform in effect, as the program held it:
    /// matrix 0 (9 floats, the images of x, y, z), translation 0 (3),
    /// then the same for matrix 1 (`topskatr`'s second one; a copy of the
    /// first elsewhere).
    pub xf: [f32; 24],
    /// SPIKE (tier 2b): the lighting entry the model's first polygon names
    /// (attribute bits 18-22) and its material in [`Frame::materials`],
    /// read from the model's own data even when a rerun removed or
    /// replaced it: what a replacement inherits.
    pub entry: Option<(u8, u16)>,
    /// SPIKE (lod-wide): byte address in the display list buffer of the
    /// object command that made the block (0: unknown).
    pub lst: u32,
}

impl Block {
    /// Direct data (fn 2): polygons given in view space with their luma.
    pub const DIRECT: u32 = 1;
    /// Normals computed from the vertices (mode bit 1, or a compact model).
    pub const COMPUTED: u32 = 2;
    /// Its faces are recorded.
    pub const HAS_FACES: u32 = 4;
    /// A back face that isn't double-sided is sent with luma 0 (2C).
    pub const BACK_UNLIT: u32 = 8;
    /// The program's arithmetic rounds toward zero (the 2B SHARC's): an
    /// IEEE recomputation may be one level off.
    pub const TRUNCATE: u32 = 0x10;
    /// Specular lighting on (mode bit 0).
    pub const SPECULAR: u32 = 0x20;
    /// A compact (skinned, two-matrix) model (`topskatr`'s fn 0x11).
    pub const COMPACT: u32 = 0x40;
    /// From the list scan, not a geometrizer.
    pub const SCANNED: u32 = 0x80;
    /// Focal lengths or light vector not known (a list scan before the
    /// list has set them).
    pub const UNKNOWN: u32 = 0x100;
    /// The light vector or the mode not known (a list scan before the list
    /// has set them): the focal lengths are.
    pub const PARTIAL: u32 = 0x200;

    /// The facts the list scan and a native geometrizer must agree on.
    pub const FACTS: u32 = Self::DIRECT | Self::COMPUTED | Self::BACK_UNLIT | Self::TRUNCATE | Self::SPECULAR;

    /// A model in polygon RAM (written by the list, may change any frame),
    /// not ROM.
    pub fn poly_ram(&self) -> bool {
        self.flags & Self::DIRECT == 0 && self.oba & 0xff_ffff < 0x80_0000
    }

    /// The focal lengths, if known and usable.
    pub fn focal(&self) -> Option<(f32, f32)> {
        let ok = |f: f32| f.is_finite() && f != 0.0;
        (self.flags & Self::UNKNOWN == 0 && ok(self.fx) && ok(self.fy)).then_some((self.fx, self.fy))
    }

    /// The block facts only (what a list scan gives), to compare with
    /// another source's.
    pub fn facts(&self) -> (u32, u32, [u32; 2], [u32; 3]) {
        (self.oba, self.flags & Self::FACTS, [self.fx.to_bits(), self.fy.to_bits()], self.light.map(f32::to_bits))
    }
}

/// Which dot product the specular term starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Base {
    /// `d`: the diffuse dot product, negated for a back face (2C).
    #[default]
    D,
    /// `max(d, 0)`: clamped first (2C given-normal triangles, `overrev`).
    S0,
    /// `n·L` as it is, whatever the facing (2B).
    Dotl,
}

/// One polygon record emitted, in stream order (4 words).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Face {
    /// The view-space normal the program lit with (rotated from the model,
    /// or computed and normalised by the program; not focal-scaled).
    pub n: [f32; 3],
    /// Index into [`Frame::materials`].
    pub mat: u16,
    /// [`Face::FRONT`] and the rest; bits 8-12 the lighting entry.
    pub flags: u16,
}

impl Face {
    /// Faces the eye by the program's own test (`n·Pa >= 0` in its
    /// arithmetic): lit with `n·L`, else with `-n·L`.
    pub const FRONT: u16 = 1;
    /// No usable normal (zero, not finite).
    pub const NONE: u16 = 2;
    /// The specular base ([`Base`]): S0, or with this n·L (else D).
    pub const BASE_S0: u16 = 4;
    pub const BASE_DOTL: u16 = 8;
    /// The normal is zero: the model's "unlit" marker (luma = ambient).
    pub const ZERO: u16 = 0x10;
    /// The program never normalised it (a culled 2C back face): the raw
    /// cross product, normalised here.
    pub const RAW: u16 = 0x20;
    /// Lit from the other side than it faces: the diffuse term is
    /// `max(-d, 0)` for a front face, `max(n·L, 0)` for a back one (2B: a
    /// polygon edge-on to the eye, n·P = ±0, gets |n·L|).
    pub const FLIP: u16 = 0x40;
    /// Bits 8-12: the lighting entry (attribute bits 18-22).
    pub const ENTRY_SHIFT: u16 = 8;

    pub fn front(&self) -> bool {
        self.flags & Self::FRONT != 0
    }

    pub fn base(&self) -> Base {
        if self.flags & Self::BASE_S0 != 0 {
            Base::S0
        } else if self.flags & Self::BASE_DOTL != 0 {
            Base::Dotl
        } else {
            Base::D
        }
    }

    /// The lighting entry (0-31).
    pub fn entry(&self) -> u8 {
        ((self.flags >> Self::ENTRY_SHIFT) & 0x1f) as u8
    }
}

/// A lighting entry's values as a polygon was lit with them (4 words),
/// deduplicated per frame by bit pattern.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Material {
    pub ambient: f32,
    pub diffuse: f32,
    pub spec_scale: f32,
    /// Times the specular term is squared; [`Material::NO_SPEC`]: no
    /// specular term.
    pub squarings: u32,
}

impl Material {
    pub const NO_SPEC: u32 = 255;
}

/// A frame's side channel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frame {
    /// The geometrizer's frame count.
    pub frame: u64,
    pub blocks: Vec<Block>,
    pub faces: Vec<Face>,
    pub materials: Vec<Material>,
}

impl Frame {
    /// As words: `MAGIC`, frame (low, high), the counts of blocks, faces
    /// and materials, then each in order (floats as their bits).
    pub fn to_words(&self) -> Vec<u32> {
        let mut w = Vec::with_capacity(6 + 9 * self.blocks.len() + 4 * self.faces.len() + 4 * self.materials.len());
        w.extend([
            MAGIC,
            self.frame as u32,
            (self.frame >> 32) as u32,
            self.blocks.len() as u32,
            self.faces.len() as u32,
            self.materials.len() as u32,
        ]);
        for b in &self.blocks {
            w.extend([b.oba, b.flags, b.fx.to_bits(), b.fy.to_bits()]);
            w.extend(b.light.map(f32::to_bits));
            w.extend([b.first, b.count]);
        }
        for f in &self.faces {
            w.extend(f.n.map(f32::to_bits));
            w.push(f.mat as u32 | (f.flags as u32) << 16);
        }
        for m in &self.materials {
            w.extend([m.ambient.to_bits(), m.diffuse.to_bits(), m.spec_scale.to_bits(), m.squarings]);
        }
        // SPIKE: model keys, two words per block, after everything else.
        if self.blocks.iter().any(|b| b.key != 0) {
            for b in &self.blocks {
                w.extend([b.key as u32, (b.key >> 32) as u32, b.lst]);
            }
            // SPIKE (lod-wide): then the transforms, 24 words per block.
            for b in &self.blocks {
                w.extend(b.xf.map(f32::to_bits));
            }
        }
        w
    }

    /// [`Frame::to_words`] read back; None if malformed (or the counts
    /// don't fit the words).
    pub fn from_words(w: &[u32]) -> Option<Frame> {
        let head = w.get(..6)?;
        if head[0] != MAGIC {
            return None;
        }
        let (nb, nf, nm) = (head[3] as usize, head[4] as usize, head[5] as usize);
        let base = 6usize
            .checked_add(nb.checked_mul(9)?)?
            .checked_add(nf.checked_mul(4)?)?
            .checked_add(nm.checked_mul(4)?)?;
        if w.len() != base && w.len() != base + 2 * nb && w.len() != base + 3 * nb && w.len() != base + 27 * nb {
            return None;
        }
        let keys: &[u32] = &w[base..];
        let stride = if keys.len() >= 3 * nb && nb > 0 { 3 } else { 2 };
        let xfs: &[u32] = if keys.len() == 27 * nb && nb > 0 { &keys[3 * nb..] } else { &[] };
        let f = |v: u32| f32::from_bits(v);
        let mut at = 6;
        let mut take = |n: usize| {
            let s = &w[at..at + n];
            at += n;
            s
        };
        let blocks = (0..nb)
            .map(|i| {
                let b = take(9);
                Block {
                    key: if keys.is_empty() { 0 } else { keys[stride * i] as u64 | (keys[stride * i + 1] as u64) << 32 },
                    lst: if stride == 3 && !keys.is_empty() { keys[3 * i + 2] } else { 0 },
                    oba: b[0],
                    flags: b[1],
                    fx: f(b[2]),
                    fy: f(b[3]),
                    light: [f(b[4]), f(b[5]), f(b[6])],
                    first: b[7],
                    count: b[8],
                    xf: if xfs.is_empty() { [0.0; 24] } else { std::array::from_fn(|k| f32::from_bits(xfs[24 * i + k])) },
            entry: None,
                }
            })
            .collect::<Vec<_>>();
        let faces = (0..nf)
            .map(|_| {
                let x = take(4);
                Face { n: [f(x[0]), f(x[1]), f(x[2])], mat: x[3] as u16, flags: (x[3] >> 16) as u16 }
            })
            .collect::<Vec<_>>();
        let materials = (0..nm)
            .map(|_| {
                let m = take(4);
                Material { ambient: f(m[0]), diffuse: f(m[1]), spec_scale: f(m[2]), squarings: m[3] }
            })
            .collect::<Vec<_>>();
        let fr = Frame { frame: head[1] as u64 | (head[2] as u64) << 32, blocks, faces, materials };
        // Every block's faces and every face's material must exist.
        let ok = fr.blocks.iter().all(|b| (b.first as usize).checked_add(b.count as usize).is_some_and(|e| e <= nf))
            && fr.faces.iter().all(|x| (x.mat as usize) < nm);
        ok.then_some(fr)
    }

    /// A block's faces (empty without [`Block::HAS_FACES`]).
    pub fn faces_of(&self, b: &Block) -> &[Face] {
        if b.flags & Block::HAS_FACES == 0 {
            return &[];
        }
        self.faces.get(b.first as usize..(b.first + b.count) as usize).unwrap_or(&[])
    }

    /// What the frame's lists did, for the tools ([`Facts`]).
    pub fn facts(&self) -> Facts {
        let mut x =
            Facts { frames: 1, blocks: self.blocks.len() as u32, faces: self.faces.len() as u32, ..Default::default() };
        let mut lights: Vec<[u32; 3]> = Vec::new();
        let mut focals: Vec<[u32; 2]> = Vec::new();
        // Per lighting entry, the values it was last seen with (not the
        // squarings, which follow the mode).
        let mut entries: HashMap<u8, [u32; 3]> = HashMap::new();
        let values = |m: u16| {
            self.materials
                .get(m as usize)
                .map_or([0; 3], |m| [m.ambient.to_bits(), m.diffuse.to_bits(), m.spec_scale.to_bits()])
        };
        let mut remapped = 0u32;
        for b in &self.blocks {
            x.direct += (b.flags & Block::DIRECT != 0) as u32;
            x.computed += (b.flags & Block::COMPUTED != 0) as u32;
            x.specular_blocks += (b.flags & Block::SPECULAR != 0) as u32;
            if b.poly_ram() {
                x.poly_ram_blocks += 1;
                x.poly_ram_faces += self.faces_of(b).len() as u32;
            }
            let l = b.light.map(f32::to_bits);
            if b.flags & Block::UNKNOWN == 0 {
                if !lights.contains(&l) {
                    lights.push(l);
                }
                let f = [b.fx.to_bits(), b.fy.to_bits()];
                if !focals.contains(&f) {
                    focals.push(f);
                }
            }
            for fc in self.faces_of(b) {
                if self.materials.get(fc.mat as usize).is_some_and(|m| m.squarings != Material::NO_SPEC) {
                    x.specular_faces += 1;
                }
                x.zero_normals += (fc.flags & Face::ZERO != 0) as u32;
                let v = values(fc.mat);
                // The same entry with other values: fn 0x06 changed it
                // within the frame.
                if entries.insert(fc.entry(), v).is_some_and(|old| old != v) {
                    remapped += 1;
                }
            }
        }
        x.lights = lights.len() as u32;
        x.light_changes = (lights.len() as u32).saturating_sub(1);
        x.frames_light_changed = (lights.len() > 1) as u32;
        x.focal_changes = (focals.len() as u32).saturating_sub(1);
        x.entry_changes = remapped;
        x.frames_entries_changed = (remapped > 0) as u32;
        x.materials = self.materials.len() as u32;
        x
    }
}

/// Counts of what a side channel holds ([`Frame::facts`]), summable over
/// frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Facts {
    pub frames: u32,
    pub blocks: u32,
    pub faces: u32,
    pub direct: u32,
    pub computed: u32,
    /// Blocks drawing a model from polygon RAM, and their faces.
    pub poly_ram_blocks: u32,
    pub poly_ram_faces: u32,
    /// Blocks with specular on, faces lit with a specular term.
    pub specular_blocks: u32,
    pub specular_faces: u32,
    pub zero_normals: u32,
    /// Distinct light vectors in the frame, the changes (distinct - 1)
    /// and whether there was one.
    pub lights: u32,
    pub light_changes: u32,
    pub frames_light_changed: u32,
    pub focal_changes: u32,
    /// Faces whose lighting entry had other values than earlier in the
    /// frame (an fn 0x06 between objects), and whether there was one.
    pub entry_changes: u32,
    pub frames_entries_changed: u32,
    pub materials: u32,
}

impl Facts {
    pub fn add(&mut self, o: &Facts) {
        self.frames += o.frames;
        self.blocks += o.blocks;
        self.faces += o.faces;
        self.direct += o.direct;
        self.computed += o.computed;
        self.poly_ram_blocks += o.poly_ram_blocks;
        self.poly_ram_faces += o.poly_ram_faces;
        self.specular_blocks += o.specular_blocks;
        self.specular_faces += o.specular_faces;
        self.zero_normals += o.zero_normals;
        self.lights = self.lights.max(o.lights);
        self.light_changes += o.light_changes;
        self.frames_light_changed += o.frames_light_changed;
        self.focal_changes += o.focal_changes;
        self.entry_changes += o.entry_changes;
        self.frames_entries_changed += o.frames_entries_changed;
        self.materials = self.materials.max(o.materials);
    }
}

/// Records a frame's side channel as a native geometrizer runs it
/// (`geo_native::Io::light`).
#[derive(Default)]
pub struct Tap {
    blocks: Vec<Block>,
    faces: Vec<Face>,
    materials: Vec<Material>,
    known: HashMap<[u32; 4], u16>,
}

impl Tap {
    pub fn new() -> Self {
        Self::default()
    }

    /// An object block starts (a raster `0x1` command): model address,
    /// [`Block`] flags, focal lengths and light vector as the program's
    /// float bits.
    pub fn block(&mut self, oba: u32, flags: u32, fx: u32, fy: u32, l: [u32; 3]) {
        self.blocks.push(Block {
            oba,
            flags,
            fx: f32::from_bits(fx),
            fy: f32::from_bits(fy),
            light: l.map(f32::from_bits),
            first: self.faces.len() as u32,
            count: 0,
            key: 0,
            xf: [0.0; 24],
            entry: None,
            lst: 0,
        });
    }

    /// SPIKE (tier 2): the transform of the current block.
    pub fn set_xf(&mut self, m: [[u32; 9]; 2], t: [[u32; 3]; 2]) {
        if let Some(b) = self.blocks.last_mut() {
            for s in 0..2 {
                for k in 0..9 {
                    b.xf[12 * s + k] = f32::from_bits(m[s][k]);
                }
                for k in 0..3 {
                    b.xf[12 * s + 9 + k] = f32::from_bits(t[s][k]);
                }
            }
        }
    }

    /// SPIKE (lod-wide): the list address of the current block's command.
    pub fn set_lst(&mut self, lst: u32) {
        if let Some(b) = self.blocks.last_mut() {
            b.lst = lst;
        }
    }

    /// SPIKE: the model key of the current block.
    pub fn set_key(&mut self, key: u64) {
        if let Some(b) = self.blocks.last_mut() {
            b.key = key;
        }
    }

    /// A polygon record of the current block: the normal it was lit with
    /// (`raw`: an un-normalised cross product, normalised here), its
    /// facing, the specular base, the attribute word (for the lighting
    /// entry), the entry's ambient, diffuse and specular scale (float bits)
    /// and how many times the specular term was squared
    /// ([`Material::NO_SPEC`]: none).
    #[allow(clippy::too_many_arguments)]
    pub fn face(&mut self, n: [u32; 3], raw: bool, front: bool, base: Base, attr: u32, m: [u32; 3], squarings: u32) {
        let Some(b) = self.blocks.last_mut() else { return };
        b.flags |= Block::HAS_FACES;
        b.count += 1;
        let mut v = n.map(f32::from_bits);
        let mut flags = (front as u16) | (((attr >> 18) & 0x1f) as u16) << Face::ENTRY_SHIFT;
        flags |= match base {
            Base::D => 0,
            Base::S0 => Face::BASE_S0,
            Base::Dotl => Face::BASE_DOTL,
        };
        if v.iter().all(|&x| x == 0.0) {
            flags |= Face::ZERO | Face::NONE;
        } else if raw {
            flags |= Face::RAW;
            let len = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            v = v.map(|x| x / len);
        }
        if !v.iter().all(|x| x.is_finite()) {
            flags |= Face::NONE;
        }
        let mat = self.intern(m, squarings);
        self.faces.push(Face { n: v, mat, flags });
    }

    fn intern(&mut self, m: [u32; 3], squarings: u32) -> u16 {
        let squarings = squarings.min(Material::NO_SPEC);
        let key = [m[0], m[1], m[2], squarings];
        match self.known.get(&key) {
            Some(&k) => k,
            None => {
                let k = self.materials.len().min(u16::MAX as usize) as u16;
                if self.materials.len() < u16::MAX as usize {
                    self.materials.push(Material {
                        ambient: f32::from_bits(m[0]),
                        diffuse: f32::from_bits(m[1]),
                        spec_scale: f32::from_bits(m[2]),
                        squarings,
                    });
                }
                self.known.insert(key, k);
                k
            }
        }
    }

    /// SPIKE (tier 2b): the current block's lighting entry (from the first
    /// polygon's attribute word as the model's own data has it) and the
    /// material it gives (ambient, diffuse, specular scale as float bits,
    /// squarings).
    pub fn set_entry(&mut self, attr: u32, m: [u32; 3], squarings: u32) {
        let k = self.intern(m, squarings);
        if let Some(b) = self.blocks.last_mut() {
            b.entry = Some((((attr >> 18) & 0x1f) as u8, k));
        }
    }

    /// The last face was lit from the other side than it faces
    /// ([`Face::FLIP`]).
    pub fn flip_last(&mut self) {
        if let Some(f) = self.faces.last_mut() {
            f.flags |= Face::FLIP;
        }
    }

    /// The frame recorded, numbered `frame`.
    pub fn finish(self, frame: u64) -> Frame {
        Frame { frame, blocks: self.blocks, faces: self.faces, materials: self.materials }
    }
}

/// A polygon's lighting as the programs compute it, in IEEE single
/// precision (the design's 5.1): `eval(n)` gives the luma before
/// truncation for normal `n`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lamp {
    pub l: [f32; 3],
    pub ambient: f32,
    pub diffuse: f32,
    pub spec_scale: f32,
    /// [`Material::NO_SPEC`]: no specular term.
    pub squarings: u32,
    pub front: bool,
    pub base: Base,
}

impl Lamp {
    /// The lamp a face was lit with.
    pub fn of(block: &Block, face: &Face, m: &Material) -> Lamp {
        Lamp {
            l: block.light,
            ambient: m.ambient,
            diffuse: m.diffuse,
            spec_scale: m.spec_scale,
            squarings: m.squarings,
            front: face.front() != (face.flags & Face::FLIP != 0),
            base: face.base(),
        }
    }

    /// The luma (0-255 when in range, before truncation) of normal `n`, in
    /// the programs' order of operations.
    pub fn eval(&self, n: [f32; 3]) -> f32 {
        let l = self.l;
        let dotl = (n[0] * l[0] + n[1] * l[1]) + n[2] * l[2];
        let d = if self.front { dotl } else { -dotl };
        let s0 = (d + 0.0).max(0.0);
        let mut lum = s0 * self.diffuse + self.ambient;
        if self.squarings != Material::NO_SPEC {
            let b = match self.base {
                Base::D => d,
                Base::S0 => s0,
                Base::Dotl => dotl,
            };
            let mut s = ((b * 2.0) * n[2] - l[2]).max(0.0);
            for _ in 0..self.squarings {
                s *= s;
            }
            lum += s * self.spec_scale;
        }
        lum
    }
}

/// How much a block's side channel can be trusted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// The stream only (no side channel, or it doesn't line up).
    #[default]
    L0,
    /// Block facts (focal lengths, model address, direct data).
    L1,
    /// Block facts and faces, every polygon's luma recomputed exactly.
    L2,
}

/// Why a recomputed luma differs from the stream's (the first that applies,
/// in this order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Cause {
    /// The normal is zero (the unlit marker).
    ZeroNormal,
    /// A back face (by the program's test), or the facing disagrees with
    /// the stream's back-face bit.
    BackFace,
    /// Lit with a specular term.
    Specular,
    /// The recomputed luma is outside 0-255 (clamped).
    Clamp,
    /// A model in polygon RAM.
    PolyRam,
    Unknown,
}

impl Cause {
    pub const ALL: [Cause; 6] =
        [Cause::ZeroNormal, Cause::BackFace, Cause::Specular, Cause::Clamp, Cause::PolyRam, Cause::Unknown];

    pub fn name(self) -> &'static str {
        match self {
            Cause::ZeroNormal => "zero normal",
            Cause::BackFace => "back face",
            Cause::Specular => "specular",
            Cause::Clamp => "clamp",
            Cause::PolyRam => "polygon RAM",
            Cause::Unknown => "unknown",
        }
    }
}

/// The self-check's counts, summable over frames.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CheckStats {
    /// Frames checked, and those whose side channel didn't line up with the
    /// stream (a different number of object blocks): all L0.
    pub frames: u32,
    pub misaligned_frames: u32,
    /// Stream blocks by level, and of the L1 ones those that had faces but
    /// failed the check (demoted), and blocks whose face count didn't match
    /// the stream's polygons (L0).
    pub l0: u32,
    pub l1: u32,
    pub l2: u32,
    pub demoted: u32,
    pub misaligned_blocks: u32,
    /// Polygon records compared: equal, one level off, further off; and
    /// those not compared (no usable normal).
    pub exact: u32,
    pub near: u32,
    pub mismatch: u32,
    pub skipped: u32,
    /// Of the exact ones: single-sided back faces sent with luma 0 as the
    /// program says ([`Block::BACK_UNLIT`]).
    pub unlit_back: u32,
    /// Mismatches and near ones by [`Cause`] (in `Cause::ALL`'s order).
    pub causes: [u32; 6],
    pub near_causes: [u32; 6],
}

impl CheckStats {
    pub fn add(&mut self, o: &CheckStats) {
        self.frames += o.frames;
        self.misaligned_frames += o.misaligned_frames;
        self.l0 += o.l0;
        self.l1 += o.l1;
        self.l2 += o.l2;
        self.demoted += o.demoted;
        self.misaligned_blocks += o.misaligned_blocks;
        self.exact += o.exact;
        self.near += o.near;
        self.mismatch += o.mismatch;
        self.skipped += o.skipped;
        self.unlit_back += o.unlit_back;
        for k in 0..6 {
            self.causes[k] += o.causes[k];
            self.near_causes[k] += o.near_causes[k];
        }
    }

    /// Polygon records compared.
    pub fn compared(&self) -> u32 {
        self.exact + self.near + self.mismatch
    }
}

/// One polygon record's check ([`check_face`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Verdict {
    Exact,
    Near(Cause),
    Mismatch(Cause),
    /// Not compared: no usable normal.
    Skipped,
}

/// Recompute a polygon record's luma from its face and compare it with the
/// stream's `luma` (0-255) and back-face bit `back`; `double_sided` is the
/// stream's attribute bit 17. Returns the verdict and the recomputed value
/// (before truncation; NaN where not computed).
pub fn check_face(
    block: &Block,
    face: &Face,
    m: &Material,
    luma: u8,
    back: bool,
    double_sided: bool,
) -> (Verdict, f32) {
    let unusable = face.flags & Face::NONE != 0 && face.flags & Face::ZERO == 0;
    if unusable {
        return (Verdict::Skipped, f32::NAN);
    }
    let front = face.front();
    let cause = |v: f32| {
        if face.flags & Face::ZERO != 0 {
            Cause::ZeroNormal
        } else if !front {
            Cause::BackFace
        } else if m.squarings != Material::NO_SPEC {
            Cause::Specular
        } else if !(0.0..256.0).contains(&v) {
            Cause::Clamp
        } else if block.poly_ram() {
            Cause::PolyRam
        } else {
            Cause::Unknown
        }
    };
    // The stream's facing must be the program's: a culled back face
    // carries the back-face bit (and so does a lit 2B back face).
    let unlit_back = block.flags & Block::BACK_UNLIT != 0 && !front && !double_sided;
    let expect_back = if block.flags & Block::BACK_UNLIT != 0 { unlit_back } else { !front };
    if back != expect_back {
        return (Verdict::Mismatch(Cause::BackFace), f32::NAN);
    }
    if unlit_back {
        let v = if luma == 0 { Verdict::Exact } else { Verdict::Mismatch(Cause::BackFace) };
        return (v, 0.0);
    }
    let v = Lamp::of(block, face, m).eval(face.n);
    let q = quantise(v);
    let verdict = match q.abs_diff(luma as u32) {
        0 => Verdict::Exact,
        1 => Verdict::Near(cause(v)),
        _ => Verdict::Mismatch(cause(v)),
    };
    (verdict, v)
}

/// The self-check of a frame's side channel against its stream, per mesh
/// block ([`check_mesh`]).
#[derive(Clone, Debug, Default)]
pub struct Checked {
    /// Per [`Mesh::blocks`] entry: its level and its side-channel block
    /// (None at L0).
    pub blocks: Vec<(Level, Option<usize>)>,
    pub stats: CheckStats,
    /// Blocks that failed (demoted): side block index, model address,
    /// polygons mismatching.
    pub failed: Vec<(usize, u32, u32)>,
}

/// Line `side` up with `mesh` (the stream's blocks, by the order of their
/// `0x1` commands) and check every polygon record that has a face
/// ([`check_face`]). A frame whose block count differs is all L0; a block
/// whose face count differs is L0; a block with faces and any mismatch is
/// L1 (demoted); with faces and none, L2; without faces (direct data, a
/// list scan), L1.
pub fn check_mesh(mesh: &Mesh, side: &Frame) -> Checked {
    let mut out = Checked { blocks: vec![(Level::L0, None); mesh.blocks.len()], ..Default::default() };
    out.stats.frames = 1;
    if side.blocks.len() != mesh.commands as usize {
        out.stats.misaligned_frames = 1;
        out.stats.l0 = mesh.blocks.len() as u32;
        return out;
    }
    for (bi, mb) in mesh.blocks.iter().enumerate() {
        let si = mb.cmd as usize;
        let sb = &side.blocks[si];
        if sb.flags & Block::HAS_FACES == 0 {
            let lv = if sb.flags & Block::UNKNOWN != 0 { Level::L0 } else { Level::L1 };
            out.blocks[bi] = (lv, (lv != Level::L0).then_some(si));
            match lv {
                Level::L0 => out.stats.l0 += 1,
                _ => out.stats.l1 += 1,
            }
            continue;
        }
        let faces = side.faces_of(sb);
        if faces.len() != mb.polys.len() {
            out.stats.misaligned_blocks += 1;
            out.stats.l0 += 1;
            continue;
        }
        let mut bad = 0;
        for (face, p) in faces.iter().zip(&mesh.polys[mb.polys.clone()]) {
            let Some(m) = side.materials.get(face.mat as usize) else {
                out.stats.skipped += 1;
                continue;
            };
            let (v, _) = check_face(sb, face, m, p.luma, p.back, p.double_sided);
            let s = &mut out.stats;
            match v {
                Verdict::Exact => {
                    s.exact += 1;
                    let unlit = sb.flags & Block::BACK_UNLIT != 0 && !face.front() && !p.double_sided;
                    s.unlit_back += unlit as u32;
                }
                Verdict::Near(c) => {
                    s.near += 1;
                    s.near_causes[c as usize] += 1;
                }
                Verdict::Mismatch(c) => {
                    s.mismatch += 1;
                    s.causes[c as usize] += 1;
                    bad += 1;
                }
                Verdict::Skipped => s.skipped += 1,
            }
        }
        if bad > 0 {
            out.stats.demoted += 1;
            out.stats.l1 += 1;
            out.blocks[bi] = (Level::L1, Some(si));
            out.failed.push((si, sb.oba, bad));
        } else {
            out.stats.l2 += 1;
            out.blocks[bi] = (Level::L2, Some(si));
        }
    }
    out
}

/// [`check_mesh`] on a raster stream.
pub fn self_check(stream: &[u32], side: &Frame) -> Checked {
    check_mesh(&super::mesh_from_stream(stream), side)
}
