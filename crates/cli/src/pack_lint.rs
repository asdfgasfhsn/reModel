//! `pack-lint`: check a texture pack against the format
//! (docs/texture-packs.md, "Pack format") without a ROM or a GPU. Errors
//! are what readers refuse or can't load; warnings what they silently
//! ignore or draw oddly; notes what is worth knowing. Checks (letters as in
//! the format's design): a the manifest, b file names, c `index.txt`,
//! d decoding and size, e image and map proportions, f normal maps' vectors,
//! g normal maps under the image's cut-outs, h (with a dump) normal maps
//! across mirrored and repeated edges, i (with a dump) files
//! never drawn, j (with a dump) maps under both an image and a surface key,
//! k GPU memory against the budgets.

use crate::texpack_materials::{edge_line, edge_modes, read_normal, tiles};
use crate::texpack_upscale;
use model2::enhance::{address, Address, Rgba};
use model2::pack::{self, parse_name, FileKind, Manifest, MapKind};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// Tool records and the like that readers ignore.
const RECORDS: [&str; 5] = [pack::MANIFEST, pack::INDEX, "upscale.txt", "drift.txt", "materials.txt"];
/// Files named per finding before "and N more".
const SHOWN: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Error,
    Warning,
    Note,
}

/// Findings, grouped by level, check and message; each with the files (or
/// details) it concerns.
#[derive(Default)]
struct Report {
    found: BTreeMap<(Level, char, String), Vec<String>>,
}

impl Report {
    fn add(&mut self, level: Level, check: char, what: impl Into<String>, item: impl Into<String>) {
        self.found.entry((level, check, what.into())).or_default().push(item.into());
    }

    fn count(&self, level: Level) -> usize {
        self.found.keys().filter(|k| k.0 == level).count()
    }

    fn print(&self) {
        for ((level, check, what), items) in &self.found {
            let level = match level {
                Level::Error => "error",
                Level::Warning => "warning",
                Level::Note => "note",
            };
            let items: Vec<&String> = items.iter().filter(|i| !i.is_empty()).collect();
            if items.is_empty() {
                println!("{level:<8}{check}  {what}");
                continue;
            }
            let mut list = items.iter().take(SHOWN).map(|s| s.as_str()).collect::<Vec<_>>().join(", ");
            if items.len() > SHOWN {
                list += &format!(", and {} more", items.len() - SHOWN);
            }
            println!("{level:<8}{check}  {what} ({}): {list}", items.len());
        }
    }
}

/// A PNG in the pack: its stem, what its name says, and its size once
/// decoded (None: it doesn't decode).
struct File {
    stem: String,
    kind: Result<FileKind, String>,
    size: Option<(usize, usize)>,
}

/// Bytes a texture of `w` x `h` RGBA texels takes on the GPU, with its mip
/// chain down to 1x1 or without.
fn gpu_bytes(w: usize, h: usize, mips: bool) -> u64 {
    let (mut w, mut h, mut sum) = (w as u64, h as u64, 0);
    loop {
        sum += w * h * 4;
        if !mips || (w <= 1 && h <= 1) {
            return sum;
        }
        (w, h) = ((w / 2).max(1), (h / 2).max(1));
    }
}

/// `size` doesn't have the proportions of `w` x `h` texels (to a pixel).
fn off_aspect((sw, sh): (usize, usize), w: u32, h: u32) -> bool {
    let (sw, sh, w, h) = (sw as f64, sh as f64, w as f64, h as f64);
    (sh - sw * h / w).abs() > 1.0 && (sw - sh * w / h).abs() > 1.0
}

