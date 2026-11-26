//! `remodel img VERB`: the per-pixel jobs agents otherwise do with shell
//! and ImageMagick (docs/uplift/agent-environment.md, "Image tools in the
//! binary"), in Rust and the same on every platform, with `--json`.
//!
//!   img sheet OUT.png IMG.. [--cols N] [--cell PX] [--labels A,B,..] [--bg #rrggbb] [--replace]
//!   img diff A.png B.png [--threshold N] [--max-changed PCT] [--out DIFF.png] [--replace]
//!   img stats IMG.png [--region X,Y,W,H]
//!
//! No ROM, no GPU. Nothing is overwritten without `--replace`, and never an
//! input. Wrong requests are errors that name the near matches; the answer is
//! the studio's (`studio_answer`): exit 0 done, 1 a gate failed (`diff
//! --max-changed`), 2 a wrong request, 3 a missing file.

#![allow(clippy::needless_range_loop, clippy::chunks_exact_to_as_chunks)]

use crate::studio_answer::{unknown, Answer, Code};
use crate::studio_img::{self as img, Image};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

const USAGE: &str = "\
remodel img VERB ... [--json]
  sheet OUT.png IMG.. [--cols N] [--cell PX] [--labels A,B,..] [--bg #rrggbb] [--replace]
        a contact sheet, each image fitted in a PX square (default 256) with its name under it
  diff A.png B.png [--threshold N] [--max-changed PCT] [--out DIFF.png] [--replace]
        changed share (a pixel counts when its channels differ by more than N in all, default 6),
        mean and largest difference, the box of what changed; --max-changed: exit 1 above PCT
  stats IMG.png [--region X,Y,W,H]
        size, how much is opaque, transparent or partly so, mean colour and brightness, colours used
";

const VERBS: &[(&str, &[&str])] = &[
    ("sheet", &["--cols", "--cell", "--labels", "--bg", "--replace"]),
    ("diff", &["--threshold", "--max-changed", "--out", "--replace"]),
    ("stats", &["--region"]),
];

struct Opts {
    pos: Vec<String>,
    vals: HashMap<String, String>,
    sw: Vec<String>,
}

impl Opts {
    fn num<T: std::str::FromStr>(&self, k: &str) -> Result<Option<T>, Answer> {
        match self.vals.get(k) {
            None => Ok(None),
            Some(v) => v
                .parse::<T>()
                .map(Some)
                .map_err(|_| Answer::fail(Code::BadRequest, format!("{k}: `{v}` is not a number"))),
        }
    }
}

/// Which options are switches (no value).
const SWITCHES: &[&str] = &["--replace", "--json"];

fn parse(verb: &str, args: &[String], allowed: &[&str]) -> Result<Opts, Answer> {
    let mut o = Opts { pos: vec![], vals: HashMap::new(), sw: vec![] };
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if !a.starts_with("--") {
            o.pos.push(a.clone());
            continue;
        }
        if a != "--json" && !allowed.contains(&a.as_str()) {
            let all: Vec<&str> = allowed.to_vec();
            return Err(Answer::fail(
                Code::BadRequest,
                format!("img {verb}: {}\n(its options: {})", unknown("option", a, &all), allowed.join(" ")),
            ));
        }
        if SWITCHES.contains(&a.as_str()) {
            o.sw.push(a.clone());
        } else {
            let Some(v) = it.next() else {
                return Err(Answer::fail(Code::BadRequest, format!("img {verb}: {a} wants a value")));
            };
            o.vals.insert(a.clone(), v.clone());
        }
    }
    Ok(o)
}

/// Run an `img` command: its answer, and whether `--json` was asked.
pub fn run(args: &[String]) -> (Answer, bool) {
    let json = args.iter().any(|a| a == "--json");
    let Some(verb) = args.first().filter(|a| !a.starts_with("--")) else {
        return (Answer::fail(Code::BadRequest, format!("img: which verb?\n{USAGE}")), json);
    };
    let Some((_, allowed)) = VERBS.iter().find(|(n, _)| n == verb) else {
        let names: Vec<&str> = VERBS.iter().map(|(n, _)| *n).collect();
        return (Answer::fail(Code::BadRequest, format!("img: {}\n{USAGE}", unknown("verb", verb, &names))), json);
    };
    let ans = parse(verb, &args[1..], allowed).and_then(|o| match verb.as_str() {
        "sheet" => sheet(&o),
        "diff" => diff(&o),
        _ => stats(&o),
    });
    (ans.unwrap_or_else(|e| e), json)
}

