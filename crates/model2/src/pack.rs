//! The texture pack format (docs/texture-packs.md, "Pack format"): the
//! optional manifest `pack.txt`, the rules by which a reader shows or
//! refuses a pack, and the grammar of the file names in one. A modern
//! enhancement, not hardware, and board- and game-neutral: names are content
//! keys the renderers compute (`enhance::TexInfo`, `enhance::TileInfo`), and
//! the manifest names no board.
//!
//! Format 1 is a directory of `tex_` and `tile_` images and no manifest.
//! Format 2 adds the manifest and material maps (suffix files beside the
//! images). Nothing here loads an image or draws anything.

use crate::enhance::TexInfo;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

/// The manifest's file name in a pack.
pub const MANIFEST: &str = "pack.txt";
/// The pack's file list (the browser can't list a folder).
pub const INDEX: &str = "index.txt";
/// The highest `format` this build reads.
pub const FORMAT: u32 = 2;
/// The highest key scheme (`keys`) this build computes.
pub const KEYS: u32 = 1;
/// The feature names the format defines (`uses`, `needs`): `materials`
/// (material maps), `regions` (2D region overrides, `regions.txt`),
/// `artworks` (`art_` images, one for every colouring of an artwork), and
/// reserved `models` and `selflit`.
pub const FEATURES: [&str; 5] = ["materials", "regions", "artworks", "models", "selflit"];
/// The features this build's readers show. A pack that `needs` any other is
/// refused whole. `materials`: the CLI, the player and the browser draw
/// material maps (`gpu::Quality::materials`, the software renderer's
/// `--materials`); with the user's setting off such a pack still shows its
/// images, without the maps (the setting is the user's choice, not the
/// pack's).
pub const SHOWN: [&str; 3] = ["materials", "regions", "artworks"];
/// What the browser shows: it draws material maps but not regions yet (the
/// 2D region overrides need the pack's `regions.txt` fetched; docs/
/// texture-packs.md, "Regions"). A pack that `needs regions` is refused
/// there; one that only `uses` them shows its tile and texture images.
pub const SHOWN_WEB: [&str; 1] = ["materials"];
/// Largest pack-wide or per-material strength (`normal_scale`, `spec_scale`,
/// `normal=`, `spec=`).
pub const SCALE_MAX: f32 = 4.0;
/// GPU memory pack images may hold before the least recently drawn are
/// dropped: native and in the browser.
pub const BUDGET_NATIVE: u64 = 512 << 20;
pub const BUDGET_WASM: u64 = 192 << 20;

/// The values of the manifest's `ai` label.
pub const AI_LABELS: [&str; 3] = ["none", "assisted", "generated"];

/// A pack's `depends` line, read: the pack name and the version it must have
/// at least (`NAME >= 1.2`; a bare `NAME` wants any version).
pub fn parse_dependency(line: &str) -> (String, Option<String>) {
    match line.split_once(">=") {
        Some((n, v)) => (n.trim().to_string(), Some(v.trim().to_string())),
        None => (line.trim().to_string(), None),
    }
}

/// Whether dotted version `have` is at least `want` (numbers by part; a part
/// that is not a number compares as text).
pub fn version_at_least(have: &str, want: &str) -> bool {
    let parts = |v: &str| v.split('.').map(str::to_string).collect::<Vec<_>>();
    let (h, w) = (parts(have), parts(want));
    for i in 0..h.len().max(w.len()) {
        let (a, b) = (h.get(i).map(String::as_str).unwrap_or("0"), w.get(i).map(String::as_str).unwrap_or("0"));
        let ord = match (a.parse::<u64>(), b.parse::<u64>()) {
            (Ok(a), Ok(b)) => a.cmp(&b),
            _ => a.cmp(b),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord == std::cmp::Ordering::Greater;
        }
    }
    true
}

/// Per-material overrides (`material STEM k=v ...`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MaterialParams {
    /// `normal=X`, `spec=X`: multiply the pack-wide strengths.
    pub normal: Option<f32>,
    pub spec: Option<f32>,
    /// Other `k=v` fields, as written: reserved (`spec_base`, `spec_power`,
    /// `selflit`) or unknown, ignored.
    pub other: Vec<String>,
}

