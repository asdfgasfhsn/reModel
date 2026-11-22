//! `remodel coverage`: how much of a game's polygon ROM and texture ROM its
//! tours have read, by region, with the unread stretches listed by address
//! range (docs/uplift-studio.md, "Tours and coverage").
//!
//! Each `tour run` writes one map file, `DIR/LABEL.map`: the words the
//! tour read (as merged ranges), the runs of blank words (all zero or all
//! ones, at least [`BLANK_BYTES`] long), the ROMs' sizes, and the tour's
//! final state hash. Maps hold addresses and counts only, never ROM
//! contents. `coverage` merges the maps of the tours the game has now
//! (union), so it needs no ROM, and re-running a tour replaces its map
//! (the report is the same however often it is run).
//!
//! Counts are in bytes: the polygon ROM is read in 32-bit words and the
//! texture ROM in 16-bit words. "Read" means the geometrizer read the
//! polygon ROM (not polygon RAM), and the rasteriser read a texture
//! header or UV list from the texture ROM; the texels themselves come from
//! texture RAM, filled from the data ROM, and are not in these maps.

use crate::answer::Answer;
use crate::tour::{self, answer, fail, Finish};
use model2::readmap::Ranges;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// A run of blank words is at least this many bytes: shorter runs of
/// zeros are ordinary data (a model's zero coordinates).
pub const BLANK_BYTES: usize = 256;
/// A region of the report: 1 MiB of ROM.
const REGION_BYTES: u64 = 1 << 20;
/// Unread stretches listed by default (`--top N`; all of them with `--top 0`).
const TOP: usize = 20;

/// FNV-1a of a tour's line, to notice that it changed since its map.
pub fn fnv(s: &str) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for b in s.bytes() {
        h = (h ^ b as u64).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// One tour's map.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MapFile {
    pub game: String,
    pub tour: String,
    /// [`fnv`] of the tour's line when it ran.
    pub spec: String,
    pub frames: u64,
    pub hash: u64,
    /// ROM sizes in words (32-bit for polygons, 16-bit for textures).
    pub poly_words: u64,
    pub tex_words: u64,
    pub poly_read: Ranges,
    pub tex_read: Ranges,
    pub poly_blank: Ranges,
    pub tex_blank: Ranges,
}

const HEADER: &str = "remodel-readmap 1";

impl MapFile {
    pub fn path(dir: &Path, tour: &str) -> PathBuf {
        dir.join(format!("{tour}.map"))
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut s = format!(
            "{HEADER}\ngame {}\ntour {}\nspec {}\nframes {}\nhash {:016x}\npoly-words {}\ntex-words {}\n",
            self.game, self.tour, self.spec, self.frames, self.hash, self.poly_words, self.tex_words
        );
        self.poly_read.write("P", &mut s);
        self.tex_read.write("T", &mut s);
        self.poly_blank.write("PB", &mut s);
        self.tex_blank.write("TB", &mut s);
        std::fs::write(path, s)
    }

    pub fn load(path: &Path) -> Result<MapFile, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::parse(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn parse(text: &str) -> Result<MapFile, String> {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return Err("not a read map (or one from a newer reModel)".into());
        }
        let mut m = MapFile {
            game: String::new(),
            tour: String::new(),
            spec: String::new(),
            frames: 0,
            hash: 0,
            poly_words: 0,
            tex_words: 0,
            poly_read: Ranges::default(),
            tex_read: Ranges::default(),
            poly_blank: Ranges::default(),
            tex_blank: Ranges::default(),
        };
        let (mut pr, mut tr, mut pb, mut tb) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for (n, line) in lines.enumerate() {
            let bad = || format!("line {}: {line:?}", n + 2);
            let (key, rest) = line.split_once(' ').ok_or_else(bad)?;
            let num = |s: &str| s.parse::<u64>().map_err(|_| bad());
            match key {
                "game" => m.game = rest.to_string(),
                "tour" => m.tour = rest.to_string(),
                "spec" => m.spec = rest.to_string(),
                "frames" => m.frames = num(rest)?,
                "hash" => m.hash = u64::from_str_radix(rest, 16).map_err(|_| bad())?,
                "poly-words" => m.poly_words = num(rest)?,
                "tex-words" => m.tex_words = num(rest)?,
                "P" | "T" | "PB" | "TB" => {
                    let (s, e) = rest.split_once(' ').ok_or_else(bad)?;
                    let r = (num(s)?, num(e)?);
                    match key {
                        "P" => pr.push(r),
                        "T" => tr.push(r),
                        "PB" => pb.push(r),
                        _ => tb.push(r),
                    }
                }
                _ => return Err(bad()),
            }
        }
        m.poly_read = Ranges::from_unsorted(pr);
        m.tex_read = Ranges::from_unsorted(tr);
        m.poly_blank = Ranges::from_unsorted(pb);
        m.tex_blank = Ranges::from_unsorted(tb);
        Ok(m)
    }