/// Print the answer as `--json` asks, or as text (`key<TAB>value` lines and
/// the error), and give the exit code.
pub fn finish(a: Answer, json: bool) -> std::process::ExitCode {
    if json {
        a.print_json();
    } else {
        if let Some(e) = &a.error {
            eprintln!("error: {e}");
        }
        for w in &a.warnings {
            eprintln!("warning: {w}");
        }
        if let Value::Object(m) = &a.data {
            for (k, v) in m {
                match v {
                    Value::Array(rows) => {
                        for r in rows {
                            println!("{k}\t{r}");
                        }
                    }
                    Value::String(s) => println!("{k}\t{s}"),
                    other => println!("{k}\t{other}"),
                }
            }
        }
        if let Some(n) = &a.next {
            eprintln!("next: {n}");
        }
    }
    a.exit_code()
}

fn load(path: &str) -> Result<Image, Answer> {
    let p = Path::new(path);
    if !p.is_file() {
        return Err(Answer::fail(Code::Missing, format!("{path}: not found")));
    }
    img::load(p).map_err(|e| Answer::fail(Code::BadRequest, format!("{e} (not a PNG this tool reads)")))
}

/// Refuse to overwrite `out` without `--replace`, and never an input.
fn may_write(out: &str, inputs: &[&str], o: &Opts) -> Result<(), Answer> {
    let same = |a: &str, b: &str| {
        a == b || matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
    };
    if inputs.iter().any(|i| same(i, out)) {
        return Err(Answer::fail(
            Code::BadRequest,
            format!("{out} is one of the inputs; an input is never overwritten"),
        ));
    }
    if Path::new(out).exists() && !o.sw.iter().any(|s| s == "--replace") {
        let mut a = Answer::fail(Code::BadRequest, format!("{out} exists; nothing is overwritten without --replace"));
        a.next = Some("run it again with --replace, or name another file".into());
        return Err(a);
    }
    Ok(())
}

// ---------------------------------------------------------------- stats

