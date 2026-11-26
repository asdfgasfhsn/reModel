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
    /// The hues of the image drawn (a chroma-weighted histogram of its opaque texels, 24 bins of 15
    /// degrees, summing to 1), None where it has none (a grey part, or the original): what a garment's
    /// parts are held to beside their mean tone, which a mix of hues in one image can leave looking like one colour.
    pub hue: Option<Hues>,
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
    /// A part marked skin or hair (a fixed part: `NAME.skin|keys` in owners.txt) may differ from
    /// the original through its owner's palette by this colour difference (`tone_dev`).
    pub fixed: f32,
    /// A garment's parts may differ in lightness (L*) by this much from the garment's typical part.
    pub garment_light: f32,
    /// ... and in hue by this many degrees (where both are coloured enough to have one).
    pub garment_hue: f32,
    /// Scenes of the select screen, by a part of their name: what the garment check compares
    /// the other scenes (the races) with. See `parse_select`.
    pub select: Vec<String>,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            skin: 8.0,
            other: 15.0,
            variant: 10.0,
            min_pixels: 24,
            fixed: 9.0,
            garment_light: 15.0,
            garment_hue: 30.0,
            select: vec!["select".into(), "-sel-".into()],
        }
    }
}

/// An owner: palettes that are drawn together on the same objects.
#[derive(Clone, Debug)]
pub struct Owner {
    pub name: String,
    /// Named by `owners.txt` (a character), else clustered.
    pub named: bool,
    pub pals: BTreeSet<u32>,
    /// Groups of texture keys the author named (`NAME.GROUP|key,key,...`): `skin` and `hair`
    /// are fixed parts (held to the original), any other name is a garment (one design).
    pub groups: Vec<Group>,
}

/// A named set of texture keys of one owner.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    /// Keys as lowercase hex prefixes: 8 digits name a texture whatever its colour-table
    /// address, 16 digits one key.
    pub keys: Vec<String>,
    /// A garment's design tone (`NAME.garment=RRGGBB|keys`): the mean colour its parts are held to
    /// and `--fix` repaints to; without it, the garment's typical part sets it.
    pub design: Option<Tone>,
}

impl Group {
    /// Skin, face, hands, neck and hair: parts that keep the original's colour.
    pub fn fixed(&self) -> bool {
        self.name == "skin" || self.name == "hair"
    }
    pub fn has(&self, key: u64) -> bool {
        let k = format!("{key:016x}");
        self.keys.iter().any(|p| k.starts_with(p.as_str()))
    }
}

impl Owner {
    /// Whether the author marked this owner's parts (then fixed parts are held to the original).
    pub fn marked(&self) -> bool {
        self.groups.iter().any(Group::fixed)
    }
}

/// The scene-name patterns of the select screen: a line `@select|text,text` of owners.txt.
pub fn parse_select(text: &str) -> Option<Vec<String>> {
    text.lines()
        .map(str::trim)
        .find_map(|l| l.strip_prefix("@select|"))
        .map(|v| v.split(',').map(str::trim).filter(|p| !p.is_empty()).map(str::to_string).collect())
}

