//! `remodel scene probe`: what draws a game pixel, and whether a texture
//! pack supplied it (docs/texture-packs.md, "Probing a pixel").
//!
//! For each pixel of a rectangle (game pixels, the hardware's 496x384) the
//! probe lists the layers that have a say there, top to bottom: the
//! high-priority tile layers (HUD), the 3D polygon that shows, and the
//! low-priority tile layers (backdrop, menus). Nothing is drawn: it reads
//! the capture the way `texmap` does (the tile maps, the software
//! rasterizer's visibility pass), so it needs no GPU, and says for each
//! source which file of a stack of packs (`tile_KEY.png`,
//! `region_NAME.png`, `tex_KEY[~PAL]_WxH.png`) would supply it.

use crate::answer::{Answer, Code};
use crate::pack_layers::Layer;
use model2::capture::Frame;
use model2::enhance::Rgba;
use model2::raster;
use model2::regions::{RegionGrid, RegionSet};
use model2::video::{self, View, HEIGHT, WIDTH};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Pixels listed one by one in the answer; the `sources` summary always
/// covers the whole rectangle.
const LISTED: usize = 64;

/// One pack of the stack: its image names and region set.
pub struct StackLayer {
    pub name: String,
    /// Image stems (`tile_..`, `tex_..`, `region_..`) with their sizes.
    pub images: BTreeMap<String, (u32, u32)>,
    pub regions: Option<RegionSet>,
}

/// Packs, highest first.
#[derive(Default)]
pub struct Stack {
    pub layers: Vec<StackLayer>,
}

/// Who supplies an asset.
struct Supply {
    layer: String,
    file: String,
    /// "tile", "region", "texture" or "variant".
    kind: &'static str,
}

impl Stack {
    pub fn open(dirs: &[PathBuf]) -> Result<Stack, String> {
        let mut layers = Vec::new();
        for d in dirs {
            let l = Layer::open(d)?;
            let regions = if l.manifest.reads_regions() && l.dir.join(model2::regions::FILE).is_file() {
                // Images are only measured, not read: a blank stand-in of the
                // file's size, so a region drawn is told from one skipped.
                let mut stand_in = |p: &Path| -> Option<Arc<Rgba>> {
                    let stem = p.file_stem()?.to_string_lossy().into_owned();
                    let &(w, h) = l.images.get(&stem)?;
                    let (w, h) = (w as usize, h as usize);
                    (w > 0 && h > 0).then(|| Arc::new(Rgba { width: w, height: h, data: vec![0; w * h * 4] }))
                };
                Some(RegionSet::load_dir(&l.dir, &mut stand_in)?)
            } else {
                None
            };
            layers.push(StackLayer { name: l.name, images: l.images, regions });
        }
        Ok(Stack { layers })
    }

    /// Where each layer's regions match this frame.
    fn grids(&self, tiles: &[u8], palette: &[u8]) -> Vec<Option<RegionGrid>> {
        self.layers
            .iter()
            .map(|l| l.regions.as_ref().map(|set| video::match_regions(tiles, palette, set)).filter(|g| g.any))
            .collect()
    }

    /// A tile pattern's replacement: a region over its cell, else its own image.
    fn tile(&self, name: Option<&str>, region: &[Option<String>]) -> Option<Supply> {
        for (i, l) in self.layers.iter().enumerate() {
            if let Some(r) = region.get(i).and_then(|r| r.as_ref()) {
                return Some(Supply { layer: l.name.clone(), file: format!("region_{r}.png"), kind: "region" });
            }
            if let Some(name) = name.filter(|n| l.images.contains_key(*n)) {
                return Some(Supply { layer: l.name.clone(), file: format!("{name}.png"), kind: "tile" });
            }
        }
        None
    }

    /// A texture: the palette's variant first, else the plain image.
    fn texture(&self, key: u64, pal: u32, w: u32, h: u32) -> Option<Supply> {
        let variant = model2::enhance::variant_name(key, pal, w, h);
        let plain = format!("tex_{key:016x}_{w}x{h}");
        for l in &self.layers {
            for (stem, kind) in [(&variant, "variant"), (&plain, "texture")] {
                if l.images.contains_key(stem) {
                    return Some(Supply { layer: l.name.clone(), file: format!("{stem}.png"), kind });
                }
            }
        }
        None
    }
}

