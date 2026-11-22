//! `remodel asset bundle ID` and `remodel asset show ID`: an agent's whole
//! briefing for one asset, in one call (docs/uplift-studio.md, "Context
//! bundles").
//!
//! The bundle pulls together what the project already knows: the asset's
//! identity and artwork (`studio artworks`: colourings, ramps), its size, wrap
//! and cut-out, where it shows (the project's texture maps: which frames, in
//! what box), its status, provenance and gates (`assets.tsv`, `asset check`),
//! and three kinds of picture made from the map's frames: the original, a
//! mask of the texels that ever show, and crops of it in context. It needs no
//! ROM, no GPU and no emulator. `show` is the same answer with no pictures.

use super::*;
use crate::answer::{Answer, Code};
use crate::artwork;
use crate::gates;
use crate::scene::Args;
use project_cmd::{Row, St};
use serde_json::{json, Value};

/// A context crop is scaled down to fit this, so a bundle stays small.
const CROP_MAX: (usize, usize) = (640, 480);
/// The most variants listed with their ramps.
const VARIANTS_LISTED: usize = 8;
/// The most places listed under `shows`.
const SHOWS_LISTED: usize = 5;
/// How many frames the mask is read from (the ones it fills most in).
const MASK_FRAMES: usize = 60;
/// Context crops per bundle.
const CROPS: usize = 2;

/// One place an asset shows: a frame of a map and the box it covers there.
struct Show {
    map: String,
    frame: u32,
    pixels: u64,
    /// x0, y0, x1, y1, inclusive, in the map's picture.
    bbox: [i64; 4],
    size: [usize; 2],
}

impl Show {
    fn json(&self) -> Value {
        json!({"map": self.map, "frame": self.frame, "pixels": self.pixels,
               "box": [self.bbox[0], self.bbox[1], self.bbox[2] - self.bbox[0] + 1, self.bbox[3] - self.bbox[1] + 1],
               "picture": self.size})
    }
}

/// What one ID means.
struct Subject {
    kind: &'static str,
    /// The colourings (names), most drawn first; just the stem for a tile or
    /// a texture with no artwork.
    colourings: Vec<String>,
    art: Option<artwork::Artwork>,
}

fn norm(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The stem `token` means: an exact stem, a 16-digit key (of a texture, a
/// tile or an artwork) or a slug from `assets.tsv`. Strict.
fn resolve(token: &str, known: &[String], slugs: &BTreeMap<String, String>) -> Result<String, Answer> {
    let t = norm(token);
    if let Some(k) = known.iter().find(|k| **k == t) {
        return Ok(k.clone());
    }
    if let Some(id) = slugs.get(&t) {
        return Ok(id.clone());
    }
    if t.len() == 16 && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        let hits: Vec<&String> =
            known.iter().filter(|k| name_matches(k, &t) || k.starts_with(&format!("art_{t}_"))).collect();
        match hits.as_slice() {
            [one] => return Ok((*one).clone()),
            [] => {}
            many => {
                let names: Vec<String> = many.iter().map(|s| s.to_string()).collect();
                return Err(Answer::fail(
                    Code::BadRequest,
                    format!("key {t} is more than one asset (sizes differ): {}", names.join(", ")),
                ));
            }
        }
    }
    let near: Vec<String> = crate::answer::near_matches(token, known).into_iter().take(6).collect();
    Err(Answer::unknown("asset", token, &near).next("remodel project status --project DIR"))
}

fn ramp_hex(r: &artwork::Ramp) -> Vec<String> {
    r.iter().map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])).collect()
}

fn read_kv(path: &Path) -> Value {
    let mut m = serde_json::Map::new();
    for l in std::fs::read_to_string(path).unwrap_or_default().lines() {
        if let Some((k, v)) = l.split_once('\t') {
            m.insert(k.to_string(), json!(v));
        }
    }
    Value::Object(m)
}

/// One frame of a map: its number, picture size, and each texture ID's pixels and box.
type FrameRow = (u32, [usize; 2], Vec<(u32, u64, [i64; 4])>);

/// A map's `texmap.json`, read once: per frame, each texture ID's pixels and box.
struct MapFrames {
    name: String,
    /// frame number, picture size, then (id, pixels, bbox).
    frames: Vec<FrameRow>,
}

