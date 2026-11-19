//! SPIKE (`run` with MP_TEXMAP=FILE): which texture ROM words the rasteriser
//! read (UV lists, headers; `raster::texmap`) and which data ROM bytes were
//! copied into texture RAM (`texwatch` loads matched to the ROM). Draw-only:
//! every frame is built like the player's, nothing is fed back. MP_TEXMAP_PRIOR
//! lists earlier map files (comma separated) to merge in before the report.
use model2::capture::Frame;
use model2::raster::{self, texmap};
use model2::texwatch::RomIndex;

pub struct TexMap {
    index: RomIndex,
    /// Per 4-byte word of the data ROM: copied into texture RAM.
    copied: Vec<bool>,
    loaded: u64,
    matched: u64,
    pub build_ns: u64,
    pub match_ns: u64,
    frames: u64,
}

impl TexMap {
    pub fn start(m: &mut model2::Machine) -> TexMap {
        texmap::enable();
        m.bus.tex_watch = Some(Box::default());
        let data = m.bus.data_rom();
        TexMap {
            index: RomIndex::new(data.to_vec().into()),
            copied: vec![false; data.len() / 4 + 1],
            loaded: 0,
            matched: 0,
            build_ns: 0,
            match_ns: 0,
            frames: 0,
        }
    }

    pub fn frame(&mut self, m: &mut model2::Machine) {
        let t = std::time::Instant::now();
        let f = Frame::grab(&mut m.bus);
        let src = f.sources(&m.bus.texture_rom);
        let _ = raster::build_shaded_with(&f.stream, &src, None, None);
        self.build_ns += t.elapsed().as_nanos() as u64;
        self.frames += 1;
        let t = std::time::Instant::now();
        let loads = m.bus.tex_watch.as_deref_mut().map(|w| w.take().0).unwrap_or_default();
        let ram = m.bus.render_ram();
        for l in loads {
            let data = &ram[l.offset as usize..(l.offset + l.len) as usize];
            for (k, c) in data.chunks_exact(64).enumerate() {
                self.loaded += 64;
                if c.iter().all(|&b| b == c[0]) {
                    continue;
                }
                if let Some(off) = self.index.find(c) {
                    self.matched += 64;
                    let _ = k;
                    for w in (off as usize / 4)..((off as usize + 64) / 4).min(self.copied.len()) {
                        self.copied[w] = true;
                    }
                }
            }
        }
        self.match_ns += t.elapsed().as_nanos() as u64;
    }

