//! `remodel geo-check`: a run with the geometrizer in `GeoMode::Check`, the
//! chip's raster stream (the 2B SHARC's, the 2C TGP's) compared with the
//! native geometrizer's each frame (docs/hw/geometrizer-2b.md,
//! docs/hw/geometrizer-2c.md); `remodel geo-fuzz`, random lists through
//! both.

use romset::RomSet;
use std::path::PathBuf;

pub fn run(
    m: &mut model2::Machine,
    roms: &RomSet,
    steps: u64,
    from: u64,
    script: &model2::input::InputScript,
    load: Option<&PathBuf>,
) -> Result<(), String> {
    let dir = PathBuf::from("out").join(roms.game.id);
    if let Ok(bytes) = std::fs::read(dir.join("eeprom.bin")) {
        m.bus.eeprom.load(&bytes);
    }
    if let Some(path) = load {
        let d = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        model2::load_session(m, None, &d)?;
        println!("state         loaded {} (frame {})", path.display(), m.frames);
    }
    script.apply(m.frames, &mut m.bus);
    let started = std::time::Instant::now();
    let mut last = m.frames;
    let mut reported = 0;
    while m.cpu.icount < steps {
        m.run_slice(steps - m.cpu.icount).map_err(|s| format!("stopped at frame {}: {s:?}", m.frames))?;
        if m.frames != last {
            script.apply(m.frames, &mut m.bus);
            last = m.frames;
            if m.frames / 1000 != reported {
                reported = m.frames / 1000;
                m.bus.geo.sync();
                let c = &m.bus.geo.engine().check;
                println!("frame {:>6}: {} compared, {} differ", m.frames, c.frames, c.differing);
            }
        }
    }
    m.bus.geo.sync();
    let state_hash = m.bus.state_hash();
    let e = m.bus.geo.engine();
    describe(e, &m.bus.geo.program);
    let c = &e.check;
    println!(
        "compared      {} frames, {} words: {} frames differ, {} read the list differently, {} end with other state",
        c.frames, c.words, c.differing, c.list_differing, c.state_differing
    );
    stopped(e);
    if let Some(d) = &c.first {
        println!(
            "first diff    frame {}, word {} (stream lengths: SHARC {}, native {}), under command {}",
            d.frame,
            d.index,
            d.sharc_len,
            d.native_len,
            d.command.map_or("none".into(), |(k, w)| format!("{w:08x} at word {k}"))
        );
        for k in 0..d.sharc.len().max(d.native.len()) {
            let (a, b) = (d.sharc.get(k), d.native.get(k));
            let mark = if a != b { "  <-" } else { "" };
            let show = |v: Option<&u32>| v.map_or("--------".into(), |v| format!("{v:08x}"));
            println!("  {:>7}  {}  {}{mark}", d.start + k, show(a), show(b));
        }
    }
    if let Some((frame, addr, a, b)) = c.first_state {
        println!("first state   frame {frame}: word {addr:#x} chip {a:08x}, native {b:08x}");
    }
    if c.light.frames > 0 {
        // --light: the chip path's list scan against the native hooks.
        print!("{}", crate::light_report::scan_report(&c.light));
    }
    let chip = if e.sharc.is_some() { "SHARC" } else { "TGP" };
    let summary = |label: &str, t: &[(f32, f32)]| {
        if t.is_empty() {
            return;
        }
        let n = t.len() as f32;
        let (sm, nm) = (t.iter().map(|x| x.0).sum::<f32>() / n, t.iter().map(|x| x.1).sum::<f32>() / n);
        let (sw, nw) = t.iter().fold((0f32, 0f32), |a, x| (a.0.max(x.0), a.1.max(x.1)));
        println!(
            "time {label:<8} {} frames: {chip} mean {sm:.3} ms, worst {sw:.3}; native mean {nm:.3} ms, worst {nw:.3} ({:.1}x)",
            t.len(),
            sm / nm.max(1e-6)
        );
    };
    summary("all", &c.times);
    if from > 0 {
        // Frame k of the comparison is engine frame k + 1 counted from the
        // first compared one; the engine's frames run one behind the
        // machine's at most.
        let skip = (from as usize).saturating_sub((e.frames as usize).saturating_sub(c.times.len()));
        summary(&format!("from {from}"), c.times.get(skip..).unwrap_or(&[]));
    }
    let secs = started.elapsed().as_secs_f64();
    println!("emulated      {} frames in {secs:.1}s", m.frames);
    println!("state hash    {state_hash:016x} (ip {:#x})", m.cpu.ip);
    Ok(())
}