fn read_frames(name: &str, dir: &Path) -> Option<MapFrames> {
    let text = std::fs::read_to_string(dir.join("texmap.json")).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let mut frames = Vec::new();
    for f in v.get("frames")?.as_array()? {
        let size = [f.get("width")?.as_u64()? as usize, f.get("height")?.as_u64()? as usize];
        let mut ts = Vec::new();
        for t in f.get("textures")?.as_array()? {
            let b: Vec<i64> = t.get("bbox")?.as_array()?.iter().filter_map(Value::as_i64).collect();
            if let [x0, y0, x1, y1] = b[..] {
                ts.push((t.get("id")?.as_u64()? as u32, t.get("pixels")?.as_u64()?, [x0, y0, x1, y1]));
            }
        }
        frames.push((f.get("frame")?.as_u64()? as u32, size, ts));
    }
    Some(MapFrames { name: name.to_string(), frames })
}

/// The asset's places, biggest first (over the maps given).
fn shows_of(maps: &[MapFrames], ids: &BTreeMap<String, HashSet<u32>>) -> Vec<Show> {
    let mut out = Vec::new();
    for m in maps {
        let Some(want) = ids.get(&m.name) else { continue };
        for (frame, size, ts) in &m.frames {
            let (mut px, mut bb) = (0u64, [i64::MAX, i64::MAX, i64::MIN, i64::MIN]);
            for (id, pixels, b) in ts {
                if want.contains(id) {
                    px += pixels;
                    bb = [bb[0].min(b[0]), bb[1].min(b[1]), bb[2].max(b[2]), bb[3].max(b[3])];
                }
            }
            if px > 0 {
                out.push(Show { map: m.name.clone(), frame: *frame, pixels: px, bbox: bb, size: *size });
            }
        }
    }
    out.sort_by(|a, b| b.pixels.cmp(&a.pixels).then(a.frame.cmp(&b.frame)).then(a.map.cmp(&b.map)));
    out
}

fn frame_file(dir: &Path, frame: u32, suffix: &str) -> PathBuf {
    dir.join("frames").join(format!("{frame:06}{suffix}.png"))
}

/// The texels of a `w`x`h` asset that show in the frames given: where a
/// frame's ID image has one of `ids`, the texel image says which texel (to
/// 1/256 of the texture: a wider one fills the texels that step covers).
fn visible_mask(
    shows: &[Show],
    dirs: &BTreeMap<String, PathBuf>,
    ids: &BTreeMap<String, HashSet<u32>>,
    w: usize,
    h: usize,
) -> (Vec<bool>, usize) {
    let mut mask = vec![false; w * h];
    let mut used = 0;
    for s in shows.iter().take(MASK_FRAMES) {
        let (Some(dir), Some(want)) = (dirs.get(&s.map), ids.get(&s.map)) else { continue };
        let (Ok(idi), Ok(uvi)) =
            (img::load(&frame_file(dir, s.frame, "-ids")), img::load(&frame_file(dir, s.frame, "-uv")))
        else {
            continue;
        };
        if idi.width != uvi.width || idi.height != uvi.height {
            continue;
        }
        used += 1;
        for (i, px) in idi.rgba.chunks_exact(4).enumerate() {
            let id = px[0] as u32 | (px[1] as u32) << 8 | (px[2] as u32) << 16;
            if !want.contains(&id) {
                continue;
            }
            let (ur, vr) = (uvi.rgba[i * 4] as usize, uvi.rgba[i * 4 + 1] as usize);
            let span = |r: usize, n: usize| {
                let a = r * n / 256;
                a..((r + 1) * n / 256).max(a + 1).min(n)
            };
            for y in span(vr, h) {
                for x in span(ur, w) {
                    mask[y * w + x] = true;
                }
            }
        }
    }
    (mask, used)
}

fn draw_box(im: &mut Image, b: [i64; 4], c: [u8; 4]) {
    let (w, h) = (im.width as i64, im.height as i64);
    let mut put = |x: i64, y: i64| {
        if (0..w).contains(&x) && (0..h).contains(&y) {
            let o = ((y * w + x) * 4) as usize;
            im.rgba[o..o + 4].copy_from_slice(&c);
        }
    };
    for x in b[0]..=b[2] {
        put(x, b[1]);
        put(x, b[3]);
    }
    for y in b[1]..=b[3] {
        put(b[0], y);
        put(b[2], y);
    }
}