    /// Write the map and print the report.
    pub fn finish(&mut self, m: &mut model2::Machine, path: &std::path::Path) {
        // Merge priors.
        if let Some(p) = std::env::var_os("MP_TEXMAP_PRIOR") {
            for f in p.to_string_lossy().split(',').filter(|s| !s.is_empty()) {
                let t = std::fs::read_to_string(f).unwrap_or_default();
                for l in t.lines() {
                    let v: Vec<&str> = l.split(' ').collect();
                    let n = |i: usize| v[i].parse::<usize>().unwrap_or(0);
                    match v[0] {
                        "U" | "H" => (n(1)..n(2)).for_each(|w| texmap::set(if v[0] == "U" { 0 } else { 1 }, w)),
                        "S" => texmap::set(2, n(1)),
                        "D" => (n(1)..n(2).min(self.copied.len())).for_each(|w| self.copied[w] = true),
                        _ => {}
                    }
                }
            }
        }
        let rom: &[u16] = &m.bus.texture_rom;
        let n = rom.len().min(1 << 23);
        let mut out = String::new();
        for (kind, tag) in [(0, "U"), (1, "H")] {
            let mut w = 0;
            while w < n {
                if texmap::get(kind, w) {
                    let s = w;
                    while w < n && texmap::get(kind, w) {
                        w += 1;
                    }
                    out.push_str(&format!("{tag} {s} {w}\n"));
                } else {
                    w += 1;
                }
            }
        }
        for w in (0..n).filter(|&w| texmap::get(2, w)) {
            out.push_str(&format!("S {w}\n"));
        }
        {
            let mut w = 0;
            while w < self.copied.len() {
                if self.copied[w] {
                    let s = w;
                    while w < self.copied.len() && self.copied[w] {
                        w += 1;
                    }
                    out.push_str(&format!("D {s} {w}\n"));
                } else {
                    w += 1;
                }
            }
        }
        let _ = std::fs::write(path, out);

        println!("texmap        {} frames built; build+grab {:.1} ms/frame, load matching {:.1} ms total", self.frames, self.build_ns as f64 / 1e6 / self.frames.max(1) as f64, self.match_ns as f64 / 1e6);
        // ---- texture ROM ----
        let blank = blank_words(rom, 16);
        let nb = (0..n).filter(|&w| !blank[w]).count();
        let read = |w: usize| texmap::get(0, w) || texmap::get(1, w);
        let cnt = |f: &dyn Fn(usize) -> bool| (0..n).filter(|&w| f(w)).count();
        let (r_all, r_u, r_h) = (cnt(&read), cnt(&|w| texmap::get(0, w)), cnt(&|w| texmap::get(1, w)));
        let r_nb = cnt(&|w| read(w) && !blank[w]);
        println!("tex ROM       {} bytes, non-blank {} bytes ({:.1}%)", n * 2, nb * 2, 100.0 * nb as f64 / n as f64);
        println!(
            "  read        {} bytes {:.2}% of total, {:.2}% of non-blank (UV words {} B, header words {} B)",
            r_all * 2,
            100.0 * r_all as f64 / n as f64,
            100.0 * r_nb as f64 / nb.max(1) as f64,
            r_u * 2,
            r_h * 2
        );
        stretches("tex ROM unread", n, |w| !read(w) && !blank[w], 2);
        // Headers.
        let starts: Vec<usize> = (0..n).filter(|&w| texmap::get(2, w)).collect();
        let aligned = starts.iter().filter(|&&w| w % 4 == 0).count();
        let mut or = [0u16; 4];
        let mut valid = 0;
        for &s in &starts {
            if s + 4 <= n {
                for k in 0..4 {
                    or[k] |= rom[s + k];
                }
                if rom[s] & (1 << 14) != 0 {
                    valid += 1;
                }
            }
        }
        println!(
            "  headers     {} distinct read, {} start at a 4-word-aligned address, {} have bit 14 (textured); OR of header words {:04x} {:04x} {:04x} {:04x}",
            starts.len(),
            aligned,
            valid,
            or[0],
            or[1],
            or[2],
            or[3]
        );
        // Candidate predicate: any 4-word window with bit 14 of word 0 set and only bits seen in read headers.
        let pred = |s: usize| s + 4 <= n && rom[s] & (1 << 14) != 0 && (0..4).all(|k| rom[s + k] & !or[k] == 0);
        let mut by_mod = [0usize; 4];
        for &s in &starts {
            by_mod[s % 4] += 1;
        }
        let cand: Vec<usize> = (0..n).filter(|&s| pred(s)).collect();
        let mut cand_mod = [0usize; 4];
        for &s in &cand {
            cand_mod[s % 4] += 1;
        }
        let read_cand = cand.iter().filter(|&&s| texmap::get(2, s)).count();
        let nb_cand = cand.iter().filter(|&&s| !blank[s]).count();
        // False positives: windows entirely made of words read as UV and never as header.
        let (mut uvt, mut uvfp) = (0, 0);
        for s in 0..n.saturating_sub(4) {
            if (0..4).all(|k| texmap::get(0, s + k) && !texmap::get(1, s + k)) {
                uvt += 1;
                if pred(s) {
                    uvfp += 1;
                }
            }
        }
        println!(
            "  candidates  header starts read by address mod 4: {by_mod:?}; windows at any offset that look like a header: {} ({} non-blank; by mod 4 {cand_mod:?}), {} of them read as a header; false positives on UV-only windows {uvfp}/{uvt}",
            cand.len(),
            nb_cand,
            read_cand,
        );
        // ---- data ROM ----
        let data = m.bus.data_rom();
        let words: &[u8] = data;
        let dn = self.copied.len() - 1;
        let w32: Vec<u32> = (0..dn).map(|i| u32::from_le_bytes([words[4 * i], words[4 * i + 1], words[4 * i + 2], words[4 * i + 3]])).collect();
        let dblank = blank_words32(&w32, 16);
        let dnb = (0..dn).filter(|&w| !dblank[w]).count();
        let c_all = (0..dn).filter(|&w| self.copied[w]).count();
        let c_nb = (0..dn).filter(|&w| self.copied[w] && !dblank[w]).count();
        println!("data ROM      {} bytes, non-blank {} bytes ({:.1}%)", dn * 4, dnb * 4, 100.0 * dnb as f64 / dn as f64);
        println!(
            "  copied      {} bytes {:.2}% of total, {:.2}% of non-blank; texture RAM loads seen {} B, {} B ({:.1}%) found verbatim in the data ROM",
            c_all * 4,
            100.0 * c_all as f64 / dn as f64,
            100.0 * c_nb as f64 / dnb.max(1) as f64,
            self.loaded,
            self.matched,
            100.0 * self.matched as f64 / self.loaded.max(1) as f64
        );
        stretches("data ROM uncopied", dn, |w| !self.copied[w] && !dblank[w], 4);
    }
}

fn blank_words(rom: &[u16], min: usize) -> Vec<bool> {
    let mut b = vec![false; rom.len()];
    let mut i = 0;
    while i < rom.len() {
        let mut j = i;
        while j < rom.len() && rom[j] == rom[i] {
            j += 1;
        }
        if j - i >= min {
            b[i..j].iter_mut().for_each(|x| *x = true);
        }
        i = j;
    }
    b
}

fn blank_words32(rom: &[u32], min: usize) -> Vec<bool> {
    let mut b = vec![false; rom.len()];
    let mut i = 0;
    while i < rom.len() {
        let mut j = i;
        while j < rom.len() && rom[j] == rom[i] {
            j += 1;
        }
        if j - i >= min {
            b[i..j].iter_mut().for_each(|x| *x = true);
        }
        i = j;
    }
    b
}

fn stretches(label: &str, n: usize, unread: impl Fn(usize) -> bool, unit: usize) {
    let mut v = Vec::new();
    let mut w = 0;
    while w < n {
        if unread(w) {
            let s = w;
            while w < n && unread(w) {
                w += 1;
            }
            v.push((w - s) * unit);
        } else {
            w += 1;
        }
    }
    v.sort_unstable_by(|a, b| b.cmp(a));
    let tot: usize = v.iter().sum();
    let small: usize = v.iter().filter(|&&x| x < 16384).sum();
    println!(
        "  {label}: {} stretches, {} bytes; ten largest {:?}; under 16 KB total {}",
        v.len(),
        tot,
        &v[..v.len().min(10)],
        small
    );
}
