//! `studio region-capture` and `studio regions`: 2D region overrides in the
//! uplift studio (docs/uplift-studio.md, "Regions"; the pack side is
//! docs/texture-packs.md, "Regions"). A region is cut out of a captured
//! frame's tile maps: the keys of the tile patterns at the cells of a
//! rectangle, found again anywhere in a map. The studio keeps each in
//! `regions/NAME/`:
//!
//! ```text
//! region.txt        the region's line of regions.txt
//! original.png      the region as the game draws it, enlarged (for reference)
//! notes.txt         where it was cut from, and the size to draw it at
//! replacement.png   the artist's image (any size over 8 pixels a tile); optional
//! ```
//!
//! `studio pack` writes the regions that have a replacement into the pack.
//! This module is a child of `studio` and uses its project and argument
//! types.

use super::*;
use model2::capture::Frame;
use model2::regions::{self as rg, CellKey, Layer, Plane, RegionDef, RegionSet};
use model2::video;
use std::sync::Arc;

const WIDTH: usize = video::WIDTH;
const HEIGHT: usize = video::HEIGHT;

fn region_dir(p: &Project, name: &str) -> PathBuf {
    p.p("regions").join(name)
}

/// A frame of a capture directory (from `remodel --capture`, a whole-frame capture or a
/// sequence).
fn load_frame(root: &Path, n: u64) -> Result<Frame, String> {
    let d = root.join(format!("{n:06}"));
    if d.join("meta.txt").is_file() {
        return Frame::load(&d).map_err(|e| format!("{}: {e}", d.display()));
    }
    let frames = model2::capture::frames(root).map_err(|e| format!("{}: {e}", root.display()))?;
    for f in frames {
        let (_, f) = f.map_err(|e| format!("{}: {e}", root.display()))?;
        if f.frame == n {
            return Ok(f);
        }
    }
    Err(format!("{}: no frame {n} (--capture-dir wants the directory a `remodel --capture` run wrote)", root.display()))
}

/// Output scale and the game-pixel padding of the texmap frame `stem`
/// (frame width over scale is 496 plus twice the padding), from its picture.
fn frame_geometry(p: &Project, a: &Args, stem: &str) -> Result<(f32, f32), String> {
    if a.has("--game-px") {
        return Ok((1.0, 0.0));
    }
    let explicit: Option<f32> = a.num("--scale")?;
    let size = p
        .maps()
        .iter()
        .filter(|(n, _)| a.get("--map").is_none_or(|m| m == n))
        .map(|(_, d)| d.join("frames").join(format!("{stem}.png")))
        .find(|f| f.is_file())
        .and_then(|f| img::load(&f).ok())
        .map(|i| (i.width, i.height))
        .unwrap_or((1024, 768));
    let scale = explicit.unwrap_or(size.1 as f32 / HEIGHT as f32);
    let pad = ((size.0 as f32 / scale - WIDTH as f32) / 2.0).max(0.0);
    Ok((scale, pad))
}

fn parse_frames(v: &str) -> Result<Vec<u64>, String> {
    v.split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().trim_end_matches(".png").parse::<u64>().map_err(|_| format!("`{s}` is not a frame number")))
        .collect()
}