/// The uploaded program, which native implementation it has, and whether
/// the chip runs recompiled code.
fn describe(e: &model2::geo::Engine, upload: &[u64]) {
    if e.sharc.is_some() {
        let image = model2::sharc::image(upload);
        println!(
            "program       {} words, code hash {:016x}: {}",
            image.len(),
            model2::geo_native::image_hash(&image),
            match &e.native {
                Some(n) => format!("native {:?}", n.program),
                None => "no native implementation (nothing compared)".into(),
            }
        );
        println!(
            "SHARC         {}",
            if e.sharc_is_native() { "recompiled" } else { "interpreted (no recompiled code for it in this build)" }
        );
    } else {
        let hash = e.tgp.program_hash;
        println!(
            "program       {} words, program hash {hash:016x}: {}",
            upload.len(),
            match &e.native_tgp {
                Some(n) => format!("native {:?}", n.program),
                None => "no native implementation (nothing compared)".into(),
            }
        );
        println!(
            "TGP           {}",
            if model2::recomp::geo(hash).is_some() {
                "recompiled"
            } else {
                "interpreted (no recompiled code for it in this build)"
            }
        );
    }
}

/// Why the native geometrizer stopped, if it did.
fn stopped(e: &model2::geo::Engine) {
    if let Some(stop) = e.native.as_ref().and_then(|n| n.stop) {
        println!("native        stopped: {stop:?}");
    }
    if let Some(stop) = e.native_tgp.as_ref().and_then(|n| n.stop) {
        println!("native        stopped: {stop:?}");
    }
}

/// A small deterministic generator for `fuzz`.
struct Gen(u64);