fn pack_json(s: Option<Supply>, has_stack: bool) -> Value {
    if !has_stack {
        return Value::Null;
    }
    match s {
        Some(s) => json!({"supplied": true, "kind": s.kind, "layer": s.layer, "file": s.file}),
        None => json!({"supplied": false, "kind": null, "layer": null, "file": null}),
    }
}

/// The region of `set` (if any) whose slice `s` is.
fn region_of(set: &RegionSet, s: u32) -> Option<String> {
    set.regions
        .iter()
        .find(|r| s >= r.first_slice && s < r.first_slice + (r.def.w * r.def.h) as u32)
        .map(|r| r.def.name.clone())
}

/// `X,Y[,W,H]` in game pixels, as the rectangle `[x, y, w, h]` clipped to
/// the frame (1x1 when W and H are left out).
pub fn parse_rect(s: &str) -> Result<[usize; 4], Answer> {
    let bad = || {
        Answer::fail(Code::BadRequest, format!("probe wants X,Y or X,Y,W,H in game pixels (496x384), got `{s}`"))
            .next("remodel scene probe NAME --at 100,100,8,8")
    };
    let v: Vec<i64> = s.split(',').map(|p| p.trim().parse::<i64>()).collect::<Result<_, _>>().map_err(|_| bad())?;
    let (x, y, w, h) = match v[..] {
        [x, y] => (x, y, 1, 1),
        [x, y, w, h] => (x, y, w, h),
        _ => return Err(bad()),
    };
    if x < 0 || y < 0 || w < 1 || h < 1 || x >= WIDTH as i64 || y >= HEIGHT as i64 {
        return Err(Answer::fail(
            Code::BadRequest,
            format!("probe `{s}` falls outside the {WIDTH}x{HEIGHT} frame (game pixels, W and H at least 1)"),
        ));
    }
    let (w, h) = (w.min(WIDTH as i64 - x), h.min(HEIGHT as i64 - y));
    Ok([x as usize, y as usize, w as usize, h as usize])
}

/// The 3D polygon that shows at each pixel, worked out once.
struct Scene3d {
    polys: Vec<raster::Poly>,
    placed: Vec<raster::Placed>,
    vis: raster::Visible,
    uv: Vec<[f32; 2]>,
    keys: raster::TexKeys,
}

impl Scene3d {
    fn new(f: &Frame, src: &raster::Sources, view: &View) -> Scene3d {
        let mut polys = raster::build_shaded(&f.stream, src, None);
        video::HudLayout::new(&f.tiles, &f.palette, view).anchor(&mut polys, view);
        let placed = raster::place(&mut polys, view);
        let (vis, uv) = raster::visible_uv(&polys, &placed, src, view);
        Scene3d { polys, placed, vis, uv, keys: raster::TexKeys::default() }
    }
}