    /// One tour's own numbers for `tour run`: bytes and the share of the
    /// whole ROM.
    pub fn poly_summary(&self) -> Value {
        summary(&self.poly_read, self.poly_words, 4)
    }

    pub fn tex_summary(&self) -> Value {
        summary(&self.tex_read, self.tex_words, 2)
    }
}

fn summary(read: &Ranges, words: u64, width: u64) -> Value {
    let bytes = read.count() * width;
    json!({"bytes_read": bytes, "rom_bytes": words * width, "share": bytes as f64 / (words * width).max(1) as f64})
}

/// One ROM's merged coverage.
struct Rom {
    name: &'static str,
    width: u64,
    words: u64,
    read: Ranges,
    blank: Ranges,
}

impl Rom {
    fn to_json(&self, top: usize) -> Value {
        let (w, len) = (self.width, self.words);
        let bytes = |r: &Ranges| r.count() * w;
        let read = self.read.clip(len);
        let blank = self.blank.clip(len);
        let used = blank.complement(len);
        let nonblank = bytes(&used);
        let read_nonblank = bytes(&read.intersect(&used));
        // Unread, and not blank: the stretches a tour could still reach.
        let unread = read.union(&blank).complement(len);
        let mut stretches: Vec<(u64, u64)> = unread.0.clone();
        stretches.sort_by_key(|&(s, e)| (std::cmp::Reverse(e - s), s));
        let listed = if top == 0 { stretches.len() } else { top.min(stretches.len()) };
        let stretch = |&(s, e): &(u64, u64)| json!({"start": format!("0x{:x}", s * w), "end": format!("0x{:x}", e * w), "bytes": (e - s) * w});
        // Regions of 1 MiB: how much of each is read.
        let region_words = REGION_BYTES / w;
        let regions: Vec<Value> = (0..len.div_ceil(region_words))
            .map(|i| {
                let one = Ranges(vec![(i * region_words, ((i + 1) * region_words).min(len))]);
                let (size, in_read) = (bytes(&one), bytes(&read.intersect(&one)));
                let used_here = bytes(&used.intersect(&one));
                json!({
                    "start": format!("0x{:x}", i * REGION_BYTES),
                    "bytes": size,
                    "blank_bytes": size - used_here,
                    "read_bytes": in_read,
                    "share_of_non_blank": if used_here == 0 { Value::Null } else { json!(bytes(&read.intersect(&used).intersect(&one)) as f64 / used_here as f64) },
                })
            })
            .collect();
        json!({
            "rom": self.name,
            "bytes": len * w,
            "blank_bytes": len * w - nonblank,
            "read_bytes": bytes(&read),
            "read_non_blank_bytes": read_nonblank,
            "share_of_rom": bytes(&read) as f64 / (len * w).max(1) as f64,
            "share_of_non_blank": read_nonblank as f64 / nonblank.max(1) as f64,
            "regions": regions,
            "unread_stretches": stretches.len(),
            "unread_listed": stretches[..listed].iter().map(stretch).collect::<Vec<_>>(),
            "unread_not_listed_bytes": stretches[listed..].iter().map(|&(s, e)| (e - s) * w).sum::<u64>(),
        })
    }
}

