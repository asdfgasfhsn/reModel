//! `remodel studio sharing PROJECT`: who shares which texture, and whether the
//! pack draws each owner consistently (docs/uplift-studio.md, "Sharing and
//! owners"; docs/texture-packs.md, "Per-palette variants").
//!
//! A game that paints one texture per object through its colour table (Top
//! Skater's skaters) draws one `tex_` key through several palettes. This
//! reads every scene of the project (`sharing_collect`), groups the palettes
//! that are drawn together on the same object into **owners**, and compares
//! what the pack draws for each (key, palette) with the original seen
//! through that palette. The analysis here is plain data in, data out, so it
//! tests without a ROM.
//!
//! Tones are mean colours of the opaque texels, the pack's image against
//! the original through the palette. The look and the lighting multiply
//! both the same way, so they are not applied: a drift is the ratio of the
//! two, and the checks compare ratios inside one owner.

#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks)]

use crate::sharing_collect::{mean_tone, PartInfo, Sight, Tone};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// What the pack draws for a (key, palette).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Draw {
    /// An image for this key through this palette (`tex_KEY~PAL_..`).
    Variant,
    /// The key's plain image, drawn for every palette without a variant.
    Plain,
    /// Nothing: the original texture.
    Original,
}

impl Draw {
    pub fn name(self) -> &'static str {
        match self {
            Draw::Variant => "variant",
            Draw::Plain => "plain",
            Draw::Original => "original",
        }
    }
}

/// The pack's answer for one (key, palette).
#[derive(Clone, Copy, Debug)]
pub struct PackSide {
    pub draw: Draw,
    /// The mean colour of the image drawn; the original's through the palette when `Original`.
    pub tone: Tone,
}

/// Limits of the checks (the gate's threshold is `flags`, in `gates.txt`).
#[derive(Clone, Debug)]
pub struct Limits {
    /// A skin part's tone may differ from where the owner's other skin parts put it by this
    /// much (`tone_dev`, a colour difference: 6 is a visible one in skin).
    pub skin: f32,
    /// Every other part (clothes, hair, boards) may differ in brightness (L* of the tone) by
    /// this much from where the owner's other parts put it; their colour is left to the look.
    pub other: f32,
    /// Two palettes of a key that draw one plain image and that need tones this far apart
    /// (`tone_dev` for skin, brightness for the rest) need a variant each.
    pub variant: f32,
    /// A (key, palette) drawn by fewer pixels than this in an object is not counted in
    /// its owner's palettes.
    pub min_pixels: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { skin: 8.0, other: 15.0, variant: 10.0, min_pixels: 24 }
    }
}

/// An owner: palettes that are drawn together on the same objects.
#[derive(Clone, Debug)]
pub struct Owner {
    pub name: String,
    /// Named by `owners.txt` (a character), else clustered.
    pub named: bool,
    pub pals: BTreeSet<u32>,
}

/// The owners the user named (`owners.txt`: `NAME|pal,pal,...`, `#` comments).
pub fn parse_owners(text: &str) -> Result<Vec<Owner>, String> {
    let mut out: Vec<Owner> = Vec::new();
    for (n, l) in text.lines().enumerate() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') {
            continue;
        }
        let (name, pals) = l.split_once('|').ok_or(format!("owners.txt line {}: NAME|pal,pal,...", n + 1))?;
        let mut set = BTreeSet::new();
        for p in pals.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            set.insert(
                u32::from_str_radix(p, 16)
                    .map_err(|_| format!("owners.txt line {}: `{p}` is not a palette id", n + 1))?,
            );
        }
        if name.trim().is_empty() || out.iter().any(|o| o.name == name.trim()) {
            return Err(format!("owners.txt line {}: an owner needs its own name", n + 1));
        }
        out.push(Owner { name: name.trim().to_string(), named: true, pals: set });
    }
    Ok(out)
}

/// The owners, the owners of each run, and each run's palettes.
type Assigned = (Vec<Owner>, BTreeMap<Inst, Vec<usize>>, BTreeMap<Inst, BTreeSet<u32>>);

/// What owners are found among: a run of object blocks one scene draws in one 3D
/// window, numbered close together (a gap of at most `RUN_GAP` blocks). The game
/// draws a skater's parts as consecutive blocks of a window of their own, so a
/// run is one skater and his board; the course is a run (or a few) of its own.
type Inst = (String, u32);

/// The largest gap between two block numbers of one run.
pub const RUN_GAP: u32 = 3;

/// The run each sight belongs to: (scene, window << 20 | the run's first block).
fn instances(sights: &[Sight]) -> Vec<Inst> {
    let mut blocks: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
    for s in sights {
        blocks.entry(&s.scene).or_default().insert(s.block);
    }
    let mut start: HashMap<(&str, u32), u32> = HashMap::new();
    for (scene, set) in &blocks {
        let mut cur: Option<(u32, u32)> = None; // (window, first block), last block in `last`
        let mut last = 0u32;
        for &b in set {
            let (win, n) = (b >> 20, b & 0xf_ffff);
            match cur {
                Some((w, first)) if w == win && n <= last + RUN_GAP + 1 => {
                    start.insert((scene, b), win << 20 | first);
                }
                _ => {
                    cur = Some((win, n));
                    start.insert((scene, b), win << 20 | n);
                }
            }
            last = n;
        }
    }
    sights.iter().map(|s| (s.scene.clone(), start[&(s.scene.as_str(), s.block)])).collect()
}