/// A pack's manifest, `pack.txt`. `Manifest::NONE` (the default) is a pack
/// without one: format 1, read exactly as before manifests existed.
#[derive(Clone, Debug, PartialEq)]
pub struct Manifest {
    /// `format N`: the format's major number.
    pub format: u32,
    /// `keys N`: the key scheme the file names use.
    pub keys: u32,
    /// `name`, `game`: free text, never used for lookup.
    pub name: Option<String>,
    pub game: Option<String>,
    /// `uses a,b`: optional features in the pack (unknown ones ignored).
    pub uses: Vec<String>,
    /// `needs a,b`: features without which the pack must not be shown.
    pub needs: Vec<String>,
    /// `normal_y up|down`: the green channel of every normal map points to
    /// the image's top (up, the default) or bottom.
    pub normal_y_up: bool,
    /// `normal_scale X`, `spec_scale X`: pack-wide strengths, 0 to 4.
    pub normal_scale: f32,
    pub spec_scale: f32,
    /// `material` lines by stem (`tex_..` or `surf_..`, no map suffix).
    pub materials: BTreeMap<String, MaterialParams>,
    /// `version`, `author`, `licence`, `ai`, `program`: what the pack says
    /// about itself (free text, never used for lookup; `ai` is `none`,
    /// `assisted` or `generated`; `program` the ROM hashes it was made
    /// against). docs/texture-packs.md, "Project metadata".
    pub version: Option<String>,
    pub author: Option<String>,
    pub licence: Option<String>,
    pub ai: Option<String>,
    pub program: Option<String>,
    /// `depends NAME [>= VERSION]` and `conflicts NAME` lines: what must be
    /// in the layers beneath this pack, and what must not be loaded with it.
    pub depends: Vec<String>,
    pub conflicts: Vec<String>,
    /// Lines with a reserved keyword (`model`), as written: ignored.
    pub reserved: Vec<String>,
    /// Lines with a keyword this build doesn't know, as written: ignored.
    pub unknown: Vec<String>,
}

/// `Manifest::NONE` as a static, for references to it.
pub static NO_MANIFEST: Manifest = Manifest::NONE;

impl Default for Manifest {
    fn default() -> Self {
        Manifest::NONE
    }
}

/// Feature names: lowercase letters, digits and `_`.
fn feature_list(s: &str) -> Result<Vec<String>, String> {
    let mut out = vec![];
    for f in s.split(|c: char| c == ',' || c.is_whitespace()).filter(|f| !f.is_empty()) {
        if !f.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_') {
            return Err(format!("`{f}` isn't a feature name (lowercase letters, digits, _)"));
        }
        if !out.iter().any(|o| o == f) {
            out.push(f.to_string());
        }
    }
    Ok(out)
}

fn scale(s: &str) -> Result<f32, String> {
    match s.parse::<f32>() {
        Ok(v) if (0.0..=SCALE_MAX).contains(&v) => Ok(v),
        _ => Err(format!("`{s}` isn't a strength from 0 to {SCALE_MAX}")),
    }
}

impl Manifest {
    pub const NONE: Manifest = Manifest {
        format: 1,
        keys: 1,
        name: None,
        game: None,
        uses: Vec::new(),
        needs: Vec::new(),
        normal_y_up: true,
        normal_scale: 1.0,
        spec_scale: 1.0,
        materials: BTreeMap::new(),
        version: None,
        author: None,
        licence: None,
        ai: None,
        program: None,
        depends: Vec::new(),
        conflicts: Vec::new(),
        reserved: Vec::new(),
        unknown: Vec::new(),
    };