fn stats(o: &Opts) -> Result<Answer, Answer> {
    let [file] = o.pos.as_slice() else {
        return Err(Answer::fail(Code::BadRequest, format!("img stats: one image, not {}\n{USAGE}", o.pos.len())));
    };
    let full = load(file)?;
    let (x0, y0, w, h) = match o.vals.get("--region") {
        None => (0, 0, full.width, full.height),
        Some(r) => {
            let n: Vec<usize> = r.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            match n[..] {
                [x, y, w, h] if w > 0 && h > 0 && x + w <= full.width && y + h <= full.height => (x, y, w, h),
                _ => {
                    return Err(Answer::fail(
                        Code::BadRequest,
                        format!("--region: `{r}` is not X,Y,W,H inside the {}x{} image", full.width, full.height),
                    ))
                }
            }
        }
    };
    let im =
        if (x0, y0, w, h) == (0, 0, full.width, full.height) { full.clone() } else { img::crop(&full, x0, y0, w, h) };
    let n = (im.width * im.height).max(1);
    let (mut opaque, mut clear, mut partial) = (0usize, 0usize, 0usize);
    let mut sum = [0f64; 3];
    let (mut lmin, mut lmax, mut lsum) = (255f32, 0f32, 0f64);
    let mut bins = [0usize; 8];
    let mut colours: HashMap<[u8; 4], usize> = HashMap::new();
    let mut alphas = [false; 256];
    // bounding box of what is not fully transparent
    let (mut bx0, mut by0, mut bx1, mut by1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    for (i, p) in im.rgba.chunks_exact(4).enumerate() {
        alphas[p[3] as usize] = true;
        match p[3] {
            255 => opaque += 1,
            0 => clear += 1,
            _ => partial += 1,
        }
        if p[3] != 0 {
            let (x, y) = (i % im.width, i / im.width);
            bx0 = bx0.min(x);
            by0 = by0.min(y);
            bx1 = bx1.max(x);
            by1 = by1.max(y);
            for c in 0..3 {
                sum[c] += p[c] as f64;
            }
            let l = 0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32;
            lmin = lmin.min(l);
            lmax = lmax.max(l);
            lsum += l as f64;
            bins[(l as usize * 8 / 256).min(7)] += 1;
        }
        if colours.len() < 1_000_000 || colours.contains_key(&[p[0], p[1], p[2], p[3]]) {
            *colours.entry([p[0], p[1], p[2], p[3]]).or_insert(0) += 1;
        }
    }
    let seen = (opaque + partial).max(1);
    let mut top: Vec<(&[u8; 4], &usize)> = colours.iter().collect();
    top.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    let top: Vec<Value> = top
        .iter()
        .take(5)
        .map(|(c, k)| {
            json!({"colour": format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]), "alpha": c[3], "share": round(**k as f64 / n as f64, 4)})
        })
        .collect();
    let data = json!({
        "file": file,
        "width": im.width,
        "height": im.height,
        "region": [x0, y0, w, h],
        "opaque": round(opaque as f64 / n as f64, 4),
        "transparent": round(clear as f64 / n as f64, 4),
        "partial": round(partial as f64 / n as f64, 4),
        "alpha_levels": alphas.iter().filter(|a| **a).count(),
        "mean_rgb": [round(sum[0] / seen as f64, 1), round(sum[1] / seen as f64, 1), round(sum[2] / seen as f64, 1)],
        "luma_mean": round(lsum / seen as f64, 1),
        "luma_min": if seen > 0 && lmax >= lmin { round(lmin as f64, 1) } else { 0.0 },
        "luma_max": round(lmax as f64, 1),
        "luma_histogram": bins.iter().map(|b| round(*b as f64 / seen as f64, 4)).collect::<Vec<_>>(),
        "colours": colours.len(),
        "top_colours": top,
        "bbox": if bx0 == usize::MAX { Value::Null } else { json!([bx0, by0, bx1 - bx0 + 1, by1 - by0 + 1]) },
    });
    Ok(Answer::done(data))
}

fn round(v: f64, d: i32) -> f64 {
    let k = 10f64.powi(d);
    (v * k).round() / k
}

// ----------------------------------------------------------------- diff