/// Which owners each object (scene, block) belongs to, and the owners: the named
/// ones first, then the clusters made of the objects no named owner takes.
///
/// A named owner takes an object when at least half the palettes drawn on it are the
/// owner's (those of at least `min_pixels` pixels). An object two owners take equally
/// (a palette they share, alone) goes to both. The objects left over that show two
/// or more palettes are clustered: two are one cluster when their palette sets share
/// half of their union, and the cluster's palettes are all of its objects'.
pub fn assign(sights: &[Sight], named: &[Owner], min_pixels: u64) -> Assigned {
    let insts = instances(sights);
    let mut sig: BTreeMap<Inst, BTreeSet<u32>> = BTreeMap::new();
    for (s, inst) in sights.iter().zip(&insts) {
        let e = sig.entry(inst.clone()).or_default();
        if s.pixels >= min_pixels {
            e.insert(s.pal);
        }
    }
    let mut owners: Vec<Owner> = named.to_vec();
    let mut of: BTreeMap<Inst, Vec<usize>> = BTreeMap::new();
    let mut left: Vec<&Inst> = Vec::new();
    // The palettes that say whose a run is: the owner's own, not the ones many owners draw
    // (a board, shoes, and the palettes of whatever the clusters of the owners' scenes took from the course).
    let ident: Vec<BTreeSet<u32>> = named
        .iter()
        .map(|o| o.pals.iter().copied().filter(|p| named.iter().filter(|x| x.pals.contains(p)).count() <= 2).collect())
        .collect();
    for (inst, set) in &sig {
        let scores: Vec<f32> = named
            .iter()
            .enumerate()
            .map(|(i, o)| {
                // Half the run's palettes, and at least two (one when it has no more) of the owner's own.
                let own = set.iter().filter(|p| ident[i].contains(p)).count();
                if set.is_empty() || own < ident[i].len().min(2) || own == 0 {
                    0.0
                } else {
                    set.iter().filter(|p| o.pals.contains(p)).count() as f32 / set.len() as f32
                }
            })
            .collect();
        let best = scores.iter().cloned().fold(0.0f32, f32::max);
        if best >= 0.5 {
            of.insert(inst.clone(), (0..named.len()).filter(|&i| (scores[i] - best).abs() < 1e-6).collect());
        } else if set.len() >= 2 {
            left.push(inst);
        }
    }
    // Clusters of the rest, by distinct palette set.
    let mut sets: Vec<&BTreeSet<u32>> = left.iter().map(|i| &sig[*i]).collect();
    sets.sort();
    sets.dedup();
    let mut parent: Vec<usize> = (0..sets.len()).collect();
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    for i in 0..sets.len() {
        for j in i + 1..sets.len() {
            let inter = sets[i].intersection(sets[j]).count();
            let union = sets[i].len() + sets[j].len() - inter;
            if inter * 2 >= union {
                let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                if a != b {
                    parent[b] = a;
                }
            }
        }
    }
    let mut clusters: BTreeMap<usize, BTreeSet<u32>> = BTreeMap::new();
    for i in 0..sets.len() {
        let r = find(&mut parent, i);
        clusters.entry(r).or_default().extend(sets[i].iter().copied());
    }
    // Biggest first (pixels of the objects in it), so `auto-1` is the most drawn.
    let mut px: HashMap<Inst, u64> = HashMap::new();
    for (s, inst) in sights.iter().zip(&insts) {
        *px.entry(inst.clone()).or_insert(0) += s.pixels;
    }
    let set_index: HashMap<&BTreeSet<u32>, usize> = sets.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut weight: BTreeMap<usize, u64> = BTreeMap::new();
    for inst in &left {
        let r = find(&mut parent, set_index[&sig[*inst]]);
        *weight.entry(r).or_insert(0) += px.get(*inst).copied().unwrap_or(0);
    }
    let mut order: Vec<(u64, usize)> = weight.iter().map(|(r, w)| (*w, *r)).collect();
    order.sort_by(|a, b| b.cmp(a));
    let mut index: HashMap<usize, usize> = HashMap::new();
    for (n, (_, r)) in order.iter().enumerate() {
        index.insert(*r, owners.len());
        owners.push(Owner { name: format!("auto-{}", n + 1), named: false, pals: clusters[r].clone() });
    }
    for inst in &left {
        let r = find(&mut parent, set_index[&sig[*inst]]);
        of.insert((*inst).clone(), vec![index[&r]]);
    }
    (owners, of, sig)
}

/// Scene names without their trailing frame number, counted.
pub fn stems_of<'a>(names: impl Iterator<Item = &'a str>) -> Vec<(String, usize)> {
    let mut m: BTreeMap<String, BTreeSet<&str>> = BTreeMap::new();
    for n in names {
        let stem = n.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('-');
        m.entry(if stem.is_empty() { n.to_string() } else { stem.to_string() }).or_default().insert(n);
    }
    let mut v: Vec<(String, usize)> = m.into_iter().map(|(k, v)| (k, v.len())).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    v
}

