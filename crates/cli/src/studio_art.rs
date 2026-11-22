//! `studio artworks` and `studio bake`: a pack replaces an artwork, not one
//! colouring of it (docs/uplift-studio.md, "Artworks"). The identity and the
//! baking arithmetic are `artwork.rs`; this is the project side.
//!
//! The project's `pack/` holds an artwork's one image as `art_<surface>_
//! <w>x<h>.png`. `bake` makes the colourings the catalogue has seen:
//!
//! - `baked` (the default) writes a `tex_` image for each variant, from the
//!   authored one through the colour ramps, or a copy where the image is a
//!   restyle (the ramp cannot predict one) or the colourings are alike;
//! - `fixed` writes nothing: readers draw the `art_` image, the same for
//!   every colouring, the game's own colour tinting nothing of it.
//!
//! What it wrote is remembered in `status/baked.tsv` (each file's hash), so
//! a rebuild replaces its own copies and never a `tex_` image made by hand;
//! `status/artworks.tsv` holds each artwork's mode, reference, distance and
//! band. `studio pack` reads both to say `uses` or `needs artworks`.

use super::*;
use crate::artwork::{self, Artwork, How, Row};

const MODES: [&str; 2] = ["baked", "fixed"];

/// A project's artworks, from its maps, biggest first.
pub fn artworks(p: &Project) -> Result<Vec<Artwork>, String> {
    let inv = p.inventory()?;
    let rows: Vec<Row> = inv
        .iter()
        .filter(|i| i.kind == "tex")
        .map(|i| Row { name: &i.name, surface: &i.surface, pixels: i.pixels, detail: &i.detail })
        .collect();
    Ok(artwork::group(&rows))
}

/// One artwork's line of `status/artworks.tsv`.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Record {
    pub art: String,
    pub mode: String,
    pub reference: String,
    pub distance: String,
    pub band: String,
}

impl Project {
    pub(super) fn art_records(&self) -> BTreeMap<String, Record> {
        let mut m = BTreeMap::new();
        for l in std::fs::read_to_string(self.p("status").join("artworks.tsv")).unwrap_or_default().lines() {
            let f: Vec<&str> = l.split('\t').collect();
            if f.len() >= 5 {
                m.insert(
                    f[0].to_string(),
                    Record {
                        art: f[0].into(),
                        mode: f[1].into(),
                        reference: f[2].into(),
                        distance: f[3].into(),
                        band: f[4].into(),
                    },
                );
            }
        }
        m
    }

    fn write_art_records(&self, m: &BTreeMap<String, Record>) -> Result<(), String> {
        let mut s = String::new();
        for r in m.values() {
            let _ = writeln!(s, "{}\t{}\t{}\t{}\t{}\t{}", r.art, r.mode, r.reference, r.distance, r.band, today());
        }
        std::fs::create_dir_all(self.p("status")).map_err(|e| e.to_string())?;
        std::fs::write(self.p("status").join("artworks.tsv"), s).map_err(|e| e.to_string())
    }

    /// Whether `pack/` has an artwork image that its colourings are drawn
    /// from at run time: an `art_` file with no baked copies of every
    /// colouring (`fixed`, or not baked yet).
    pub(super) fn needs_artworks(&self) -> bool {
        let rec = self.art_records();
        png_names(&self.p("pack"))
            .iter()
            .filter(|n| n.starts_with("art_"))
            .any(|n| rec.get(n).is_none_or(|r| r.mode != "baked"))
    }
}

fn share(px: u64, total: u64) -> f64 {
    if total == 0 {
        0.0
    } else {
        px as f64 * 100.0 / total as f64
    }
}