    /// Parse a manifest. Lines are a keyword and its value, separated by a
    /// tab (or spaces); blank lines and `#` comments are skipped; `format`
    /// must come first. An error says what is wrong where; unknown keywords
    /// are not errors (`unknown`).
    pub fn parse(text: &str) -> Result<Manifest, String> {
        let mut m = Manifest::NONE;
        let mut seen: HashSet<&str> = HashSet::new();
        for (i, raw) in text.lines().enumerate() {
            let line = raw.trim_start_matches('\u{feff}').trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let at = |e: String| format!("{MANIFEST} line {}: {e}", i + 1);
            let (kw, value) = match line.split_once(['\t', ' ']) {
                Some((k, v)) => (k, v.trim()),
                None => (line, ""),
            };
            if seen.is_empty() && kw != "format" {
                return Err(at("the first line must be `format N`".into()));
            }
            let single = matches!(
                kw,
                "format"
                    | "keys"
                    | "name"
                    | "game"
                    | "normal_y"
                    | "normal_scale"
                    | "spec_scale"
                    | "version"
                    | "author"
                    | "licence"
                    | "ai"
                    | "program"
            );
            if single && !seen.insert(kw) {
                return Err(at(format!("`{kw}` given twice")));
            }
            match kw {
                "format" | "keys" => {
                    let n = value
                        .parse::<u32>()
                        .ok()
                        .filter(|&n| n >= 1)
                        .ok_or_else(|| at(format!("`{kw}` wants a whole number from 1, not `{value}`")))?;
                    if kw == "format" {
                        m.format = n;
                    } else {
                        m.keys = n;
                    }
                }
                "name" | "game" => {
                    let v = (!value.is_empty()).then(|| value.to_string());
                    if kw == "name" {
                        m.name = v;
                    } else {
                        m.game = v;
                    }
                }
                "version" | "author" | "licence" | "ai" | "program" => {
                    if value.is_empty() {
                        return Err(at(format!("`{kw}` wants a value")));
                    }
                    let v = Some(value.to_string());
                    match kw {
                        "version" => m.version = v,
                        "author" => m.author = v,
                        "licence" => m.licence = v,
                        "program" => m.program = v,
                        _ => {
                            if !AI_LABELS.contains(&value) {
                                return Err(at(format!("`ai` is none, assisted or generated, not `{value}`")));
                            }
                            m.ai = v;
                        }
                    }
                }
                "depends" | "conflicts" => {
                    if value.is_empty() {
                        return Err(at(format!("`{kw}` wants a pack name")));
                    }
                    if kw == "depends" {
                        m.depends.push(value.to_string());
                    } else {
                        m.conflicts.push(value.to_string());
                    }
                }
                "uses" => m.uses.extend(feature_list(value).map_err(at)?),
                "needs" => m.needs.extend(feature_list(value).map_err(at)?),
                "normal_y" => {
                    m.normal_y_up = match value {
                        "up" => true,
                        "down" => false,
                        _ => return Err(at(format!("`normal_y` is `up` or `down`, not `{value}`"))),
                    }
                }
                "normal_scale" => m.normal_scale = scale(value).map_err(at)?,
                "spec_scale" => m.spec_scale = scale(value).map_err(at)?,
                "material" => {
                    let mut fields = value.split_whitespace();
                    let stem = fields.next().ok_or_else(|| at("`material` wants a file stem".into()))?;
                    // `surf_` names only exist with a suffix: check one.
                    let ok = if stem.starts_with("surf_") {
                        !stem.contains('.') && parse_name(&map_stem(stem, MapKind::Normal)).is_ok()
                    } else {
                        matches!(parse_name(stem), Ok(FileKind::Image { .. }))
                    };
                    if !ok {
                        return Err(at(format!(
                            "`{stem}` isn't a texture or surface stem without a map suffix \
                             (tex_<key>_<w>x<h> or surf_<key>_<w>x<h>)"
                        )));
                    }
                    let mut p = MaterialParams::default();
                    for f in fields {
                        let (k, v) = f.split_once('=').ok_or_else(|| at(format!("`{f}` isn't key=value")))?;
                        match k {
                            "normal" => p.normal = Some(scale(v).map_err(at)?),
                            "spec" => p.spec = Some(scale(v).map_err(at)?),
                            _ => p.other.push(f.to_string()),
                        }
                    }
                    if m.materials.insert(stem.to_string(), p).is_some() {
                        return Err(at(format!("`material {stem}` given twice")));
                    }
                }
                "model" => m.reserved.push(line.to_string()),
                _ => m.unknown.push(line.to_string()),
            }
        }
        if seen.is_empty() {
            return Err(format!("{MANIFEST}: no `format N` line"));
        }
        Ok(m)
    }

    /// The manifest as `parse` reads it back (defaults left out, except
    /// `format` and `keys`).
    pub fn to_text(&self) -> String {
        let mut s = String::from("# reModel pack (docs/texture-packs.md, \"Pack format\")\n");
        let _ = writeln!(s, "format\t{}", self.format);
        let _ = writeln!(s, "keys\t{}", self.keys);
        for (k, v) in [("name", &self.name), ("game", &self.game)] {
            if let Some(v) = v {
                let _ = writeln!(s, "{k}\t{v}");
            }
        }
        for (k, v) in [
            ("version", &self.version),
            ("author", &self.author),
            ("licence", &self.licence),
            ("ai", &self.ai),
            ("program", &self.program),
        ] {
            if let Some(v) = v {
                let _ = writeln!(s, "{k}\t{v}");
            }
        }
        for d in &self.depends {
            let _ = writeln!(s, "depends\t{d}");
        }
        for c in &self.conflicts {
            let _ = writeln!(s, "conflicts\t{c}");
        }
        for (k, v) in [("uses", &self.uses), ("needs", &self.needs)] {
            if !v.is_empty() {
                let _ = writeln!(s, "{k}\t{}", v.join(","));
            }
        }
        if !self.normal_y_up {
            s += "normal_y\tdown\n";
        }
        for (k, v) in [("normal_scale", self.normal_scale), ("spec_scale", self.spec_scale)] {
            if v != 1.0 {
                let _ = writeln!(s, "{k}\t{v}");
            }
        }
        for (stem, p) in &self.materials {
            s += "material\t";
            s += stem;
            for (k, v) in [("normal", p.normal), ("spec", p.spec)] {
                if let Some(v) = v {
                    let _ = write!(s, "\t{k}={v}");
                }
            }
            for o in &p.other {
                s += "\t";
                s += o;
            }
            s += "\n";
        }
        for l in self.reserved.iter().chain(&self.unknown) {
            s += l;
            s += "\n";
        }
        s
    }