/// The frame cropped round the asset's box with a margin, scaled to fit
/// `CROP_MAX`, with the box drawn on it.
fn context_crop(frame: &Image, s: &Show) -> Image {
    let (bw, bh) = (s.bbox[2] - s.bbox[0] + 1, s.bbox[3] - s.bbox[1] + 1);
    let (mx, my) = ((bw / 5).max(24), (bh / 5).max(24));
    let (fw, fh) = (frame.width as i64, frame.height as i64);
    let x0 = (s.bbox[0] - mx).clamp(0, fw - 1);
    let y0 = (s.bbox[1] - my).clamp(0, fh - 1);
    let x1 = (s.bbox[2] + 1 + mx).clamp(x0 + 1, fw);
    let y1 = (s.bbox[3] + 1 + my).clamp(y0 + 1, fh);
    let crop = img::crop(frame, x0 as usize, y0 as usize, (x1 - x0) as usize, (y1 - y0) as usize);
    let k = (CROP_MAX.0 as f64 / crop.width as f64).min(CROP_MAX.1 as f64 / crop.height as f64).min(1.0);
    let (nw, nh) =
        (((crop.width as f64 * k).round() as usize).max(1), ((crop.height as f64 * k).round() as usize).max(1));
    let mut out = img::resize(&crop, nw, nh);
    let sc = |v: i64, o: i64| ((v - o) as f64 * k).round() as i64;
    draw_box(
        &mut out,
        [sc(s.bbox[0], x0), sc(s.bbox[1], y0), sc(s.bbox[2] + 1, x0) - 1, sc(s.bbox[3] + 1, y0) - 1],
        [255, 0, 255, 255],
    );
    out
}

fn status_note(r: &Row, w: usize, h: usize) -> Option<String> {
    Some(match r.status {
        St::Todo => format!(
            "write draft/{}.png (any whole scale of {w}x{h}, 1 to 8), then `remodel asset check {}`",
            r.id, r.id
        ),
        St::Drafted => format!("remodel asset check {}", r.id),
        St::Checked => format!("remodel project approve {}   (a lead's step)", r.id),
        St::Rejected => format!("draft/{}.png again with `--replace` after reading the reason in `last`", r.id),
        St::Stale => format!("the approved image changed: write draft/{}.png again, or reject", r.id),
        St::Approved => return None,
    })
}

/// Everything a bundle holds: the answer's data, and the pictures to write.
pub struct Built {
    pub data: Value,
    pub files: Vec<(String, Vec<u8>)>,
    pub stem: String,
    pub warnings: Vec<String>,
}