/// CIE L*a*b* of an sRGB colour (0 to 255 a channel), D65.
pub fn lab(t: Tone) -> [f32; 3] {
    let lin = |c: f32| {
        let c = (c / 255.0).clamp(0.0, 1.0);
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(t[0]), lin(t[1]), lin(t[2]));
    let x = (0.4124 * r + 0.3576 * g + 0.1805 * b) / 0.95047;
    let y = 0.2126 * r + 0.7152 * g + 0.0722 * b;
    let z = (0.0193 * r + 0.1192 * g + 0.9505 * b) / 1.08883;
    let f = |v: f32| if v > 0.008856 { v.cbrt() } else { 7.787 * v + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

/// How far two tones are apart: the CIE76 colour difference of their L*a*b*
/// (about 2 is the least a person sees side by side, 8 a visible difference in
/// skin, 20 a different colour of the same kind).
pub fn tone_dev(a: Tone, b: Tone) -> f32 {
    let (x, y) = (lab(a), lab(b));
    ((x[0] - y[0]).powi(2) + (x[1] - y[1]).powi(2) + (x[2] - y[2]).powi(2)).sqrt()
}

/// Orange to pink, mid-light and not too saturated: the tones of skin (of the
/// original through the palette). A rule of thumb, not a classifier: it takes the
/// hue between red and yellow-orange, so blond hair and red trunks are not skin.
pub fn skin_like(t: Tone) -> bool {
    let (r, g, b) = (t[0], t[1], t[2]);
    let mx = r.max(g).max(b);
    if mx < 80.0 || r < g || g < b - 2.0 {
        return false;
    }
    let d = mx - r.min(g).min(b);
    let sat = d / mx;
    // Hue in degrees, 0 red to 60 yellow.
    let hue = if d > 0.0 { 60.0 * (g - b) / d } else { 0.0 };
    (0.10..=0.75).contains(&sat) && (8.0..=42.0).contains(&hue)
}

fn wmedian(mut v: Vec<(f32, f32)>) -> f32 {
    v.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    let total: f32 = v.iter().map(|x| x.1).sum();
    let mut acc = 0.0;
    for (x, w) in &v {
        acc += w;
        if acc * 2.0 >= total {
            return *x;
        }
    }
    v.last().map_or(0.0, |x| x.0)
}

/// What a (key, palette) is flagged for.
#[derive(Clone, Debug, PartialEq)]
pub enum Flag {
    /// Its tone is off its owner's by more than the limit.
    Outlier,
    /// A character's part the pack does not draw.
    Original,
    /// Draws the plain image while another palette of the key (this one) differs a lot.
    NeedsVariant(u32),
}

impl Flag {
    pub fn text(&self) -> String {
        match self {
            Flag::Outlier => "outlier".into(),
            Flag::Original => "original".into(),
            Flag::NeedsVariant(p) => format!("needs-variant:{p:08x}"),
        }
    }
}

/// One (owner, key, palette) of the report.
#[derive(Clone, Debug)]
pub struct Row {
    pub owner: String,
    pub character: bool,
    pub key: u64,
    pub pal: u32,
    pub w: u32,
    pub h: u32,
    pub scenes: usize,
    pub groups: usize,
    pub pixels: u64,
    /// Share of the key's pixels (over all palettes) this palette draws.
    pub share: f32,
    pub draw: Draw,
    pub skin: bool,
    pub orig: Tone,
    pub pack: Tone,
    /// Luma of the pack's tone over the original's.
    pub ratio: f32,
    /// How far the pack's tone is from where this owner's others put it (`tone_dev`).
    pub dev: f32,
    /// The tone this owner's other parts put the pack's image at (None when the owner has
    /// nothing to compare with): what `--fix` repaints to.
    pub target: Option<Tone>,
    pub flags: Vec<Flag>,
}

impl Row {
    pub fn name(&self) -> String {
        model2::enhance::variant_name(self.key, self.pal, self.w, self.h)
    }
}

/// An owner's summary.
#[derive(Clone, Debug)]
pub struct OwnerSummary {
    pub name: String,
    pub named: bool,
    pub pals: Vec<u32>,
    pub objects: usize,
    pub scenes: usize,
    /// The scenes it is in, as name stems with a count (the scene names with
    /// their trailing frame number cut off), most first.
    pub stems: Vec<(String, usize)>,
    pub pixels: u64,
    pub parts: usize,
    /// The owner's skin parts' typical pack tone over the original's, per channel.
    pub skin_ratio: Option<Tone>,
    pub flags: usize,
}

pub struct Report {
    pub owners: Vec<OwnerSummary>,
    pub rows: Vec<Row>,
}

impl Report {
    /// Flags on characters (named owners): what the gate counts.
    pub fn character_flags(&self) -> usize {
        self.rows.iter().filter(|r| r.character).map(|r| r.flags.len()).sum()
    }
    pub fn other_flags(&self) -> usize {
        self.rows.iter().filter(|r| !r.character).map(|r| r.flags.len()).sum()
    }
}

#[derive(Default)]
struct Agg {
    pixels: u64,
    scenes: BTreeSet<String>,
    groups: BTreeSet<(String, u32)>,
}

/// The whole analysis. `pack` answers for a (key, palette); `None` for a part it
/// has no tone for (then it counts as the original).
pub fn analyse(
    sights: &[Sight],
    parts: &BTreeMap<(u64, u32), PartInfo>,
    named: &[Owner],
    pack: &dyn Fn(&PartInfo) -> PackSide,
    lim: &Limits,
) -> Report {
    let (owners, of, _) = assign(sights, named, lim.min_pixels);
    let insts = instances(sights);
    // (owner index or usize::MAX for none) -> (key, pal) -> aggregate.
    let mut agg: BTreeMap<(usize, (u64, u32)), Agg> = BTreeMap::new();
    let mut key_px: HashMap<u64, u64> = HashMap::new();
    let mut key_pal_px: HashMap<(u64, u32), u64> = HashMap::new();
    for (s, inst) in sights.iter().zip(&insts) {
        *key_px.entry(s.key).or_insert(0) += s.pixels;
        *key_pal_px.entry((s.key, s.pal)).or_insert(0) += s.pixels;
        let owns: Vec<usize> = of.get(inst).cloned().unwrap_or_else(|| vec![usize::MAX]);
        for o in owns {
            let a = agg.entry((o, (s.key, s.pal))).or_default();
            a.pixels += s.pixels;
            a.scenes.insert(s.scene.clone());
            a.groups.insert((s.scene.clone(), s.block));
        }
    }
    // Palettes of each key, for the plain-draws-where-palettes-differ check.
    let mut pals_of: BTreeMap<u64, Vec<u32>> = BTreeMap::new();
    for &(k, p) in key_pal_px.keys() {
        pals_of.entry(k).or_default().push(p);
    }
    for v in pals_of.values_mut() {
        v.sort();
    }
    let mut rows: Vec<Row> = Vec::new();
    for ((o, (key, pal)), a) in &agg {
        let Some(info) = parts.get(&(*key, *pal)) else { continue };
        let side = pack(info);
        let (owner, character) =
            if *o == usize::MAX { ("-".to_string(), false) } else { (owners[*o].name.clone(), owners[*o].named) };
        let lum = |t: Tone| 0.299 * t[0] + 0.587 * t[1] + 0.114 * t[2];
        rows.push(Row {
            owner,
            character,
            key: *key,
            pal: *pal,
            w: info.w,
            h: info.h,
            scenes: a.scenes.len(),
            groups: a.groups.len(),
            pixels: a.pixels,
            share: a.pixels as f32 / key_px[key].max(1) as f32,
            draw: side.draw,
            skin: skin_like(info.orig) && (side.draw == Draw::Original || skin_like(side.tone)),
            orig: info.orig,
            pack: side.tone,
            ratio: (lum(side.tone) + 1.0) / (lum(info.orig) + 1.0),
            dev: 0.0,
            target: None,
            flags: Vec::new(),
        });
    }
    // Per owner: the typical ratio of its skin parts and of its others, then the deviations.
    let mut summaries: Vec<OwnerSummary> = Vec::new();
    for (oi, ow) in owners.iter().enumerate().map(|(i, o)| (Some(i), Some(o))).chain([(None, None)]) {
        let name = ow.map_or("-".to_string(), |o| o.name.clone());
        let idx: Vec<usize> = (0..rows.len()).filter(|&i| rows[i].owner == name).collect();
        if idx.is_empty() && oi.is_some() {
            continue;
        }
        let ratio_of = |r: &Row| -> [f32; 3] {
            [
                (r.pack[0] + 8.0) / (r.orig[0] + 8.0),
                (r.pack[1] + 8.0) / (r.orig[1] + 8.0),
                (r.pack[2] + 8.0) / (r.orig[2] + 8.0),
            ]
        };
        let rows_ro = &rows;
        // The weighted median ratio, per channel, of the parts `sel` picks (pack-drawn ones only).
        let typical = |sel: &dyn Fn(usize, &Row) -> bool, scalar: bool| -> Option<[f32; 3]> {
            // Near-black originals have no ratio worth the name (a dark navy for black is 30 against 0).
            let lit = |r: &Row| 0.299 * r.orig[0] + 0.587 * r.orig[1] + 0.114 * r.orig[2] >= 40.0;
            let pick = |dark: bool| -> Vec<usize> {
                idx.iter()
                    .copied()
                    .filter(|&i| rows_ro[i].draw != Draw::Original && sel(i, &rows_ro[i]) && (dark || lit(&rows_ro[i])))
                    .collect()
            };
            let mut set = pick(false);
            if set.is_empty() {
                set = pick(true);
            }
            if set.is_empty() {
                return None;
            }
            let mut m = [0.0; 3];
            for (c, mc) in m.iter_mut().enumerate() {
                // Per channel for skin parts (they are alike); one brightness for the rest, whose
                // colours differ part to part.
                let of = |i: usize| if scalar { rows_ro[i].ratio } else { ratio_of(&rows_ro[i])[c] };
                *mc = wmedian(
                    set.iter().map(|&i| (of(i).ln(), (1.0 + (1.0 + rows_ro[i].pixels as f32).log10()))).collect(),
                )
                .exp();
            }
            Some(m)
        };
        let skin = typical(&|_, r| r.skin, false);
        let n_skin = idx.iter().filter(|&&i| rows_ro[i].skin && rows_ro[i].draw != Draw::Original).count();
        let n_other = idx.iter().filter(|&&i| !rows_ro[i].skin && rows_ro[i].draw != Draw::Original).count();
        // (row, deviation, outlier, target) of each part, from the others.
        let mut devs: Vec<(usize, f32, bool, Tone)> = Vec::new();
        for &i in &idx {
            // Where this owner's others put it: the other skin parts for a skin part (two or
            // more of them), the other parts that are not skin for the rest (two or more),
            // else all the others.
            let me = rows_ro[i].skin;
            let pack_drawn = rows_ro[i].draw != Draw::Original;
            let n_same = (if me { n_skin } else { n_other }) - pack_drawn as usize;
            let limit = if me { lim.skin } else { lim.other };
            let expect =
                if n_same >= 2 { typical(&|j, r| j != i && r.skin == me, !me) } else { typical(&|j, _| j != i, true) };
            if let Some(g) = expect {
                let r = &rows_ro[i];
                let at = |c: usize| ((r.orig[c] + 8.0) * g[c] - 8.0).clamp(0.0, 255.0);
                let exp: Tone = [at(0), at(1), at(2)];
                // A skin part is held to the colour; any other part to the brightness only (a comic
                // look recolours clothes on purpose, and a jacket in another hue at the same
                // lightness is not what makes a skater read as two).
                let d = match (pack_drawn, me) {
                    (false, _) => 0.0,
                    (true, true) => tone_dev(r.pack, exp),
                    (true, false) => (lab(r.pack)[0] - lab(exp)[0]).abs(),
                };
                // What to repaint to: a skin part to the expected colour, another part to its own
                // hue at the expected brightness.
                let lum = |t: Tone| 0.299 * t[0] + 0.587 * t[1] + 0.114 * t[2];
                let target = if me || !pack_drawn {
                    exp
                } else {
                    let k = (lum(exp) + 1.0) / (lum(r.pack) + 1.0);
                    [r.pack[0] * k, r.pack[1] * k, r.pack[2] * k]
                };
                devs.push((i, d, pack_drawn && d > limit, target));
            }
        }
        for (i, d, out, exp) in devs {
            rows[i].dev = d;
            rows[i].target = Some(exp);
            if out {
                rows[i].flags.push(Flag::Outlier);
            }
        }
        if ow.is_some_and(|o| o.named) {
            for &i in &idx {
                if rows[i].draw == Draw::Original {
                    rows[i].flags.push(Flag::Original);
                }
            }
        }
        let flags: usize = idx.iter().map(|&i| rows[i].flags.len()).sum();
        let insts: BTreeSet<&Inst> = of
            .iter()
            .filter(|(_, v)| match oi {
                Some(i) => v.contains(&i),
                None => false,
            })
            .map(|(k, _)| k)
            .collect();
        summaries.push(OwnerSummary {
            name,
            named: ow.is_some_and(|o| o.named),
            pals: ow.map_or(Vec::new(), |o| o.pals.iter().copied().collect()),
            objects: insts.len(),
            scenes: insts.iter().map(|i| &i.0).collect::<BTreeSet<_>>().len(),
            stems: stems_of(insts.iter().map(|i| i.0.as_str())),
            pixels: idx.iter().map(|&i| rows[i].pixels).sum(),
            parts: idx.len(),
            skin_ratio: skin,
            flags,
        });
    }
    // A plain image that serves palettes that need different tones: another palette of the key
    // that draws the same plain image (no variant of its own) and whose tone should be far
    // from this one's (what the owner's others put it at; the original's where it has no owner).
    // Skin is held to the colour, the rest to the brightness, as `outlier` is.
    let draw_of: HashMap<(u64, u32), Draw> = rows.iter().map(|r| ((r.key, r.pal), r.draw)).collect();
    let mut should: HashMap<(u64, u32), (u64, Tone, bool)> = HashMap::new();
    for r in &rows {
        let t = r.target.unwrap_or(r.orig);
        let e = should.entry((r.key, r.pal)).or_insert((0, t, r.skin));
        if r.target.is_some() && r.pixels >= e.0 {
            *e = (r.pixels, t, r.skin);
        }
    }
    for r in rows.iter_mut() {
        if r.draw != Draw::Plain {
            continue;
        }
        let me = should[&(r.key, r.pal)];
        let sharing: Vec<(f32, u32)> = pals_of[&r.key]
            .iter()
            .filter(|&&q| q != r.pal && draw_of.get(&(r.key, q)) == Some(&Draw::Plain))
            .filter_map(|&q| should.get(&(r.key, q)).map(|o| (q, o)))
            .map(|(q, o)| {
                let d = if me.2 && o.2 { tone_dev(me.1, o.1) } else { (lab(me.1)[0] - lab(o.1)[0]).abs() };
                (d, q)
            })
            .filter(|(d, _)| *d > lim.variant)
            .collect();
        if let Some(&(_, q)) = sharing.iter().max_by(|a, b| a.0.partial_cmp(&b.0).unwrap()) {
            r.flags.push(Flag::NeedsVariant(q));
        }
    }
    for s in summaries.iter_mut() {
        s.flags = rows.iter().filter(|r| r.owner == s.name).map(|r| r.flags.len()).sum();
    }
    rows.sort_by(|a, b| {
        (!a.character, &a.owner, std::cmp::Reverse(a.pixels), a.key, a.pal).cmp(&(
            !b.character,
            &b.owner,
            std::cmp::Reverse(b.pixels),
            b.key,
            b.pal,
        ))
    });
    summaries.sort_by_key(|a| (!a.named, std::cmp::Reverse(a.pixels)));
    Report { owners: summaries, rows }
}

// ------------------------------------------------------------------ the pack

/// The files of a pack by key (and palette): `tex_KEY_WxH.png` and `tex_KEY~PAL_WxH.png`.
pub struct PackIndex {
    files: HashMap<(u64, Option<u32>), PathBuf>,
    cache: std::sync::Mutex<HashMap<PathBuf, Option<Tone>>>,
}

impl PackIndex {
    pub fn load(dir: &Path) -> PackIndex {
        let mut files = HashMap::new();
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            let Some(stem) = n.strip_suffix(".png").and_then(|s| s.strip_prefix("tex_")) else { continue };
            let Some((id, size)) = stem.rsplit_once('_') else { continue };
            let sized = size.split_once('x').is_some_and(|(a, b)| {
                !a.is_empty()
                    && !b.is_empty()
                    && a.bytes().all(|c| c.is_ascii_digit())
                    && b.bytes().all(|c| c.is_ascii_digit())
            });
            if !sized {
                continue;
            }
            let (key, pal) = match id.split_once('~') {
                Some((k, p)) => (u64::from_str_radix(k, 16), u32::from_str_radix(p, 16).map(Some)),
                None => (u64::from_str_radix(id, 16), Ok(None)),
            };
            if let (Ok(k), Ok(p)) = (key, pal) {
                files.insert((k, p), e.path());
            }
        }
        PackIndex { files, cache: Default::default() }
    }

    fn tone(&self, path: &Path) -> Option<Tone> {
        let mut c = self.cache.lock().unwrap();
        *c.entry(path.to_path_buf()).or_insert_with(|| {
            let im = crate::studio_img::load(path).ok()?;
            let (t, share) = mean_tone(&im.rgba);
            (share > 0.0).then_some(t)
        })
    }

    pub fn file(&self, key: u64, pal: u32) -> Option<(Draw, &PathBuf)> {
        self.files
            .get(&(key, Some(pal)))
            .map(|p| (Draw::Variant, p))
            .or_else(|| self.files.get(&(key, None)).map(|p| (Draw::Plain, p)))
    }

    pub fn side(&self, info: &PartInfo) -> PackSide {
        match self.file(info.key, info.pal) {
            Some((draw, p)) => match self.tone(p) {
                Some(tone) => PackSide { draw, tone },
                None => PackSide { draw: Draw::Original, tone: info.orig },
            },
            None => PackSide { draw: Draw::Original, tone: info.orig },
        }
    }
}

// ------------------------------------------------------------------ output

fn rgb(t: Tone) -> String {
    format!("{:.0}/{:.0}/{:.0}", t[0], t[1], t[2])
}

pub fn rows_tsv(r: &Report) -> String {
    let mut s = String::from(
        "owner\tcharacter\tkey\tpal\tsize\tscenes\tgroups\tpixels\tshare\tdraw\tkind\torig\tpack\tratio\tdev\tflags\n",
    );
    for x in &r.rows {
        let _ = writeln!(
            s,
            "{}\t{}\t{:016x}\t{:08x}\t{}x{}\t{}\t{}\t{}\t{:.3}\t{}\t{}\t{}\t{}\t{:.2}\t{:.3}\t{}",
            x.owner,
            if x.character { "yes" } else { "no" },
            x.key,
            x.pal,
            x.w,
            x.h,
            x.scenes,
            x.groups,
            x.pixels,
            x.share,
            x.draw.name(),
            if x.skin { "skin" } else { "other" },
            rgb(x.orig),
            rgb(x.pack),
            x.ratio,
            x.dev,
            x.flags.iter().map(Flag::text).collect::<Vec<_>>().join(",")
        );
    }
    s
}

pub fn owners_text(r: &Report) -> String {
    let mut s = String::new();
    let _ = writeln!(
        s,
        "{:<12}{:>8}{:>8}{:>11}{:>7}{:>7}  scenes; palettes",
        "owner", "objects", "scenes", "pixels", "parts", "flags"
    );
    for o in &r.owners {
        let _ = writeln!(
            s,
            "{:<12}{:>8}{:>8}{:>11}{:>7}{:>7}  {} | {}{}",
            o.name,
            o.objects,
            o.scenes,
            o.pixels,
            o.parts,
            o.flags,
            o.stems.iter().take(4).map(|(n, c)| format!("{n} x{c}")).collect::<Vec<_>>().join(" "),
            o.pals.iter().take(14).map(|p| format!("{p:08x}")).collect::<Vec<_>>().join(","),
            if o.pals.len() > 14 { format!(",+{}", o.pals.len() - 14) } else { String::new() }
        );
    }
    s
}

/// A contact sheet of one owner's parts: the original through the palette on the
/// left, what the pack draws on the right, a label under each pair.
pub fn contact_sheet(rows: &[&Row], store: &Path, pack: &PackIndex, cell: usize) -> Option<png::Image> {
    use crate::studio_img::Image;
    if rows.is_empty() {
        return None;
    }
    let cols = 4usize.min(rows.len());
    let (pad, label_h) = (6usize, 18usize);
    let (cw, ch) = (cell * 2 + pad * 3, cell + label_h + pad * 2);
    let nrows = rows.len().div_ceil(cols);
    let (w, h) = (cols * cw, nrows * ch);
    let mut im = Image { width: w, height: h, rgba: vec![0; w * h * 4] };
    for px in im.rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&[28, 28, 34, 255]);
    }
    let put = |im: &mut Image, src: &Image, x0: usize, y0: usize| {
        let k = (cell / src.width.max(src.height).max(1)).max(1);
        let (tw, th) = if src.width.max(src.height) * k <= cell {
            (src.width * k, src.height * k)
        } else {
            let m = src.width.max(src.height);
            ((src.width * cell / m).max(1), (src.height * cell / m).max(1))
        };
        for y in 0..th {
            for x in 0..tw {
                let (sx, sy) = (x * src.width / tw, y * src.height / th);
                let p = &src.rgba[(sy * src.width + sx) * 4..][..4];
                let checker = if (x / 4 + y / 4) % 2 == 0 { 70u32 } else { 100 };
                let d = ((y0 + y) * im.width + x0 + x) * 4;
                for c in 0..3 {
                    im.rgba[d + c] = ((p[c] as u32 * p[3] as u32 + checker * (255 - p[3] as u32)) / 255) as u8;
                }
            }
        }
    };
    for (i, r) in rows.iter().enumerate() {
        let (x0, y0) = ((i % cols) * cw + pad, (i / cols) * ch + pad);
        let name = r.name();
        if let Ok(o) = crate::studio_img::load(&store.join("orig").join(format!("{name}.png"))) {
            put(&mut im, &o, x0, y0);
            let drawn = pack.file(r.key, r.pal).and_then(|(_, p)| crate::studio_img::load(p).ok());
            put(&mut im, drawn.as_ref().unwrap_or(&o), x0 + cell + pad, y0);
        }
        let colour = if r.flags.is_empty() { [200, 200, 200] } else { [255, 120, 100] };
        let label = format!(
            "{} {} {} {:.2}",
            &format!("{:016x}", r.key)[..8],
            &format!("{:08x}", r.pal)[..4],
            &r.draw.name()[..1],
            r.dev
        );
        crate::img_cmd::draw_text(&mut im, &label, x0, y0 + cell + 3, 2, cell * 2 + pad, colour);
    }
    Some(im)
}