    /// Whether this build may show the pack (docs/texture-packs.md,
    /// "Compatibility"): the reason it must draw the original textures
    /// instead. Never guesses: a higher `format` or `keys`, or a `needs`
    /// this build doesn't show, refuses the whole pack.
    pub fn check(&self) -> Result<(), String> {
        self.check_for(&SHOWN)
    }

    /// [`Manifest::check`] for a reader that shows the features `shown`
    /// (the browser: [`SHOWN_WEB`]).
    pub fn check_for(&self, shown: &[&str]) -> Result<(), String> {
        if self.format > FORMAT {
            return Err(format!(
                "the pack is format {}, this reModel reads up to {FORMAT}: it needs a newer reModel",
                self.format
            ));
        }
        if self.keys > KEYS {
            return Err(format!(
                "the pack names files by key scheme {}, this reModel computes up to {KEYS}: it needs a newer reModel",
                self.keys
            ));
        }
        let missing: Vec<&str> = self.needs.iter().map(String::as_str).filter(|f| !shown.contains(f)).collect();
        if !missing.is_empty() {
            return Err(format!("the pack needs {}, which this reModel can't show", missing.join(", ")));
        }
        Ok(())
    }

    /// A manifest's text (None: the pack has none), parsed and checked:
    /// what a reader shows the pack with, or why it draws the originals.
    pub fn accept(text: Option<&str>) -> Result<Manifest, String> {
        Manifest::accept_for(text, &SHOWN)
    }

    /// [`Manifest::accept`] for a reader that shows the features `shown`.
    pub fn accept_for(text: Option<&str>, shown: &[&str]) -> Result<Manifest, String> {
        let Some(text) = text else { return Ok(Manifest::NONE) };
        let m = Manifest::parse(text)?;
        m.check_for(shown)?;
        Ok(m)
    }

    /// The manifest of the pack in `dir` (`accept`): no `pack.txt` is a
    /// format 1 pack.
    pub fn for_dir(dir: &Path) -> Result<Manifest, String> {
        let path = dir.join(MANIFEST);
        match std::fs::read_to_string(&path) {
            Ok(text) => Manifest::accept(Some(&text)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Manifest::NONE),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }

    /// Whether readers load the pack's material maps: only from a pack with
    /// a manifest of format 2 or later. A pack without `pack.txt` is format 1
    /// and is read exactly as before materials existed, whatever files it
    /// holds.
    pub fn reads_maps(&self) -> bool {
        self.format >= 2
    }

    /// Whether readers load the pack's `regions.txt`: only with a manifest
    /// of format 2 or later that says `uses regions` (or `needs regions`).
    /// A pack without that never has its regions file opened, so an older
    /// pack behaves as before whatever it holds.
    pub fn reads_regions(&self) -> bool {
        self.format >= 2 && self.uses.iter().chain(&self.needs).any(|f| f == crate::regions::FEATURE)
    }

    /// One line for the reader to print about what it ignores, if anything.
    pub fn notice(&self) -> Option<String> {
        let first = self.unknown.first()?;
        let first = first.split(['\t', ' ']).next().unwrap_or(first);
        Some(format!(
            "{MANIFEST}: ignoring {} line{} this reModel doesn't know (first: `{first}`)",
            self.unknown.len(),
            if self.unknown.len() == 1 { "" } else { "s" }
        ))
    }
}

/// A material map's kind, its file suffix after the stem.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum MapKind {
    /// `.n`: tangent-space normal.
    Normal,
    /// `.s`: specular scale.
    Spec,
    /// `.g`: gloss (specular exponent factor).
    Gloss,
}

impl MapKind {
    pub const ALL: [MapKind; 3] = [MapKind::Normal, MapKind::Spec, MapKind::Gloss];

    pub fn suffix(self) -> &'static str {
        match self {
            MapKind::Normal => "n",
            MapKind::Spec => "s",
            MapKind::Gloss => "g",
        }
    }

    pub fn from_suffix(s: &str) -> Option<MapKind> {
        MapKind::ALL.into_iter().find(|k| k.suffix() == s)
    }

    /// The kind's bit in `enhance::Material::has`: normal 1, specular 2,
    /// gloss 4.
    pub fn bit(self) -> u8 {
        1 << self as u8
    }
}