pub(super) fn capture(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let name = a.pos.get(1).ok_or("region-capture PROJECT NAME: what to call the region")?;
    if !rg::valid_name(name) {
        return Err(format!("`{name}` isn't a region name (lowercase letters, digits, - and _)"));
    }
    let meta = p.meta();
    let capdir = a
        .get("--capture-dir")
        .map(PathBuf::from)
        .or_else(|| meta.get("capture").map(PathBuf::from))
        .ok_or("--capture-dir DIR: the capture directory the frame is in (or `init --capture-dir DIR`)")?;
    // A catalogue as the capture directory: its saved frames' tile data,
    // numbered 1000000 * (session + 1) + frame (`catalogue captures`).
    let capdir = if crate::catalogue::is_catalogue(&capdir) {
        let ix = crate::catalogue::Index::load(&capdir)?;
        let out = capdir.join("captures");
        let n = crate::catalogue::export_captures(&ix, &out)?;
        println!("catalogue     {n} frames with tile data in {} (index.tsv: numbers)", out.display());
        out
    } else {
        capdir
    };
    let frame_no: u64 = a
        .get("--frame")
        .ok_or("--frame F")?
        .trim_end_matches(".png")
        .parse()
        .map_err(|_| "--frame: not a frame number".to_string())?;
    let stem = format!("{frame_no:06}");
    let rect = a.rect("--region")?.ok_or("--region X,Y,W,H: the frame picture's pixels the region covers")?;
    let (scale, pad) = frame_geometry(p, a, &stem)?;
    // The rectangle in game pixels (496 x 384).
    let gx0 = (rect.0 as f32 / scale - pad).round().max(0.0) as usize;
    let gy0 = (rect.1 as f32 / scale).round().max(0.0) as usize;
    let gx1_raw = ((rect.0 + rect.2) as f32 / scale - pad).round().max(0.0) as usize;
    let gy1_raw = ((rect.1 + rect.3) as f32 / scale).round().max(0.0) as usize;
    if gx0 >= WIDTH || gy0 >= HEIGHT {
        return Err(format!(
            "--region is off the frame (game pixels {gx0},{gy0}..{gx1_raw},{gy1_raw} of {WIDTH}x{HEIGHT})"
        ));
    }
    let gx1 = gx1_raw.clamp(gx0 + 1, WIDTH);
    let gy1 = gy1_raw.clamp(gy0 + 1, HEIGHT);
    let f = load_frame(&capdir, frame_no)?;
    let (tiles, palette) = (&f.tiles, &f.palette);

    // The planes to try, the cells of the rectangle in the map of each.
    struct Cut {
        plane: usize,
        map: usize,
        cx: usize,
        cy: usize,
        /// Screen position of the first included cell's corner, game pixels.
        ox: i32,
        oy: i32,
        w: usize,
        h: usize,
        keys: Vec<(u64, bool)>,
        odd: usize,
    }
    let try_plane = |plane: usize| -> Option<Cut> {
        let (ax, ay) = (gx0.min(WIDTH - 1), gy0.min(HEIGHT - 1));
        let c = video::cell_at(tiles, plane, ax, ay)?;
        let (bx, by) = (ax as i32 - c.col as i32, ay as i32 - c.row as i32);
        let covered = |lo: i32, hi: i32, o: i32, i: i32| {
            let ov = (o + 8 * i + 8).min(hi) - (o + 8 * i).max(lo);
            ov >= 4i32.min(hi - lo)
        };
        let xs: Vec<i32> = (0..64).filter(|&i| covered(gx0 as i32, gx1 as i32, bx, i)).collect();
        let ys: Vec<i32> = (0..64).filter(|&j| covered(gy0 as i32, gy1 as i32, by, j)).collect();
        let (i0, j0) = (*xs.first()?, *ys.first()?);
        let (w, h) = (xs.len(), ys.len());
        let (cx, cy) = ((c.cx + i0 as usize) & 63, (c.cy + j0 as usize) & 63);
        let mut keys = Vec::new();
        let mut odd = 0;
        for j in 0..h {
            for i in 0..w {
                keys.push(video::cell_key(tiles, palette, c.map, cx + i, cy + j));
                // Is the cell where a contiguous map layout puts it?
                let (px, py) = (bx + 8 * (i0 + i as i32) + 4, by + 8 * (j0 + j as i32) + 4);
                if (0..WIDTH as i32).contains(&px) && (0..HEIGHT as i32).contains(&py) {
                    let at = video::cell_at(tiles, plane, px as usize, py as usize);
                    if at.is_none_or(|t| t.map != c.map || t.cx != (cx + i) & 63 || t.cy != (cy + j) & 63) {
                        odd += 1;
                    }
                }
            }
        }
        Some(Cut { plane, map: c.map, cx, cy, ox: bx + 8 * i0, oy: by + 8 * j0, w, h, keys, odd })
    };
    let nonblank = |c: &Cut| c.keys.iter().filter(|k| k.0 != rg::BLANK).count();
    let cut = match a.get("--plane") {
        Some("a") => try_plane(0),
        Some("b") => try_plane(1),
        Some(o) => return Err(format!("--plane {o}: a or b")),
        None => {
            let (ca, cb) = (try_plane(0), try_plane(1));
            match (ca, cb) {
                (Some(x), Some(y)) => Some(if nonblank(&y) > nonblank(&x) { y } else { x }),
                (x, y) => x.or(y),
            }
        }
    }
    .ok_or("no tile map is drawn at the region's corner (a disabled map, or nothing there): try --plane a or b")?;
    if cut.w > 64 || cut.h > 64 {
        return Err(format!("{}x{} tiles is too big for a region (64 x 64 at most)", cut.w, cut.h));
    }
    let plane_name = if cut.plane == 0 { Plane::A } else { Plane::B };
    println!(
        "cells         {}x{} tiles of map {} (plane {}) from cell {},{}; {} of them not blank",
        cut.w,
        cut.h,
        cut.map,
        if cut.plane == 0 { 'a' } else { 'b' },
        cut.cx,
        cut.cy,
        nonblank(&cut)
    );
    if cut.odd > 0 {
        println!(
            "warning       {} cells are not where one contiguous block of the map puts them (line scroll or a split): the region may not match",
            cut.odd
        );
    }

    // Wildcards: cells that differ in the other frames, and the ones named.
    let mut cells: Vec<CellKey> =
        cut.keys.iter().map(|&(k, _)| if k == rg::BLANK { CellKey::Blank } else { CellKey::Tile(k) }).collect();
    let also = a.get("--also").map(parse_frames).transpose()?.unwrap_or_default();
    let mut others: Vec<(u64, Frame)> = Vec::new();
    for n in &also {
        let o = load_frame(&capdir, *n)?;
        let mut wild = 0;
        for j in 0..cut.h {
            for i in 0..cut.w {
                if video::cell_key(&o.tiles, &o.palette, cut.map, cut.cx + i, cut.cy + j).0 != cut.keys[j * cut.w + i].0
                    && cells[j * cut.w + i] != CellKey::Any
                {
                    cells[j * cut.w + i] = CellKey::Any;
                    wild += 1;
                }
            }
        }
        println!("also          frame {n}: {wild} more cells differ there: wildcards");
        others.push((*n, o));
    }
    if let Some(w) = a.get("--wild") {
        for pair in w.split(';').filter(|s| !s.trim().is_empty()) {
            let xy: Vec<usize> = pair.split(',').filter_map(|v| v.trim().parse().ok()).collect();
            match xy[..] {
                [x, y] if x < cut.w && y < cut.h => cells[y * cut.w + x] = CellKey::Any,
                _ => return Err(format!("--wild: `{pair}` is not a cell X,Y inside the {}x{} region", cut.w, cut.h)),
            }
        }
    }
    let layer = match a.get("--layer") {
        None | Some("any") => Layer::Any,
        Some("high") => Layer::High,
        Some("low") => Layer::Low,
        Some(o) => return Err(format!("--layer {o}: high, low or any")),
    };
    let def = RegionDef {
        name: name.clone(),
        w: cut.w,
        h: cut.h,
        cells: cells.clone(),
        plane: plane_name,
        layer,
        at: a.has("--pin").then_some((cut.map, cut.cx, cut.cy)),
        unknown: vec![],
    };
    if cells.iter().all(|c| matches!(c, CellKey::Any | CellKey::Blank)) {
        return Err(
            "the region has no tile of its own (all wildcards or blank): it would match anywhere; widen it".into()
        );
    }
    let distinct: BTreeSet<u64> =
        cells.iter().filter_map(|c| if let CellKey::Tile(k) = c { Some(*k) } else { None }).collect();
    if distinct.len() == 1 && cells.len() > 1 {
        println!(
            "warning       every tile of the region is the same pattern: it matches any run of it; widen the rectangle to take in something distinctive (or --pin)"
        );
    }

    // Where it matches: this frame and the others.
    let probe = RegionSet::new(vec![def.clone()], |_| {
        Some(Arc::new(model2::enhance::Rgba {
            width: 8 * cut.w,
            height: 8 * cut.h,
            data: vec![0; 8 * cut.w * 8 * cut.h * 4],
        }))
    });
    let report = |n: u64, f: &Frame| {
        let g = video::match_regions(&f.tiles, &f.palette, &probe);
        let mut at = Vec::new();
        for m in 0..4 {
            for c in 0..4096 {
                if g.cells[m][c] == 0 {
                    at.push(format!("{m}:{},{}", c & 63, c >> 6));
                }
            }
        }
        println!(
            "matches       frame {n}: {} place(s){}",
            at.len(),
            if at.is_empty() { String::new() } else { format!(" ({})", at.join(" ")) }
        );
        at.len()
    };
    let here = report(frame_no, &f);
    if here == 0 {
        println!("warning       the region does not match the frame it was cut from: a bug, or the layout is not contiguous in the map");
    } else if here > 1 {
        println!("note          it matches {here} places in this frame: fine for a logo drawn several times, else narrow it (--pin)");
    }
    for (n, o) in &others {
        report(*n, o);
    }

    // The reference image: the game's own picture of the region, enlarged.
    let native = video::render_frame(tiles, palette, None);
    let (cw, ch) = (cut.w * 8, cut.h * 8);
    let mut crop = img::blank(cw, ch, [255, 0, 255, 255]);
    for y in 0..ch {
        for x in 0..cw {
            let (sx, sy) = (cut.ox + x as i32, cut.oy + y as i32);
            if (0..WIDTH as i32).contains(&sx) && (0..HEIGHT as i32).contains(&sy) {
                let s = (sy as usize * WIDTH + sx as usize) * 3;
                crop.rgba[(y * cw + x) * 4..(y * cw + x) * 4 + 3].copy_from_slice(&native[s..s + 3]);
            }
        }
    }
    let k: usize = a.num("--ref-scale")?.unwrap_or(4usize).clamp(1, 16);
    let dir = region_dir(p, name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    img::save(&dir.join("original.png"), &img::nearest(&crop, k))?;
    std::fs::write(dir.join("region.txt"), format!("{}\n", def.to_line())).map_err(|e| e.to_string())?;
    let notes = format!(
        "region {name}: {w}x{h} tiles = {gw}x{gh} game pixels\n\
         cut from frame {frame_no} of {cap}, game pixels {x},{y} (map {map} cell {cx},{cy}, plane {pl})\n\
         original.png: the region as the game draws it, {k}x (magenta: off the screen)\n\
         draw replacement.png at {rw}x{rh} (4x; any size of at least {gw}x{gh} works, in proportion): it replaces the\n\
         whole rectangle, so paint the background in too; transparent pixels show the original (plane B) or what is\n\
         behind (plane A and the HUD)\n",
        w = cut.w,
        h = cut.h,
        gw = cw,
        gh = ch,
        cap = capdir.display(),
        x = cut.ox,
        y = cut.oy,
        map = cut.map,
        cx = cut.cx,
        cy = cut.cy,
        pl = if cut.plane == 0 { "a" } else { "b" },
        rw = cw * 4,
        rh = ch * 4,
    );
    std::fs::write(dir.join("notes.txt"), notes).map_err(|e| e.to_string())?;
    if let Some(f) = a.get("--image") {
        let im = img::load(Path::new(f))?;
        if im.width < cw || im.height < ch {
            return Err(format!("--image is {}x{}, under 8 pixels per tile ({cw}x{ch})", im.width, im.height));
        }
        if im.width * ch != im.height * cw {
            println!("warning       --image's proportions differ from the region's ({}x{} against {cw}x{ch}): it is cut unevenly", im.width, im.height);
        }
        copy_file(Path::new(f), &dir.join("replacement.png"))?;
        println!("replacement   {} ({}x{})", dir.join("replacement.png").display(), im.width, im.height);
    }
    println!("region        {} ({})", name, dir.display());
    Ok(true)
}

/// The regions of the project: name, size, tiles of its own, replacement.
pub(super) fn list(p: &Project, a: &Args) -> Result<bool, String> {
    p.require()?;
    let mut names: Vec<String> = std::fs::read_dir(p.p("regions"))
        .map(|d| {
            d.filter_map(|e| e.ok())
                .filter(|e| e.path().join("region.txt").is_file())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    if a.has("--tsv") {
        println!("name\ttiles\tplane\tpinned\treplacement\tstatus");
    }
    let mut have = 0;
    for n in &names {
        let dir = region_dir(p, n);
        let defs = rg::parse(&std::fs::read_to_string(dir.join("region.txt")).unwrap_or_default());
        let rep = img::load(&dir.join("replacement.png")).ok();
        let status = if rep.is_some() { "replacement" } else { "original" };
        have += rep.is_some() as usize;
        match defs {
            Ok(d) if d.len() == 1 => {
                let d = &d[0];
                println!(
                    "{}\t{}x{}\t{:?}\t{}\t{}\t{status}",
                    n,
                    d.w,
                    d.h,
                    d.plane,
                    d.at.map_or("-".to_string(), |(m, x, y)| format!("{m}:{x},{y}")),
                    rep.as_ref().map_or("-".to_string(), |r| format!("{}x{}", r.width, r.height)),
                );
            }
            Ok(_) => println!("{n}\t?\t\t\t\tbroken: region.txt wants one region line"),
            Err(e) => println!("{n}\t?\t\t\t\tbroken: {e}"),
        }
    }
    if !a.has("--tsv") {
        println!("regions       {} ({have} with a replacement)", names.len());
    }
    Ok(true)
}

/// Write the project's regions that have a replacement into the pack
/// directory: `regions.txt` and `region_NAME.png` (stale ones removed).
/// Returns how many.
pub(super) fn write_pack(p: &Project, pack: &Path) -> Result<usize, String> {
    // Forget what an earlier run wrote.
    for n in png_names(pack).into_iter().filter(|n| n.starts_with(rg::IMAGE_PREFIX)) {
        let _ = std::fs::remove_file(pack.join(format!("{n}.png")));
    }
    let _ = std::fs::remove_file(pack.join(rg::FILE));
    let mut names: Vec<String> = std::fs::read_dir(p.p("regions"))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().to_string()).collect())
        .unwrap_or_default();
    names.sort();
    let mut defs = Vec::new();
    for n in names {
        let dir = region_dir(p, &n);
        let (Ok(text), true) = (std::fs::read_to_string(dir.join("region.txt")), dir.join("replacement.png").is_file())
        else {
            continue;
        };
        let mut d = rg::parse(&text).map_err(|e| format!("regions/{n}/region.txt: {e}"))?;
        if d.len() != 1 {
            return Err(format!("regions/{n}/region.txt: wants exactly one region line"));
        }
        copy_file(&dir.join("replacement.png"), &pack.join(format!("{}{n}.png", rg::IMAGE_PREFIX)))?;
        defs.push(d.remove(0));
    }
    if !defs.is_empty() {
        std::fs::write(pack.join(rg::FILE), rg::to_text(&defs)).map_err(|e| e.to_string())?;
    }
    Ok(defs.len())
}