/// The merged coverage of `maps` as the report's data. The maps are of one
/// game's tours; ROMs that differ in size between them are refused.
fn report(maps: &[MapFile], top: usize) -> Result<Value, String> {
    let first = maps.first().ok_or("no maps")?;
    let mut poly = Rom {
        name: "polygon",
        width: 4,
        words: first.poly_words,
        read: Ranges::default(),
        blank: first.poly_blank.clone(),
    };
    let mut tex = Rom {
        name: "texture",
        width: 2,
        words: first.tex_words,
        read: Ranges::default(),
        blank: first.tex_blank.clone(),
    };
    for m in maps {
        if m.poly_words != poly.words || m.tex_words != tex.words {
            return Err(format!("tour {}'s map was made against ROMs of another size: run the tours again", m.tour));
        }
        poly.read = poly.read.union(&m.poly_read);
        tex.read = tex.read.union(&m.tex_read);
    }
    Ok(json!({
        "game": first.game,
        "tours": maps.iter().map(|m| json!({
            "label": m.tour, "frames": m.frames, "state_hash": format!("{:016x}", m.hash),
            "polygon_bytes_read": m.poly_read.count() * 4, "texture_bytes_read": m.tex_read.count() * 2,
        })).collect::<Vec<_>>(),
        "polygon": poly.to_json(top),
        "texture": tex.to_json(top),
    }))
}

fn pct(v: &Value) -> String {
    format!("{:.1}%", v.as_f64().unwrap_or(0.0) * 100.0)
}

fn kb(v: &Value) -> String {
    let b = v.as_u64().unwrap_or(0);
    if b >= 1 << 20 {
        format!("{:.2} MB", b as f64 / (1 << 20) as f64)
    } else {
        format!("{} KB", b / 1024)
    }
}

fn text(d: &Value) -> String {
    let mut s = format!("coverage {}\n", d["game"].as_str().unwrap_or(""));
    for rom in [&d["polygon"], &d["texture"]] {
        s.push_str(&format!(
            "{} ROM  {} of non-blank bytes read, {} of all {} ({} blank)   ({} tours)\n",
            rom["rom"].as_str().unwrap_or(""),
            pct(&rom["share_of_non_blank"]),
            pct(&rom["share_of_rom"]),
            kb(&rom["bytes"]),
            kb(&rom["blank_bytes"]),
            d["tours"].as_array().map_or(0, |t| t.len()),
        ));
    }
    s.push_str("data ROM, tiles, code: not in these maps\n");
    for rom in [&d["polygon"], &d["texture"]] {
        s.push_str(&format!("\n{} ROM by region (read of non-blank):\n", rom["rom"].as_str().unwrap_or("")));
        for r in rom["regions"].as_array().into_iter().flatten() {
            s.push_str(&format!(
                "  {:>10}  {:>6}  of {}\n",
                r["start"].as_str().unwrap_or(""),
                if r["share_of_non_blank"].is_null() { "blank".to_string() } else { pct(&r["share_of_non_blank"]) },
                kb(&json!(r["bytes"].as_u64().unwrap_or(0) - r["blank_bytes"].as_u64().unwrap_or(0))),
            ));
        }
        s.push_str(&format!("unread stretches: {}, largest first\n", rom["unread_stretches"]));
        for r in rom["unread_listed"].as_array().into_iter().flatten() {
            s.push_str(&format!(
                "  {} - {}  {}\n",
                r["start"].as_str().unwrap_or(""),
                r["end"].as_str().unwrap_or(""),
                kb(&r["bytes"])
            ));
        }
        let rest = rom["unread_not_listed_bytes"].as_u64().unwrap_or(0);
        if rest > 0 {
            s.push_str(&format!("  and {} more in smaller stretches (--top 0 lists all)\n", kb(&json!(rest))));
        }
    }
    s
}