/// What draws each pixel of `rect` in frame `f`, against the pack stack
/// `stack` (empty: the pack fields are null).
pub fn probe_frame(f: &Frame, rom: &[u16], rect: [usize; 4], stack: &Stack) -> Value {
    let view = View::native();
    let src = f.sources(rom);
    let mut sc = (!f.stream.is_empty()).then(|| Scene3d::new(f, &src, &view));
    let grids = stack.grids(&f.tiles, &f.palette);
    let has_stack = !stack.layers.is_empty();
    let [rx, ry, rw, rh] = rect;
    let mut pixels = Vec::new();
    // Visible sources: id -> (first layer row, pixel count, bbox x0 y0 x1 y1).
    let mut sources: BTreeMap<String, (Value, usize, [usize; 4])> = BTreeMap::new();
    for y in ry..ry + rh {
        for x in rx..rx + rw {
            let tiles = video::probe_tiles(&f.tiles, &f.palette, x, y);
            let entry = |t: &video::TileProbe| -> Value {
                let name = t.info.as_ref().map(|i| i.name());
                let region: Vec<Option<String>> = stack
                    .layers
                    .iter()
                    .zip(&grids)
                    .map(|(l, g)| {
                        let (set, g) = (l.regions.as_ref()?, g.as_ref()?);
                        region_of(set, g.at(t.map, t.cx, t.cy)?)
                    })
                    .collect();
                json!({
                    "kind": "tile", "priority": if t.high { "high" } else { "low" },
                    "plane": if t.plane == 0 { "A" } else { "B" },
                    "map": t.map, "cell": [t.cx, t.cy], "tile": name,
                    "tile_number": t.info.as_ref().map(|i| i.tile), "bank": t.info.as_ref().map(|i| i.bank),
                    "texel": [t.col, t.row], "pen": t.pen, "opaque": t.opaque,
                    "region": region.iter().flatten().next(),
                    "pack": pack_json(stack.tile(name.as_deref(), &region), has_stack),
                })
            };
            let mut layers: Vec<Value> = tiles.iter().filter(|t| t.high).map(entry).collect();
            if let Some(sc) = sc.as_mut() {
                let o = y * view.width + x;
                let k = sc.vis.owner[o];
                if k != u32::MAX {
                    let p = &sc.polys[sc.placed[k as usize].poly];
                    let mut e = json!({
                        "kind": "3d", "poly": k, "window": p.window, "object": p.block,
                        "texture": "flat", "key": null, "variant": null, "size": null, "uv": null,
                        "opaque": true, "pack": pack_json(None, has_stack),
                    });
                    let info =
                        p.texture.as_ref().and_then(|t| raster::tex_info(p, t, &src, &mut sc.keys).map(|i| (t, i)));
                    if let Some((t, mut info)) = info {
                        info.pal = raster::texture_palette(&src, &info);
                        let [u, v] = sc.uv[o];
                        e["texture"] = json!(info.name());
                        e["key"] = json!(format!("{:016x}", info.key));
                        e["variant"] = json!(format!("{:08x}", info.pal));
                        e["size"] = json!([info.width, info.height]);
                        e["uv"] = json!([(u * 100.0).round() / 100.0, (v * 100.0).round() / 100.0]);
                        e["mirror"] = json!([t.mirror_x, t.mirror_y]);
                        e["pack"] = pack_json(stack.texture(info.key, info.pal, info.width, info.height), has_stack);
                    }
                    layers.push(e);
                }
            }
            layers.extend(tiles.iter().filter(|t| !t.high).map(entry));
            // The first opaque layer shows; everything under it is covered.
            let mut shown = false;
            for l in layers.iter_mut() {
                let opaque = l["opaque"].as_bool().unwrap_or(false);
                l["visible"] = json!(opaque && !shown);
                if opaque && !shown {
                    shown = true;
                    let s = sources.entry(source_id(l)).or_insert_with(|| (l.clone(), 0, [x, y, x, y]));
                    s.1 += 1;
                    s.2 = [s.2[0].min(x), s.2[1].min(y), s.2[2].max(x), s.2[3].max(y)];
                }
            }
            if pixels.len() < LISTED {
                pixels.push(json!({"x": x, "y": y, "layers": layers}));
            }
        }
    }
    let mut rows: Vec<(Value, usize, [usize; 4])> = sources.into_values().collect();
    rows.sort_by_key(|a| std::cmp::Reverse(a.1));
    let rows: Vec<Value> = rows
        .into_iter()
        .map(|(mut row, n, bbox)| {
            row["pixels"] = json!(n);
            row["bbox"] = json!(bbox);
            if let Some(o) = row.as_object_mut() {
                for k in ["texel", "pen", "opaque", "visible", "uv"] {
                    o.remove(k);
                }
            }
            row
        })
        .collect();
    json!({
        "rect": rect, "pixels_total": rw * rh, "pixels_listed": pixels.len(),
        "layers": stack.layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>(),
        "sources": rows, "pixels": pixels,
    })
}

/// What makes two visible pixels the same source in the summary.
fn source_id(l: &Value) -> String {
    match l["kind"].as_str() {
        Some("3d") => format!("3d {} {} {}", l["texture"], l["variant"], l["poly"]),
        _ => format!("tile {} {} {} {}", l["tile"], l["plane"], l["priority"], l["cell"]),
    }
}

fn describe(l: &Value) -> String {
    let pack = match l["pack"]["supplied"].as_bool() {
        None => String::new(),
        Some(true) => format!(
            "  pack: {} ({}, {})",
            l["pack"]["file"].as_str().unwrap_or(""),
            l["pack"]["kind"].as_str().unwrap_or(""),
            l["pack"]["layer"].as_str().unwrap_or("")
        ),
        Some(false) => "  pack: not supplied".to_string(),
    };
    match l["kind"].as_str() {
        Some("3d") => format!(
            "3D poly {} window {} object {:#x}  {}{}{}",
            l["poly"],
            l["window"],
            l["object"].as_u64().unwrap_or(0),
            l["texture"].as_str().unwrap_or("?"),
            l["variant"].as_str().map(|v| format!(" pal {v}")).unwrap_or_default(),
            pack
        ),
        _ => format!(
            "{} plane {} map {} cell {},{}  {}{}{}",
            l["priority"].as_str().unwrap_or("?"),
            l["plane"].as_str().unwrap_or("?"),
            l["map"],
            l["cell"][0],
            l["cell"][1],
            l["tile"].as_str().unwrap_or("blank"),
            l["region"].as_str().map(|r| format!("  region {r}")).unwrap_or_default(),
            pack
        ),
    }
}