pub fn build(dir: &Path, token: &str, pictures: bool) -> Result<Built, Answer> {
    let p = Project { dir: dir.to_path_buf() };
    p.require().map_err(|e| {
        Answer::fail(Code::Missing, e).next("remodel project init --project DIR   (or --from-studio WORKSPACE)")
    })?;
    let internal = |e: String| Answer::fail(Code::Internal, e);
    let inv = p.inventory().map_err(|e| {
        Answer::fail(Code::Missing, e).next("make texmap, then remodel studio init DIR --map TEXMAP_DIR")
    })?;
    let arts = art::artworks(&p).map_err(internal)?;
    let rows = p.derive();
    let row_of: HashMap<&str, &Row> = rows.iter().map(|r| (r.id.as_str(), r)).collect();
    let slugs: BTreeMap<String, String> =
        rows.iter().filter(|r| !r.name.is_empty()).map(|r| (norm(&r.name), r.id.clone())).collect();
    let mut known: Vec<String> = inv.iter().map(|i| i.name.clone()).collect();
    known.extend(arts.iter().map(|a| a.stem()));
    known.extend(rows.iter().map(|r| r.id.clone()));
    known.sort();
    known.dedup();
    let stem = resolve(token, &known, &slugs)?;
    let subject = if stem.starts_with("art_") {
        let aw = arts
            .iter()
            .find(|a| a.stem() == stem)
            .cloned()
            .ok_or_else(|| Answer::unknown("asset", &stem, &known).next("remodel studio artworks DIR"))?;
        Subject { kind: "art", colourings: aw.variants.iter().map(|v| v.name.clone()).collect(), art: Some(aw) }
    } else {
        let art = arts.iter().find(|a| a.variants.iter().any(|v| v.name == stem)).cloned();
        let kind = if stem.starts_with("tile_") { "tile" } else { "tex" };
        Subject { kind, colourings: vec![stem.clone()], art }
    };
    let mut warnings = Vec::new();

    // The elected image: an artwork's most drawn colouring.
    let original_path = p.original(&stem);
    let original = original_path.as_ref().and_then(|f| img::load(f).ok());
    let (w, h) = match (&subject.art, &original) {
        (Some(a), _) if subject.kind == "art" => (a.w as usize, a.h as usize),
        (_, Some(o)) => (o.width, o.height),
        _ => inv.iter().find(|i| i.name == stem).map_or((0, 0), |i| (i.w, i.h)),
    };
    if original.is_none() {
        warnings.push(format!("no original image for {stem}: the map has no textures/{stem}.png"));
    }
    let cutout = original.as_ref().map(|o| o.rgba.chunks_exact(4).any(|px| px[3] < 128));
    let own_inv = |name: &str| inv.iter().find(|i| i.name == name);
    let lead_inv = subject.colourings.first().and_then(|n| own_inv(n)).or_else(|| own_inv(&stem));
    let (wu, wv) = asset_gates::wraps(lead_inv, &stem);

    // Which map IDs are this asset (all its colourings), per map.
    let mut ids: BTreeMap<String, HashSet<u32>> = BTreeMap::new();
    let mut pixels = 0u64;
    for c in &subject.colourings {
        if let Some(i) = own_inv(c) {
            ids.entry(i.map.clone()).or_default().insert(i.id);
            pixels += i.pixels;
        }
    }
    let total: u64 = inv.iter().map(|i| i.pixels).sum();
    let maps: Vec<MapFrames> = p.maps().iter().filter_map(|(n, d)| read_frames(n, d)).collect();
    let dirs: BTreeMap<String, PathBuf> = p.maps().into_iter().collect();
    let shows = shows_of(&maps, &ids);
    let frames_n = shows.len();
    if maps.is_empty() {
        warnings.push("the project's map has no texmap.json: no frames, so no `shows`, mask or context crops".into());
    }

    // Colourings with their ramps.
    let rec = p.art_records();
    let variants: Vec<Value> = subject
        .art
        .as_ref()
        .map(|a| {
            a.variants
                .iter()
                .take(VARIANTS_LISTED)
                .enumerate()
                .map(|(n, v)| {
                    json!({"name": v.name, "pixels": v.pixels,
                           "share_pct": (v.pixels as f64 * 1000.0 / a.pixels().max(1) as f64).round() / 10.0,
                           "elected": n == 0, "ramp": v.ramp.as_ref().map(ramp_hex)})
                })
                .collect()
        })
        .unwrap_or_default();
    let artwork = subject.art.as_ref().map(|a| {
        let r = rec.get(&a.stem());
        json!({"art": a.stem(), "surface": format!("{:016x}", a.surface), "colourings": a.variants.len(),
               "colourings_listed": variants.len(), "variants": variants,
               "mode": r.map(|r| r.mode.as_str()), "reference": r.map(|r| r.reference.as_str())})
    });

    // Status, provenance, the log.
    let row = row_of.get(stem.as_str()).copied();
    let log: Vec<Value> = std::fs::read_to_string(p.p("log.tsv"))
        .unwrap_or_default()
        .lines()
        .skip(1)
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.get(3) == Some(&stem.as_str()))
                .then(|| json!({"time": f[0], "actor": f[1], "verb": f[2], "from": f.get(4), "to": f.get(5), "note": f.get(6)}))
        })
        .collect();
    let last = log.last().cloned();
    let mut status = row.map(|r| {
        json!({"status": r.status.name(), "category": r.category, "colour": r.colour, "source": r.source,
               "hash": r.hash, "by": r.by, "date": r.date, "note": r.note, "last": last,
               "provenance": read_kv(&p.provenance_path(&stem))})
    });
    if row.is_none() && subject.kind == "art" {
        // An artwork is not a selection row of its own: say what its colourings' rows are.
        let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
        for c in &subject.colourings {
            if let Some(r) = row_of.get(c.as_str()) {
                *counts.entry(r.status.name()).or_default() += 1;
            }
        }
        status = Some(json!({"status": null, "colourings": counts,
                             "note": "the artwork has no row of its own; each selected colouring has one (asset show NAME)"}));
    } else if row.is_none() {
        warnings.push(format!("{stem} is not in the project's selection: no status; `remodel studio select` adds it"));
    }

    // Gates: the last results when there is an image, else what will apply.
    let limits = gates::Limits::load(dir).unwrap_or_default();
    let has_image =
        p.p("draft").join(format!("{stem}.png")).is_file() || p.p("pack").join(format!("{stem}.png")).is_file();
    let results = if has_image {
        asset_gates::check_assets(dir, std::slice::from_ref(&stem), None)
            .ok()
            .and_then(|mut v| v.pop())
            .map(|c| (c.from, c.gates))
    } else {
        None
    };
    let gate_json = json!({
        "required": [
            {"gate": "file", "error": true, "rule": format!("a PNG at a whole scale 1 to {} of {w}x{h}, at most 4 MB", limits.get("file", gates::FILE_MAX_SCALE as f64))},
            {"gate": "alpha", "error": true, "rule": "keeps the original's cut-out (transparent texels stay transparent)"},
            {"gate": "stale", "error": true, "rule": "an approved image is not edited after approval"},
            {"gate": "seam", "error": false, "rule": format!("the tiling seam on a repeating axis is at most {} levels", limits.get("seam", gates::SEAM_LEVELS as f64))},
            {"gate": "changed", "error": false, "rule": format!("at least {}% of the pixels differ from the original", limits.get("changed", gates::CHANGED_MIN_PCT))},
            {"gate": "noise", "error": false, "rule": format!("noise at most {}", limits.get("noise", gates::NOISE_MAX))},
        ],
        "last": results.as_ref().map(|(from, g)| json!({"from": if *from == Src::Draft { "draft" } else { "pack" },
                                                         "gates": g.iter().map(gates::GateResult::to_json).collect::<Vec<_>>()})),
    });

    // Flags a brief needs.
    let mut flags: Vec<&str> = Vec::new();
    if cutout == Some(true) {
        flags.push("cutout");
    }
    if lead_inv.is_some_and(|i| i.span_u > 1.0 || i.span_v > 1.0) {
        flags.push("repeats");
    }
    if subject.art.as_ref().is_some_and(|a| a.variants.len() > 1) {
        flags.push("shared");
    }
    if subject.kind == "tile" {
        flags.push("tile");
    }

    // Pictures.
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    let mut file_names = serde_json::Map::new();
    let mut mask_json = Value::Null;
    if pictures {
        if let Some(o) = &original {
            // As dumped, so a repaint is judged against the same texels.
            files.push(("original.png".into(), std::fs::read(original_path.as_ref().unwrap()).unwrap_or_default()));
            file_names.insert("original".into(), json!("original.png"));
            let k = 256usize.div_ceil(o.width.max(o.height).max(1));
            if k > 1 {
                let big = img::nearest(o, k);
                files.push(("original-big.png".into(), png::encode_rgba(big.width, big.height, &big.rgba)));
                file_names.insert("original_big".into(), json!("original-big.png"));
            }
        }
        if w > 0 && !shows.is_empty() {
            let (mask, used) = visible_mask(&shows, &dirs, &ids, w, h);
            let on = mask.iter().filter(|&&m| m).count();
            let rgb: Vec<u8> = mask.iter().flat_map(|&m| if m { [255u8; 3] } else { [0u8; 3] }).collect();
            files.push(("uv-mask.png".into(), png::encode_rgb(w, h, &rgb)));
            file_names.insert("mask".into(), json!("uv-mask.png"));
            mask_json = json!({"visible_texels": on, "texels": w * h,
                               "share_pct": (on as f64 * 1000.0 / (w * h) as f64).round() / 10.0, "frames_read": used,
                               "note": "white = a texel some frame of the map draws; black = never seen (a repaint there is never shown)"});
        }
        let mut crops = Vec::new();
        for (n, s) in shows.iter().take(CROPS).enumerate() {
            let Some(dir) = dirs.get(&s.map) else { continue };
            let Ok(mut frame) = img::load(&frame_file(dir, s.frame, "")) else { continue };
            // What the asset draws stays as it is; the rest of the frame is dimmed.
            if let (Ok(idi), Some(want)) = (img::load(&frame_file(dir, s.frame, "-ids")), ids.get(&s.map)) {
                if idi.width == frame.width && idi.height == frame.height {
                    for (i, px) in idi.rgba.chunks_exact(4).enumerate() {
                        if !want.contains(&(px[0] as u32 | (px[1] as u32) << 8 | (px[2] as u32) << 16)) {
                            for c in 0..3 {
                                frame.rgba[i * 4 + c] = (frame.rgba[i * 4 + c] as u32 * 35 / 100) as u8;
                            }
                        }
                    }
                }
            }
            let c = context_crop(&frame, s);
            let name = format!("context-{}.png", n + 1);
            files.push((name.clone(), png::encode_rgba(c.width, c.height, &c.rgba)));
            crops
                .push(json!({"file": name, "map": s.map, "frame": s.frame, "box_in_picture": s.json()["box"].clone()}));
        }
        file_names.insert("context".into(), Value::Array(crops));
    }

    let kind_name = subject.kind;
    let mut data = json!({
        "id": stem, "name": row.map(|r| r.name.clone()).filter(|n| !n.is_empty()), "kind": kind_name,
        "size": [w, h], "wrap": [wu.name(), wv.name()],
        "repeats": lead_inv.map(|i| json!([(i.span_u * 100.0).round() / 100.0, (i.span_v * 100.0).round() / 100.0])),
        "flags": flags, "cutout": cutout,
        "category": row.map(|r| r.category.clone()),
        "screen": {"pixels": pixels, "share_pct": (pixels as f64 * 10000.0 / total.max(1) as f64).round() / 100.0,
                   "frames": frames_n, "shows_listed": shows.len().min(SHOWS_LISTED)},
        "artwork": artwork,
        "shows": shows.iter().take(SHOWS_LISTED).map(Show::json).collect::<Vec<_>>(),
        "status": status, "gates": gate_json,
        "style": {"sheet": p.p("style.md").is_file().then_some("style.md"), "recipes": p.p("recipes.txt").is_file().then_some("recipes.txt")},
        "output": {"file": format!("draft/{stem}.png"), "sizes": [[w, h], [w * 2, h * 2]], "rule": "PNG, the original's proportion, a whole scale 1 to 8"},
        "mask": mask_json,
        "files": if pictures { Value::Object(file_names) } else { Value::Null },
    });
    if let Some(r) = row {
        if let Some(n) = status_note(r, w, h) {
            data["next"] = json!(n);
        }
    }
    Ok(Built { data, files, stem, warnings })
}