fn diff(o: &Opts) -> Result<Answer, Answer> {
    let [fa, fb] = o.pos.as_slice() else {
        return Err(Answer::fail(Code::BadRequest, format!("img diff: two images, not {}\n{USAGE}", o.pos.len())));
    };
    let threshold: u32 = o.num("--threshold")?.unwrap_or(6);
    let max_changed: Option<f64> = o.num("--max-changed")?;
    let (a, b) = (load(fa)?, load(fb)?);
    if let Some(out) = o.vals.get("--out") {
        may_write(out, &[fa, fb], o)?;
    }
    let same_size = (a.width, a.height) == (b.width, b.height);
    let mut warnings = vec![];
    let (changed, mean, max, bbox, heat);
    if !same_size {
        warnings.push(format!(
            "sizes differ ({}x{} and {}x{}): nothing to compare pixel by pixel",
            a.width, a.height, b.width, b.height
        ));
        (changed, mean, max, bbox, heat) = (a.width * a.height, 255.0, 255u32, None, None);
    } else {
        let (mut n, mut sum, mut mx) = (0usize, 0u64, 0u32);
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
        let mut h = img::blank(a.width, a.height, [0, 0, 0, 255]);
        for (i, (p, q)) in a.rgba.chunks_exact(4).zip(b.rgba.chunks_exact(4)).enumerate() {
            let d: u32 = (0..4).map(|c| (p[c] as i32 - q[c] as i32).unsigned_abs()).max().unwrap_or(0);
            let ds: u32 = (0..3).map(|c| (p[c] as i32 - q[c] as i32).unsigned_abs()).sum();
            sum += ds as u64;
            mx = mx.max(d);
            // Brightness shows how much; changed pixels are tinted red.
            let v = (d * 4).min(255) as u8;
            let px = &mut h.rgba[i * 4..i * 4 + 4];
            if ds > threshold || (p[3] as i32 - q[3] as i32).unsigned_abs() > threshold {
                n += 1;
                let (x, y) = (i % a.width, i / a.width);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
                px.copy_from_slice(&[255, v / 2, v / 2, 255]);
            } else {
                px.copy_from_slice(&[v, v, v, 255]);
            }
        }
        let px = (a.width * a.height).max(1);
        changed = n;
        mean = sum as f64 / (3 * px) as f64;
        max = mx;
        bbox = (n > 0).then(|| json!([x0, y0, x1 - x0 + 1, y1 - y0 + 1]));
        heat = Some(h);
    }
    let px = (a.width * a.height).max(1);
    let pct = 100.0 * changed as f64 / px as f64;
    if let (Some(out), Some(h)) = (o.vals.get("--out"), &heat) {
        img::save(Path::new(out), h).map_err(|e| Answer::fail(Code::Internal, e))?;
    }
    let mut data = json!({
        "a": fa, "b": fb,
        "a_size": [a.width, a.height], "b_size": [b.width, b.height],
        "same_size": same_size,
        "identical": same_size && changed == 0 && max == 0,
        "changed_px": changed,
        "changed_pct": round(pct, 3),
        "mean_abs_diff": round(mean, 3),
        "max_diff": max,
        "bbox": bbox,
        "threshold": threshold,
    });
    if let Some(out) = o.vals.get("--out").filter(|_| heat.is_some()) {
        data["picture"] = json!(out);
    }
    let mut ans = Answer::done(data);
    ans.warnings = warnings;
    if let Some(m) = max_changed {
        if pct > m || !same_size {
            ans.code = Code::GateFailed;
            ans.error = Some(format!("{pct:.3}% of the pixels changed, more than --max-changed {m}"));
        }
    }
    Ok(ans)
}

// ---------------------------------------------------------------- sheet

/// A 3x5 pixel font: A-Z, 0-9 and `-_.:/`. Anything else is a blank.
fn glyph(c: char) -> [&'static str; 5] {
    match c.to_ascii_uppercase() {
        'A' => [".#.", "#.#", "###", "#.#", "#.#"],
        'B' => ["##.", "#.#", "##.", "#.#", "##."],
        'C' => [".##", "#..", "#..", "#..", ".##"],
        'D' => ["##.", "#.#", "#.#", "#.#", "##."],
        'E' => ["###", "#..", "##.", "#..", "###"],
        'F' => ["###", "#..", "##.", "#..", "#.."],
        'G' => [".##", "#..", "#.#", "#.#", ".##"],
        'H' => ["#.#", "#.#", "###", "#.#", "#.#"],
        'I' => ["###", ".#.", ".#.", ".#.", "###"],
        'J' => ["..#", "..#", "..#", "#.#", ".#."],
        'K' => ["#.#", "#.#", "##.", "#.#", "#.#"],
        'L' => ["#..", "#..", "#..", "#..", "###"],
        'M' => ["#.#", "###", "###", "#.#", "#.#"],
        'N' => ["##.", "#.#", "#.#", "#.#", "#.#"],
        'O' => [".#.", "#.#", "#.#", "#.#", ".#."],
        'P' => ["##.", "#.#", "##.", "#..", "#.."],
        'Q' => [".#.", "#.#", "#.#", "###", ".##"],
        'R' => ["##.", "#.#", "##.", "#.#", "#.#"],
        'S' => [".##", "#..", ".#.", "..#", "##."],
        'T' => ["###", ".#.", ".#.", ".#.", ".#."],
        'U' => ["#.#", "#.#", "#.#", "#.#", "###"],
        'V' => ["#.#", "#.#", "#.#", "#.#", ".#."],
        'W' => ["#.#", "#.#", "###", "###", "#.#"],
        'X' => ["#.#", "#.#", ".#.", "#.#", "#.#"],
        'Y' => ["#.#", "#.#", ".#.", ".#.", ".#."],
        'Z' => ["###", "..#", ".#.", "#..", "###"],
        '0' => ["###", "#.#", "#.#", "#.#", "###"],
        '1' => [".#.", "##.", ".#.", ".#.", "###"],
        '2' => ["##.", "..#", ".#.", "#..", "###"],
        '3' => ["##.", "..#", ".#.", "..#", "##."],
        '4' => ["#.#", "#.#", "###", "..#", "..#"],
        '5' => ["###", "#..", "##.", "..#", "##."],
        '6' => [".##", "#..", "###", "#.#", "###"],
        '7' => ["###", "..#", ".#.", ".#.", ".#."],
        '8' => ["###", "#.#", "###", "#.#", "###"],
        '9' => ["###", "#.#", "###", "..#", "##."],
        '-' => ["...", "...", "###", "...", "..."],
        '_' => ["...", "...", "...", "...", "###"],
        '.' => ["...", "...", "...", "...", ".#."],
        ':' => ["...", ".#.", "...", ".#.", "..."],
        '/' => ["..#", "..#", ".#.", "#..", "#.."],
        _ => ["...", "...", "...", "...", "..."],
    }
}