/// The answer's text: one line per source, then the pixels if few.
pub fn text_of(v: &Value) -> String {
    let rect: Vec<u64> = v["rect"].as_array().map(|a| a.iter().filter_map(Value::as_u64).collect()).unwrap_or_default();
    let mut t = format!("rect          {rect:?} ({} pixels, {} listed)\n", v["pixels_total"], v["pixels_listed"]);
    t += "sources (what shows, most pixels first)\n";
    for s in v["sources"].as_array().into_iter().flatten() {
        t += &format!("  {:>5} px {}  {}\n", s["pixels"], s["bbox"], describe(s));
    }
    let px = v["pixels"].as_array().cloned().unwrap_or_default();
    if px.len() <= 4 {
        for p in px {
            t += &format!("pixel {},{} top to bottom\n", p["x"], p["y"]);
            for l in p["layers"].as_array().into_iter().flatten() {
                let mark = match (l["visible"].as_bool(), l["opaque"].as_bool()) {
                    (Some(true), _) => "*",
                    (_, Some(true)) => "-",
                    _ => " ",
                };
                t += &format!("  {mark} {}\n", describe(l));
            }
        }
        t += "  (* shows, - opaque but covered, blank: transparent here)\n";
    }
    t
}

/// Probe the capture in `scene_dir`.
pub fn probe_scene(scene_dir: &Path, rect: [usize; 4], stack: &Stack) -> Result<Value, Answer> {
    let cap = scene_dir.join("capture");
    let name = scene_dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let missing =
        |m: String| Answer::fail(Code::Missing, m).next(format!("remodel scene make {name}   (needs the ROM)"));
    let frame_dir =
        crate::scene::capture_frame_dir(&cap).ok_or_else(|| missing(format!("scene `{name}` has no capture")))?;
    let rom = model2::capture::load_texture_rom(&cap).map_err(|e| missing(format!("{}: {e}", cap.display())))?;
    let f =
        Frame::load(&frame_dir).map_err(|e| Answer::fail(Code::Internal, format!("{}: {e}", frame_dir.display())))?;
    Ok(probe_frame(&f, &rom, rect, stack))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plane A (map 0) showing tile 1 (pen 1) at cell (10, 10) and tile 2
    /// (pen 2) at (11, 10), the second with the priority bit; the other
    /// maps off. No 3D.
    fn frame() -> Frame {
        let mut tiles = vec![0u8; 0x10_0000];
        let mut put = |off: usize, v: u16| tiles[off..off + 2].copy_from_slice(&v.to_le_bytes());
        put((10 * 64 + 10) * 2, 1);
        put((10 * 64 + 11) * 2, 2 | 0x8000);
        for map in 1..4 {
            put(0xa008 + 2 * map, 0x8000);
        }
        tiles[0x8_0000 + 32..0x8_0000 + 64].fill(0x11);
        tiles[0x8_0000 + 64..0x8_0000 + 96].fill(0x22);
        let mut palette = vec![0u8; 0x2_0000];
        palette[2..4].copy_from_slice(&0x03e0u16.to_le_bytes());
        palette[4..6].copy_from_slice(&0x03ffu16.to_le_bytes());
        Frame {
            frame: 0,
            stream: vec![],
            texture_point_ram: vec![],
            render_ram: vec![],
            palette,
            tiles,
            xoff: 0.0,
            yoff: 0.0,
            light: None,
            scan: None,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let root = std::env::var("REMODEL_SCRATCH").unwrap_or_else(|_| "out/scratch".into());
        let d = Path::new(&root).join(format!("probe-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn png16() -> Vec<u8> {
        png::encode_rgba(16, 16, &[200; 16 * 16 * 4])
    }

    #[test]
    fn rect_words_are_strict_and_clipped() {
        assert_eq!(parse_rect("95,100").unwrap(), [95, 100, 1, 1]);
        assert_eq!(parse_rect("490,380,20,20").unwrap(), [490, 380, 6, 4]);
        for bad in ["", "1", "1,2,3", "a,b", "-1,0", "496,0", "0,384", "0,0,0,1"] {
            assert!(parse_rect(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn a_pixel_lists_its_tiles_top_to_bottom_without_a_pack() {
        let f = frame();
        // Cell 10 is x 80..88, y 80..88; cell 11 is x 88..96.
        let v = probe_frame(&f, &[], [90, 82, 1, 1], &Stack::default());
        let layers = v["pixels"][0]["layers"].as_array().unwrap();
        assert_eq!(layers[0]["priority"], "high");
        assert_eq!(layers[0]["cell"], json!([11, 10]));
        assert_eq!(layers[0]["visible"], true);
        assert!(layers[0]["pack"].is_null(), "no pack given");
        let key = layers[0]["tile"].as_str().unwrap();
        assert!(key.starts_with("tile_") && key.len() == 21);
        assert_eq!(v["sources"][0]["tile"], layers[0]["tile"]);
        assert_eq!(v["pixels_total"], 1);
    }

    #[test]
    fn the_pack_file_that_supplies_a_tile_is_named_and_a_missing_one_is_said() {
        let f = frame();
        let plain = probe_frame(&f, &[], [84, 82, 12, 1], &Stack::default());
        let names: Vec<String> =
            plain["sources"].as_array().unwrap().iter().map(|s| s["tile"].as_str().unwrap().to_string()).collect();
        assert_eq!(names.len(), 2, "two tiles over a 12 pixel run");
        let first = names[0].clone();
        let dir = scratch("tile");
        std::fs::write(dir.join(format!("{first}.png")), png16()).unwrap();
        let stack = Stack::open(std::slice::from_ref(&dir)).unwrap();
        let v = probe_frame(&f, &[], [84, 82, 12, 1], &stack);
        for s in v["sources"].as_array().unwrap() {
            if s["tile"] == first.as_str() {
                assert_eq!(s["pack"]["supplied"], true);
                assert_eq!(s["pack"]["kind"], "tile");
                assert_eq!(s["pack"]["file"], format!("{first}.png"));
            } else {
                assert_eq!(s["pack"]["supplied"], false);
                assert!(s["pack"]["file"].is_null());
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_region_covering_the_cell_supplies_it_and_is_named() {
        let f = frame();
        let key = probe_frame(&f, &[], [90, 82, 1, 1], &Stack::default())["sources"][0]["tile"]
            .as_str()
            .unwrap()
            .trim_start_matches("tile_")
            .to_string();
        let dir = scratch("region");
        std::fs::write(dir.join("pack.txt"), "format\t2\nuses\tregions\n").unwrap();
        std::fs::write(dir.join("regions.txt"), format!("region strip 1x1 cells={key} layer=high\n")).unwrap();
        std::fs::write(dir.join("region_strip.png"), png16()).unwrap();
        let stack = Stack::open(std::slice::from_ref(&dir)).unwrap();
        let v = probe_frame(&f, &[], [90, 82, 1, 1], &stack);
        let l = &v["pixels"][0]["layers"][0];
        assert_eq!(l["region"], "strip", "{v}");
        assert_eq!(l["pack"]["kind"], "region");
        assert_eq!(l["pack"]["file"], "region_strip.png");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_textures_variant_wins_over_its_plain_image() {
        let dir = scratch("tex");
        let (key, pal) = (0xabcdu64, 7u32);
        std::fs::write(dir.join(format!("tex_{key:016x}_16x16.png")), png16()).unwrap();
        let stack = Stack::open(std::slice::from_ref(&dir)).unwrap();
        assert_eq!(stack.texture(key, pal, 16, 16).unwrap().kind, "texture");
        std::fs::write(dir.join(format!("{}.png", model2::enhance::variant_name(key, pal, 16, 16))), png16()).unwrap();
        let stack = Stack::open(std::slice::from_ref(&dir)).unwrap();
        assert_eq!(stack.texture(key, pal, 16, 16).unwrap().kind, "variant");
        assert_eq!(stack.texture(key, 8, 16, 16).unwrap().kind, "texture");
        assert!(stack.texture(key + 1, pal, 16, 16).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
