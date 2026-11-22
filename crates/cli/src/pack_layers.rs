//! `remodel pack resolve`: which of several stacked packs wins each asset,
//! and what the stack costs (docs/texture-packs.md, "Layers"). The stack is
//! drawn by `model2::layered::LayeredPack`; this reads the same directories
//! and answers without a GPU or a ROM.
//!
//! A layer is a pack directory. Layers are listed highest first. A layer's
//! manifest may say `depends NAME [>= VERSION]` (a pack that must sit below
//! it) and `conflicts NAME` (one that must not be in the stack at all); a
//! stack that breaks either is refused, here and by the renderer.

use crate::answer::{Answer, Code};
use crate::scene::Args;
use model2::pack::{self, Manifest};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// Winning assets named in the answer before the rest are only counted.
const SHOWN: usize = 50;

pub struct Layer {
    pub dir: PathBuf,
    /// The manifest's `name`, else the folder's name.
    pub name: String,
    pub manifest: Manifest,
    /// Image stems (`tex_..`, `art_..`, region images, maps) and their
    /// pixel sizes.
    pub images: BTreeMap<String, (u32, u32)>,
}

impl Layer {
    pub fn open(dir: &Path) -> Result<Layer, String> {
        if !dir.is_dir() {
            return Err(format!("{}: texture pack directory not found", dir.display()));
        }
        let manifest = match std::fs::read_to_string(dir.join(pack::MANIFEST)) {
            Ok(t) => Manifest::parse(&t).map_err(|e| format!("{}: {e}", dir.display()))?,
            Err(_) => Manifest::NONE,
        };
        let name = manifest
            .name
            .clone()
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| dir.display().to_string());
        let mut images = BTreeMap::new();
        for e in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?.flatten() {
            let file = e.file_name().to_string_lossy().to_string();
            if let Some(stem) = file.strip_suffix(".png") {
                images.insert(stem.to_string(), png_size(&e.path()).unwrap_or((0, 0)));
            }
        }
        Ok(Layer { dir: dir.to_path_buf(), name, manifest, images })
    }
}

/// A PNG's size from its header, without decoding it.
fn png_size(path: &Path) -> Option<(u32, u32)> {
    use std::io::Read;
    let mut b = [0u8; 24];
    std::fs::File::open(path).ok()?.read_exact(&mut b).ok()?;
    (b[..8] == [0x89, b'P', b'N', b'G', 13, 10, 26, 10])
        .then(|| (u32::from_be_bytes(b[16..20].try_into().unwrap()), u32::from_be_bytes(b[20..24].try_into().unwrap())))
}

/// What is wrong with the order of `layers` (highest first), one line each:
/// a `depends` with no such pack beneath it, or older than wanted, and a
/// `conflicts` with a pack anywhere in the stack.
pub fn order_problems(layers: &[Layer]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        for d in &l.manifest.depends {
            let (want, ver) = pack::parse_dependency(d);
            match layers[i + 1..].iter().find(|b| b.name == want) {
                None if layers[..i].iter().any(|b| b.name == want) => {
                    out.push(format!("{} depends on {want}, which is above it: put {want} below", l.name))
                }
                None => out.push(format!("{} depends on {want}, which is not in the stack", l.name)),
                Some(b) => {
                    if let Some(v) = ver {
                        match b.manifest.version.as_deref() {
                            Some(have) if pack::version_at_least(have, &v) => {}
                            Some(have) => out.push(format!("{} needs {want} >= {v}, the stack has {have}", l.name)),
                            None => out.push(format!("{} needs {want} >= {v}, which says no version", l.name)),
                        }
                    }
                }
            }
        }
        for c in &l.manifest.conflicts {
            if let Some(b) = layers.iter().enumerate().find(|(j, b)| *j != i && &b.name == c) {
                out.push(format!("{} conflicts with {}, which is in the stack", l.name, b.1.name));
            }
        }
    }
    out
}

/// The renderer's check: `dirs` (highest first) open, and none of their
/// `depends` or `conflicts` is broken. A single pack always passes.
pub fn check_layers(dirs: &[PathBuf]) -> Result<(), String> {
    if dirs.len() < 2 {
        return Ok(());
    }
    let layers = dirs.iter().map(|d| Layer::open(d)).collect::<Result<Vec<_>, _>>()?;
    let problems = order_problems(&layers);
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("the stack of packs is refused: {}", problems.join("; ")))
    }
}

const USAGE: &str = "\
remodel pack resolve DIR [DIR ...] [--json]
  Which of the stacked packs (highest first) wins each asset, what it hides, and what the stack costs:
  GPU memory of the winning images with mipmaps against the native (512 MB) and browser (192 MB) budgets.
  A `depends` that is not below its pack, a `conflicts` in the stack, or a layer that uses a feature the browser
  does not show (regions, artworks) is listed. Exit 1 when the order breaks a depends or conflicts.
  `remodel scene render NAME --pack TOP --pack BELOW` draws the same stack.
";