impl Gen {
    fn next(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
    fn below(&mut self, n: u32) -> u32 {
        (self.next() >> 32) as u32 % n.max(1)
    }
    fn chance(&mut self, percent: u32) -> bool {
        self.below(100) < percent
    }
    /// A float: mostly ordinary values of either sign and a wide range of
    /// sizes, sometimes zeros, denormals, infinities and NaNs.
    fn float(&mut self, scale: f32) -> u32 {
        if self.chance(3) {
            const ODD: [u32; 9] =
                [0, 0x8000_0000, 1, 0x8000_0001, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x0080_0000, 0x7f7f_ffff];
            return ODD[self.below(ODD.len() as u32) as usize];
        }
        let u = (self.next() >> 40) as f32 / (1u64 << 24) as f32;
        let v = (u * 2.0 - 1.0) * scale * if self.chance(10) { 1e-6 } else { 1.0 };
        v.to_bits()
    }
}

/// A command word for function `fnc` that passes the command check, with
/// an eye select sometimes.
fn command(g: &mut Gen, fnc: u32) -> u32 {
    let eye = if g.chance(10) { g.below(4) } else { 0 };
    fnc << 23 | eye << 29 | fnc << 8 | fnc
}

/// A model: two vertices, then `n` polygons of 10 words, maybe a
/// terminator.
fn model(g: &mut Gen, n: u32) -> Vec<u32> {
    let mut v = Vec::new();
    for _ in 0..6 {
        v.push(g.float(500.0));
    }
    for _ in 0..n {
        let kind = [1, 1, 2, 2, 3][g.below(5) as usize];
        let attr = kind
            | g.below(4) << 8
            | g.below(4) << 10
            | g.below(32) << 12
            | g.below(2) << 17
            | g.below(32) << 18
            | g.below(32) << 27;
        v.push(attr);
        for _ in 0..9 {
            v.push(g.float(500.0));
        }
    }
    if g.chance(80) {
        v.push(0);
    }
    v
}

/// A frame's display list of random commands with valid operands. `ring`:
/// the program has `lastbrnx`'s commands 0x1A-0x1C (a texture point ring)
/// in place of the aliases of 0x0A-0x0C; its fn 0x1B isn't implemented
/// natively, so the list leaves it out.
fn list(g: &mut Gen, rom_words: u32, ring: bool) -> Vec<u32> {
    let mut l = Vec::new();
    l.push(command(g, 0x16));
    l.push(g.float(65536.0));
    for _ in 0..g.below(60) + 10 {
        let k = g.below(20);
        match k {
            0 => {
                l.push(command(g, 7));
                l.push(g.below(4));
            }
            1 => {
                l.push(command(g, 9));
                l.push(g.float(1000.0));
                l.push(g.float(1000.0));
            }
            2 => {
                l.push(command(g, 0x0a));
                for _ in 0..3 {
                    l.push(g.float(1.0));
                }
            }
            3 => {
                l.push(command(g, 6));
                l.push(if g.chance(95) { g.below(32) } else { g.below(40) });
                let n = g.below(4) + 1;
                l.push(n);
                for _ in 0..n {
                    l.push((g.next() >> 32) as u32);
                    l.push(g.float(4.0));
                }
            }
            4 | 5 => {
                let f = if g.chance(50) || ring { 0x0b } else { 0x1b };
                l.push(command(g, f));
                for _ in 0..12 {
                    l.push(g.float(if l.len() % 12 < 9 { 1.5 } else { 2000.0 }));
                }
            }
            6 => {
                l.push(command(g, 0x0c));
                for _ in 0..3 {
                    l.push(g.float(2000.0));
                }
            }
            7 => {
                l.push(command(g, 8));
                l.push(g.float(16.0));
            }
            8 => {
                l.push(command(g, 3));
                for _ in 0..6 {
                    l.push((g.next() >> 32) as u32);
                }
            }
            9 => {
                // Raster memory writes and log data.
                let f = if g.chance(50) { 4 } else { 0x14 };
                l.push(command(g, f));
                l.push(g.below(0x1_0000) | if g.chance(50) { 0x80_0000 } else { 0 });
                let n = g.below(8) + 1;
                l.push(n);
                for _ in 0..n {
                    l.push((g.next() >> 32) as u32);
                }
            }
            10 => {
                // Direct data.
                l.push(command(g, 2));
                l.push((g.next() >> 32) as u32);
                l.push((g.next() >> 32) as u32);
                for _ in 0..6 {
                    l.push(g.float(500.0));
                }
                let n = g.below(5) + 1;
                for k in 0..n {
                    let quad = g.chance(50);
                    l.push(if quad { 1 } else { 2 } | g.below(1 << 16) << 2);
                    for _ in 0..if quad { 8 } else { 5 } {
                        l.push(g.float(500.0));
                    }
                    if k + 1 == n {
                        l.push(0);
                    }
                }
            }
            11 => {
                // A model into fast polygon RAM or polygon RAM, then drawn.
                let fast = g.chance(60);
                let at = g.below(0x4000);
                let n = g.below(30) + 1;
                let m = model(g, n);
                l.push(command(g, 5));
                l.push(if fast { 0x100_0000 } else { 0 } | at);
                l.push(m.len() as u32);
                l.extend_from_slice(&m);
                let f = if g.chance(50) { 1 } else { 0x11 };
                l.push(command(g, f));
                l.push((g.next() >> 40) as u32);
                l.push((g.next() >> 40) as u32);
                l.push(if fast { 0x100_0000 } else { 0 } | at);
                l.push(if g.chance(50) { 0 } else { g.below(n + 2) + 1 });
            }
            12..=15 => {
                // An object from the polygon ROM (anything there: real
                // models or not).
                l.push(command(g, 1));
                l.push((g.next() >> 40) as u32);
                l.push((g.next() >> 40) as u32);
                l.push(0x80_0000 + g.below(rom_words.max(16) - 16));
                l.push(g.below(40) + 1);
            }
            16 => {
                // Data memory push: the program's own words, or the
                // polygon memory port.
                l.push(command(g, 0x0d));
                l.push(if g.chance(80) { 0x3_0000 + g.below(0x200) } else { 0x43_0001 });
                l.push(g.below(16) + 1);
            }
            17 => {
                // The board test, with small blocks.
                l.push(command(g, 0x0e));
                for i in 0..32 {
                    // The pattern the i960 sends: a word that doesn't match
                    // makes the SHARC's loop count off (docs/hw/geometrizer-2b.md).
                    l.push(1 << i);
                }
                let n = g.below(3);
                l.push(n);
                for _ in 0..n {
                    l.push(0x80_0000 + g.below(rom_words.max(16)));
                    l.push(g.below(12) + 1);
                    l.push((g.next() >> 32) as u32);
                }
            }
            18 => {
                l.push(command(g, 0x10));
                l.push((g.next() >> 32) as u32);
            }
            19 if ring => {
                // The ring's recording flag, and its flush to point RAM.
                if g.chance(50) {
                    l.push(command(g, 0x1a));
                    l.push(g.below(3));
                } else {
                    l.push(command(g, 0x1c));
                    l.push(0x80_0000 | g.below(0x1_0000));
                }
            }
            _ => l.push(command(g, 0)),
        }
    }
    l.push(command(g, 0x0f));
    l
}

/// `remodel geo-fuzz`: random display lists, covering every command and
/// path of the program (also those no game's runs reach: fast polygon RAM,
/// direct data, the test command), through the chip and the native
/// geometrizer side by side.
pub fn fuzz(roms: &RomSet, frames: u64, seed: u64) -> Result<(), String> {
    use model2::geo::{Core, Engine, GeoMode, Job};
    // The game's own upload.
    let mut m = model2::Machine::new(roms);
    m.bus.geo.set_threaded(false);
    while !m.bus.geo.booted && m.frames < 3000 {
        m.run_slice(u64::MAX).map_err(|s| format!("stopped at frame {}: {s:?}", m.frames))?;
    }
    if !m.bus.geo.booted {
        return Err(format!("{} uploaded no geometrizer program", roms.game.id));
    }
    if m.bus.sharc.is_none() {
        return fuzz_tgp(roms, &m.bus.geo.program.clone(), frames, seed);
    }
    let program = m.bus.geo.program.clone();
    let rom = model2::bus::polygon_rom(roms);
    let rom_words = rom.len() as u32;
    let mut e = Engine::new(rom, Core::Sharc);
    e.set_mode(GeoMode::Check);
    e.run_job(Job::Boot(program));
    if e.native.is_none() {
        return Err("no native implementation of this program".into());
    }
    let ring = e.native.as_ref().is_some_and(|n| n.program == model2::geo_native::Program::Lastbrnx);
    let mut g = Gen(seed.max(1));
    for _ in 0..frames {
        let words = list(&mut g, rom_words, ring);
        let mut buffer = vec![0u8; 0x2_0000];
        for (k, w) in words.iter().take(buffer.len() / 4).enumerate() {
            buffer[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
        }
        let before = e.check.differing + e.check.state_differing;
        e.run_job(Job::Frame { buffer, booted: true, read_ptr: 0, jumps: Vec::new() });
        if let Some(path) = std::env::var_os("REMODEL_FUZZ_DUMP") {
            if before == 0 && e.check.differing + e.check.state_differing > 0 {
                let text: String = words.iter().map(|w| format!("{w:08x}\n")).collect();
                let _ = std::fs::write(&path, text);
                let text: String = e.stream.iter().map(|w| format!("{w:08x}\n")).collect();
                let _ = std::fs::write(format!("{}.stream", path.to_string_lossy()), text);
            }
        }
        if std::env::var_os("REMODEL_FUZZ_TRACE").is_some() {
            let s = e.sharc.as_deref().unwrap();
            println!(
                "frame {}: list {} words, read {}, stream {}, sharc pc {:#x} steps {}",
                e.frames,
                words.len(),
                e.list_words,
                e.stream.len(),
                s.pc,
                s.steps
            );
            if let Some(k) = words.iter().position(|w| (w >> 23) & 0x1f == 0x0e && w & 0x1f == 0x0e) {
                println!("  test at {k}: {:08x?}", &words[k..(k + 40).min(words.len())]);
            }
            if e.list_words < words.len() {
                let at = e.list_words;
                println!("  list around the stop: {:08x?}", &words[at.saturating_sub(8)..(at + 4).min(words.len())]);
            }
        }
    }
    fuzz_report(&e, frames)
}

/// What `fuzz` found.
fn fuzz_report(e: &model2::geo::Engine, frames: u64) -> Result<(), String> {
    let c = &e.check;
    println!(
        "fuzz          {} frames ({} compared), {} words: {} differ, {} read the list differently, {} end with other state",
        frames, c.frames, c.words, c.differing, c.list_differing, c.state_differing
    );
    stopped(e);
    if let Some(d) = &c.first {
        println!(
            "first diff    frame {}, word {} (lengths {} / {}), under command {}",
            d.frame,
            d.index,
            d.sharc_len,
            d.native_len,
            d.command.map_or("none".into(), |(k, w)| format!("{w:08x} at word {k}"))
        );
        for k in 0..d.sharc.len().max(d.native.len()) {
            let (a, b) = (d.sharc.get(k), d.native.get(k));
            let mark = if a != b { "  <-" } else { "" };
            let show = |v: Option<&u32>| v.map_or("--------".into(), |v| format!("{v:08x}"));
            println!("  {:>7}  {}  {}{mark}", d.start + k, show(a), show(b));
        }
    }
    if let Some((frame, addr, a, b)) = c.first_state {
        println!("first state   frame {frame}: word {addr:#x} chip {a:08x}, native {b:08x}");
    }
    if c.differing + c.state_differing + c.list_differing > 0 {
        return Err("the native geometrizer differs".into());
    }
    Ok(())
}

/// A TGP (2C) model: two vertices, then `n` polygons of 10 words (attr,
/// normal, Pa, Pb), or in `topskatr`'s compact format 7 (a quad) or 4 (a
/// triangle) with no normal; maybe a terminator.
fn model_tgp(g: &mut Gen, n: u32, compact: bool) -> Vec<u32> {
    let mut v = Vec::new();
    for _ in 0..6 {
        v.push(g.float(500.0));
    }
    for _ in 0..n {
        let kind = [1, 1, 2, 2, 3][g.below(5) as usize];
        let attr = kind
            | g.below(4) << 8
            | g.below(4) << 10
            | g.below(32) << 12
            | g.below(2) << 17
            | g.below(32) << 18
            | g.below(4) << 23
            | g.below(32) << 27;
        v.push(attr);
        let words = if compact { 3 + 3 * (kind & 1) } else { 9 };
        for _ in 0..words {
            v.push(g.float(500.0));
        }
    }
    if g.chance(80) {
        v.push(0);
    }
    v
}

/// A frame's display list for a 2C program: random commands with operands
/// its checks accept (the rest make it loop forever). `skater`: the
/// commands `topskatr`'s program adds (fn 0x11, 0x1B, 0x1C).
fn list_tgp(g: &mut Gen, rom_words: u32, skater: bool) -> Vec<u32> {
    let mut l = Vec::new();
    l.push(command(g, 0x16));
    l.push(g.float(65536.0));
    for _ in 0..g.below(60) + 10 {
        match g.below(22) {
            0 => {
                l.push(command(g, 7));
                l.push(g.below(4));
            }
            1 => {
                l.push(command(g, 9));
                l.push(g.float(1000.0));
                l.push(g.float(1000.0));
            }
            2 => {
                l.push(command(g, 0x0a));
                for _ in 0..3 {
                    l.push(g.float(1.0));
                }
            }
            3 => {
                // Lighting entries, all within the 32 (the coefficients of
                // more would overwrite the table of squarings). The
                // specular control (byte 3) indexes that table, which runs
                // on into other words: values 15 and up can square
                // thousands of times, which the TGP's frame model takes for
                // an idle program and cuts short.
                l.push(command(g, 6));
                let n = g.below(4) + 1;
                l.push(g.below(33 - n));
                l.push(n);
                for _ in 0..n {
                    let control = if g.chance(70) { g.below(9) } else { g.below(15) };
                    l.push(control << 24 | (g.next() >> 40) as u32);
                    l.push(g.float(4.0));
                }
            }
            4 | 5 => {
                let f = if skater && g.chance(40) { 0x1b } else { 0x0b };
                l.push(command(g, f));
                for k in 0..12 {
                    l.push(g.float(if k < 9 { 1.5 } else { 2000.0 }));
                }
            }
            6 => {
                l.push(command(g, 0x0c));
                for _ in 0..3 {
                    l.push(g.float(2000.0));
                }
            }
            7 => {
                l.push(command(g, 8));
                l.push(g.float(16.0));
            }
            8 => {
                l.push(command(g, 3));
                for _ in 0..6 {
                    l.push((g.next() >> 32) as u32);
                }
            }
            9 => {
                let f = if g.chance(50) { 4 } else { 0x14 };
                l.push(command(g, f));
                l.push(g.below(0x1_0000) | if g.chance(50) { 0x80_0000 } else { 0 });
                let n = g.below(8) + 1;
                l.push(n);
                for _ in 0..n {
                    l.push((g.next() >> 32) as u32);
                }
            }
            10 => {
                l.push(command(g, 2));
                l.push((g.next() >> 32) as u32);
                l.push((g.next() >> 32) as u32);
                for _ in 0..6 {
                    l.push(g.float(500.0));
                }
                let n = g.below(5) + 1;
                for k in 0..n {
                    let quad = g.chance(50);
                    l.push(if quad { 1 } else { 2 } | g.below(1 << 16) << 2);
                    for _ in 0..if quad { 8 } else { 5 } {
                        l.push(g.float(500.0));
                    }
                    if k + 1 == n {
                        l.push(0);
                    }
                }
            }
            11 | 12 => {
                // A model into polygon RAM, then drawn.
                let compact = skater && g.chance(50);
                let at = g.below(0x4000);
                let n = g.below(30) + 1;
                let m = model_tgp(g, n, compact);
                l.push(command(g, 5));
                l.push(at);
                l.push(m.len() as u32);
                l.extend_from_slice(&m);
                let f = if compact { 0x11 } else { 1 };
                l.push(command(g, f));
                l.push((g.next() >> 40) as u32);
                l.push((g.next() >> 40) as u32);
                l.push(at);
                l.push(if g.chance(50) { 0 } else { g.below(n + 2) + 1 });
            }
            13..=15 => {
                // An object from the polygon ROM (anything there).
                let f = if skater && g.chance(40) { 0x11 } else { 1 };
                l.push(command(g, f));
                l.push((g.next() >> 40) as u32);
                l.push((g.next() >> 40) as u32);
                l.push(0x80_0000 + g.below(rom_words.max(16) - 16));
                l.push(g.below(40) + 1);
            }
            16 => {
                // Data push from polygon RAM or ROM.
                l.push(command(g, 0x0d));
                l.push(if g.chance(50) { g.below(0x4000) } else { 0x80_0000 + g.below(rom_words.max(16)) });
                l.push(g.below(16) + 1);
            }
            17 => {
                // The board test, with small blocks (`overrev`'s needs one).
                l.push(command(g, 0x0e));
                for i in 0..32 {
                    l.push(1 << i);
                }
                let n = g.below(3) + 1;
                l.push(n);
                for _ in 0..n {
                    l.push(0x80_0000 + g.below(rom_words.max(16)));
                    l.push(g.below(12) + 1);
                    l.push((g.next() >> 32) as u32);
                }
            }
            18 => {
                l.push(command(g, 0x10));
                l.push((g.next() >> 32) as u32);
            }
            19 if skater => l.push(command(g, 0x1c)),
            _ => l.push(command(g, 0)),
        }
    }
    l.push(command(g, 0x0f));
    l
}

/// `fuzz` for a 2C game: its TGP geometrizer program and the native one.
fn fuzz_tgp(roms: &RomSet, program: &[u64], frames: u64, seed: u64) -> Result<(), String> {
    use model2::geo::{Core, Engine, GeoMode, Job};
    let rom = model2::bus::polygon_rom(roms);
    let rom_words = rom.len() as u32;
    let mut e = Engine::new(rom, Core::Tgp);
    e.set_mode(GeoMode::Check);
    e.run_job(Job::Boot(program.to_vec()));
    let Some(skater) = e.native_tgp.as_ref().map(|n| n.program == model2::geo_native_2c::Program::TopSkater) else {
        return Err("no native implementation of this program".into());
    };
    let mut g = Gen(seed.max(1));
    for _ in 0..frames {
        let words = list_tgp(&mut g, rom_words, skater);
        let mut buffer = vec![0u8; 0x2_0000];
        for (k, w) in words.iter().take(buffer.len() / 4).enumerate() {
            buffer[4 * k..4 * k + 4].copy_from_slice(&w.to_le_bytes());
        }
        let before = e.check.differing + e.check.state_differing + e.check.list_differing;
        e.run_job(Job::Frame { buffer, booted: true, read_ptr: 0, jumps: Vec::new() });
        if std::env::var_os("REMODEL_FUZZ_TRACE").is_some() {
            println!(
                "frame {}: list {} words, read {}, stream {}, tgp pc {:#x} steps {}",
                e.frames,
                words.len(),
                e.fed.len(),
                e.stream.len(),
                e.tgp.pc,
                e.tgp.steps
            );
        }
        if let Some(path) = std::env::var_os("REMODEL_FUZZ_DUMP") {
            if before == 0 && e.check.differing + e.check.state_differing + e.check.list_differing > 0 {
                let text: String = words.iter().map(|w| format!("{w:08x}\n")).collect();
                let _ = std::fs::write(&path, text);
                let text: String = e.stream.iter().map(|w| format!("{w:08x}\n")).collect();
                let _ = std::fs::write(format!("{}.stream", path.to_string_lossy()), text);
            }
        }
    }
    fuzz_report(&e, frames)
}