fn text_of(d: &Value, folder: Option<&Path>) -> String {
    let g = |k: &str| d.get(k).cloned().unwrap_or(Value::Null);
    let mut s = format!(
        "{}  {} {}x{}  wrap {}  {}\n",
        g("id").as_str().unwrap_or(""),
        g("kind").as_str().unwrap_or(""),
        d["size"][0],
        d["size"][1],
        d["wrap"],
        d["flags"]
    );
    if let Some(a) = d["artwork"].as_object() {
        s += &format!("artwork       {} ({} colourings)\n", a["art"].as_str().unwrap_or(""), a["colourings"]);
    }
    s += &format!(
        "screen        {}% of the map's pixels, in {} frames\n",
        d["screen"]["share_pct"], d["screen"]["frames"]
    );
    if let Some(st) = d["status"]["status"].as_str() {
        s += &format!("status        {st} ({})\n", d["category"].as_str().unwrap_or(""));
    } else if d["status"]["colourings"].is_object() {
        s += &format!("status        colourings {}\n", d["status"]["colourings"]);
    }
    for sh in d["shows"].as_array().into_iter().flatten().take(3) {
        s += &format!(
            "shows         {} frame {} box {} ({} px)\n",
            sh["map"].as_str().unwrap_or(""),
            sh["frame"],
            sh["box"],
            sh["pixels"]
        );
    }
    if let Some(f) = folder {
        s += &format!("folder        {}\n", f.display());
    }
    if let Some(n) = d["next"].as_str() {
        s += &format!("next          {n}\n");
    }
    s
}