/// `remodel coverage` (no ROMs needed).
pub fn main(o: &crate::Opts) -> ExitCode {
    let mut args = o.args[1..].to_vec();
    let c = match tour::common(&mut args) {
        Ok(c) => c,
        Err(e) => return fail(answer::BAD_REQUEST, e, None).finish(false, |_| String::new()),
    };
    let top = tour::take_opt(&mut args, "--top");
    let a = answer_for(o, &c, top.as_deref(), &args);
    a.finish(c.json, text)
}

fn answer_for(o: &crate::Opts, c: &tour::Common, top: Option<&str>, args: &[String]) -> Answer {
    if let Some(x) = args.first() {
        return fail(
            answer::BAD_REQUEST,
            format!("coverage: unexpected {x}"),
            Some("remodel coverage [--top N] [--json]"),
        );
    }
    let top = match top.map(str::parse::<usize>) {
        None => TOP,
        Some(Ok(n)) => n,
        Some(Err(_)) => return fail(answer::BAD_REQUEST, "--top wants a number (0: all)", None),
    };
    let game = match romset::games::find(&o.game_id) {
        Ok(g) => g,
        Err(e) => return fail(answer::BAD_REQUEST, e.to_string(), Some("remodel games")),
    };
    let tours = match tour::load(game, c.tours.as_deref()) {
        Ok(t) => t,
        Err(e) => return fail(answer::BAD_REQUEST, e, None),
    };
    let dir = c.dir.clone().unwrap_or_else(|| tour::default_dir(game.id));
    let run_all = format!("remodel tour run --all --game {}", game.id);
    let (mut maps, mut missing, mut changed, mut warnings) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for t in &tours {
        let path = MapFile::path(&dir, &t.label);
        if !path.exists() {
            missing.push(t.label.clone());
            continue;
        }
        match MapFile::load(&path) {
            Ok(m) if m.game != game.id => {
                warnings.push(format!("{}: a map of {}, not {}: left out", path.display(), m.game, game.id))
            }
            Ok(m) => {
                if m.spec != fnv(&t.spec) {
                    changed.push(t.label.clone());
                }
                maps.push(m);
            }
            Err(e) => return fail(answer::INTERNAL, e, Some(&run_all)),
        }
    }
    // What `remodel crawl` read, if it has run: counted beside the tours'.
    let crawl_map = dir.with_file_name("crawl").join("crawl.map");
    let crawled = match MapFile::load(&crawl_map) {
        Ok(m) if m.game == game.id => {
            maps.push(m);
            true
        }
        _ => false,
    };
    if maps.is_empty() {
        return fail(
            answer::MISSING,
            format!("no tour of {} has been run: there is no read map in {}", game.id, dir.display()),
            Some(&run_all),
        );
    }
    let mut data = match report(&maps, top) {
        Ok(d) => d,
        Err(e) => return fail(answer::INTERNAL, e, Some(&run_all)),
    };
    data["missing_tours"] = json!(missing);
    data["changed_tours"] = json!(changed);
    data["crawl"] = json!(crawled);
    // Tours that hold RAM values (cheat runs) are counted, and named.
    let poked: Vec<String> = tours
        .iter()
        .filter(|t| {
            maps.iter().any(|m| m.tour == t.label) && tour::uses_pokes(&tours, &t.label, game.controls.coin_start)
        })
        .map(|t| t.label.clone())
        .collect();
    data["poked_tours"] = json!(poked);
    let mut a = Answer::done(data);
    if !missing.is_empty() {
        a = a.warn(format!("tours not run yet, so not counted: {}", missing.join(", ")));
    }
    if !changed.is_empty() {
        a = a.warn(format!("tours changed since their maps were made (counted as they were): {}", changed.join(", ")));
    }
    if crawled {
        a = a.warn(format!(
            "counts what `remodel crawl` read ({}): its texture count is a floor (headers sampled every few frames)",
            crawl_map.display()
        ));
    }
    if !poked.is_empty() {
        a = a.warn(format!(
            "counted, but they poke RAM (cheat runs, reaching what play would need skill for): {}",
            poked.join(", ")
        ));
    }
    for w in warnings {
        a = a.warn(w);
    }
    if !missing.is_empty() || !changed.is_empty() {
        a = a.next(run_all);
    }
    a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(tour: &str, poly: &[(u64, u64)], tex: &[(u64, u64)]) -> MapFile {
        MapFile {
            game: "g".into(),
            tour: tour.into(),
            spec: fnv(tour),
            frames: 10,
            hash: 0xabc,
            poly_words: 1000,
            tex_words: 2000,
            poly_read: Ranges::from_unsorted(poly.to_vec()),
            tex_read: Ranges::from_unsorted(tex.to_vec()),
            poly_blank: Ranges(vec![(900, 1000)]),
            tex_blank: Ranges::default(),
        }
    }

    /// A map keeps what it was given through its file form.
    #[test]
    fn a_map_file_round_trips() {
        let m = map("coin-start", &[(0, 10), (50, 60)], &[(5, 9)]);
        let dir = std::env::temp_dir().join(format!("remodel-map-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = MapFile::path(&dir, "coin-start");
        m.save(&p).unwrap();
        assert_eq!(MapFile::load(&p).unwrap(), m);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(MapFile::parse("junk").is_err());
        assert!(MapFile::parse("remodel-readmap 1\nP 1").is_err());
    }

    /// The report merges tours by union, counts bytes (4 a polygon word,
    /// 2 a texture word) and leaves blank runs out of the unread list.
    #[test]
    fn the_report_merges_maps_and_counts_bytes() {
        let a = map("a", &[(0, 100), (200, 300)], &[(0, 10)]);
        let b = map("b", &[(50, 150), (200, 250)], &[(5, 20)]);
        let r = report(&[a, b], 0).unwrap();
        // 0..150 and 200..300: 250 words, 1000 bytes; 900..1000 is blank.
        assert_eq!(r["polygon"]["read_bytes"], 1000);
        assert_eq!(r["polygon"]["bytes"], 4000);
        assert_eq!(r["polygon"]["blank_bytes"], 400);
        assert_eq!(r["polygon"]["share_of_rom"], 0.25);
        assert_eq!(r["polygon"]["share_of_non_blank"], 1000.0 / 3600.0);
        // Unread: 150..200 and 300..900.
        assert_eq!(r["polygon"]["unread_stretches"], 2);
        let l = r["polygon"]["unread_listed"].as_array().unwrap();
        assert_eq!((l[0]["start"].as_str().unwrap(), l[0]["bytes"].as_u64().unwrap()), ("0x4b0", 2400));
        assert_eq!((l[1]["start"].as_str().unwrap(), l[1]["end"].as_str().unwrap()), ("0x258", "0x320"));
        assert_eq!(r["texture"]["read_bytes"], 40);
        // A shorter list says how much it left out.
        let r = report(&[map("a", &[(0, 100)], &[])], 1).unwrap();
        assert_eq!(r["polygon"]["unread_listed"].as_array().unwrap().len(), 1);
        assert_eq!(r["polygon"]["unread_not_listed_bytes"], 0);
        // A map from ROMs of another size is refused.
        let mut other = map("c", &[], &[]);
        other.poly_words = 5;
        assert!(report(&[map("a", &[], &[]), other], 0).is_err());
    }

    /// Re-merging the same map changes nothing.
    #[test]
    fn merging_is_idempotent() {
        let a = map("a", &[(0, 100)], &[(0, 10)]);
        let once = report(std::slice::from_ref(&a), 0).unwrap();
        let twice = report(&[a.clone(), a], 0).unwrap();
        assert_eq!(once["polygon"], twice["polygon"]);
        assert_eq!(once["texture"], twice["texture"]);
    }

    #[test]
    fn the_report_lists_addresses_and_sizes_only() {
        let t = text(&report(&[map("a", &[(0, 100)], &[(0, 10)])], 5).unwrap());
        assert!(t.contains("polygon ROM"), "{t}");
        assert!(t.contains("unread stretches"), "{t}");
    }
}
