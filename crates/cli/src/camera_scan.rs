//! `camera-scan`: where a game keeps its camera, for photo mode's true
//! camera (docs/photo-mode.md, "A true camera"). A spike: it finds
//! candidates, it doesn't prove them.
//!
//! The display list sets each object's transform in the game camera's view
//! space: for scenery that doesn't move, `M = C W` with `W` the object's
//! fixed place in the world and `C` the camera. Between two frames A and B
//! the same scenery object's matrices then differ by the camera's turn
//! alone: `M_B M_Aᵀ = C_B C_Aᵀ`, the same for every such object. The most
//! common such turn among the objects drawn in both frames (matched by
//! model address) is taken as the camera's. A 3x3 rotation in the i960's
//! RAM that turned by exactly that between the two frames (as stored, or
//! transposed: world to view or view to world, rows or columns) is a
//! candidate for the camera the game draws with; the three words after it
//! may be its position.

use model2::smooth::{Kind, Xform};
use model2::Machine;
use romset::RomSet;
use std::collections::HashMap;

type M3 = [[f32; 3]; 3];

/// A matrix transform's 3x3 part, column by column.
fn rotation(x: &Xform) -> M3 {
    let f = |k: usize| f32::from_bits(x.words[k]);
    std::array::from_fn(|c| std::array::from_fn(|r| f(3 * c + r)))
}

fn mul(a: &M3, b: &M3) -> M3 {
    std::array::from_fn(|c| std::array::from_fn(|r| (0..3).map(|k| a[k][r] * b[c][k]).sum()))
}

fn t(a: &M3) -> M3 {
    std::array::from_fn(|c| std::array::from_fn(|r| a[r][c]))
}

fn dist(a: &M3, b: &M3) -> f32 {
    (0..3).flat_map(|c| (0..3).map(move |r| (c, r))).map(|(c, r)| (a[c][r] - b[c][r]).powi(2)).sum::<f32>().sqrt()
}

/// The angle a rotation turns by, radians.
fn angle(a: &M3) -> f32 {
    ((a[0][0] + a[1][1] + a[2][2] - 1.0) / 2.0).clamp(-1.0, 1.0).acos()
}

/// Columns of unit length at right angles, within `eps`.
fn orthonormal(a: &M3, eps: f32) -> bool {
    let dot = |x: &[f32; 3], y: &[f32; 3]| x[0] * y[0] + x[1] * y[1] + x[2] * y[2];
    (0..3).all(|i| (dot(&a[i], &a[i]) - 1.0).abs() < eps)
        && (dot(&a[0], &a[1]).abs() < eps)
        && (dot(&a[0], &a[2]).abs() < eps)
        && (dot(&a[1], &a[2]).abs() < eps)
}

/// The frame's matrices by model address (the first of each).
fn matrices(m: &mut Machine) -> HashMap<u32, M3> {
    m.bus.geo.sync();
    let mut out = HashMap::new();
    if let Some(f) = m.bus.geo.engine().smooth_frame() {
        for x in f.plan.transforms().iter().filter(|x| x.kind == Kind::Matrix && x.key != model2::smooth::NO_KEY) {
            out.entry(x.key).or_insert_with(|| rotation(x));
        }
    }
    out
}

/// RAM to search: the i960's work RAM windows.
const RAM: [(u32, u32); 2] = [(0x0020_0000, 0x0024_0000), (0x0050_0000, 0x0060_0000)];

fn ram(m: &mut Machine) -> Vec<(u32, Vec<u32>)> {
    use i960::cpu::Bus;
    RAM.iter().map(|&(a, b)| (a, (a..b).step_by(4).map(|x| m.bus.read32(x)).collect())).collect()
}

/// Run to frame `at` with `spec`'s inputs, then `gap` frames on, and list
/// the camera candidates between the two.
pub fn scan(roms: &RomSet, at: u64, gap: u64, spec: &str, interp: bool) -> Result<(), String> {
    let script = model2::input::InputScript::parse(spec, &roms.game.controls).map_err(|e| format!("--input: {e}"))?;
    let mut m = crate::machine(roms);
    if interp {
        m.disable_native();
    }
    m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    m.bus.geo.set_smooth(true);
    if let Ok(bytes) = std::fs::read(std::path::Path::new("out").join(roms.game.id).join("eeprom.bin")) {
        m.bus.eeprom.load(&bytes);
    }
    let run_to = |m: &mut Machine, frame: u64| -> Result<(), String> {
        while m.frames < frame {
            script.apply(m.frames, &mut m.bus);
            m.run_slice(u64::MAX).map_err(|e| format!("stopped: {e:?}"))?;
            m.bus.uart_log.clear();
        }
        Ok(())
    };
    run_to(&mut m, at)?;
    let (ma, ra) = (matrices(&mut m), ram(&mut m));
    run_to(&mut m, at + gap)?;
    let (mb, rb) = (matrices(&mut m), ram(&mut m));
    // The turn of every object drawn in both frames; the camera's is the
    // one most of them share.
    let turns: Vec<M3> =
        ma.iter().filter_map(|(k, a)| mb.get(k).map(|b| mul(b, &t(a)))).filter(|d| orthonormal(d, 0.02)).collect();
    let (best, shared) = turns
        .iter()
        .map(|d| (d, turns.iter().filter(|e| dist(d, e) < 0.01).count()))
        .max_by_key(|x| x.1)
        .ok_or("no object drawn in both frames (is smooth motion's recording running?)")?;
    println!(
        "camera-scan   {}: frames {at} and {}: {} matrices in both, the camera's turn shared by {shared}: {:.2} degrees",
        roms.game.id,
        at + gap,
        turns.len(),
        angle(best).to_degrees()
    );
    if angle(best) < 1f32.to_radians() {
        println!("camera-scan   the camera hardly turned: pick frames where it does (a corner)");
        return Ok(());
    }
    let mut found = 0;
    for ((base, a), (_, b)) in ra.iter().zip(&rb) {
        for i in 0..a.len().saturating_sub(12) {
            let f = |v: &[u32], k: usize| f32::from_bits(v[i + k]);
            let cols = |v: &[u32]| -> M3 { std::array::from_fn(|c| std::array::from_fn(|r| f(v, 3 * c + r))) };
            let (pa, pb) = (cols(a), cols(b));
            if !orthonormal(&pa, 1e-3) || !orthonormal(&pb, 1e-3) || dist(&pa, &pb) < 1e-4 {
                continue;
            }
            // Stored by columns world to view (or by rows view to world),
            // or the other way round.
            let ways = [
                ("columns world to view, or rows view to world", mul(&pb, &t(&pa))),
                ("rows world to view, or columns view to world", mul(&t(&pb), &pa)),
            ];
            for (how, e) in ways {
                let d = dist(&e, best);
                if d < 0.02 {
                    let pos = [9, 10, 11].map(|k| f(b, k));
                    println!(
                        "camera?       {:#010x} ({how}, off by {d:.4}); next three words {:.3} {:.3} {:.3}",
                        base + 4 * i as u32,
                        pos[0],
                        pos[1],
                        pos[2]
                    );
                    found += 1;
                    break;
                }
            }
        }
    }
    if found == 0 {
        println!(
            "camera-scan   no rotation in RAM turned with it: the camera may be angles, kept elsewhere, or \
             multiplied in by the geometry processor"
        );
    }
    Ok(())
}