/// Decode every file (in parallel), noting failures and oversize images.
fn decode_all(dir: &Path, stems: &[String], max_side: usize) -> Vec<Result<(usize, usize), String>> {
    let threads = std::thread::available_parallelism().map_or(4, |n| n.get()).min(16);
    let chunk = stems.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        let jobs: Vec<_> = stems
            .chunks(chunk)
            .map(|part| {
                s.spawn(move || {
                    part.iter()
                        .map(|stem| {
                            let bytes = std::fs::read(dir.join(format!("{stem}.png"))).map_err(|e| e.to_string())?;
                            let img = png::decode(&bytes)?;
                            if img.width > max_side || img.height > max_side {
                                return Err(format!(
                                    "{}x{}, more than --max-side {max_side}: drawn as the original",
                                    img.width, img.height
                                ));
                            }
                            Ok((img.width, img.height))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        jobs.into_iter().flat_map(|j| j.join().unwrap()).collect()
    })
}

/// A dump's `index.txt`: names, and the surface key of each texture line
/// that has one (`surf=`).
struct Dump {
    dir: PathBuf,
    names: BTreeSet<String>,
    surf: HashMap<String, String>,
    /// Per texture: how each axis is drawn (`wrap=`, as the tools pad) and
    /// its uses.
    drawn: HashMap<String, ([Address; 2], u64)>,
}

fn read_dump(dir: &Path) -> Result<Dump, String> {
    let path = dir.join(pack::INDEX);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let drawn = texpack_upscale::parse_index(&text)
        .into_iter()
        .filter(|d| !d.tile)
        .map(|d| (d.name, (d.modes, d.uses)))
        .collect();
    let mut d = Dump { dir: dir.to_path_buf(), names: BTreeSet::new(), surf: HashMap::new(), drawn };
    for line in text.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()) {
        let mut fields = line.split('\t');
        let name = fields.next().unwrap_or("").trim().to_string();
        if let Some(s) = fields.find_map(|f| f.strip_prefix("surf=")) {
            d.surf.insert(name.clone(), s.to_string());
        }
        d.names.insert(name);
    }
    Ok(d)
}

/// The stem without a map suffix.
fn base(stem: &str) -> &str {
    stem.split_once('.').map_or(stem, |(b, _)| b)
}

/// A normal map to look into: its file, the image whose alpha is its
/// base (cut-outs), and how its texture is drawn, where known.
struct NormalMap {
    stem: String,
    base: Option<PathBuf>,
    modes: Option<[Address; 2]>,
}

/// Share over which a finding is reported (of the texels concerned).
const SHARE: f64 = 0.01;
/// Largest difference of a component (-1 to 1) from what it should be:
/// texels under a cut-out against their opaque neighbours, and the mean
/// component across a mirror line.
const NEAR: f32 = 0.15;
const ACROSS: f32 = 0.1;

/// Checks f (unit normals, x² + y² <= 1, z not negative, entirely flat),
/// g (texels under the base image's transparent texels near their opaque
/// neighbours) and h (with a dump: no lean across a mirrored edge, opposite
/// edges continue on a repeated axis) on every normal map.
fn check_normal_maps(
    r: &mut Report,
    dir: &Path,
    files: &[File],
    stems: &BTreeSet<String>,
    dump: Option<&Dump>,
    y_up: bool,
) {
    // Dump textures by surface map base, for surf_ maps.
    let mut by_surface: HashMap<String, Vec<&str>> = HashMap::new();
    if let Some(d) = dump {
        for (name, s) in &d.surf {
            let size = name.rsplit('_').next().unwrap_or("");
            by_surface.entry(format!("surf_{s}_{size}")).or_default().push(name);
        }
    }
    let image = |name: &str| -> Option<PathBuf> {
        if stems.contains(name) {
            return Some(dir.join(format!("{name}.png")));
        }
        dump.map(|d| d.dir.join(format!("{name}.png"))).filter(|p| p.exists())
    };
    let mut maps = vec![];
    for f in files {
        let (Ok(FileKind::Map { surface, kind: MapKind::Normal, .. }), Some(_)) = (&f.kind, f.size) else { continue };
        let surface = *surface;
        let b = base(&f.stem);
        let (base_image, modes) = if !surface {
            (image(b), dump.and_then(|d| d.drawn.get(b)).map(|m| m.0))
        } else {
            let members: Vec<(&str, ([Address; 2], u64))> = by_surface
                .get(b)
                .map(|m| m.iter().filter_map(|n| Some((*n, *dump?.drawn.get(*n)?))).collect())
                .unwrap_or_default();
            // The image the tools make it from: one in the pack, most used.
            let best = members.iter().max_by_key(|(n, (_, uses))| (stems.contains(*n), *uses)).map(|(n, _)| *n);
            let modes = (!members.is_empty()).then(|| {
                [0, 1].map(|a| {
                    if members.iter().all(|(_, (m, _))| m[a] == Address::Repeat) {
                        Address::Repeat
                    } else {
                        Address::Mirror
                    }
                })
            });
            (best.and_then(image), modes)
        };
        maps.push(NormalMap { stem: f.stem.clone(), base: base_image, modes });
    }
    let found = texpack_upscale::par_map(&maps, |m| check_normal_map(dir, m, y_up));
    for (level, check, what, item) in found.into_iter().flatten() {
        r.add(level, check, what, item);
    }
}

fn check_normal_map(dir: &Path, m: &NormalMap, y_up: bool) -> Vec<(Level, char, &'static str, String)> {
    let mut out = vec![];
    let Ok(img) =
        std::fs::read(dir.join(format!("{}.png", m.stem))).map_err(|e| e.to_string()).and_then(|b| png::decode(&b))
    else {
        return out; // reported by d
    };
    let (w, h) = (img.width, img.height);
    let px: Vec<&[u8]> = img.rgba.chunks(4).collect();
    let n = px.len();
    let over = |k: usize, of: usize| of > 0 && k as f64 > SHARE * of as f64;
    let pct = |k: usize, of: usize| format!("{:.1}%", 100.0 * k as f64 / of.max(1) as f64);

    // f
    let raw = |v: u8| (v as f32 - 128.0) / 127.0;
    let blue = px.iter().any(|p| p[2] != 0);
    let (mut off_len, mut long, mut under, mut flat) = (0, 0, 0, true);
    for p in &px {
        let (x, y, z) = (raw(p[0]), raw(p[1]), raw(p[2]));
        flat &= (p[0] as i32 - 128).abs() <= 1 && (p[1] as i32 - 128).abs() <= 1;
        long += (x * x + y * y > 1.02) as usize;
        if blue {
            off_len += !(0.9..=1.1).contains(&(x * x + y * y + z * z).sqrt()) as usize;
            under += (p[2] < 127) as usize;
        }
    }
    if over(off_len, n) {
        out.push((
            Level::Warning,
            'f',
            "normal map's red, green and blue aren't a unit vector on over 1% of texels (blue is ignored: z is rebuilt from red and green)",
            format!("{} ({})", m.stem, pct(off_len, n)),
        ));
    }
    if over(long, n) {
        out.push((
            Level::Warning,
            'f',
            "normal map's x² + y² over 1 on over 1% of texels (scaled back to length 1 when loaded)",
            format!("{} ({})", m.stem, pct(long, n)),
        ));
    }
    if under > 0 {
        out.push((
            Level::Warning,
            'f',
            "normal map points into the surface (blue under 128)",
            format!("{} ({})", m.stem, pct(under, n)),
        ));
    }
    if flat {
        out.push((Level::Warning, 'f', "normal map entirely flat (changes nothing)", m.stem.clone()));
    }
    let nrm: Vec<[f32; 3]> = px.iter().map(|p| read_normal(p, y_up)).collect();

    // g: under the base's cut-outs.
    let base = m.base.as_ref().and_then(|p| std::fs::read(p).ok()).and_then(|b| png::decode(&b).ok());
    let base = base.map(|b| Rgba { width: b.width, height: b.height, data: b.rgba });
    let opaque: Vec<bool> = match &base {
        Some(b) => (0..n)
            .map(|i| {
                let (bx, by) = ((i % w) * b.width / w, (i / w) * b.height / h);
                b.data[(by * b.width + bx) * 4 + 3] >= 128
            })
            .collect(),
        None => vec![true; n],
    };
    if let Some(b) = &base {
        // Neighbours as the tools take them.
        let modes = edge_modes(b, m.modes.unwrap_or([Address::Mirror; 2]));
        let (mut border, mut far) = (0, 0);
        for i in (0..n).filter(|&i| !opaque[i]) {
            let (x, y) = ((i % w) as i32, (i / w) as i32);
            let mut sum = [0f32; 3];
            let mut any = false;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let j = address(y + dy, h, modes[1]) * w + address(x + dx, w, modes[0]);
                    if (dx, dy) != (0, 0) && opaque[j] {
                        (0..3).for_each(|c| sum[c] += nrm[j][c]);
                        any = true;
                    }
                }
            }
            if !any {
                continue;
            }
            border += 1;
            let l = (sum[0] * sum[0] + sum[1] * sum[1] + sum[2] * sum[2]).sqrt().max(1e-6);
            far += ((nrm[i][0] - sum[0] / l).abs() > NEAR || (nrm[i][1] - sum[1] / l).abs() > NEAR) as usize;
        }
        if over(far, border) {
            out.push((
                Level::Warning,
                'g',
                "normal map under the image's transparent texels differs from its opaque neighbours (shows in mip levels and at cut-out edges)",
                format!("{} ({} of the edge)", m.stem, pct(far, border)),
            ));
        }
    }

    // h: the edges, as the texture is drawn.
    if let Some(modes) = m.modes {
        for axis in 0..2 {
            let (along, across) = if axis == 0 { (h, w) } else { (w, h) };
            if across < 3 {
                continue;
            }
            let name = ["u", "v"][axis];
            if modes[axis] == Address::Mirror {
                let at = |i: usize, j: usize| if axis == 0 { i * w + j } else { j * w + i };
                let edge: Vec<f32> = (0..along)
                    .flat_map(|i| [at(i, 0), at(i, across - 1)])
                    .filter(|&k| opaque[k])
                    .map(|k| nrm[k][axis].abs())
                    .collect();
                let lean = edge.iter().sum::<f32>() / edge.len().max(1) as f32;
                if lean > ACROSS {
                    out.push((
                        Level::Warning,
                        'h',
                        "normal map leans across an edge where the texture is drawn mirrored (a crease or step at the mirror line)",
                        format!("{} ({name}: mean {lean:.2})", m.stem),
                    ));
                }
                continue;
            }
            // Repeated: the step from the last texels to the first against
            // the steps next to it, on each side (a seam is a jump where its
            // neighbourhood has none). Not where the image itself doesn't
            // tile (that seam is the image's), nor where it has a line
            // along the edge (relief centred on the edge is a crease there).
            if base.as_ref().is_some_and(|b| !tiles(b, axis) || edge_line(b, axis)) {
                continue;
            }
            let at = |i: usize, j: usize| if axis == 0 { i * w + j } else { j * w + i };
            let step = |a: usize, b: usize| {
                (opaque[a] && opaque[b]).then(|| (nrm[a][0] - nrm[b][0]).abs() + (nrm[a][1] - nrm[b][1]).abs())
            };
            let mean = |pairs: &mut dyn Iterator<Item = (usize, usize)>| {
                let (s, k) = pairs.filter_map(|(a, b)| step(a, b)).fold((0f32, 0usize), |(s, k), v| (s + v, k + 1));
                s / k.max(1) as f32
            };
            let last = across - 1;
            let seam = mean(&mut (0..along).map(|i| (at(i, last), at(i, 0))));
            let near = mean(&mut (0..along).flat_map(|i| [(at(i, 0), at(i, 1)), (at(i, last - 1), at(i, last))]));
            if seam > 2.0 * near + 0.05 {
                out.push((
                    Level::Warning,
                    'h',
                    "normal map's opposite edges don't continue where the texture is drawn repeated (a seam)",
                    format!("{} ({name}: step {seam:.2} across the edge, {near:.2} beside it)", m.stem),
                ));
            }
        }
    }
    out
}

/// Lint the pack in `dir`, print the findings and return them.
fn lint_report(dir: &Path, dump: Option<&Path>, max_side: usize) -> Result<Report, String> {
    let mut r = Report::default();
    let mut files = vec![];
    let mut other = BTreeSet::new();
    for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().into_owned();
        if e.file_type().map_err(|e| e.to_string())?.is_dir() {
            if name == "models" {
                r.add(Level::Note, 'b', "directory reserved for replacement models, ignored by readers", "models/");
            }
            continue;
        }
        match name.strip_suffix(".png") {
            Some(stem) => files.push(File { stem: stem.to_string(), kind: parse_name(stem), size: None }),
            None => {
                other.insert(name);
            }
        }
    }
    files.sort_by(|a, b| a.stem.cmp(&b.stem));
    let stems: BTreeSet<String> = files.iter().map(|f| f.stem.clone()).collect();

    // a: the manifest.
    let manifest = match std::fs::read_to_string(dir.join(pack::MANIFEST)) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("{}: {e}", dir.join(pack::MANIFEST).display())),
        Ok(text) => match Manifest::parse(&text) {
            Err(e) => {
                r.add(Level::Error, 'a', format!("{e}; readers refuse the pack"), "");
                None
            }
            Ok(m) => Some(m),
        },
    };
    let has_maps = files.iter().any(|f| matches!(f.kind, Ok(FileKind::Map { .. })));
    match &manifest {
        None if has_maps && other.contains(pack::MANIFEST) => {}
        None if has_maps => r.add(
            Level::Note,
            'a',
            "material maps but no pack.txt: `format 2` and `uses materials` say what the pack holds",
            "",
        ),
        None => {}
        Some(m) => {
            if let Err(e) = m.check() {
                r.add(Level::Error, 'a', format!("{e}: readers refuse the pack and draw the original textures"), "");
            }
            for l in &m.unknown {
                r.add(Level::Note, 'a', "keyword this reModel doesn't know, ignored by readers", l.replace('\t', " "));
            }
            for l in &m.reserved {
                r.add(Level::Note, 'a', "reserved keyword, ignored by readers", l.replace('\t', " "));
            }
            for f in m.uses.iter().chain(&m.needs) {
                if !pack::FEATURES.contains(&f.as_str()) {
                    r.add(Level::Note, 'a', "feature the format doesn't define (`uses`: ignored)", f.clone());
                } else if f != "materials" {
                    r.add(Level::Note, 'a', "feature reserved by the format, not drawn by any reader", f.clone());
                }
            }
            if has_maps && !m.uses.iter().any(|f| f == "materials") {
                r.add(Level::Note, 'a', "material maps but pack.txt has no `uses materials`", "");
            }
            if let Some(g) = &m.game {
                if romset::games::find(g).is_err() {
                    r.add(Level::Warning, 'a', "`game` isn't a set reModel knows", g.clone());
                }
            }
            for stem in m.materials.keys() {
                let any = stems.contains(stem.as_str())
                    || MapKind::ALL.iter().any(|&k| stems.contains(pack::map_stem(stem, k).as_str()));
                if !any {
                    r.add(Level::Note, 'a', "`material` line for a stem with no files", stem.clone());
                }
            }
        }
    }

    // b: file names.
    for o in other.iter().filter(|o| !RECORDS.contains(&o.as_str())) {
        r.add(Level::Note, 'b', "not a pack file, ignored by readers", o.clone());
    }
    for f in &files {
        match &f.kind {
            Err(why) => r.add(Level::Warning, 'b', format!("not a pack name, never opened by readers: {why}"), &f.stem),
            Ok(FileKind::Reserved) => {
                r.add(Level::Note, 'b', "name reserved by the format, ignored by readers", &f.stem)
            }
            Ok(_) => {}
        }
    }

    // c: index.txt, which the browser fetches by.
    match std::fs::read_to_string(dir.join(pack::INDEX)) {
        Err(_) => r.add(
            Level::Warning,
            'c',
            "no index.txt: the browser can't use the pack (`ls *.png > index.txt` in it makes one)",
            "",
        ),
        Ok(text) => {
            let listed: BTreeSet<&str> = text
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .map(|l| l.strip_suffix(".png").unwrap_or(l))
                .collect();
            for f in &files {
                if !listed.contains(f.stem.as_str()) {
                    r.add(
                        Level::Warning,
                        'c',
                        "not in index.txt: the browser never fetches it",
                        format!("{}.png", f.stem),
                    );
                }
            }
            for l in &listed {
                let there = if l.contains('/') { dir.join(l).exists() } else { stems.contains(*l) };
                if !there {
                    r.add(Level::Warning, 'c', "in index.txt but not in the pack: the browser's fetch fails", *l);
                }
            }
        }
    }

    // d: every file decodes, within the size limit.
    let decoded = decode_all(dir, &files.iter().map(|f| f.stem.clone()).collect::<Vec<_>>(), max_side);
    for (f, d) in files.iter_mut().zip(decoded) {
        match d {
            Ok(size) => f.size = Some(size),
            Err(e) => r.add(Level::Error, 'd', "doesn't load, drawn as the original", format!("{}.png ({e})", f.stem)),
        }
    }

    // e: proportions; maps against their image and each other.
    let size_of: HashMap<&str, (usize, usize)> =
        files.iter().filter_map(|f| f.size.map(|s| (f.stem.as_str(), s))).collect();
    let mut maps_by_base: BTreeMap<&str, Vec<&File>> = BTreeMap::new();
    for f in &files {
        let Some(size) = f.size else { continue };
        let (w, h) = match f.kind {
            Ok(FileKind::Image { width, height, .. }) => (width, height),
            Ok(FileKind::Map { width, height, .. }) => {
                maps_by_base.entry(base(&f.stem)).or_default().push(f);
                (width, height)
            }
            Ok(FileKind::Tile { .. }) => {
                if size.0 != size.1 || size.0 % 8 != 0 {
                    r.add(
                        Level::Warning,
                        'e',
                        "tile image not a square multiple of 8x8",
                        format!("{} ({}x{})", f.stem, size.0, size.1),
                    );
                }
                continue;
            }
            _ => continue,
        };
        if off_aspect(size, w, h) {
            r.add(
                Level::Warning,
                'e',
                "proportions differ from the texture's w x h (stretched when drawn)",
                format!("{} ({}x{})", f.stem, size.0, size.1),
            );
        }
    }
    for (b, maps) in &maps_by_base {
        let sizes: BTreeSet<(usize, usize)> = maps.iter().filter_map(|f| f.size).collect();
        if sizes.len() > 1 {
            let list: Vec<String> = maps.iter().filter_map(|f| f.size.map(|(w, h)| format!("{w}x{h}"))).collect();
            r.add(Level::Warning, 'e', "maps of one material differ in size", format!("{b} ({})", list.join(", ")));
        }
        if let (Some(&img), Some(&map)) = (size_of.get(b), sizes.iter().next()) {
            if img != map {
                r.add(
                    Level::Warning,
                    'e',
                    "map size isn't its image's (resampled when loaded)",
                    format!("{b} (image {}x{}, map {}x{})", img.0, img.1, map.0, map.1),
                );
            }
        }
    }

    // f, g, h: what is in the normal maps.
    let dump_info = dump.map(read_dump).transpose()?;
    let y_up = manifest.as_ref().is_none_or(|m| m.normal_y_up);
    check_normal_maps(&mut r, dir, &files, &stems, dump_info.as_ref(), y_up);

    // i, j: against a dump.
    match &dump_info {
        Some(d) => {
            let surfs: BTreeSet<String> =
                d.surf.iter().map(|(name, s)| format!("surf_{s}_{}", name.rsplit('_').next().unwrap_or(""))).collect();
            if d.surf.is_empty() && files.iter().any(|f| matches!(f.kind, Ok(FileKind::Map { surface: true, .. }))) {
                r.add(Level::Note, 'i', "the dump has no surf= (made before surface keys): surf_ maps not checked", "");
            }
            for f in &files {
                let b = base(&f.stem);
                let known = match f.kind {
                    Ok(FileKind::Image { .. } | FileKind::Tile { .. } | FileKind::Map { surface: false, .. }) => {
                        d.names.contains(b)
                    }
                    Ok(FileKind::Map { surface: true, .. }) => d.surf.is_empty() || surfs.contains(b),
                    _ => true,
                };
                if !known {
                    r.add(Level::Note, 'i', "key not in the dump (never drawn in its run)", &f.stem);
                }
                // j: an image's own map and a surface map of the same kind.
                if let Ok(FileKind::Map { surface: false, width, height, kind, .. }) = f.kind {
                    if let Some(s) = d.surf.get(b) {
                        let shared = pack::map_stem(&format!("surf_{s}_{width}x{height}"), kind);
                        if stems.contains(shared.as_str()) {
                            r.add(
                                Level::Note,
                                'j',
                                "map for the image and for its surface: the image's wins (meant?)",
                                format!("{} over {shared}", f.stem),
                            );
                        }
                    }
                }
            }
        }
        None => {
            let maps =
                |s: bool| files.iter().any(|f| matches!(f.kind, Ok(FileKind::Map { surface, .. }) if surface == s));
            if maps(false) && maps(true) {
                r.add(
                    Level::Note,
                    'j',
                    "image and surface maps both present: --dump DIR (TEXDUMP=) checks which overlap",
                    "",
                );
            }
        }
    }

    // k: GPU memory, everything drawn at once.
    let (mut images, mut materials, mut n_mat) = ([0u64; 2], [0u64; 2], 0);
    for f in &files {
        if let (Ok(FileKind::Image { .. }), Some((w, h))) = (&f.kind, f.size) {
            images[0] += gpu_bytes(w, h, false);
            images[1] += gpu_bytes(w, h, true);
        }
    }
    for maps in maps_by_base.values() {
        // One texture per material, the size of its normal map (else
        // specular, else gloss).
        let first = MapKind::ALL
            .iter()
            .find_map(|&k| maps.iter().find(|f| matches!(f.kind, Ok(FileKind::Map { kind, .. }) if kind == k)))
            .and_then(|f| f.size);
        if let Some((w, h)) = first {
            n_mat += 1;
            materials[0] += gpu_bytes(w, h, false);
            materials[1] += gpu_bytes(w, h, true);
        }
    }
    let mb = |b: u64| format!("{:.1} MB", b as f64 / (1 << 20) as f64);
    let total = [images[0] + materials[0], images[1] + materials[1]];
    let over = |b: u64| match (b > pack::BUDGET_NATIVE, b > pack::BUDGET_WASM) {
        (true, _) => "over both budgets",
        (false, true) => "over the browser's budget",
        _ => "within both budgets",
    };
    r.add(
        Level::Note,
        'k',
        format!(
            "GPU memory if all drawn at once: images {} ({} mipmapped), {n_mat} materials {} ({} mipmapped); \
             total {}, {}, and {} mipmapped, {} (budgets: {} native, {} browser; past them the least recently \
             drawn are dropped; tiles are drawn from the tile atlas, not counted)",
            mb(images[0]),
            mb(images[1]),
            mb(materials[0]),
            mb(materials[1]),
            mb(total[0]),
            over(total[0]),
            mb(total[1]),
            over(total[1]),
            mb(pack::BUDGET_NATIVE),
            mb(pack::BUDGET_WASM),
        ),
        "",
    );

    let count = |p: fn(&FileKind) -> bool| files.iter().filter(|f| f.kind.as_ref().is_ok_and(p)).count();
    println!(
        "pack-lint {}: {} PNGs: {} images, {} tiles, {} material maps; manifest: {}",
        dir.display(),
        files.len(),
        count(|k| matches!(k, FileKind::Image { .. })),
        count(|k| matches!(k, FileKind::Tile { .. })),
        count(|k| matches!(k, FileKind::Map { .. })),
        match (&manifest, other.contains(pack::MANIFEST)) {
            (Some(m), _) => format!("format {}", m.format),
            (None, true) => "unreadable".into(),
            (None, false) => "none (format 1)".into(),
        }
    );
    if let Some(d) = dump {
        println!("dump          {}", d.display());
    }
    r.print();
    let (e, w, n) = (r.count(Level::Error), r.count(Level::Warning), r.count(Level::Note));
    println!("pack-lint: {e} errors, {w} warnings, {n} notes");
    Ok(r)
}