pub fn resolve(rest: &[String]) -> Result<(Answer, String), Answer> {
    let a = Args::parse(rest, &["--json"], &[])?;
    if a.pos.is_empty() {
        return Err(Answer::fail(Code::BadRequest, "pack resolve: which packs? DIR... (highest first)")
            .next("remodel pack resolve TOP BELOW"));
    }
    let layers: Vec<Layer> = a
        .pos
        .iter()
        .map(|d| Layer::open(Path::new(d)))
        .collect::<Result<_, _>>()
        .map_err(|e| Answer::fail(Code::Missing, e))?;
    // The winner of each asset is the first layer with it.
    let mut winner: BTreeMap<&str, usize> = BTreeMap::new();
    let mut hidden: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (i, l) in layers.iter().enumerate() {
        for stem in l.images.keys() {
            match winner.get(stem.as_str()) {
                None => {
                    winner.insert(stem, i);
                }
                Some(_) => hidden.entry(stem).or_default().push(i),
            }
        }
    }
    let mut won = vec![0usize; layers.len()];
    let mut lost = vec![0usize; layers.len()];
    let mut bytes = 0u64;
    for (stem, &i) in &winner {
        won[i] += 1;
        let (w, h) = layers[i].images[*stem];
        // Four bytes a pixel, a third more for the mip chain.
        bytes += (w as u64 * h as u64 * 4) * 4 / 3;
    }
    for hs in hidden.values() {
        for &i in hs {
            lost[i] += 1;
        }
    }
    let problems = order_problems(&layers);
    let mb = |b: u64| (b as f64 / (1 << 20) as f64 * 10.0).round() / 10.0;
    let mut warnings = Vec::new();
    for l in &layers {
        let web: Vec<&String> = l
            .manifest
            .uses
            .iter()
            .chain(&l.manifest.needs)
            .filter(|f| !pack::SHOWN_WEB.contains(&f.as_str()))
            .collect();
        if !web.is_empty() {
            warnings.push(format!(
                "{} uses {}, which the browser does not show",
                l.name,
                web.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let regioned: Vec<&Layer> = layers.iter().filter(|l| l.manifest.uses.iter().any(|f| f == "regions")).collect();
    if regioned.len() > 1 {
        warnings.push(format!(
            "{} layers have regions: only the highest ({}) is drawn, region sets are not merged",
            regioned.len(),
            regioned[0].name
        ));
    }
    let native = bytes <= pack::BUDGET_NATIVE;
    let browser = bytes <= pack::BUDGET_WASM;
    if !native {
        warnings.push(format!("the stack's {:.0} MB is over the native budget of 512 MB", mb(bytes)));
    } else if !browser {
        warnings.push(format!("the stack's {:.0} MB is over the browser's budget of 192 MB", mb(bytes)));
    }
    let hides: Vec<Value> = hidden
        .iter()
        .take(SHOWN)
        .map(|(stem, hs)| {
            json!({"asset": stem, "winner": layers[winner[stem]].name,
                   "hid": hs.iter().map(|&i| layers[i].name.clone()).collect::<Vec<_>>()})
        })
        .collect();
    let data = json!({
        "layers": layers.iter().enumerate().map(|(i, l)| json!({
            "order": i + 1, "dir": l.dir.display().to_string(), "name": l.name,
            "version": l.manifest.version, "images": l.images.len(), "won": won[i], "hidden": lost[i],
            "depends": l.manifest.depends, "conflicts": l.manifest.conflicts,
        })).collect::<Vec<_>>(),
        "assets": winner.len(), "hidden_total": hidden.len(), "hides": hides,
        "memory": {"bytes": bytes, "mb": mb(bytes), "native_limit_mb": pack::BUDGET_NATIVE >> 20,
                   "browser_limit_mb": pack::BUDGET_WASM >> 20, "native_ok": native, "browser_ok": browser},
        "problems": problems,
    });
    let mut text = String::new();
    for (i, l) in layers.iter().enumerate() {
        let _ = writeln!(
            text,
            "layer {}      {} ({}): {} images, wins {}, hidden {}",
            i + 1,
            l.name,
            l.dir.display(),
            l.images.len(),
            won[i],
            lost[i]
        );
    }
    let _ = writeln!(
        text,
        "stack         {} assets, {} named by more than one layer; {:.1} MB with mipmaps (native {}, browser {})",
        winner.len(),
        hidden.len(),
        mb(bytes),
        if native { "ok" } else { "over" },
        if browser { "ok" } else { "over" }
    );
    for p in &problems {
        let _ = writeln!(text, "problem       {p}");
    }
    let mut ans = if problems.is_empty() {
        Answer::done(data)
    } else {
        Answer::gate(format!("the stack breaks {} rule(s): {}", problems.len(), problems.join("; ")), data)
    };
    for w in warnings {
        ans = ans.warn(w);
    }
    Ok((ans, text))
}

pub fn usage() -> &'static str {
    USAGE
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str, manifest: &str, images: &[&str]) -> Layer {
        Layer {
            dir: PathBuf::from(name),
            name: name.to_string(),
            manifest: Manifest::parse(manifest).unwrap(),
            images: images.iter().map(|i| (i.to_string(), (8, 8))).collect(),
        }
    }

    #[test]
    fn depends_and_conflicts_judge_the_order() {
        let top = layer("top", "format 2\ndepends\tbase >= 1.2\nconflicts\trival\n", &[]);
        let base = layer("base", "format 2\nname base\nversion 1.10\n", &[]);
        let old = layer("base", "format 2\nversion 1.1\n", &[]);
        let rival = layer("rival", "format 2\n", &[]);
        assert!(order_problems(&[top, base]).is_empty(), "1.10 is above 1.2");
        let top = layer("top", "format 2\ndepends\tbase >= 1.2\nconflicts\trival\n", &[]);
        let p = order_problems(&[top, old, rival]);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("1.1") && p[1].contains("conflicts"), "{p:?}");
        // Above, not below: refused with a hint.
        let base = layer("base", "format 2\n", &[]);
        let top = layer("top", "format 2\ndepends\tbase\n", &[]);
        assert!(order_problems(&[base, top])[0].contains("which is above it"));
    }
}