pub fn run(sub: &str, argv: &[String]) -> Result<(Answer, String), Answer> {
    let a = Args::parse(argv, &["--json"], &["--project", "--out"])?;
    let [token] = a.pos.as_slice() else {
        return Err(Answer::fail(Code::BadRequest, format!("asset {sub}: which asset? ID (a name, a key or a slug)"))
            .next("remodel project status --project DIR"));
    };
    let dir = PathBuf::from(a.one("--project").unwrap_or("."));
    let t = std::time::Instant::now();
    let pictures = sub == "bundle";
    let mut b = build(&dir, token, pictures)?;
    let mut folder = None;
    if pictures {
        let out = a.one("--out").map(PathBuf::from).unwrap_or_else(|| dir.join("cache/bundles").join(&b.stem));
        std::fs::create_dir_all(&out).map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", out.display())))?;
        // The same files every call: old pictures this build does not make are removed.
        for old in ["original.png", "original-big.png", "uv-mask.png", "context-1.png", "context-2.png", "bundle.json"]
        {
            let _ = std::fs::remove_file(out.join(old));
        }
        for (name, bytes) in &b.files {
            std::fs::write(out.join(name), bytes)
                .map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", out.join(name).display())))?;
        }
        b.data["folder"] = json!(out.strip_prefix(&dir).unwrap_or(&out).display().to_string());
        let text = serde_json::to_string_pretty(&b.data).unwrap_or_default();
        std::fs::write(out.join("bundle.json"), text + "\n")
            .map_err(|e| Answer::fail(Code::Internal, format!("{}/bundle.json: {e}", out.display())))?;
        folder = Some(out);
    }
    b.data["ms"] = json!(t.elapsed().as_millis() as u64);
    let text = text_of(&b.data, folder.as_deref());
    let mut ans = Answer::done(b.data.clone());
    for w in &b.warnings {
        ans = ans.warn(w.clone());
    }
    if let Some(n) = b.data["next"].as_str() {
        ans = ans.next(n);
    }
    Ok((ans, text))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch").join(format!("bundle-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rgb_png(w: usize, h: usize, f: impl Fn(usize, usize) -> [u8; 3]) -> Vec<u8> {
        let mut v = Vec::new();
        for y in 0..h {
            for x in 0..w {
                v.extend_from_slice(&f(x, y));
            }
        }
        png::encode_rgb(w, h, &v)
    }

    /// A project with one 16x16 texture that shows in the left half of a
    /// 64x48 frame, with its map: inventory, texmap.json, ID and texel images.
    fn project(tag: &str) -> PathBuf {
        let dir = scratch(tag);
        let map = dir.join("map");
        for d in ["textures", "frames"] {
            std::fs::create_dir_all(map.join(d)).unwrap();
        }
        std::fs::write(dir.join("project.txt"), "name\tt\ngame\ttopskatr\n").unwrap();
        std::fs::write(
            dir.join("selection.tsv"),
            format!("{SEL_HEADER}\ntex_0123456789abcdef_16x16\ttex\t0\t-\t100\twall\tr\n"),
        )
        .unwrap();
        std::fs::write(
            map.join("inventory.tsv"),
            "id\tname\tkind\tw\th\tsurface\tframes\tpixels\tbest_frame\tbest_pixels\tpolys\tuv_used_pct\twrap\tspan_u\tspan_v\tgroup\tgroup_size\tdetail\n\
             2\ttex_0123456789abcdef_16x16\ttex\t16\t16\t00000000000000aa\t1\t800\t300\t800\t2\t50\trc\t2.00\t1.00\t1\t1\tbank=1 x=0 y=0 luma=0 colour=0000\n\
             3\ttex_ffffffffffffffff_32x32\ttex\t32\t32\t00000000000000bb\t1\t100\t300\t100\t1\t100\trr\t1.00\t1.00\t2\t1\tbank=1\n",
        )
        .unwrap();
        std::fs::write(
            map.join("textures/tex_0123456789abcdef_16x16.png"),
            rgb_png(16, 16, |x, y| [(x * 16) as u8, (y * 16) as u8, 90]),
        )
        .unwrap();
        std::fs::write(
            map.join("texmap.json"),
            r#"{"game":"t","frames":[{"frame":300,"width":64,"height":48,"textures":[
                {"id":2,"pixels":800,"bbox":[0,0,31,24],"polys":2,"uv_box":[0,0,7,15]},
                {"id":3,"pixels":100,"bbox":[40,10,49,19],"polys":1,"uv_box":[0,0,15,15]}]}],"textures":[],"groups":[]}"#,
        )
        .unwrap();
        // ID image: id 2 in x < 32, y < 25; texel image: u runs 0..127 over the width (left half of the texture).
        let (mut ids, mut uv) = (Vec::new(), Vec::new());
        for y in 0..48 {
            for x in 0..64 {
                let on = x < 32 && y < 25;
                ids.extend_from_slice(&[if on { 2 } else { 0 }, 0, 0]);
                uv.extend_from_slice(&if on { [(x * 4) as u8, (y * 10) as u8, 255] } else { [0, 0, 0] });
            }
        }
        std::fs::write(map.join("frames/000300-ids.png"), png::encode_rgb(64, 48, &ids)).unwrap();
        std::fs::write(map.join("frames/000300-uv.png"), png::encode_rgb(64, 48, &uv)).unwrap();
        std::fs::write(map.join("frames/000300.png"), rgb_png(64, 48, |x, y| [x as u8 * 4, y as u8 * 5, 40])).unwrap();
        dir
    }

    #[test]
    fn a_bundle_holds_the_identity_the_status_the_pictures_and_strict_names() {
        let dir = project("one");
        // By name, by key; an unknown name lists near ones (exit code 2).
        let b = build(&dir, "tex_0123456789abcdef_16x16", true).unwrap();
        assert_eq!(build(&dir, "0123456789abcdef", false).unwrap().stem, b.stem);
        let e = build(&dir, "tex_0123456789abcdef_16x1", false).err().unwrap();
        assert_eq!(e.code, Code::BadRequest);
        let d = &b.data;
        assert_eq!(d["size"], json!([16, 16]));
        assert_eq!(d["wrap"], json!(["repeat", "clamp"]));
        assert_eq!(d["category"], json!("wall"));
        assert_eq!(d["status"]["status"], json!("todo"));
        assert_eq!(d["shows"][0]["frame"], json!(300));
        assert_eq!(d["shows"][0]["box"], json!([0, 0, 32, 25]));
        assert_eq!(d["gates"]["required"].as_array().unwrap().len(), 6);
        // original, mask and one context crop (one frame only).
        let names: Vec<&str> = b.files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["original.png", "original-big.png", "uv-mask.png", "context-1.png"]);
        // The mask: the u image reaches texel columns 0 to 31/4*... only a left part shows.
        let mask = png::decode(&b.files[2].1).unwrap();
        assert_eq!((mask.width, mask.height), (16, 16));
        let white = mask.rgba.chunks_exact(4).filter(|p| p[0] == 255).count();
        assert!(white > 0 && white < 256, "partly visible: {white}");
        assert_eq!(d["mask"]["visible_texels"], json!(white));
        // `show` writes no pictures.
        assert!(build(&dir, "tex_0123456789abcdef_16x16", false).unwrap().files.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn run_writes_the_folder_and_rewrites_it_the_same() {
        let dir = project("run");
        let argv = vec!["tex_0123456789abcdef_16x16".to_string(), "--project".into(), dir.display().to_string()];
        let (a, _) = run("bundle", &argv).unwrap();
        let folder = dir.join(a.data["folder"].as_str().unwrap());
        let first = std::fs::read(folder.join("uv-mask.png")).unwrap();
        run("bundle", &argv).unwrap();
        assert_eq!(first, std::fs::read(folder.join("uv-mask.png")).unwrap());
        let j: Value = serde_json::from_str(&std::fs::read_to_string(folder.join("bundle.json")).unwrap()).unwrap();
        assert_eq!(j["id"], json!("tex_0123456789abcdef_16x16"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