/// Lint the pack in `dir` and print the findings. Ok(false): it has errors.
pub fn lint(dir: &Path, dump: Option<&Path>, max_side: usize) -> Result<bool, String> {
    Ok(lint_report(dir, dump, max_side)?.count(Level::Error) == 0)
}

/// `remodel pack-lint PACK [--dump DIR] [--max-side N]`.
pub fn main(args: &[String]) -> Result<bool, String> {
    let (mut pack, mut dump, mut max_side) = (None, None, 8192);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dump" => dump = Some(PathBuf::from(it.next().ok_or("--dump wants a directory")?)),
            "--max-side" => {
                max_side = it
                    .next()
                    .and_then(|v| v.parse().ok())
                    .filter(|&n| n > 0)
                    .ok_or("--max-side wants a size in pixels")?
            }
            p if pack.is_none() && !p.starts_with("--") => pack = Some(PathBuf::from(p)),
            _ => return Err(format!("pack-lint: unexpected `{a}` (pack-lint PACK [--dump DIR] [--max-side N])")),
        }
    }
    let pack = pack.ok_or("pack-lint wants a pack directory")?;
    if !pack.is_dir() {
        return Err(format!("{}: texture pack directory not found", pack.display()));
    }
    lint(&pack, dump.as_deref(), max_side)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_and_proportions() {
        assert_eq!(gpu_bytes(4, 4, false), 64);
        assert_eq!(gpu_bytes(4, 4, true), 64 + 16 + 4);
        assert_eq!(gpu_bytes(8, 2, true), 64 + 16 + 8 + 4);
        assert!(!off_aspect((256, 32), 64, 8));
        assert!(!off_aspect((100, 13), 64, 8), "within a pixel");
        assert!(off_aspect((256, 256), 64, 8));
    }

    #[test]
    fn finds_what_is_wrong() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/pack-lint-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let img = |w: usize, h: usize| png::encode_rgba(w, h, &vec![128; w * h * 4]);
        let key = "tex_0123456789abcdef_8x8";
        std::fs::write(dir.join(format!("{key}.png")), img(32, 32)).unwrap();
        std::fs::write(dir.join(format!("{key}.n.png")), img(32, 32)).unwrap();
        std::fs::write(dir.join(format!("{key}.s.png")), img(16, 16)).unwrap();
        std::fs::write(dir.join("tile_0123456789abcdef.n.png"), img(8, 8)).unwrap();
        std::fs::write(dir.join("tex_0000000000000001_8x8.png"), b"not a png").unwrap();
        std::fs::write(dir.join("index.txt"), format!("{key}.png\ngone.png\n")).unwrap();
        // Clean apart from the files: the manifest is fine.
        std::fs::write(dir.join("pack.txt"), "format 2\nuses materials\nshiny yes\n").unwrap();
        assert!(!lint(&dir, None, 8192).unwrap(), "a file that doesn't decode is an error");
        std::fs::remove_file(dir.join("tex_0000000000000001_8x8.png")).unwrap();
        assert!(lint(&dir, None, 8192).unwrap(), "warnings alone pass");
        assert!(!lint(&dir, None, 16).unwrap(), "over --max-side");
        std::fs::write(dir.join("pack.txt"), "format 3\n").unwrap();
        assert!(!lint(&dir, None, 8192).unwrap(), "a pack readers refuse");
        std::fs::write(dir.join("pack.txt"), "uses materials\n").unwrap();
        assert!(!lint(&dir, None, 8192).unwrap(), "a malformed manifest");
        // Against a dump: a key it never drew (i), and an image map that
        // shadows a surface map (j).
        std::fs::write(dir.join("pack.txt"), "format 2\nuses materials\n").unwrap();
        let dump = dir.join("dump");
        std::fs::create_dir_all(&dump).unwrap();
        std::fs::write(dump.join("index.txt"), format!("# a dump\n{key}\tuses=1\tsurf=00000000000000ff\twrap=rr\n"))
            .unwrap();
        std::fs::write(dir.join("surf_00000000000000ff_8x8.n.png"), img(32, 32)).unwrap();
        std::fs::write(dir.join("tex_00000000000000ee_8x8.png"), img(32, 32)).unwrap();
        let r = lint_report(&dir, Some(&dump), 8192).unwrap();
        let has = |c: char, item: &str| r.found.iter().any(|(k, v)| k.1 == c && v.iter().any(|i| i.contains(item)));
        assert!(has('i', "tex_00000000000000ee_8x8"));
        assert!(!has('i', "surf_00000000000000ff_8x8.n"), "the surface is in the dump");
        assert!(has('j', &format!("{key}.n over surf_00000000000000ff_8x8.n")));
        assert!(has('b', "tile_0123456789abcdef.n") && has('c', "gone") && has('e', key));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checks_what_is_in_normal_maps() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/pack-lint-normals-test");
        let _ = std::fs::remove_dir_all(&dir);
        let dump = dir.join("dump");
        std::fs::create_dir_all(&dump).unwrap();
        let rgb = |w: usize, h: usize, f: &dyn Fn(usize, usize) -> [u8; 3]| {
            png::encode_rgb(w, h, &(0..w * h).flat_map(|i| f(i % w, i / w)).collect::<Vec<u8>>())
        };
        let name = |k: u8| format!("tex_00000000000000{k:02x}_16x16");
        let mut index = String::new();
        let mut put = |k: u8, wrap: &str, map: Vec<u8>| {
            std::fs::write(dir.join(format!("{}.n.png", name(k))), map).unwrap();
            index += &format!("{}\tuses=1\twrap={wrap}\n", name(k));
        };
        // f: over-long x, y with blue unused; pointing inwards; all flat.
        put(1, "rr", rgb(16, 16, &|_, _| [255, 255, 0]));
        put(2, "rr", rgb(16, 16, &|x, _| [128 + x as u8, 128, 50]));
        put(3, "rr", rgb(16, 16, &|_, _| [128, 128, 255]));
        // h: leaning across a mirrored u edge; a seam on a repeated v axis
        // (green runs 100..160 down the rows, then jumps back).
        let lean = |x: usize, y: usize| {
            let v = 128 + (x as i32 - 8) * 6 + (y as i32 % 3);
            [v as u8, 128, 250]
        };
        put(4, "mr", rgb(16, 16, &lean));
        put(5, "rr", rgb(16, 16, &|x, y| [128 + (x % 2) as u8, (100 + 4 * y) as u8, 250]));
        // g: a cut-out (the image's transparent middle) whose map texels
        // lean hard while everything around is flat.
        put(6, "rr", rgb(16, 16, &|x, _| if (6..10).contains(&x) { [250, 128, 160] } else { [128, 128, 255] }));
        let hole: Vec<u8> =
            (0..256).flat_map(|i| [90, 90, 90, if (6..10).contains(&(i % 16)) { 0 } else { 255 }]).collect();
        std::fs::write(dir.join(format!("{}.png", name(6))), png::encode_rgba(16, 16, &hole)).unwrap();
        // Clean: a gentle relief that tiles, under the same cut-out but
        // filled from its neighbours.
        put(7, "rr", rgb(16, 16, &|x, y| [128 + ((x + y) % 4) as u8, 128, 254]));
        std::fs::write(dump.join("index.txt"), index).unwrap();
        std::fs::write(dir.join("pack.txt"), "format 2\nuses materials\n").unwrap();
        let r = lint_report(&dir, Some(&dump), 8192).unwrap();
        let has = |c: char, k: u8| {
            r.found.iter().any(|(key, v)| key.1 == c && v.iter().any(|i| i.starts_with(&format!("{}.n", name(k)))))
        };
        assert!(has('f', 1) && has('f', 2) && has('f', 3));
        assert!(has('h', 4) && has('h', 5), "{:?}", r.found);
        assert!(has('g', 6));
        assert!(!has('f', 7) && !has('g', 7) && !has('h', 7));
        assert!(!has('h', 1) && !has('h', 3) && !has('h', 6));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