/// `text` drawn at (x, y), each font pixel k x k, clipped to `max_w` pixels.
pub(crate) fn draw_text(im: &mut Image, text: &str, x: usize, y: usize, k: usize, max_w: usize, colour: [u8; 3]) {
    let adv = 4 * k;
    for (i, c) in text.chars().enumerate() {
        if (i + 1) * adv > max_w {
            break;
        }
        for (gy, row) in glyph(c).iter().enumerate() {
            for (gx, ch) in row.chars().enumerate() {
                if ch != '#' {
                    continue;
                }
                for dy in 0..k {
                    for dx in 0..k {
                        let (px, py) = (x + i * adv + gx * k + dx, y + gy * k + dy);
                        if px < im.width && py < im.height {
                            let d = (py * im.width + px) * 4;
                            im.rgba[d..d + 3].copy_from_slice(&colour);
                            im.rgba[d + 3] = 255;
                        }
                    }
                }
            }
        }
    }
}

fn sheet(o: &Opts) -> Result<Answer, Answer> {
    let Some((out, files)) = o.pos.split_first().filter(|(_, f)| !f.is_empty()) else {
        return Err(Answer::fail(Code::BadRequest, format!("img sheet: OUT.png and at least one image\n{USAGE}")));
    };
    let inputs: Vec<&str> = files.iter().map(String::as_str).collect();
    let cell: usize = o.num("--cell")?.unwrap_or(256);
    if !(16..=2048).contains(&cell) {
        return Err(Answer::fail(Code::BadRequest, "--cell: 16 to 2048 pixels"));
    }
    let cols: usize = o.num("--cols")?.unwrap_or(files.len().clamp(1, 4)).clamp(1, 64);
    let bg = match o.vals.get("--bg") {
        Some(c) => img::parse_colour(c).map_err(|e| Answer::fail(Code::BadRequest, format!("--bg: {e}")))?,
        None => [32, 32, 36],
    };
    let labels: Vec<String> = match o.vals.get("--labels") {
        Some(l) => {
            let v: Vec<String> = l.split(',').map(str::to_string).collect();
            if v.len() != files.len() {
                return Err(Answer::fail(
                    Code::BadRequest,
                    format!("--labels: {} labels for {} images", v.len(), files.len()),
                ));
            }
            v
        }
        None => files
            .iter()
            .map(|f| PathBuf::from(f).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default())
            .collect(),
    };
    let imgs: Vec<Image> = files.iter().map(|f| load(f)).collect::<Result<_, _>>()?;
    may_write(out, &inputs, o)?;
    let (k, pad) = (if cell >= 128 { 2 } else { 1 }, 6);
    let label_h = 5 * k + 2 * pad / 2;
    let rows = imgs.len().div_ceil(cols);
    let (cw, ch) = (cell + pad * 2, cell + label_h + pad * 2);
    let mut sheet = img::blank(cols * cw, rows * ch, [bg[0], bg[1], bg[2], 255]);
    let mut cells = vec![];
    for (i, (im, label)) in imgs.iter().zip(&labels).enumerate() {
        let (cx, cy) = ((i % cols) * cw + pad, (i / cols) * ch + pad);
        // Fit: whole-number enlargement for small art (pixels stay square), else resampled.
        let s = (cell as f32 / im.width.max(im.height) as f32).max(f32::MIN_POSITIVE);
        let fitted = if s >= 2.0 {
            img::nearest(im, s.floor() as usize)
        } else if s < 1.0 {
            img::resize(im, ((im.width as f32 * s) as usize).max(1), ((im.height as f32 * s) as usize).max(1))
        } else {
            im.clone()
        };
        let (ox, oy) = (cx + (cell - fitted.width.min(cell)) / 2, cy + (cell - fitted.height.min(cell)) / 2);
        for y in 0..fitted.height.min(cell) {
            for x in 0..fitted.width.min(cell) {
                let sp = &fitted.rgba[(y * fitted.width + x) * 4..][..4];
                // A checker shows what is transparent.
                let chk = if (x / 8 + y / 8) % 2 == 0 { 70u32 } else { 100 };
                let d = ((oy + y) * sheet.width + ox + x) * 4;
                let a = sp[3] as u32;
                for c in 0..3 {
                    sheet.rgba[d + c] = ((sp[c] as u32 * a + chk * (255 - a)) / 255) as u8;
                }
            }
        }
        draw_text(&mut sheet, label, cx, cy + cell + pad / 2, k, cell, [230, 230, 230]);
        cells.push(json!({
            "n": i + 1, "label": label, "file": files[i], "width": im.width, "height": im.height,
            "box": [ox, oy, fitted.width.min(cell), fitted.height.min(cell)],
        }));
    }
    img::save(Path::new(out), &sheet).map_err(|e| Answer::fail(Code::Internal, e))?;
    Ok(Answer::done(json!({
        "picture": out, "width": sheet.width, "height": sheet.height, "cols": cols, "rows": rows, "cells": cells,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../out/scratch/img-tests"))
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn save(d: &Path, n: &str, im: &Image) -> String {
        let p = d.join(n);
        img::save(&p, im).unwrap();
        p.to_string_lossy().to_string()
    }

    #[test]
    fn stats_of_a_cutout() {
        let d = tmp("stats");
        let mut im = img::blank(4, 4, [200, 100, 50, 255]);
        for i in 0..8 {
            im.rgba[i * 4 + 3] = 0; // top half transparent
        }
        let f = save(&d, "c.png", &im);
        let (r, _) = run(&a(&["stats", &f]));
        assert!(r.ok(), "{:?}", r.error);
        assert_eq!(r.data["transparent"], 0.5);
        assert_eq!(r.data["opaque"], 0.5);
        assert_eq!(r.data["bbox"], json!([0, 2, 4, 2]));
        assert_eq!(r.data["mean_rgb"], json!([200.0, 100.0, 50.0]));
        let (r, _) = run(&a(&["stats", &f, "--region", "0,2,4,2"]));
        assert_eq!(r.data["opaque"], 1.0);
        let (r, _) = run(&a(&["stats", &f, "--region", "0,2,9,2"]));
        assert_eq!(r.code, Code::BadRequest);
        let (r, _) = run(&a(&["stats", &d.join("nope.png").to_string_lossy()]));
        assert_eq!(r.code, Code::Missing);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn diff_counts_and_gates() {
        let d = tmp("diff");
        let base = img::blank(10, 10, [10, 10, 10, 255]);
        let mut other = base.clone();
        for y in 2..4 {
            for x in 5..9 {
                other.rgba[(y * 10 + x) * 4] = 200;
            }
        }
        let (fa, fb) = (save(&d, "a.png", &base), save(&d, "b.png", &other));
        let (r, _) = run(&a(&["diff", &fa, &fb]));
        assert!(r.ok());
        assert_eq!(r.data["changed_px"], 8);
        assert_eq!(r.data["changed_pct"], 8.0);
        assert_eq!(r.data["max_diff"], 190);
        assert_eq!(r.data["bbox"], json!([5, 2, 4, 2]));
        let (r, _) = run(&a(&["diff", &fa, &fa]));
        assert_eq!(r.data["identical"], true);
        // a gate
        let (r, _) = run(&a(&["diff", &fa, &fb, "--max-changed", "5"]));
        assert_eq!(r.code, Code::GateFailed);
        assert!(!r.ok());
        let (r, _) = run(&a(&["diff", &fa, &fb, "--max-changed", "9"]));
        assert!(r.ok());
        // a picture, never over an existing file or an input
        let out = d.join("h.png").to_string_lossy().to_string();
        let (r, _) = run(&a(&["diff", &fa, &fb, "--out", &out]));
        assert!(r.ok() && Path::new(&out).is_file());
        let (r, _) = run(&a(&["diff", &fa, &fb, "--out", &out]));
        assert_eq!(r.code, Code::BadRequest);
        assert!(r.error.clone().unwrap().contains("--replace"));
        let (r, _) = run(&a(&["diff", &fa, &fb, "--out", &fa, "--replace"]));
        assert_eq!(r.code, Code::BadRequest);
        // different sizes
        let small = save(&d, "s.png", &img::blank(5, 5, [0, 0, 0, 255]));
        let (r, _) = run(&a(&["diff", &fa, &small]));
        assert_eq!(r.data["same_size"], false);
        assert!(!r.warnings.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn sheet_lays_out_and_labels() {
        let d = tmp("sheet");
        let f1 = save(&d, "wall.png", &img::blank(8, 8, [200, 0, 0, 255]));
        let f2 = save(&d, "sky.png", &img::blank(16, 8, [0, 0, 200, 255]));
        let out = d.join("sheet.png").to_string_lossy().to_string();
        let (r, _) = run(&a(&["sheet", &out, &f1, &f2, "--cell", "64", "--cols", "2"]));
        assert!(r.ok(), "{:?}", r.error);
        assert_eq!(r.data["cells"].as_array().unwrap().len(), 2);
        assert_eq!(r.data["cells"][0]["label"], "wall");
        let s = img::load(Path::new(&out)).unwrap();
        assert_eq!(s.width, r.data["width"].as_u64().unwrap() as usize);
        // the first cell holds the red picture, enlarged whole-number (64/8 = 8)
        let b = r.data["cells"][0]["box"].as_array().unwrap();
        let (bx, by) = (b[0].as_u64().unwrap() as usize, b[1].as_u64().unwrap() as usize);
        assert_eq!(&s.rgba[(by * s.width + bx) * 4..][..3], &[200, 0, 0]);
        // labels are drawn: some light pixels under the cell
        let under = (by + 64 + 3) * s.width * 4;
        assert!(s.rgba[under..under + s.width * 4 * 6].chunks_exact(4).any(|p| p[0] > 200 && p[2] > 200));
        // nothing overwritten without --replace, no input as output
        let (r, _) = run(&a(&["sheet", &out, &f1]));
        assert_eq!(r.code, Code::BadRequest);
        let (r, _) = run(&a(&["sheet", &out, &f1, "--replace"]));
        assert!(r.ok());
        let (r, _) = run(&a(&["sheet", &f1, &f1, "--replace"]));
        assert_eq!(r.code, Code::BadRequest);
        let (r, _) = run(&a(&["sheet", &out, &f1, &f2, "--labels", "one"]));
        assert_eq!(r.code, Code::BadRequest);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn strict_names() {
        let (r, _) = run(&a(&["shee"]));
        assert_eq!(r.code, Code::BadRequest);
        assert!(r.error.clone().unwrap().contains("did you mean sheet"));
        let (r, _) = run(&a(&["diff", "a", "b", "--thresold", "3"]));
        assert!(r.error.clone().unwrap().contains("did you mean --threshold"));
        let (r, j) = run(&a(&["stats", "--json"]));
        assert!(j && r.code == Code::BadRequest);
    }
}
