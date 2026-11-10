//! `--light`: smooth shading's side channel (`model2::light::Frame`) over
//! a run, every frame: the self-check (each polygon's flat luma recomputed
//! from the side channel against the stream's), the blocks by level, and
//! what the lists did (specular, polygon-RAM models, light vectors and
//! lighting entries changed within a frame, the true focal lengths against
//! the window width the stream-only layer guesses from).

use model2::light::{self, Cause, CheckStats, Facts};
use std::collections::HashMap;
use std::fmt::Write as _;

#[derive(Default)]
pub struct Acc {
    /// Frames the engine ran, with and without a side channel.
    pub frames: u64,
    pub without: u64,
    last: Option<u64>,
    pub check: CheckStats,
    pub facts: Facts,
    /// Most distinct light vectors in a frame.
    pub max_lights: u32,
    /// Failing blocks by model address: blocks, polygons.
    failed: HashMap<u32, (u32, u32)>,
    /// Focal length over window width (x, y), one sample per distinct
    /// value a frame.
    ratios: Vec<(f32, f32)>,
    /// The first mismatching polygons, described.
    examples: Vec<String>,
}

impl Acc {
    /// The engine's last frame, if it is one not seen yet.
    pub fn frame(&mut self, m: &mut model2::Machine) {
        m.bus.geo.sync();
        let e = m.bus.geo.engine();
        if self.last == Some(e.frames) || !e.light_on() {
            return;
        }
        self.last = Some(e.frames);
        self.frames += 1;
        let Some(side) = e.light_frame() else {
            self.without += 1;
            return;
        };
        self.add(&e.stream, &side);
    }

    /// One frame's stream and side channel.
    pub fn add(&mut self, stream: &[u32], side: &light::Frame) {
        let mesh = light::mesh_from_stream(stream);
        let c = light::check_mesh(&mesh, side);
        self.check.add(&c.stats);
        let facts = side.facts();
        self.max_lights = self.max_lights.max(facts.lights);
        self.facts.add(&facts);
        for &(si, oba, bad) in &c.failed {
            let e = self.failed.entry(oba).or_default();
            e.0 += 1;
            e.1 += bad;
            // A few examples, with what the check saw.
            let (Some(mb), true) = (mesh.blocks.iter().find(|b| b.cmd as usize == si), self.examples.len() < 16) else {
                continue;
            };
            let b = &side.blocks[si];
            for (k, (face, p)) in side.faces_of(b).iter().zip(&mesh.polys[mb.polys.clone()]).enumerate() {
                let m = side.materials[face.mat as usize];
                let (v, value) = light::check_face(b, face, &m, p.luma, p.back, p.double_sided);
                if matches!(v, light::Verdict::Mismatch(_)) && self.examples.len() < 16 {
                    self.examples.push(format!(
                        "frame {} model {oba:#08x} block flags {:#x} polygon {k}: stream luma {} back {} 2-sided {} link {}, recomputed {value:.3} ({v:?}); face n {:?} flags {:#x}, material {m:?}, light {:?}",
                        side.frame, b.flags, p.luma, p.back, p.double_sided, p.link, face.n, face.flags, b.light
                    ));
                }
            }
        }
        let mut seen: Vec<(u32, u32)> = Vec::new();
        for (mb, &(level, si)) in mesh.blocks.iter().zip(&c.blocks) {
            let (Some(si), true) = (si, level != light::Level::L0) else { continue };
            let Some((fx, fy)) = side.blocks[si].focal() else { continue };
            // The stream-only layer's guess is FOCAL_GUESS x the window's
            // width: the ratio is what the guess should have been.
            let w = mb.focal.0 / light::FOCAL_GUESS;
            let r = (fx / w, fy / w);
            let key = (r.0.to_bits(), r.1.to_bits());
            if !seen.contains(&key) {
                seen.push(key);
                self.ratios.push(r);
            }
        }
    }