/// Map suffixes reserved by the format: `.e` (self-lit), `.m` (maps packed
/// in one file).
pub const RESERVED_SUFFIXES: [&str; 2] = ["e", "m"];
/// File name prefixes reserved by the format: `mdl_` (replacement models),
/// `img_` (pack-own images).
pub const RESERVED_PREFIXES: [&str; 2] = ["mdl_", "img_"];

/// What a file in a pack is, from its stem (the name without `.png`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileKind {
    /// `tex_<key>_<w>x<h>`: a replacement image for a 3D texture.
    Image { key: u64, width: u32, height: u32 },
    /// `tile_<key>`: a replacement 8x8 tile pattern.
    Tile { key: u64 },
    /// `tex_<key>_<w>x<h>.<k>` (one image key, `surface` false) or
    /// `surf_<key>_<w>x<h>.<k>` (a surface key, true): a material map.
    Map { surface: bool, key: u64, width: u32, height: u32, kind: MapKind },
    /// `art_<surface>_<w>x<h>`: one image for an artwork (a surface key),
    /// drawn for every colouring of it that has no `tex_` image of its own
    /// (`artworks` feature, docs/texture-packs.md).
    Art { surface: u64, width: u32, height: u32 },
    /// `region_<name>`: the image of a 2D region override (`regions.txt`).
    Region,
    /// A name the format reserves (`.e`, `.m`, `mdl_`, `img_`): ignored.
    Reserved,
}