/// `studio artworks PROJECT [--tsv] [--min-variants N]`.
pub(super) fn list(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let arts = artworks(p)?;
    let min: usize = a.num("--min-variants")?.unwrap_or(1);
    let total: u64 = arts.iter().map(Artwork::pixels).sum();
    let rec = p.art_records();
    let pack = png_names(&p.p("pack"));
    println!("art\tsize\tvariants\tpixels\tshare_pct\tcolours\twith_ramp\tpaint_over\timage\tmode\treference\tdistance\tband");
    let (mut shown, mut multi, mut multi_px, mut have) = (0, 0, 0u64, 0);
    for aw in &arts {
        let n = aw.variants.len();
        if n > 1 {
            multi += 1;
            multi_px += aw.pixels();
        }
        let stem = aw.stem();
        let has = pack.contains(&stem);
        have += has as usize;
        if n < min {
            continue;
        }
        shown += 1;
        let r = rec.get(&stem);
        println!(
            "{stem}\t{}x{}\t{n}\t{}\t{:.2}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            aw.w,
            aw.h,
            aw.pixels(),
            share(aw.pixels(), total),
            aw.distinct(),
            aw.variants.iter().filter(|v| v.ramp.is_some()).count(),
            aw.richest().map_or("-", |v| v.name.as_str()),
            if has { "art" } else { "-" },
            r.map_or(if has { "fixed" } else { "-" }, |r| r.mode.as_str()),
            r.map_or("-", |r| r.reference.as_str()),
            r.map_or("-", |r| r.distance.as_str()),
            r.map_or("-", |r| r.band.as_str()),
        );
    }
    if !a.has("--tsv") {
        println!(
            "artworks      {} ({shown} listed): {} keys, {multi} artworks under more than one key carry {:.0}% of the pixels; {have} have an art_ image in pack/",
            arts.len(),
            arts.iter().map(|x| x.variants.len()).sum::<usize>(),
            share(multi_px, total),
        );
    }
    Ok(true)
}

/// The artworks `names` mean: an `art_` stem, a 16-digit surface key, or the
/// name or key of any colouring of one. Strict: an unknown name is an error.
fn resolve<'a>(arts: &'a [Artwork], names: &[&String]) -> Result<Vec<&'a Artwork>, String> {
    let mut out: Vec<&Artwork> = Vec::new();
    for n in names {
        let found = arts.iter().find(|aw| {
            let n = n.trim_end_matches(".png");
            aw.stem() == n
                || format!("{:016x}", aw.surface) == n
                || aw.variants.iter().any(|v| v.name == n || v.name.split('_').nth(1) == Some(n))
        });
        match found {
            Some(aw) => {
                if !out.iter().any(|o| o.stem() == aw.stem()) {
                    out.push(aw);
                }
            }
            None => {
                let stems: Vec<String> = arts.iter().take(500).map(Artwork::stem).collect();
                let refs: Vec<&str> = stems.iter().map(String::as_str).collect();
                return Err(format!(
                    "no artwork `{n}` in the project's maps ({}); `studio artworks PROJECT` lists them",
                    unknown("artwork", n, &refs)
                ));
            }
        }
    }
    Ok(out)
}