    /// The report's lines.
    pub fn report(&self) -> String {
        let mut o = String::new();
        let s = &self.check;
        let pct = |a: u32, b: u32| 100.0 * a as f64 / b.max(1) as f64;
        let _ = writeln!(
            o,
            "light         {} frames, {} with a side channel; {} misaligned (all L0)",
            self.frames,
            self.frames - self.without,
            s.misaligned_frames
        );
        let blocks = s.l0 + s.l1 + s.l2;
        let _ = writeln!(
            o,
            "light blocks  {blocks}: L2 {:.2}%, L1 {:.2}%, L0 {:.2}%; demoted (failed the check) {} = {:.3}%, faces not lining up {}",
            pct(s.l2, blocks),
            pct(s.l1, blocks),
            pct(s.l0, blocks),
            s.demoted,
            pct(s.demoted, blocks),
            s.misaligned_blocks
        );
        let n = s.compared();
        let _ = writeln!(
            o,
            "light check   {n} polygons: exact {:.4}%, within 1 {:.4}%, mismatching {:.4}% ({} exact, {} within 1, {} mismatching); {} not compared (no normal); {} unlit back faces (luma 0) among the exact",
            pct(s.exact, n),
            pct(s.near, n),
            pct(s.mismatch, n),
            s.exact,
            s.near,
            s.mismatch,
            s.skipped,
            s.unlit_back
        );
        let causes = |v: &[u32; 6]| {
            Cause::ALL.iter().map(|c| format!("{} {}", c.name(), v[*c as usize])).collect::<Vec<_>>().join(", ")
        };
        let _ = writeln!(o, "light causes  mismatching: {}", causes(&s.causes));
        let _ = writeln!(o, "light causes  within 1: {}", causes(&s.near_causes));
        let f = &self.facts;
        let _ = writeln!(
            o,
            "light facts   blocks {}: direct data {}, computed normals {}, specular on {}, models in polygon RAM {} ({} polygons); polygons {}: lit with a specular term {} ({:.2}%), zero normals {} ({:.2}%)",
            f.blocks,
            f.direct,
            f.computed,
            f.specular_blocks,
            f.poly_ram_blocks,
            f.poly_ram_faces,
            f.faces,
            f.specular_faces,
            pct(f.specular_faces, f.faces),
            f.zero_normals,
            pct(f.zero_normals, f.faces)
        );
        let _ = writeln!(
            o,
            "light changes frames with more than one light vector {} (most in a frame {}; {} changes in all); with a lighting entry changed between objects {} ({} polygons); focal length changes within a frame {}; materials in a frame up to {}",
            f.frames_light_changed,
            self.max_lights,
            f.light_changes,
            f.frames_entries_changed,
            f.entry_changes,
            f.focal_changes,
            f.materials
        );
        if !self.ratios.is_empty() {
            let mut x: Vec<f32> = self.ratios.iter().map(|r| r.0).collect();
            let mut y: Vec<f32> = self.ratios.iter().map(|r| r.1).collect();
            x.sort_by(f32::total_cmp);
            y.sort_by(f32::total_cmp);
            let q = |v: &[f32], k: f32| v[((v.len() - 1) as f32 * k) as usize];
            let _ = writeln!(
                o,
                "light focal   fx / window width: min {:.3}, median {:.3}, max {:.3}; fy / width: min {:.3}, median {:.3}, max {:.3} ({} distinct)",
                x[0],
                q(&x, 0.5),
                x[x.len() - 1],
                y[0],
                q(&y, 0.5),
                y[y.len() - 1],
                x.len()
            );
        }
        let mut worst: Vec<_> = self.failed.iter().collect();
        worst.sort_by_key(|(oba, (_, n))| (std::cmp::Reverse(*n), **oba));
        for (oba, (blocks, polys)) in worst.iter().take(8) {
            let _ = writeln!(o, "light failed  model {oba:#08x}: {blocks} blocks, {polys} polygons mismatching");
        }
        for e in &self.examples {
            let _ = writeln!(o, "light example {e}");
        }
        o
    }
}

/// The list scan against the native hooks over the run (the engine's
/// `CheckReport::light`), as a report line.
pub fn scan_report(l: &model2::geo::LightCheck) -> String {
    let mut o = format!(
        "light scan    {} frames: {} the scan gave up on, {} with another block count; blocks: {} equal, {} equal where known (light or mode not set yet), {} focal lengths not known yet, {} differ\n",
        l.frames, l.scan_failed, l.count_differing, l.equal, l.partial, l.unknown, l.differing
    );
    if let Some((frame, k, n, s)) = &l.first {
        o += &format!("light first   frame {frame} block {k}: native {n:?}, scan {s:?}\n");
    }
    o
}