fn hex_key(s: &str) -> Option<u64> {
    (s.len() == 16 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .then(|| u64::from_str_radix(s, 16).ok())
        .flatten()
}

/// A decimal size as `TexInfo::name` writes it: no sign, no leading zero,
/// at least 1.
fn size(s: &str) -> Option<u32> {
    s.parse::<u32>().ok().filter(|&n| n >= 1 && n.to_string() == s)
}

/// What the file stem `stem` names in a pack, or why it names nothing a
/// reader would ever open. Keys are 16 lowercase hex digits and sizes plain
/// decimals, exactly as the renderers write them.
pub fn parse_name(stem: &str) -> Result<FileKind, String> {
    if RESERVED_PREFIXES.iter().any(|p| stem.starts_with(p)) {
        return Ok(FileKind::Reserved);
    }
    let (base, suffix) = match stem.split_once('.') {
        Some((b, s)) => (b, Some(s)),
        None => (stem, None),
    };
    if let Some(s) = suffix {
        if RESERVED_SUFFIXES.contains(&s) && (base.starts_with("tex_") || base.starts_with("surf_")) {
            return Ok(FileKind::Reserved);
        }
    }
    if let Some(n) = stem.strip_prefix(crate::regions::IMAGE_PREFIX) {
        return if crate::regions::valid_name(n) {
            Ok(FileKind::Region)
        } else {
            Err("a region image is region_<name> (lowercase letters, digits, - and _)".into())
        };
    }
    if let Some(rest) = base.strip_prefix("art_") {
        if suffix.is_some() {
            return Err("an art_ image has no material maps yet: name the maps by tex_ or surf_".into());
        }
        let bad = || "an art name is art_<16 lowercase hex digits>_<w>x<h>".to_string();
        let (key, wh) = rest.split_once('_').ok_or_else(bad)?;
        let surface = hex_key(key).ok_or_else(bad)?;
        let (w, h) = wh.split_once('x').ok_or_else(bad)?;
        return Ok(FileKind::Art { surface, width: size(w).ok_or_else(bad)?, height: size(h).ok_or_else(bad)? });
    }
    let kind = suffix.map(|s| MapKind::from_suffix(s).ok_or_else(|| format!("`.{s}` isn't a map suffix (.n, .s, .g)")));
    if let Some(key) = base.strip_prefix("tile_") {
        let key = hex_key(key).ok_or("a tile name is tile_<16 lowercase hex digits>")?;
        return match suffix {
            None => Ok(FileKind::Tile { key }),
            Some(_) => Err("tiles have no material maps".into()),
        };
    }
    let (surface, prefix, rest) = if let Some(r) = base.strip_prefix("tex_") {
        (false, "tex", r)
    } else if let Some(r) = base.strip_prefix("surf_") {
        (true, "surf", r)
    } else {
        return Err("not a pack name (tex_, art_, surf_ or tile_)".into());
    };
    let bad = || format!("a {prefix}_ name is {prefix}_<16 lowercase hex digits>_<w>x<h>");
    let (key, wh) = rest.split_once('_').ok_or_else(bad)?;
    let key = hex_key(key).ok_or_else(bad)?;
    let (w, h) = wh.split_once('x').ok_or_else(bad)?;
    let (width, height) = (size(w).ok_or_else(bad)?, size(h).ok_or_else(bad)?);
    match kind {
        None if surface => Err("a surf_ name is a material map: it needs a suffix (.n, .s or .g)".into()),
        None => Ok(FileKind::Image { key, width, height }),
        Some(kind) => Ok(FileKind::Map { surface, key, width, height, kind: kind? }),
    }
}

/// The stem of the `kind` map for the image or surface stem `base`.
pub fn map_stem(base: &str, kind: MapKind) -> String {
    format!("{base}.{}", kind.suffix())
}

/// The file stems a reader looks for a texture's `kind` map under, in
/// order: this image in this colour (`tex_..`), then these texels in any
/// colour (`surf_..`).
pub fn map_candidates(info: &TexInfo, kind: MapKind) -> [String; 2] {
    [map_stem(&info.name(), kind), map_stem(&info.surface_name(), kind)]
}

/// The stems of `info`'s normal, specular and gloss maps among the files
/// `exists` says the pack has (None: no map of that kind), each by the
/// order of `map_candidates`: the image's own map wins over the surface's.
pub fn resolve_maps(info: &TexInfo, exists: impl Fn(&str) -> bool) -> [Option<String>; 3] {
    MapKind::ALL.map(|k| map_candidates(info, k).into_iter().find(|s| exists(s)))
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = "# reModel pack\n\
        format\t2\n\
        keys\t1\n\
        name\ttopskatr 4x, hand-tuned materials\n\
        game\ttopskatr\n\
        uses\tmaterials\n\
        needs\n\
        normal_y\tdown\n\
        normal_scale\t1.5\n\
        spec_scale\t0.25\n\
        material\ttex_0123456789abcdef_64x64\tnormal=0.5\tspec=1.5\n\
        material\tsurf_fedcba9876543210_64x64\tnormal=0\tspec_base=3\n\
        model\tmdl_0011223344556677\tat=0x00812340\n\
        lamp_colour\twarm\n";

    #[test]
    fn manifest_round_trips() {
        let m = Manifest::parse(FULL).unwrap();
        assert_eq!(m.format, 2);
        assert_eq!(m.name.as_deref(), Some("topskatr 4x, hand-tuned materials"));
        assert_eq!(m.uses, ["materials"]);
        assert!(m.needs.is_empty() && !m.normal_y_up);
        assert_eq!((m.normal_scale, m.spec_scale), (1.5, 0.25));
        let t = &m.materials["tex_0123456789abcdef_64x64"];
        assert_eq!((t.normal, t.spec), (Some(0.5), Some(1.5)));
        let s = &m.materials["surf_fedcba9876543210_64x64"];
        assert_eq!((s.normal, s.spec, &s.other[..]), (Some(0.0), None, &["spec_base=3".to_string()][..]));
        assert_eq!(m.reserved.len(), 1);
        assert_eq!(m.unknown, ["lamp_colour\twarm"]);
        assert_eq!(Manifest::parse(&m.to_text()).unwrap(), m);
        // The default writes and reads back as itself; spaces work as tabs.
        assert_eq!(Manifest::parse(&Manifest::NONE.to_text()).unwrap(), Manifest::NONE);
        let spaced = Manifest::parse("format 2\nuses  materials, selflit\nnormal_y up\n").unwrap();
        assert_eq!(spaced.uses, ["materials", "selflit"]);
        assert_eq!(Manifest::parse(&spaced.to_text()).unwrap(), spaced);
    }

    #[test]
    fn regions_feature_is_shown_here_refused_by_the_browser_and_read_only_when_used() {
        // A pack that merely uses regions is accepted everywhere.
        let uses = "format 2\nuses regions\n";
        assert!(Manifest::accept(Some(uses)).unwrap().reads_regions());
        assert!(Manifest::accept_for(Some(uses), &SHOWN_WEB).unwrap().reads_regions());
        // One that needs them is shown by readers that draw them, refused by the rest.
        let needs = "format 2\nuses regions\nneeds regions\n";
        assert!(Manifest::accept(Some(needs)).is_ok());
        let e = Manifest::accept_for(Some(needs), &SHOWN_WEB).unwrap_err();
        assert!(e.contains("needs regions") && e.contains("can't show"), "{e}");
        // Older readers know nothing of the feature name and ignore it in `uses`.
        assert!(FEATURES.contains(&"regions"));
        // No manifest, format 1, or no `uses regions`: regions.txt is never opened.
        assert!(!Manifest::NONE.reads_regions());
        assert!(!Manifest::accept(Some("format 1\nuses regions\n")).unwrap().reads_regions());
        assert!(!Manifest::accept(Some("format 2\nuses materials\n")).unwrap().reads_regions());
        // Region image names.
        assert_eq!(parse_name("region_logo-1_a"), Ok(FileKind::Region));
        assert!(parse_name("region_Logo").is_err());
        assert!(parse_name("region_").is_err());
    }

    #[test]
    fn unknown_keywords_are_ignored_and_reported() {
        let m = Manifest::accept(Some("format 2\nsparkle\tyes\nsparkle\tno\nuses\tmaterials,hologram\n")).unwrap();
        assert_eq!(m.unknown.len(), 2);
        assert!(m.notice().unwrap().contains("2 lines") && m.notice().unwrap().contains("`sparkle`"));
        // Unknown features in `uses` don't refuse the pack.
        assert_eq!(m.uses, ["materials", "hologram"]);
        assert_eq!(Manifest::NONE.notice(), None);
    }

    #[test]
    fn refuses_what_it_cannot_show() {
        assert_eq!(Manifest::accept(None).unwrap(), Manifest::NONE);
        assert!(Manifest::accept(Some("format 1\n")).is_ok());
        assert!(Manifest::accept(Some(&format!("format {FORMAT}\n"))).is_ok());
        let e = Manifest::accept(Some(&format!("format {}\n", FORMAT + 1))).unwrap_err();
        assert!(e.contains("newer reModel"), "{e}");
        // A higher format parses (the lint reports it) but is refused.
        assert_eq!(Manifest::parse("format 9\nwhatever 1\n").unwrap().format, 9);
        assert!(Manifest::accept(Some("format 2\nkeys 2\n")).unwrap_err().contains("key scheme 2"));
        let e = Manifest::accept(Some("format 2\nneeds\tteleport\n")).unwrap_err();
        assert!(e.contains("needs teleport"), "{e}");
        // Every reader draws material maps: a pack that needs them is shown.
        assert!(Manifest::accept(Some("format 2\nneeds materials\n")).is_ok());
        assert!(Manifest::accept(Some("format 2\nneeds materials, models\n")).unwrap_err().contains("needs models"));
        assert!(Manifest::accept(Some("format 2\nuses materials\n")).is_ok());
    }

    #[test]
    fn malformed_manifests_are_errors() {
        for (text, why) in [
            ("", "no `format"),
            ("# only a comment\n", "no `format"),
            ("keys 1\nformat 2\n", "first line"),
            ("format two\n", "whole number"),
            ("format 0\n", "whole number"),
            ("format 2\nformat 2\n", "twice"),
            ("format 2\nnormal_y sideways\n", "`up` or `down`"),
            ("format 2\nnormal_scale 5\n", "strength"),
            ("format 2\nspec_scale -1\n", "strength"),
            ("format 2\nuses Materials\n", "feature name"),
            ("format 2\nmaterial\n", "file stem"),
            ("format 2\nmaterial tex_0123456789abcdef_64x64.n\n", "without a map suffix"),
            ("format 2\nmaterial tile_0123456789abcdef\n", "without a map suffix"),
            ("format 2\nmaterial surf_0123_64x64\n", "surf_"),
            ("format 2\nmaterial tex_0123456789abcdef_64x64 normal\n", "key=value"),
            ("format 2\nmaterial tex_0123456789abcdef_64x64 spec=9\n", "strength"),
            ("format 2\nmaterial tex_0123456789abcdef_64x64\nmaterial tex_0123456789abcdef_64x64\n", "twice"),
        ] {
            let e = Manifest::parse(text).unwrap_err();
            assert!(e.contains(why), "{text:?}: {e}");
        }
        // Errors say where.
        assert!(Manifest::parse("format 2\n\nnormal_y no\n").unwrap_err().starts_with("pack.txt line 3:"));
    }

    #[test]
    fn name_grammar() {
        let key = 0x0123_4567_89ab_cdef;
        assert_eq!(parse_name("tex_0123456789abcdef_64x32"), Ok(FileKind::Image { key, width: 64, height: 32 }));
        assert_eq!(parse_name("tile_0123456789abcdef"), Ok(FileKind::Tile { key }));
        for (stem, surface, kind) in [
            ("tex_0123456789abcdef_64x32.n", false, MapKind::Normal),
            ("tex_0123456789abcdef_64x32.s", false, MapKind::Spec),
            ("surf_0123456789abcdef_64x32.g", true, MapKind::Gloss),
        ] {
            assert_eq!(parse_name(stem), Ok(FileKind::Map { surface, key, width: 64, height: 32, kind }));
        }
        for stem in ["tex_0123456789abcdef_64x32.e", "surf_0123456789abcdef_8x8.m", "mdl_0123456789abcdef", "img_logo"]
        {
            assert_eq!(parse_name(stem), Ok(FileKind::Reserved), "{stem}");
        }
        for (stem, why) in [
            ("tile_0123456789abcdef.n", "no material maps"),
            ("surf_0123456789abcdef_64x32", "needs a suffix"),
            ("tex_0123456789abcdef_64x32.x", "map suffix"),
            ("tex_0123456789ABCDEF_64x32", "lowercase"),
            ("tex_0123456789abcde_64x32", "lowercase"),
            ("tex_0123456789abcdef_064x32", "<w>x<h>"),
            ("tex_0123456789abcdef_0x32", "<w>x<h>"),
            ("tex_0123456789abcdef_64", "<w>x<h>"),
            ("tile_01234", "tile_<16"),
            ("upscale", "not a pack name"),
            ("tex_0123456789abcdef_64x32.n.s", "map suffix"),
        ] {
            let e = parse_name(stem).unwrap_err();
            assert!(e.contains(why), "{stem}: {e}");
        }
    }

    #[test]
    fn an_artwork_name_parses_and_the_manifest_may_need_artworks() {
        assert_eq!(
            parse_name("art_0123456789abcdef_64x32"),
            Ok(FileKind::Art { surface: 0x0123_4567_89ab_cdef, width: 64, height: 32 })
        );
        for (stem, why) in [
            ("art_0123456789abcdef_64x32.n", "no material maps"),
            ("art_0123456789ABCDEF_64x32", "art_<16"),
            ("art_0123456789abcdef_0x32", "<w>x<h>"),
            ("art_01234_8x8", "art_<16"),
        ] {
            let e = parse_name(stem).unwrap_err();
            assert!(e.contains(why), "{stem}: {e}");
        }
        // Shown here; the browser does not read art_ files, so a pack that
        // needs them is refused there and one that only uses them is not.
        assert!(Manifest::accept(Some("format 2\nneeds artworks\n")).is_ok());
        let e = Manifest::accept_for(Some("format 2\nneeds artworks\n"), &SHOWN_WEB).unwrap_err();
        assert!(e.contains("artworks"), "{e}");
        assert!(Manifest::accept_for(Some("format 2\nuses artworks\n"), &SHOWN_WEB).is_ok());
        let info = TexInfo {
            key: 1,
            surface: 0xff,
            width: 8,
            height: 4,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        assert_eq!(info.art_name(), "art_00000000000000ff_8x4");
        assert!(matches!(parse_name(&info.art_name()), Ok(FileKind::Art { surface: 0xff, .. })));
    }

    #[test]
    fn names_the_renderer_writes_parse() {
        let info = TexInfo {
            key: 0xfedc_ba98_7654_3210,
            surface: 0x0000_0000_0000_00ff,
            width: 128,
            height: 8,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        assert_eq!(info.surface_name(), "surf_00000000000000ff_128x8");
        assert_eq!(parse_name(&info.name()), Ok(FileKind::Image { key: info.key, width: 128, height: 8 }));
        let [own, shared] = map_candidates(&info, MapKind::Spec);
        assert_eq!(own, "tex_fedcba9876543210_128x8.s");
        assert_eq!(shared, "surf_00000000000000ff_128x8.s");
        assert_eq!(
            parse_name(&shared),
            Ok(FileKind::Map { surface: true, key: 0xff, width: 128, height: 8, kind: MapKind::Spec })
        );
    }

    #[test]
    fn maps_resolve_by_image_key_then_surface_key() {
        let info = TexInfo {
            key: 0xfedc_ba98_7654_3210,
            surface: 0xff,
            width: 8,
            height: 8,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        // Per kind: the image's own normal map wins over the surface's; the
        // specular map exists only by surface; no gloss map.
        let files = [
            "tex_fedcba9876543210_8x8.n",
            "surf_00000000000000ff_8x8.n",
            "surf_00000000000000ff_8x8.s",
            "tex_0000000000000001_8x8.g",
        ];
        let got = resolve_maps(&info, |s| files.contains(&s));
        assert_eq!(
            got,
            [Some("tex_fedcba9876543210_8x8.n".to_string()), Some("surf_00000000000000ff_8x8.s".to_string()), None]
        );
        assert_eq!(resolve_maps(&info, |_| false), [None, None, None]);
        // Another colour of the same texels (another image key) gets the
        // surface's maps.
        let other = TexInfo { key: 1, ..info };
        assert_eq!(resolve_maps(&other, |s| files.contains(&s))[0].as_deref(), Some("surf_00000000000000ff_8x8.n"));
        assert_eq!([MapKind::Normal, MapKind::Spec, MapKind::Gloss].map(MapKind::bit), [1, 2, 4]);
        // Maps are read from format 2 packs only.
        assert!(!Manifest::NONE.reads_maps() && Manifest::parse("format 2\n").unwrap().reads_maps());
        assert!(!Manifest::parse("format 1\n").unwrap().reads_maps());
    }
}
