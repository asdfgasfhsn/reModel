//! `run-ahead-check`: run-ahead (`model2::runahead`) against plain runs.

use model2::runahead::RunAhead;
use model2::Machine;
use romset::RomSet;
use std::collections::BTreeMap;
use std::path::Path;
use std::time::{Duration, Instant};

/// The inputs a frame runs with: ports, analog channels, gun counters.
type Inputs = ([u8; 4], [u8; 8], [u8; 16]);

fn inputs(m: &Machine) -> Inputs {
    let b = &m.bus;
    (b.inputs, b.analog, b.gun.as_ref().map_or([0; 16], |g| g.counters))
}

/// A frame the shadow showed, to compare with the plain run's.
struct Shown {
    picture: Vec<u8>,
    hash: u64,
    /// With smooth motion, its in-between at t = 0.5, if the frame shown
    /// before it can be compared with the plain run's too.
    between: Option<Vec<u8>>,
}

/// Two machines from the same start, one plain (`a`), one (`b`) with
/// run-ahead of `frames`, through a run of `steps` instructions with the
/// `spec` inputs. Checks that the real timelines agree (state hash at the
/// end) and, every `every` frames F whose inputs hold for the frames run
/// ahead, that the frame shown after F is frame F + `frames` of the plain
/// run (picture byte for byte, and state hash); with smooth motion, the
/// in-betweens too, when the frame shown before it had the same inputs.
/// Prints what it cost. Ok(false) if anything differs.
pub fn check(
    roms: &RomSet,
    steps: u64,
    frames: u32,
    every: u64,
    spec: &str,
    interp: bool,
    single_thread: bool,
) -> Result<bool, String> {
    let script = model2::input::InputScript::parse(spec, &roms.game.controls).map_err(|e| format!("--input: {e}"))?;
    let eeprom = std::fs::read(Path::new("out").join(roms.game.id).join("eeprom.bin")).ok();
    let new = || -> Result<Machine, String> {
        let mut m = crate::machine(roms);
        if interp {
            m.disable_native();
        }
        m.bus.geo.set_threaded(!single_thread);
        if let Some(bytes) = &eeprom {
            m.bus.eeprom.load(bytes);
        }
        if let Some(path) = crate::LOAD_STATE.get() {
            let d = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            model2::load_session(&mut m, None, &d)?;
        }
        Ok(m)
    };
    let (mut a, mut b) = (new()?, new()?);
    let smooth = crate::SMOOTH.get().is_some();
    let mut ra = RunAhead::new(roms, &mut b, frames);
    // The real machine's frames aren't shown: smooth motion is the
    // shadow's (the plain run keeps its own, to compare with).
    b.bus.geo.set_smooth(false);
    println!(
        "run-ahead     {}: {frames} frame{} ahead, i960 {}, geometrizer {:?}{}{}",
        roms.game.id,
        if frames == 1 { "" } else { "s" },
        if a.is_native() { "recompiled" } else { "interpreted" },
        crate::GEO_MODE.get().copied().unwrap_or_default(),
        if single_thread { " inline" } else { " on its thread" },
        if smooth { ", smooth motion" } else { "" },
    );
    let n = frames as u64;
    // The inputs each frame ran with, by frame count after it.
    let mut ran_with: BTreeMap<u64, Inputs> = BTreeMap::new();
    // The shadow's frames waiting for the plain run to get there, by the
    // frame they ran ahead from.
    let mut pending: BTreeMap<u64, Shown> = BTreeMap::new();
    let (mut checked, mut pictures, mut hashes, mut betweens, mut between_checks) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut first_bad = None;
    let mut real_time = Duration::ZERO;
    let (start, first) = (a.cpu.icount, a.frames);
    script.apply(a.frames, &mut a.bus);
    script.apply(b.frames, &mut b.bus);
    while a.cpu.icount - start < steps {
        let used = inputs(&a);
        let t = Instant::now();
        a.run_slice(u64::MAX).map_err(|s| format!("stopped: {s:?}"))?;
        real_time += t.elapsed();
        b.run_slice(u64::MAX).map_err(|s| format!("stopped: {s:?}"))?;
        if a.frames != b.frames {
            return Err(format!("the runs drifted apart at frame {}", a.frames));
        }
        let f = a.frames;
        ran_with.insert(f, used);
        // As a front end does: the frames ahead run with the inputs the
        // real frame had (frame f + j of the plain run with frame f + j's).
        ra.run(&mut b).map_err(|s| format!("stopped ahead of frame {f}: {s:?}"))?;
        // Checkpoints whose frames the plain run ran with other inputs.
        pending.retain(|&from, _| ran_with[&from] == used);
        // The plain run reached a frame shown earlier.
        if let Some(shown) = f.checked_sub(n).and_then(|from| pending.remove(&from)) {
            checked += 1;
            a.bus.geo.sync();
            let picture = crate::render(&a, None, Path::new("plain")).1;
            let same_picture = picture == shown.picture;
            let same_hash = a.bus.state_hash() == shown.hash;
            pictures += same_picture as u32;
            hashes += same_hash as u32;
            let mut same_between = true;
            if let Some(theirs) = &shown.between {
                between_checks += 1;
                let mine = crate::in_between(&mut a, 0.5).map(|st| crate::render(&a, Some(&st), Path::new("")).1);
                same_between = mine.as_ref() == Some(theirs);
                betweens += same_between as u32;
            }
            if !(same_picture && same_hash && same_between) && first_bad.is_none() {
                first_bad = Some(f - n);
            }
        }
        if f.is_multiple_of(every) {
            let sh = ra.shadow();
            sh.bus.geo.sync();
            let picture = crate::render(sh, None, Path::new("ahead")).1;
            let hash = sh.bus.state_hash();
            // Its in-between blends the frame shown before (run ahead from
            // f - 1, with frame f - 1's inputs) with it: the plain run's
            // frames f + n - 1 and f + n, if those inputs were the same.
            let between = (smooth && ran_with.get(&(f - 1)) == Some(&used))
                .then(|| crate::in_between(sh, 0.5).map(|st| crate::render(sh, Some(&st), Path::new("")).1))
                .flatten();
            pending.insert(f, Shown { picture, hash, between });
        }
        while ran_with.len() > n as usize + 4 {
            ran_with.pop_first();
        }
        script.apply(f, &mut a.bus);
        script.apply(f, &mut b.bus);
    }
    let (ha, hb) = (a.bus.state_hash(), b.bus.state_hash());
    println!("frames        {} to {}", first, a.frames);
    println!(
        "state hash    plain {ha:016x}, with run-ahead {hb:016x}: {}",
        if ha == hb { "same" } else { "DIFFERENT" }
    );
    println!(
        "shown frames  {checked} checked against the plain run's (every {every}th frame, inputs held): \
         pictures {pictures} same, state hashes {hashes} same{}",
        if smooth { format!(", in-betweens {betweens} of {between_checks} same") } else { String::new() }
    );
    if let Some(f) = first_bad {
        println!("first difference: the frame shown after frame {f}");
    }
    let st = ra.stats;
    let per = |d: Duration| d.as_secs_f64() * 1e3 / st.forks.max(1) as f64;
    println!(
        "cost          per shown frame: geometrizer wait {:.2} ms, copy {:.2} ms, {frames} frame{} ahead {:.2} ms; \
         a real frame {:.2} ms",
        per(st.waiting),
        per(st.copying),
        if frames == 1 { "" } else { "s" },
        per(st.running),
        real_time.as_secs_f64() * 1e3 / (a.frames - first).max(1) as f64,
    );
    // A whole save state and load, for comparison.
    let t = Instant::now();
    let state = b.save_state();
    let saved = t.elapsed();
    let mut c = new()?;
    let t = Instant::now();
    c.load_state(&state)?;
    let loaded = t.elapsed();
    println!(
        "save state    {:.1} MB: saved in {:.2} ms, loaded in {:.2} ms; run-ahead's last copy moved {:.1} MB",
        state.len() as f64 / 1e6,
        saved.as_secs_f64() * 1e3,
        loaded.as_secs_f64() * 1e3,
        st.copied as f64 / 1e6,
    );
    Ok(ha == hb && checked > 0 && pictures == checked && hashes == checked && betweens == between_checks)
}