/// `studio bake PROJECT [ART..|--all] [--from DIR] [--mode baked|fixed] [--replace] [--dry-run]`.
pub(super) fn bake(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let mode = a.get("--mode").unwrap_or("baked");
    if !MODES.contains(&mode) {
        return Err(format!("--mode: {}", unknown("mode", mode, &MODES)));
    }
    let arts = artworks(p)?;
    let names: Vec<&String> = a.pos.iter().skip(1).collect();
    let from = a.get("--from").map(PathBuf::from);
    let picked: Vec<&Artwork> = if names.is_empty() {
        if !a.has("--all") {
            return Err("which artworks? name them (an art_ stem, a surface key or a texture's name), or --all".into());
        }
        // --all: every artwork that has an image to bake from.
        arts.iter()
            .filter(|aw| {
                p.p("pack").join(format!("{}.png", aw.stem())).is_file()
                    || from.as_ref().is_some_and(|d| d.join(format!("{}.png", aw.stem())).is_file())
            })
            .collect()
    } else {
        resolve(&arts, &names)?
    };
    if picked.is_empty() {
        return Err("no artwork has an art_ image in pack/ (or in --from): nothing to bake".into());
    }
    let dry = a.has("--dry-run");
    let mut baked_ledger = p.hashes("baked.tsv");
    let mut approved: Vec<(String, Option<u64>)> = Vec::new();
    let mut records = p.art_records();
    let (mut wrote, mut kept_hand, mut removed) = (0, 0, 0);
    let mut review: Vec<String> = Vec::new();
    let mut restyles: Vec<String> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for aw in picked {
        let stem = aw.stem();
        let target = p.p("pack").join(format!("{stem}.png"));
        if let Some(dir) = &from {
            let src = dir.join(format!("{stem}.png"));
            if src.is_file() {
                let (new, old) = (std::fs::read(&src).map_err(|e| e.to_string())?, std::fs::read(&target).ok());
                if old.as_ref().is_some_and(|o| *o != new) && !a.has("--replace") {
                    println!("kept          {stem}: pack/ has a different image (--replace takes {})", src.display());
                } else if old.as_ref() != Some(&new) && !dry {
                    copy_file(&src, &target)?;
                    approved.push((stem.clone(), Some(crate::gates::hash_bytes(&new))));
                }
            }
        }
        if !target.is_file() && !(dry && from.as_ref().is_some_and(|d| d.join(format!("{stem}.png")).is_file())) {
            missing.push(stem);
            continue;
        }
        let art_path =
            if target.is_file() { target.clone() } else { from.as_ref().unwrap().join(format!("{stem}.png")) };
        let art = img::load(&art_path)?;
        if (art.width * aw.h as usize).abs_diff(art.height * aw.w as usize) > art.width.max(art.height) {
            warn(&format!("{stem}: {}x{} is not the artwork's {}x{} in proportion", art.width, art.height, aw.w, aw.h));
        }
        let plan = artwork::plan(&art, aw);
        let (mut baked, mut copied, mut kept) = (0, 0, 0);
        for (i, v) in aw.variants.iter().enumerate() {
            let file = p.p("pack").join(format!("{}.png", v.name));
            let ours = baked_ledger.get(&v.name).copied();
            let on_disk = std::fs::read(&file).ok().map(|b| crate::gates::hash_bytes(&b));
            if mode == "fixed" {
                // Take back what an earlier bake wrote, if untouched.
                if ours.is_some() && ours == on_disk {
                    if !dry {
                        let _ = std::fs::remove_file(&file);
                        baked_ledger.remove(&v.name);
                        approved.push((v.name.clone(), None));
                    }
                    removed += 1;
                }
                continue;
            }
            // A `tex_` image the bake did not write is the author's own.
            let hand = on_disk.is_some() && ours != on_disk;
            if hand && !a.has("--replace") {
                kept += 1;
                continue;
            }
            let (out, how) = artwork::make(&art, aw, &plan, i);
            match how {
                How::Baked => baked += 1,
                _ => copied += 1,
            }
            if !dry {
                img::save(&file, &out)?;
                let bytes = std::fs::read(&file).map_err(|e| e.to_string())?;
                let h = crate::gates::hash_bytes(&bytes);
                baked_ledger.insert(v.name.clone(), h);
                approved.push((v.name.clone(), Some(h)));
            }
            wrote += 1;
        }
        kept_hand += kept;
        if mode == "baked" {
            match plan.band {
                artwork::Band::Review => review.push(aw.stem()),
                artwork::Band::Restyle => restyles.push(aw.stem()),
                _ => {}
            }
        }
        println!(
            "artwork       {} {}x{}: {} colourings; {mode}: reference {} ({:.1} levels from its ramp, {}); baked {baked}, copied {copied}, kept {kept} hand-made",
            aw.stem(),
            aw.w,
            aw.h,
            aw.variants.len(),
            aw.variants[plan.reference].name,
            plan.distance,
            plan.band.name(),
        );
        if !dry {
            records.insert(
                aw.stem(),
                Record {
                    art: aw.stem(),
                    mode: mode.into(),
                    reference: aw.variants[plan.reference].name.clone(),
                    distance: format!("{:.1}", plan.distance),
                    band: plan.band.name().into(),
                },
            );
        }
    }
    if !dry {
        p.write_art_records(&records)?;
        // `baked.tsv` is the ledger of what the bake wrote; `approved.tsv`
        // (the `stale` gate) gets the authored images only.
        let ledger: Vec<(String, Option<u64>)> =
            approved.iter().filter(|(n, _)| !n.starts_with("art_")).cloned().collect();
        p.record_hashes("baked.tsv", &ledger)?;
        let authored: Vec<(String, Option<u64>)> =
            approved.into_iter().filter(|(n, _)| n.starts_with("art_")).collect();
        p.record_hashes("approved.tsv", &authored)?;
    }
    if !review.is_empty() {
        println!(
            "review        {} artworks lie {}-{} levels from the ramp: their baked colourings are worth a look ({})",
            review.len(),
            artwork::BAKE_SILENT,
            artwork::BAKE_REVIEW,
            review.join(", ")
        );
    }
    if !restyles.is_empty() {
        println!(
            "restyle       {} artworks lie over {} levels from every ramp: the image is copied to each colouring; they may want their own art ({})",
            restyles.len(), artwork::BAKE_REVIEW, restyles.join(", ")
        );
    }
    println!(
        "{}  {wrote} images written, {removed} taken back, {kept_hand} hand-made kept{}; `studio pack` writes the manifest",
        if dry { "dry run      " } else { "baked        " },
        if missing.is_empty() { String::new() } else { format!(", no image for {}", missing.join(", ")) }
    );
    if !missing.is_empty() {
        return Err(format!("no art_ image in pack/ for {}: put it there, or --from DIR", missing.join(", ")));
    }
    Ok(true)
}