/// `img` repainted so the mean colour of its opaque texels is `target`: each channel
/// scaled by the ratio (at most 5 and at least 0.2 times), clipped at 255, alpha kept.
pub fn repaint(img: &png::Image, target: Tone) -> png::Image {
    let (mean, _) = mean_tone(&img.rgba);
    let gain: Vec<f32> = (0..3).map(|c| (target[c].max(1.0) / mean[c].max(1.0)).clamp(0.2, 5.0)).collect();
    let mut out = img.clone();
    for p in out.rgba.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = (p[c] as f32 * gain[c]).round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sight(scene: &str, block: u32, key: u64, pal: u32, pixels: u64) -> Sight {
        Sight { scene: scene.into(), block: (3 << 20) | block, key, pal, pixels }
    }

    fn part(key: u64, pal: u32, orig: Tone) -> ((u64, u32), PartInfo) {
        ((key, pal), PartInfo { key, pal, w: 8, h: 8, surface: 0, orig, opaque: 1.0, ramp: String::new() })
    }

    // Two skaters: A (palettes 1 skin, 2 shirt, 9 shared board) and B (palettes 3 skin,
    // 4 shirt, 9 shared board), the key 100 drawn on both (a hand, 1 and 3), and a course
    // (window 2, palettes 7 and 8, with the key 300 drawn through 7 and 8 as it fades).
    fn world() -> (Vec<Sight>, BTreeMap<(u64, u32), PartInfo>) {
        let mut s = Vec::new();
        for sc in ["a-1", "a-2", "a-3"] {
            s.push(sight(sc, 10, 100, 1, 900));
            s.push(sight(sc, 11, 200, 2, 800));
            s.push(sight(sc, 12, 100, 1, 300));
            s.push(sight(sc, 14, 500, 9, 400)); // the board, in the skater's run
            s.push(Sight { scene: sc.into(), block: (2 << 20) | 4, key: 300, pal: 7, pixels: 5000 });
            s.push(Sight { scene: sc.into(), block: (2 << 20) | 5, key: 301, pal: 8, pixels: 4000 });
        }
        for sc in ["b-1", "b-2"] {
            s.push(sight(sc, 10, 100, 3, 900));
            s.push(sight(sc, 11, 400, 4, 800));
            s.push(sight(sc, 14, 500, 9, 400));
            s.push(Sight { scene: sc.into(), block: (2 << 20) | 4, key: 300, pal: 8, pixels: 5000 });
        }
        let parts: BTreeMap<_, _> = [
            part(100, 1, [240.0, 180.0, 160.0]),
            part(100, 3, [170.0, 125.0, 95.0]),
            part(200, 2, [60.0, 90.0, 200.0]),
            part(400, 4, [200.0, 40.0, 40.0]),
            part(500, 9, [80.0, 80.0, 80.0]),
            part(300, 7, [100.0, 150.0, 60.0]),
            part(300, 8, [60.0, 90.0, 40.0]),
            part(301, 8, [90.0, 90.0, 90.0]),
        ]
        .into_iter()
        .collect();
        (s, parts)
    }

    fn scale(t: Tone, k: f32) -> Tone {
        [t[0] * k, t[1] * k, t[2] * k]
    }

    #[test]
    fn owners_are_the_palettes_drawn_together_and_a_shared_palette_joins_both() {
        let (sights, _) = world();
        let (owners, of, _) = assign(&sights, &[], 24);
        // The two skaters, and the course. The board palette 9 is in both skaters.
        let skaters: Vec<&Owner> = owners.iter().filter(|o| o.pals.contains(&9)).collect();
        assert_eq!(skaters.len(), 2, "{owners:?}");
        let a = skaters.iter().find(|o| o.pals.contains(&1)).unwrap();
        let b = skaters.iter().find(|o| o.pals.contains(&3)).unwrap();
        assert_eq!(a.pals, BTreeSet::from([1, 2, 9]));
        assert_eq!(b.pals, BTreeSet::from([3, 4, 9]));
        assert!(!a.named && a.name.starts_with("auto-"));
        // The course is its own owner and has neither skater's palettes.
        let course = owners.iter().find(|o| o.pals.contains(&7)).unwrap();
        assert!(!course.pals.contains(&1) && !course.pals.contains(&9));
        // Each scene's skater run, and the course where it shows two palettes (a-1 to a-3).
        assert_eq!(of.len(), 5 + 3, "{of:?}");
    }

    #[test]
    fn a_named_owner_takes_the_runs_most_of_whose_palettes_it_has() {
        let (sights, _) = world();
        let named = parse_owners("# the skaters\nAda|1,2,9\nBo | 3,4,9\n").unwrap();
        assert_eq!(named.len(), 2);
        let (owners, of, _) = assign(&sights, &named, 24);
        let ada = of.get(&("a-1".to_string(), 3 << 20 | 10)).unwrap();
        let bo = of.get(&("b-1".to_string(), 3 << 20 | 10)).unwrap();
        assert_eq!((ada.clone(), bo.clone()), (vec![0], vec![1]), "the shared board palette does not tie them");
        assert!(owners[0].named && owners[1].named);
        assert_eq!(owners.len(), 3, "the course is clustered: {owners:?}");
        assert!(parse_owners("Ada").is_err());
        assert!(parse_owners("Ada|zz").is_err());
        assert!(parse_owners("Ada|1\nAda|2").is_err());
    }

    #[test]
    fn flags_an_outlier_in_an_owner_a_plain_where_palettes_differ_and_an_original() {
        let (sights, mut parts) = world();
        // A third key on A: the skin of key 150, whose pack image will drift.
        let mut sights = sights;
        for sc in ["a-1", "a-2", "a-3"] {
            sights.push(sight(sc, 13, 150, 1, 600));
            sights.push(sight(sc, 13, 160, 2, 600));
        }
        parts.extend([part(150, 1, [240.0, 180.0, 160.0]), part(160, 2, [60.0, 90.0, 200.0])]);
        let named = parse_owners("Ada|1,2,9\nBo|3,4,9\n").unwrap();
        // The pack draws 90% of every original, but key 150 at 60%, key 500 not at all
        // (original) and the plain image of key 100 for both palettes.
        let pack = |i: &PartInfo| -> PackSide {
            match i.key {
                150 => PackSide { draw: Draw::Variant, tone: scale(i.orig, 0.6) },
                500 => PackSide { draw: Draw::Original, tone: i.orig },
                100 => PackSide { draw: Draw::Plain, tone: scale(parts[&(100, 1)].orig, 0.9) },
                _ => PackSide { draw: Draw::Plain, tone: scale(i.orig, 0.9) },
            }
        };
        let r = analyse(&sights, &parts, &named, &pack, &Limits::default());
        let flags = |owner: &str, key: u64, pal: u32| -> Vec<Flag> {
            r.rows.iter().find(|x| x.owner == owner && x.key == key && x.pal == pal).unwrap().flags.clone()
        };
        assert_eq!(flags("Ada", 150, 1), vec![Flag::Outlier], "a skin part off by 0.6 against 0.9");
        assert!(flags("Ada", 200, 2).is_empty(), "the shirt is at the owner's typical ratio");
        assert_eq!(flags("Ada", 500, 9), vec![Flag::Original], "a character's part the pack does not draw");
        // Key 100 draws plain tone of Ada's skin on Bo's (darker) palette too.
        assert!(flags("Bo", 100, 3).contains(&Flag::NeedsVariant(1)), "{:?}", flags("Bo", 100, 3));
        assert!(flags("Bo", 100, 3).contains(&Flag::Outlier), "the plain image is far off Bo's own palette");
        // Key 300 fades through two palettes on the course: no variant, no flag on a non-character.
        let course: Vec<&Row> = r.rows.iter().filter(|x| x.key == 300).collect();
        assert!(course.iter().all(|x| !x.character));
        assert!(
            course.iter().any(|x| x.flags.iter().any(|f| matches!(f, Flag::NeedsVariant(_)))),
            "listed, not a character's"
        );
        assert_eq!(r.character_flags(), r.rows.iter().filter(|x| x.character).map(|x| x.flags.len()).sum::<usize>());
        // The summary: Ada has the flags counted.
        let ada = r.owners.iter().find(|o| o.name == "Ada").unwrap();
        assert_eq!(ada.flags, 3, "the outlier, the original board and the plain hand that needs a variant");
        assert!(ada.named && ada.skin_ratio.is_some());
        // A variant for each palette cures Key 100.
        let fixed = |i: &PartInfo| -> PackSide {
            if i.key == 100 {
                PackSide { draw: Draw::Variant, tone: scale(i.orig, 0.9) }
            } else {
                pack(i)
            }
        };
        let r2 = analyse(&sights, &parts, &named, &fixed, &Limits::default());
        assert!(r2.rows.iter().filter(|x| x.key == 100).all(|x| x.flags.is_empty()));
    }

    #[test]
    fn the_tsv_and_the_owner_table_say_what_was_found() {
        let (sights, parts) = world();
        let named = parse_owners("Ada|1,2,9\n").unwrap();
        let r = analyse(&sights, &parts, &named, &|i| PackSide { draw: Draw::Plain, tone: i.orig }, &Limits::default());
        let t = rows_tsv(&r);
        assert!(t.starts_with("owner\tcharacter\tkey\tpal\t"));
        assert!(t.lines().any(|l| l.starts_with("Ada\tyes\t0000000000000064\t00000001\t8x8\t3\t")), "{t}");
        let o = owners_text(&r);
        assert!(o.contains("Ada") && o.contains("a x3"), "{o}");
        assert_eq!(stems_of(["a-1", "a-2", "b-10"].into_iter()), vec![("a".to_string(), 2), ("b".to_string(), 1)]);
    }

    #[test]
    fn skin_is_a_warm_mid_tone_and_tone_distance_is_relative() {
        assert!(skin_like([230.0, 170.0, 140.0]) && skin_like([150.0, 90.0, 50.0]));
        assert!(!skin_like([60.0, 90.0, 200.0]) && !skin_like([240.0, 240.0, 240.0]) && !skin_like([40.0, 30.0, 20.0]));
        assert!(tone_dev([100.0; 3], [100.0; 3]) < 1e-3);
        assert!(tone_dev([100.0; 3], [200.0; 3]) > 30.0);
        // Ten per cent of a skin tone is a visible difference, and a navy for black is a different colour.
        assert!(tone_dev([230.0, 170.0, 140.0], [207.0, 153.0, 126.0]) > 6.0);
        assert!(tone_dev([30.0, 18.0, 64.0], [0.0; 3]) > 18.0);
        assert!(!skin_like([245.0, 245.0, 127.0]), "blond hair is yellow, not skin");
    }

    #[test]
    fn a_pack_is_read_by_key_and_palette() {
        let d = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/scratch/sharing-tests"))
            .join(format!("pack-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let img = |c: [u8; 4]| png::encode_rgba(2, 2, &c.repeat(4));
        std::fs::write(d.join("tex_0000000000000064_8x8.png"), img([100, 50, 0, 255])).unwrap();
        std::fs::write(d.join("tex_0000000000000064~00000003_8x8.png"), img([200, 100, 0, 255])).unwrap();
        std::fs::write(d.join("tex_0000000000000064~00000003_8x8n.png"), img([0, 0, 255, 255])).unwrap();
        std::fs::write(d.join("tile_0123456789abcdef.png"), img([1, 2, 3, 255])).unwrap();
        let idx = PackIndex::load(&d);
        let info =
            |pal| PartInfo { key: 100, pal, w: 8, h: 8, surface: 0, orig: [9.0; 3], opaque: 1.0, ramp: String::new() };
        let v = idx.side(&info(3));
        assert_eq!((v.draw, v.tone), (Draw::Variant, [200.0, 100.0, 0.0]));
        let p = idx.side(&info(1));
        assert_eq!((p.draw, p.tone), (Draw::Plain, [100.0, 50.0, 0.0]));
        let o = idx.side(&PartInfo { key: 7, ..info(1) });
        assert_eq!((o.draw, o.tone), (Draw::Original, [9.0; 3]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn repainting_moves_the_mean_to_the_target_and_keeps_the_cutout() {
        let mut rgba = [100u8, 50, 20, 255].repeat(3);
        rgba.extend([9, 9, 9, 0]);
        let im = png::Image { width: 2, height: 2, rgba };
        let out = repaint(&im, [150.0, 100.0, 40.0]);
        let (mean, share) = mean_tone(&out.rgba);
        assert!(
            (mean[0] - 150.0).abs() < 1.0 && (mean[1] - 100.0).abs() < 1.0 && (mean[2] - 40.0).abs() < 1.0,
            "{mean:?}"
        );
        assert!((share - 0.75).abs() < 1e-6, "the transparent texel stays transparent");
        assert_eq!(out.rgba[15], 0);
        // A target far out is limited to 5 times, and a white cannot pass 255.
        let hot = repaint(&im, [255.0, 255.0, 255.0]);
        assert!(hot.rgba[0] == 255 && hot.rgba[2] == 100);
    }
}