/// The owners the user named (`owners.txt`: `NAME|pal,pal,...`, `#` comments), and
/// their groups of keys (`NAME.GROUP|key,key,...`, after the owner's line; `@select|..`
/// lines are read by `parse_select`).
pub fn parse_owners(text: &str) -> Result<Vec<Owner>, String> {
    let mut out: Vec<Owner> = Vec::new();
    for (n, l) in text.lines().enumerate() {
        let l = l.trim();
        if l.is_empty() || l.starts_with('#') || l.starts_with('@') {
            continue;
        }
        let (name, pals) = l.split_once('|').ok_or(format!("owners.txt line {}: NAME|pal,pal,...", n + 1))?;
        if let Some((owner, group)) = name.trim().split_once('.') {
            let (group, design) = match group.split_once('=') {
                Some((g, c)) => {
                    let v = u32::from_str_radix(c.trim().trim_start_matches('#'), 16)
                        .ok()
                        .filter(|_| c.trim().trim_start_matches('#').len() == 6)
                        .ok_or(format!("owners.txt line {}: `{c}` is not a colour (RRGGBB)", n + 1))?;
                    (g, Some([(v >> 16 & 255) as f32, (v >> 8 & 255) as f32, (v & 255) as f32]))
                }
                None => (group, None),
            };
            let keys: Vec<String> =
                pals.split(',').map(|k| k.trim().to_lowercase()).filter(|k| !k.is_empty()).collect();
            if let Some(bad) =
                keys.iter().find(|k| k.len() < 8 || k.len() > 16 || !k.bytes().all(|c| c.is_ascii_hexdigit()))
            {
                return Err(format!("owners.txt line {}: `{bad}` is not a texture key (8 to 16 hex digits)", n + 1));
            }
            let o = out
                .iter_mut()
                .find(|o| o.name == owner)
                .ok_or(format!("owners.txt line {}: owner `{owner}` is named on an earlier line", n + 1))?;
            match o.groups.iter_mut().find(|g| g.name == group) {
                Some(g) => {
                    g.keys.extend(keys);
                    g.design = g.design.or(design);
                }
                None => o.groups.push(Group { name: group.to_string(), keys, design }),
            }
            continue;
        }
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
        out.push(Owner { name: name.trim().to_string(), named: true, pals: set, groups: Vec::new() });
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
        owners.push(Owner {
            name: format!("auto-{}", n + 1),
            named: false,
            pals: clusters[r].clone(),
            groups: Vec::new(),
        });
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
    /// A fixed part (skin, face, hands, neck, hair) whose colour is off the original through
    /// its owner's palette by more than `Limits::fixed`.
    SkinOff,
    /// A part of a garment that differs in hue or lightness from the garment's typical part.
    Garment(String),
    /// A garment drawn in a different colour on the select screen than in the races.
    GarmentSelect(String),
}

impl Flag {
    pub fn text(&self) -> String {
        match self {
            Flag::Outlier => "outlier".into(),
            Flag::Original => "original".into(),
            Flag::SkinOff => "skin-off".into(),
            Flag::Garment(g) => format!("garment:{g}"),
            Flag::GarmentSelect(g) => format!("garment-select:{g}"),
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
    /// The hues of the image the pack draws (`PackSide::hue`).
    pub hue: Option<Hues>,
    /// Luma of the pack's tone over the original's.
    pub ratio: f32,
    /// How far the pack's tone is from where this owner's others put it (`tone_dev`).
    pub dev: f32,
    /// The tone this owner's other parts put the pack's image at (None when the owner has
    /// nothing to compare with): what `--fix` repaints to.
    pub target: Option<Tone>,
    /// Marked by hand as skin or hair (owners.txt): held to the original, `target` is the original.
    pub fixed: bool,
    /// The garment (owners.txt group) it belongs to, whose parts must look alike: `target` is the
    /// garment's typical tone.
    pub group: Option<String>,
    /// Its garment has a design tone in owners.txt (`NAME.garment=RRGGBB`): `--recipe` rebuilds every part of it.
    pub designed: bool,
    pub flags: Vec<Flag>,
}

impl Row {
    /// `skin` (found by colour), `fixed` (marked skin or hair), `garment:NAME` or `other`.
    pub fn kind(&self) -> String {
        match (&self.group, self.fixed, self.skin) {
            (Some(g), _, _) => format!("garment:{g}"),
            (_, true, _) => "fixed".into(),
            (_, _, true) => "skin".into(),
            _ => "other".into(),
        }
    }
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
            hue: side.hue,
            ratio: (lum(side.tone) + 1.0) / (lum(info.orig) + 1.0),
            dev: 0.0,
            target: None,
            fixed: false,
            group: None,
            designed: false,
            flags: Vec::new(),
        });
    }
    // The author's marks: skin and hair parts (held to the original), garments (one design).
    for r in rows.iter_mut() {
        let Some(o) = named.iter().find(|o| o.name == r.owner) else { continue };
        if o.marked() {
            r.fixed = o.groups.iter().any(|g| g.fixed() && g.has(r.key));
            r.skin = r.fixed;
        }
        if !r.fixed {
            r.group = o.groups.iter().find(|g| !g.fixed() && g.has(r.key)).map(|g| g.name.clone());
        }
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
            if rows_ro[i].fixed || rows_ro[i].group.is_some() {
                continue;
            }
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
        // Marked skin and hair: the original through the palette is the target, whatever the others do.
        for &i in &idx {
            if rows[i].fixed {
                let d = if rows[i].draw == Draw::Original { 0.0 } else { tone_dev(rows[i].pack, rows[i].orig) };
                rows[i].dev = d;
                rows[i].target = Some(rows[i].orig);
                if d > lim.fixed {
                    rows[i].flags.push(Flag::SkinOff);
                }
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
    garments(&mut rows, sights, &insts, &of, &owners, named.len(), lim);
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

/// Lightness and hue differences of two tones: (|dL*|, degrees of hue). The hue is 0 where either
/// is too grey to have one, and 180 where one is coloured and the other grey.
pub fn light_hue_diff(a: Tone, b: Tone) -> (f32, f32) {
    let (x, y) = (lab(a), lab(b));
    let (ca, cb) = (x[1].hypot(x[2]), y[1].hypot(y[2]));
    let hue = if ca >= 10.0 && cb >= 10.0 {
        let d = (x[2].atan2(x[1]) - y[2].atan2(y[1])).to_degrees().abs() % 360.0;
        d.min(360.0 - d)
    } else if ca.max(cb) >= 25.0 && ca.min(cb) < 10.0 {
        180.0
    } else {
        0.0
    };
    ((x[0] - y[0]).abs(), hue)
}

/// The hue (degrees) of a tone, 0 for a grey.
#[cfg(test)]
pub fn hue_of(t: Tone) -> f32 {
    let l = lab(t);
    l[2].atan2(l[1]).to_degrees().rem_euclid(360.0)
}

/// Whether the select screen draws a scene (by a part of its name).
fn is_select(scene: &str, patterns: &[String]) -> bool {
    patterns.iter().any(|p| scene.contains(p.as_str()))
}

/// The garment check: for each garment of each named owner, every part (key and palette) is held
/// to the garment's typical part (the member the others are nearest to, by pixels) in hue and
/// lightness, and the garment as the select screen draws it to the garment as the races do.
fn garments(
    rows: &mut [Row],
    sights: &[Sight],
    insts: &[Inst],
    of: &BTreeMap<Inst, Vec<usize>>,
    owners: &[Owner],
    named: usize,
    lim: &Limits,
) {
    let weight = |r: &Row| 1.0 + (1.0 + r.pixels as f32).log10();
    for (oi, o) in owners.iter().enumerate().take(named) {
        for g in o.groups.iter().filter(|g| !g.fixed()) {
            let members: Vec<usize> = (0..rows.len())
                .filter(|&i| rows[i].owner == o.name && rows[i].group.as_deref() == Some(&g.name))
                .collect();
            if members.is_empty() {
                continue;
            }
            let best = *members
                .iter()
                .min_by(|&&a, &&b| {
                    let cost = |i: usize| -> f32 {
                        members.iter().map(|&j| weight(&rows[j]) * tone_dev(rows[i].pack, rows[j].pack)).sum()
                    };
                    cost(a).partial_cmp(&cost(b)).unwrap()
                })
                .unwrap();
            let typical = g.design.unwrap_or(rows[best].pack);
            // The hues the garment shows: its parts' histograms pooled by pixels.
            let mut pooled = [0f32; 24];
            for &i in &members {
                if let Some(h) = &rows[i].hue {
                    let w = weight(&rows[i]);
                    for k in 0..24 {
                        pooled[k] += h[k] * w;
                    }
                }
            }
            for &i in &members {
                let (dl, dh) = light_hue_diff(rows[i].pack, typical);
                // A part that is one colour on average but a mix of others in its drawing (magenta, ochre
                // and green blocks that average to brown) shares few hues with the rest of its garment.
                let shares = match &rows[i].hue {
                    Some(a) => {
                        // against the others': the pool without this part
                        let w = weight(&rows[i]);
                        let mut rest = pooled;
                        for k in 0..24 {
                            rest[k] = (rest[k] - a[k] * w).max(0.0);
                        }
                        let tot: f32 = rest.iter().sum();
                        if tot > 1e-3 {
                            hue_overlap(a, &rest.map(|v| v / tot))
                        } else {
                            1.0
                        }
                    }
                    None => 1.0,
                };
                rows[i].dev = tone_dev(rows[i].pack, typical);
                rows[i].target = Some(typical);
                rows[i].designed = g.design.is_some();
                if rows[i].draw != Draw::Original && (dl > lim.garment_light || dh > lim.garment_hue || shares < 0.3) {
                    rows[i].flags.push(Flag::Garment(g.name.clone()));
                }
            }
            // As the select screen draws it and as the races do: the pixel-weighted mean tone of each.
            let tone_of: HashMap<(u64, u32), Tone> =
                members.iter().map(|&i| ((rows[i].key, rows[i].pal), rows[i].pack)).collect();
            let row_of: HashMap<(u64, u32), usize> = members.iter().map(|&i| ((rows[i].key, rows[i].pal), i)).collect();
            let mut sum = [[0f64; 3]; 2];
            let mut px = [0f64; 2];
            let mut top: HashMap<usize, u64> = HashMap::new();
            for (s, inst) in sights.iter().zip(insts) {
                if !of.get(inst).is_some_and(|v| v.contains(&oi)) || !g.has(s.key) {
                    continue;
                }
                let Some(t) = tone_of.get(&(s.key, s.pal)) else { continue };
                let c = usize::from(!is_select(&s.scene, &lim.select));
                for k in 0..3 {
                    sum[c][k] += t[k] as f64 * s.pixels as f64;
                }
                px[c] += s.pixels as f64;
                if c == 0 {
                    *top.entry(row_of[&(s.key, s.pal)]).or_insert(0) += s.pixels;
                }
            }
            if px[0] > 0.0 && px[1] > 0.0 {
                let mean = |c: usize| -> Tone { [0, 1, 2].map(|k| (sum[c][k] / px[c]) as f32) };
                let (dl, dh) = light_hue_diff(mean(0), mean(1));
                if dl > lim.garment_light || dh > lim.garment_hue {
                    if let Some((&i, _)) = top.iter().max_by_key(|(i, p)| (**p, std::cmp::Reverse(**i))) {
                        rows[i].flags.push(Flag::GarmentSelect(g.name.clone()));
                    }
                }
            }
        }
    }
}

// ------------------------------------------------------------------ the pack

/// A histogram of hues: 24 bins of 15 degrees from 0, summing to 1.
pub type Hues = [f32; 24];

/// The hues of the texels with alpha of at least 128 and chroma of at least 18 (L*a*b*), weighted by
/// chroma. None when fewer than 5% of the texels are coloured.
pub fn hue_histogram(rgba: &[u8]) -> Option<Hues> {
    let mut h = [0f32; 24];
    let (mut n, mut coloured) = (0usize, 0usize);
    for p in rgba.chunks_exact(4) {
        if p[3] < 128 {
            continue;
        }
        n += 1;
        let l = lab([p[0] as f32, p[1] as f32, p[2] as f32]);
        let c = l[1].hypot(l[2]);
        if c >= 18.0 {
            coloured += 1;
            let a = l[2].atan2(l[1]).to_degrees().rem_euclid(360.0);
            h[((a / 15.0) as usize).min(23)] += c;
        }
    }
    let total: f32 = h.iter().sum();
    if n == 0 || (coloured as f32) < 0.05 * n as f32 || total <= 0.0 {
        return None;
    }
    Some(h.map(|v| v / total))
}

/// How much two hue histograms share (0 to 1): the overlap of their histograms once each is blurred over the
/// neighbouring bins (a hue 15 degrees off is the same colour). Stripes of red, navy and cream share
/// with any garment of those colours; magenta, ochre and green blocks do not.
pub fn hue_overlap(a: &Hues, b: &Hues) -> f32 {
    let blur = |h: &Hues| -> Hues {
        let mut o = [0f32; 24];
        for i in 0..24 {
            o[i] = 0.25 * h[(i + 23) % 24] + 0.5 * h[i] + 0.25 * h[(i + 1) % 24];
        }
        o
    };
    let (x, y) = (blur(a), blur(b));
    (0..24).map(|i| x[i].min(y[i])).sum()
}

/// A pack image's mean tone and hues.
type ToneHues = (Tone, Option<Hues>);

/// The files of a pack by key (and palette): `tex_KEY_WxH.png` and `tex_KEY~PAL_WxH.png`.
pub struct PackIndex {
    files: HashMap<(u64, Option<u32>), PathBuf>,
    cache: std::sync::Mutex<HashMap<PathBuf, Option<ToneHues>>>,
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

    fn tone(&self, path: &Path) -> Option<ToneHues> {
        let mut c = self.cache.lock().unwrap();
        *c.entry(path.to_path_buf()).or_insert_with(|| {
            let im = crate::studio_img::load(path).ok()?;
            let (t, share) = mean_tone(&im.rgba);
            (share > 0.0).then_some((t, hue_histogram(&im.rgba)))
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
                Some((tone, hue)) => PackSide { draw, tone, hue },
                None => PackSide { draw: Draw::Original, tone: info.orig, hue: None },
            },
            None => PackSide { draw: Draw::Original, tone: info.orig, hue: None },
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
            x.kind(),
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

/// A starting point for the groups of owners.txt, from the original's colours: per character, the
/// parts whose original looks like skin as `NAME.skin|keys`, the rest clustered by the original's
/// hue and lightness as `NAME.auto-HUE-LIGHT|keys` (a hue in degrees, or grey), each with
/// its pixels in a comment. Keys are 8-digit texture prefixes. The author renames the garments, splits
/// the clusters that are two garments and moves hair into `NAME.hair`; the tool cannot know.
pub fn suggest(rows: &[Row], min_pixels: u64) -> String {
    let mut s = String::new();
    let owners: BTreeSet<&str> = rows.iter().filter(|r| r.character).map(|r| r.owner.as_str()).collect();
    for o in owners {
        let mut groups: BTreeMap<String, (u64, BTreeSet<String>)> = BTreeMap::new();
        for r in rows.iter().filter(|r| r.owner == o && r.pixels >= min_pixels) {
            let l = lab(r.orig);
            let c = l[1].hypot(l[2]);
            let band = if l[0] < 30.0 {
                "dark"
            } else if l[0] < 65.0 {
                "mid"
            } else {
                "light"
            };
            let name = if skin_like(r.orig) {
                "skin".to_string()
            } else if c < 10.0 {
                format!("auto-grey-{band}")
            } else {
                let h = l[2].atan2(l[1]).to_degrees().rem_euclid(360.0);
                format!("auto-{:03}-{band}", (h / 30.0).floor() as u32 * 30)
            };
            let e = groups.entry(name).or_default();
            e.0 += r.pixels;
            e.1.insert(format!("{:016x}", r.key)[..8].to_string());
        }
        let mut v: Vec<_> = groups.into_iter().collect();
        v.sort_by_key(|a| (a.0 != "skin", std::cmp::Reverse(a.1 .0)));
        for (name, (px, keys)) in v {
            let _ = writeln!(s, "# {px} pixels");
            let _ = writeln!(s, "{o}.{name}|{}", keys.into_iter().collect::<Vec<_>>().join(","));
        }
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

fn unlab(l: [f32; 3]) -> Tone {
    let fy = (l[0] + 16.0) / 116.0;
    let (fx, fz) = (fy + l[1] / 500.0, fy - l[2] / 200.0);
    let inv = |f: f32| if f.powi(3) > 0.008856 { f.powi(3) } else { (f - 16.0 / 116.0) / 7.787 };
    let (x, y, z) = (inv(fx) * 0.95047, inv(fy), inv(fz) * 1.08883);
    let r = 3.2406 * x - 1.5372 * y - 0.4986 * z;
    let g = -0.9689 * x + 1.8758 * y + 0.0415 * z;
    let b = 0.0557 * x - 0.2040 * y + 1.0570 * z;
    let enc = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        255.0 * if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
    };
    [enc(r), enc(g), enc(b)]
}

/// `img` recoloured so the mean colour of its opaque texels is `target`, keeping its drawing: the
/// lightness of every texel follows a curve that keeps black and white where they are, its hue turns
/// by the hue difference of the two means and its chroma scales by their ratio (so a dark ink line
/// stays an ink line, and a grey stays grey). Alpha is kept. A few rounds correct what the clipping
/// takes off the mean.
pub fn recolour(img: &png::Image, target: Tone) -> png::Image {
    let mut want = target;
    let mut best = img.clone();
    for _ in 0..6 {
        let (mean, _) = mean_tone(&img.rgba);
        let (m, t) = (lab(mean), lab(want));
        let (cm, ct) = (m[1].hypot(m[2]).max(1.0), t[1].hypot(t[2]));
        let dh = t[2].atan2(t[1]) - m[2].atan2(m[1]);
        // A grey mean has no hue to turn: add the target's colour instead.
        let grey = cm < 4.0;
        let k = if grey { 1.0 } else { ct / cm };
        let g = (t[0].clamp(1.0, 99.0) / 100.0).ln() / (m[0].clamp(1.0, 99.0) / 100.0).ln();
        let (s, c) = dh.sin_cos();
        let mut out = img.clone();
        for p in out.rgba.chunks_exact_mut(4) {
            let l = lab([p[0] as f32, p[1] as f32, p[2] as f32]);
            let l2 = 100.0 * (l[0].clamp(0.0, 100.0) / 100.0).powf(g);
            let (a, b) = if grey {
                let w = (l2 / 100.0).min(1.0 - l2 / 100.0) * 2.0 + 0.1;
                (t[1] * w.min(1.0), t[2] * w.min(1.0))
            } else {
                ((l[1] * c - l[2] * s) * k, (l[1] * s + l[2] * c) * k)
            };
            let rgb = unlab([l2, a, b]);
            for i in 0..3 {
                p[i] = rgb[i].round().clamp(0.0, 255.0) as u8;
            }
        }
        let (got, _) = mean_tone(&out.rgba);
        best = out;
        let err = tone_dev(got, target);
        if err < 1.0 {
            break;
        }
        for i in 0..3 {
            want[i] = (want[i] + (target[i] - got[i])).clamp(0.0, 255.0);
        }
    }
    best
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
                150 => PackSide { draw: Draw::Variant, tone: scale(i.orig, 0.6), hue: None },
                500 => PackSide { draw: Draw::Original, tone: i.orig, hue: None },
                100 => PackSide { draw: Draw::Plain, tone: scale(parts[&(100, 1)].orig, 0.9), hue: None },
                _ => PackSide { draw: Draw::Plain, tone: scale(i.orig, 0.9), hue: None },
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
                PackSide { draw: Draw::Variant, tone: scale(i.orig, 0.9), hue: None }
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
        let r = analyse(
            &sights,
            &parts,
            &named,
            &|i| PackSide { draw: Draw::Plain, tone: i.orig, hue: None },
            &Limits::default(),
        );
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

    type World = (Vec<Sight>, BTreeMap<(u64, u32), PartInfo>, Vec<Owner>);

    // A: skin keys 100 (hand) and 150, shirt = a garment of keys 200 (body) and 210 (sleeve), drawn in
    // the select scene "a-select" and the race "a-1".
    fn marked_world() -> World {
        let mut s = Vec::new();
        for sc in ["a-select", "a-1", "a-2"] {
            s.push(sight(sc, 10, 100, 1, 900));
            s.push(sight(sc, 11, 200, 2, 800));
            s.push(sight(sc, 12, 210, 2, 400));
            s.push(sight(sc, 13, 150, 1, 600));
            s.push(sight(sc, 14, 500, 9, 400));
        }
        let parts: BTreeMap<_, _> = [
            part(100, 1, [240.0, 180.0, 160.0]),
            part(150, 1, [240.0, 180.0, 160.0]),
            part(200, 2, [60.0, 90.0, 200.0]),
            part(210, 2, [60.0, 90.0, 200.0]),
            part(500, 9, [80.0, 80.0, 80.0]),
        ]
        .into_iter()
        .collect();
        let named =
            parse_owners("Ada|1,2,9\nAda.skin|00000000 , 0000000000000096\nAda.shirt|00000000000000c8,000000d2\n");
        (s, parts, named.unwrap_or_default())
    }

    #[test]
    fn groups_are_read_from_owners_txt() {
        let o = parse_owners("Ada|1,2\nAda.skin|0000006400000000\nAda.hoodie|00000064,0000009600000000\nAda.hoodie|000000c8\n@select|x\n").unwrap();
        assert_eq!(o.len(), 1);
        assert_eq!(o[0].groups.len(), 2);
        assert!(o[0].groups[0].fixed() && o[0].groups[0].has(0x64_0000_0000) && !o[0].groups[0].has(0x65_0000_0000));
        assert!(!o[0].groups[1].fixed() && o[0].groups[1].keys.len() == 3 && o[0].marked());
        assert!(parse_owners("Bo.skin|00000000\n").is_err(), "the owner is named first");
        assert!(parse_owners("Ada|1\nAda.skin|zz\n").is_err());
        assert!(parse_owners("Ada|1\nAda.skin|1234\n").is_err(), "a key is at least 8 digits");
        assert_eq!(parse_select("Ada|1\n@select|sel, -s-\n"), Some(vec!["sel".to_string(), "-s-".to_string()]));
    }

    #[test]
    fn marked_skin_is_held_to_the_original_not_to_the_owners_others() {
        let (sights, parts, _) = marked_world();
        let named = parse_owners("Ada|1,2,9\nAda.skin|0000000000000064,0000000000000096\n").unwrap();
        // Every part is drawn at 0.8 of the original: consistent, but the skin is 20% darker than the original.
        let pack = |i: &PartInfo| PackSide { draw: Draw::Variant, tone: scale(i.orig, 0.8), hue: None };
        let r = analyse(&sights, &parts, &named, &pack, &Limits::default());
        let skin = r.rows.iter().find(|x| x.key == 100).unwrap();
        assert!(skin.fixed && skin.flags.contains(&Flag::SkinOff), "{:?} dev {}", skin.flags, skin.dev);
        assert_eq!(skin.target, Some(skin.orig));
        // At the original through the palette it is clean, however dark the rest of the owner is.
        let pack2 = |i: &PartInfo| PackSide {
            draw: Draw::Variant,
            hue: None,
            tone: if i.key == 100 || i.key == 150 { i.orig } else { scale(i.orig, 0.8) },
        };
        let r2 = analyse(&sights, &parts, &named, &pack2, &Limits::default());
        assert!(
            r2.rows.iter().filter(|x| x.fixed).all(|x| x.flags.is_empty()),
            "{:?}",
            r2.rows.iter().map(|x| (x.key, x.flags.clone())).collect::<Vec<_>>()
        );
        // A bit warmer than the original, a hue shift well within brightness: still off.
        let pack3 = |i: &PartInfo| PackSide {
            draw: Draw::Variant,
            hue: None,
            tone: if i.key == 100 { [i.orig[0], i.orig[1] * 0.8, i.orig[2] * 0.6] } else { i.orig },
        };
        let r3 = analyse(&sights, &parts, &named, &pack3, &Limits::default());
        assert!(r3.rows.iter().find(|x| x.key == 100).unwrap().flags.contains(&Flag::SkinOff));
    }

    #[test]
    fn a_garment_is_one_design_across_its_keys_and_between_select_and_race() {
        let (sights, parts, _) = marked_world();
        let named = parse_owners(
            "Ada|1,2,9\nAda.skin|0000000000000064,0000000000000096\nAda.shirt|00000000000000c8,00000000000000d2\n",
        )
        .unwrap();
        let blue = [50.0, 80.0, 190.0];
        let orange = [230.0, 130.0, 40.0];
        // Body blue, sleeve orange: two designs.
        let pack = |i: &PartInfo| PackSide {
            draw: Draw::Variant,
            hue: None,
            tone: match i.key {
                210 => orange,
                200 => blue,
                _ => i.orig,
            },
        };
        let r = analyse(&sights, &parts, &named, &pack, &Limits::default());
        let flags = |k: u64| r.rows.iter().find(|x| x.key == k).unwrap().flags.clone();
        let (a, b) = (flags(200), flags(210));
        assert!(a.iter().chain(&b).any(|f| matches!(f, Flag::Garment(g) if g == "shirt")), "{a:?} {b:?}");
        assert!(r.rows.iter().find(|x| x.key == 210).unwrap().group.as_deref() == Some("shirt"));
        // Both blue: clean (a slightly different blue at the same lightness and hue is one design).
        let pack2 = |i: &PartInfo| PackSide {
            draw: Draw::Variant,
            hue: None,
            tone: match i.key {
                210 => [55.0, 85.0, 185.0],
                200 => blue,
                _ => i.orig,
            },
        };
        let r2 = analyse(&sights, &parts, &named, &pack2, &Limits::default());
        assert!(
            r2.rows.iter().all(|x| x.flags.is_empty()),
            "{:?}",
            r2.rows.iter().map(|x| (x.key, x.flags.clone())).collect::<Vec<_>>()
        );
        // The select screen draws the shirt one colour (key 200 only there) and the races another.
        let mut sights3 = sights.clone();
        sights3.retain(|x| !(x.key == 200 && x.scene != "a-select") && !(x.key == 210 && x.scene == "a-select"));
        for sc in ["a-1", "a-2"] {
            sights3.push(sight(sc, 11, 210, 2, 800));
        }
        let pack3 = |i: &PartInfo| PackSide {
            draw: Draw::Variant,
            hue: None,
            tone: match i.key {
                200 => blue,
                210 => orange,
                _ => i.orig,
            },
        };
        let named3 = parse_owners("Ada|1,2,9\nAda.shirt|00000000000000c8,00000000000000d2\n").unwrap();
        let r3 = analyse(&sights3, &parts, &named3, &pack3, &Limits::default());
        assert!(
            r3.rows.iter().any(|x| x.flags.contains(&Flag::GarmentSelect("shirt".into()))),
            "{:?}",
            r3.rows.iter().map(|x| (x.key, x.flags.clone())).collect::<Vec<_>>()
        );
    }

    #[test]
    fn light_and_hue_differences_and_the_suggested_groups() {
        let (dl, dh) = light_hue_diff([50.0, 80.0, 190.0], [230.0, 130.0, 40.0]);
        assert!(dl > 10.0 && dh > 90.0, "{dl} {dh}");
        let (dl, dh) = light_hue_diff([50.0, 80.0, 190.0], [55.0, 85.0, 185.0]);
        assert!(dl < 5.0 && dh < 10.0, "{dl} {dh}");
        assert_eq!(light_hue_diff([50.0, 80.0, 190.0], [100.0, 100.0, 100.0]).1, 180.0, "coloured against grey");
        let (sights, parts, _) = marked_world();
        let named = parse_owners("Ada|1,2,9\n").unwrap();
        let r = analyse(
            &sights,
            &parts,
            &named,
            &|i| PackSide { draw: Draw::Plain, tone: i.orig, hue: None },
            &Limits::default(),
        );
        let s = suggest(&r.rows, 100);
        assert!(s.contains("Ada.skin|00000000"), "{s}");
        assert!(s.contains("Ada.auto-"), "{s}");
        // What it writes reads back.
        assert!(parse_owners(&format!("Ada|1,2,9\n{s}")).is_ok(), "{s}");
    }

    #[test]
    fn recolouring_turns_the_hue_keeps_the_drawing_and_lands_on_the_target() {
        // A blue jacket with a dark fold line and a transparent corner.
        let mut rgba = Vec::new();
        for i in 0..16 {
            rgba.extend(if i % 4 == 0 { [10u8, 10, 40, 255] } else { [60, 90, 200, 255] });
        }
        rgba.extend([0, 0, 0, 0]);
        let im = png::Image { width: 17, height: 1, rgba };
        let target = [220.0, 120.0, 40.0];
        let out = recolour(&im, target);
        let (mean, _) = mean_tone(&out.rgba);
        assert!(tone_dev(mean, target) < 4.0, "{mean:?}");
        // The fold line is still the darkest and still dark; the cut-out is kept.
        assert!(out.rgba[0] < 130 && out.rgba[4] > 150, "{:?}", &out.rgba[..8]);
        assert_eq!(out.rgba[67], 0);
        let (dl, dh) = light_hue_diff([out.rgba[4] as f32, out.rgba[5] as f32, out.rgba[6] as f32], target);
        assert!(dh < 20.0 && dl < 25.0, "{dl} {dh}");
        // A garment with a design tone is held to it: a part at another hue is flagged by it.
        let o = parse_owners("Ada|1,2\nAda.skin|00000000\nAda.shirt=C8641E|000000c8\n").unwrap();
        assert_eq!(o[0].groups[1].design, Some([200.0, 100.0, 30.0]));
        assert!(parse_owners("Ada|1\nAda.shirt=zz|00000000\n").is_err());
    }

    fn hist_at(deg: f32) -> Hues {
        let mut h = [0f32; 24];
        h[(deg / 15.0) as usize % 24] = 1.0;
        h
    }

    #[test]
    fn a_part_that_averages_to_the_garments_colour_but_draws_another_hue_is_flagged() {
        let (sights, parts, _) = marked_world();
        let named = parse_owners("Ada|1,2,9\nAda.shirt|00000000000000c8,00000000000000d2\n").unwrap();
        let grey_brown = [150.0, 110.0, 90.0];
        let pack = |hues: [f32; 2]| {
            move |i: &PartInfo| PackSide {
                draw: Draw::Variant,
                tone: if i.key == 200 || i.key == 210 { grey_brown } else { i.orig },
                hue: match i.key {
                    200 => Some(hist_at(hues[0])),
                    210 => Some(hist_at(hues[1])),
                    _ => None,
                },
            }
        };
        // The same mean tone and the same dominant hue: one design.
        let ok = analyse(&sights, &parts, &named, &pack([40.0, 45.0]), &Limits::default());
        assert!(
            ok.rows.iter().all(|x| !x.flags.iter().any(|f| matches!(f, Flag::Garment(_)))),
            "{:?}",
            ok.rows.iter().map(|x| x.flags.clone()).collect::<Vec<_>>()
        );
        // The sleeve is magenta, ochre and green blocks that average to the same brown: another design.
        let bad = analyse(&sights, &parts, &named, &pack([40.0, 320.0]), &Limits::default());
        assert!(bad.rows.iter().any(|x| x.flags.iter().any(|f| matches!(f, Flag::Garment(_)))));
    }

    #[test]
    fn hue_histograms_say_which_colours_an_image_shows_and_how_much_two_share() {
        let mut rgba = Vec::new();
        for _ in 0..8 {
            rgba.extend([200u8, 100, 20, 255]);
        }
        rgba.extend([20u8, 150, 60, 255, 128, 128, 128, 255, 0, 0, 0, 0]);
        let h = hue_histogram(&rgba).unwrap();
        assert!((h.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        let top = (0..24).max_by(|&a, &b| h[a].partial_cmp(&h[b]).unwrap()).unwrap();
        assert!((2..=5).contains(&top), "orange is about 40 to 60 degrees in L*a*b*: bin {top}");
        assert_eq!(hue_histogram(&[128, 128, 128, 255].repeat(8)), None, "a grey has no hue");
        assert!(hue_overlap(&h, &h) > 0.99);
        assert!(hue_overlap(&hist_at(40.0), &hist_at(50.0)) >= 0.45, "neighbouring bins are one colour");
        assert!(hue_overlap(&hist_at(40.0), &hist_at(280.0)) < 0.05);
        assert!(hue_of([200.0, 100.0, 20.0]) > 40.0);
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
