//! The glyph mapping helper (docs/uplift-studio.md, "Glyph mapping").
//!
//! A game's HUD numbers are tiles: each digit is one 8x8 tile, or a block
//! of them (2x2 for a big digit: its quarters), picked out of a font. The
//! names of the tiles say nothing about which digit they show. This finds
//! out from the game's behaviour: watch the tile layer every frame, keep
//! only the cells whose tile changes (a counter's digits change, the rest
//! of the layout does not), and read the digits off how they change.
//!
//! * A counter's units digit steps through the ten digits in order, so
//!   its transitions form a cycle: each digit's successor is the next
//!   digit (or the previous one, for a counter that counts down).
//! * A digit one place up changes exactly when the one below wraps, so
//!   the tile the lower digit shows right after the higher one changes is
//!   the digit that follows 9: a zero for a counter that counts up.
//! * Failing that, the cycle is laid against a 3x5 template font and the
//!   rotation that looks most like 0..9 is taken.
//!
//! Output: one row per tile, `tile key`, `glyph`, `quarter` (`-` for a
//! one-tile glyph, else `tl`, `tr`, `bl`, `br`, or `r<row>c<col>` for a
//! bigger block), how it was decided and how sure. The result is a
//! hypothesis from the game's behaviour, not read from the ROM: rows say
//! which evidence gave them.

use model2::video;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;
use std::path::Path;

/// A tile instance's place: layer (0 low, 1 high), column and row in tiles.
pub type Cell = (u8, i32, i32);

/// One change of a cell's tile.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Event {
    pub frame: u64,
    pub cell: Cell,
    pub from: Option<u64>,
    pub to: Option<u64>,
}

/// Frames that change more cells than this are scene changes or scrolls,
/// not counters.
const MAX_CHANGES: usize = 48;

/// Records the 2D layer's changes frame by frame.
#[derive(Default)]
pub struct Watch {
    prev: HashMap<Cell, u64>,
    pub events: Vec<Event>,
    /// Each tile pattern seen changing: its pens (ink where not 0).
    pub pens: HashMap<u64, [u8; 64]>,
    pub frames: u64,
    pub skipped: u64,
}

impl Watch {
    pub fn frame(&mut self, frame: u64, tiles: &[u8], palette: &[u8]) {
        let tp = video::tile_pixels(tiles, palette);
        let mut cur: HashMap<Cell, u64> = HashMap::new();
        for (layer, hits) in [(0u8, &tp.low), (1u8, &tp.high)] {
            for (i, h) in hits.iter().enumerate() {
                if h.info == u32::MAX || h.row != 3 || h.col != 3 {
                    continue;
                }
                let (x, y) = (i % video::WIDTH, i / video::WIDTH);
                if x < 3 || y < 3 {
                    continue;
                }
                let info = &tp.infos[h.info as usize];
                cur.insert((layer, ((x - 3) / 8) as i32, ((y - 3) / 8) as i32), info.key);
                self.pens.entry(info.key).or_insert(info.pens);
            }
        }
        self.diff(frame, cur);
    }

    /// Log what changed from the frame before to `cur`: nothing for a
    /// frame that changed a lot (a scene change or a scroll).
    fn diff(&mut self, frame: u64, cur: HashMap<Cell, u64>) {
        self.frames += 1;
        let mut changed: Vec<Event> = Vec::new();
        for (c, &k) in &cur {
            if self.prev.get(c) != Some(&k) {
                changed.push(Event { frame, cell: *c, from: self.prev.get(c).copied(), to: Some(k) });
            }
        }
        for (c, &k) in &self.prev {
            if !cur.contains_key(c) {
                changed.push(Event { frame, cell: *c, from: Some(k), to: None });
            }
        }
        if !self.prev.is_empty() {
            if changed.len() <= MAX_CHANGES {
                changed.sort_by_key(|e| e.cell);
                self.events.extend(changed);
            } else {
                self.skipped += 1;
            }
        }
        self.prev = cur;
    }

    pub fn finish(self, dir: &Path, game: &str) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let counts = crate::GLYPH_COUNTS.get().copied().unwrap_or(Counts::Up);
        let found = infer(&self.events, &self.pens, counts);
        let mut tsv = String::from("tile\tglyph\tquarter\tevidence\tconfidence\n");
        for r in &found.rows {
            let _ = writeln!(tsv, "{:016x}\t{}\t{}\t{}\t{:.2}", r.tile, r.glyph, r.quarter, r.evidence, r.confidence);
        }
        let put = |name: &str, text: String| {
            let p = dir.join(name);
            std::fs::write(&p, text).map_err(|e| format!("{}: {e}", p.display()))
        };
        put("glyphs.tsv", tsv)?;
        put("glyphs.txt", found.text(game, self.frames, self.skipped, &self.pens))?;
        let mut ev = String::from("frame\tlayer\tcol\trow\tfrom\tto\n");
        for e in &self.events {
            let k = |k: Option<u64>| k.map_or("-".to_string(), |k| format!("{k:016x}"));
            let _ = writeln!(ev, "{}\t{}\t{}\t{}\t{}\t{}", e.frame, e.cell.0, e.cell.1, e.cell.2, k(e.from), k(e.to));
        }
        put("events.tsv", ev)?;
        let mut tiles = String::from("tile\tpens\n");
        let mut keys: Vec<&u64> = self.pens.keys().collect();
        keys.sort();
        for k in keys {
            let _ = writeln!(tiles, "{k:016x}\t{}", self.pens[k].iter().map(|p| format!("{p:x}")).collect::<String>());
        }
        put("tiles.tsv", tiles)?;
        println!(
            "glyphs        {} frames watched ({} scene changes skipped), {} tile changes; {} counter fields, {} tiles mapped -> {}",
            self.frames,
            self.skipped,
            self.events.len(),
            found.fields.len(),
            found.rows.len(),
            dir.join("glyphs.tsv").display()
        );
        Ok(())
    }
}

/// Read a `--glyphs` directory back (`events.tsv`, `tiles.tsv`) and infer
/// again, with `counts`: to try another direction or a changed inference
/// without playing the game again.
pub fn reinfer(dir: &Path, counts: Counts) -> Result<(), String> {
    let rows = |name: &str| crate::catalogue::read_tsv(&dir.join(name));
    let key = |s: &str| u64::from_str_radix(s, 16).ok();
    let mut events = Vec::new();
    for r in rows("events.tsv") {
        let k = |c: &str| r.get(c).and_then(|v| key(v));
        events.push(Event {
            frame: r["frame"].parse().map_err(|_| "bad frame")?,
            cell: (
                r["layer"].parse().map_err(|_| "bad layer")?,
                r["col"].parse().map_err(|_| "bad col")?,
                r["row"].parse().map_err(|_| "bad row")?,
            ),
            from: k("from"),
            to: k("to"),
        });
    }
    let mut pens = HashMap::new();
    for r in rows("tiles.tsv") {
        let mut p = [0u8; 64];
        for (i, c) in r["pens"].chars().take(64).enumerate() {
            p[i] = c.to_digit(16).unwrap_or(0) as u8;
        }
        pens.insert(key(&r["tile"]).ok_or("bad tile key")?, p);
    }
    if events.is_empty() {
        return Err(format!("{}: no events.tsv (run `make catalogue-glyphs` first)", dir.display()));
    }
    let found = infer(&events, &pens, counts);
    let mut tsv = String::from("tile\tglyph\tquarter\tevidence\tconfidence\n");
    for r in &found.rows {
        let _ = writeln!(tsv, "{:016x}\t{}\t{}\t{}\t{:.2}", r.tile, r.glyph, r.quarter, r.evidence, r.confidence);
    }
    std::fs::write(dir.join("glyphs.tsv"), tsv).map_err(|e| e.to_string())?;
    std::fs::write(dir.join("glyphs.txt"), found.text("", 0, 0, &pens)).map_err(|e| e.to_string())?;
    println!(
        "glyphs        {} counter fields, {} tiles mapped -> {}",
        found.fields.len(),
        found.rows.len(),
        dir.join("glyphs.tsv").display()
    );
    Ok(())
}

/// Which way the counters of the run count (they all do in a race: a
/// clock down, a score up): `up` or `down`. Only the rotation of the
/// cycle depends on it where there is no carry to read it from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Counts {
    Up,
    Down,
}

/// One tile's glyph.
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub tile: u64,
    pub glyph: String,
    /// `-` for a one-tile glyph, else which part of the block (`tl`, ...).
    pub quarter: String,
    pub evidence: String,
    pub confidence: f64,
}

/// A counter's digit block: the cells it covers and its cycle.
#[derive(Clone, Debug)]
pub struct Field {
    pub cells: Vec<Cell>,
    /// Block size in tiles.
    pub size: (i32, i32),
    /// The blocks' distinct states (tuples of tile keys, one per cell in
    /// `cells` order) in the order the counter steps through them.
    pub cycle: Vec<Vec<Option<u64>>>,
    pub changes: usize,
}

#[derive(Default, Debug)]
pub struct Found {
    pub fields: Vec<Field>,
    pub rows: Vec<Row>,
}

/// 3x5 digits, one row per byte (bit 2 the left pixel): the template font.
const FONT: [[u8; 5]; 10] = [
    [7, 5, 5, 5, 7],
    [2, 6, 2, 2, 7],
    [7, 1, 7, 4, 7],
    [7, 1, 7, 1, 7],
    [5, 5, 7, 1, 1],
    [7, 4, 7, 1, 7],
    [7, 4, 7, 5, 7],
    [7, 1, 1, 1, 1],
    [7, 5, 7, 5, 7],
    [7, 5, 7, 1, 7],
];

/// How alike a block (its tiles' ink laid out as one picture) is to digit
/// `d` of the template font, 0 to 1: the share of agreeing cells over a
/// 3x5 grid sampled out of the block's inked area.
fn template_score(block: &[(i32, i32, [u8; 64])], size: (i32, i32), d: usize) -> f64 {
    let (w, h) = ((size.0 * 8) as usize, (size.1 * 8) as usize);
    let mut ink = vec![false; w * h];
    for (cx, cy, pens) in block {
        for r in 0..8 {
            for c in 0..8 {
                if pens[r * 8 + c] != 0 {
                    ink[(*cy as usize * 8 + r) * w + *cx as usize * 8 + c] = true;
                }
            }
        }
    }
    // The bounding box of the ink, divided into 3 columns and 5 rows.
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if ink[y * w + x] {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
            }
        }
    }
    if x1 < x0 {
        return 0.0;
    }
    let (bw, bh) = ((x1 - x0 + 1) as f64, (y1 - y0 + 1) as f64);
    let mut agree = 0;
    for (gy, template_row) in FONT[d].iter().enumerate() {
        for gx in 0..3 {
            let (xa, xb) = (x0 + (bw * gx as f64 / 3.0) as usize, x0 + (bw * (gx + 1) as f64 / 3.0).ceil() as usize);
            let (ya, yb) = (y0 + (bh * gy as f64 / 5.0) as usize, y0 + (bh * (gy + 1) as f64 / 5.0).ceil() as usize);
            let mut on = 0;
            let mut n = 0;
            for y in ya..yb.min(h) {
                for x in xa..xb.min(w) {
                    on += ink[y * w + x] as usize;
                    n += 1;
                }
            }
            let cell = n > 0 && on * 2 >= n;
            let want = template_row >> (2 - gx) & 1 == 1;
            agree += (cell == want) as usize;
        }
    }
    agree as f64 / 15.0
}

/// A state of a digit block: its tiles' keys, one per cell in the block's
/// cell order (None: the cell shows nothing, a blank part of a glyph).
type State = Vec<Option<u64>>;

/// Find the counters in `events` and read their digits.
pub fn infer(events: &[Event], pens: &HashMap<u64, [u8; 64]>, counts: Counts) -> Found {
    // Changes per cell, in frame order.
    let mut by_cell: BTreeMap<Cell, Vec<&Event>> = BTreeMap::new();
    for e in events {
        by_cell.entry(e.cell).or_default().push(e);
    }
    // A counter cell changes often (a glyph's blank parts change less, but
    // some part of every digit does).
    let counter: BTreeSet<Cell> = by_cell.iter().filter(|(_, ev)| ev.len() >= 3).map(|(c, _)| *c).collect();
    // Cells of one glyph block change in the same frames: join adjacent
    // counter cells whose change frames largely coincide.
    let frames_of = |c: &Cell| -> BTreeSet<u64> { by_cell[c].iter().map(|e| e.frame).collect() };
    let mut left: BTreeSet<Cell> = counter.clone();
    let mut blocks: Vec<Vec<Cell>> = Vec::new();
    while let Some(&start) = left.iter().next() {
        left.remove(&start);
        let mut block = vec![start];
        let mut i = 0;
        while i < block.len() {
            let c = block[i];
            i += 1;
            for (dx, dy) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                let n = (c.0, c.1 + dx, c.2 + dy);
                if left.contains(&n) {
                    let (a, b) = (frames_of(&c), frames_of(&n));
                    let both = a.intersection(&b).count();
                    if both * 10 >= a.union(&b).count() * 7 {
                        left.remove(&n);
                        block.push(n);
                    }
                }
            }
        }
        block.sort_by_key(|c| (c.2, c.1));
        blocks.push(block);
    }
    let mut found = Found::default();
    let mut walks: Vec<Walk> = Vec::new();
    for cells in blocks {
        let (x0, y0) = (cells.iter().map(|c| c.1).min().unwrap(), cells.iter().map(|c| c.2).min().unwrap());
        let (x1, y1) = (cells.iter().map(|c| c.1).max().unwrap(), cells.iter().map(|c| c.2).max().unwrap());
        // States over time: replay the cells' changes.
        let mut cur: State = vec![None; cells.len()];
        let mut seq: Vec<(u64, State)> = Vec::new();
        let mut all: Vec<&Event> = cells.iter().flat_map(|c| by_cell[c].iter().copied()).collect();
        all.sort_by_key(|e| e.frame);
        let mut i = 0;
        while i < all.len() {
            let f = all[i].frame;
            while i < all.len() && all[i].frame == f {
                let k = cells.iter().position(|c| *c == all[i].cell).unwrap();
                cur[k] = all[i].to;
                i += 1;
            }
            // An all-blank block is a gap, not a state.
            if cur.iter().any(|c| c.is_some()) && seq.last().map(|s| &s.1) != Some(&cur) {
                seq.push((f, cur.clone()));
            }
        }
        let walk = walk_cycle(&seq);
        found.fields.push(Field {
            cells,
            size: (x1 - x0 + 1, y1 - y0 + 1),
            cycle: walk.cycle.clone(),
            changes: all.len(),
        });
        walks.push(walk);
    }
    // Digit places: a block that steps through ten states in a clean
    // closed cycle. Its carry partner is another block that changes only
    // when it wraps.
    let mut cands: Vec<(bool, Vec<Row>)> = Vec::new();
    for (ui, u) in walks.iter().enumerate() {
        let n = u.cycle.len();
        if n != 10 || !u.closed || u.purity < 0.85 || found.fields[ui].size.0 > 3 || found.fields[ui].size.1 > 3 {
            continue;
        }
        let uf = &found.fields[ui];
        // Where the carry happens: the step k -> k+1 that others' changes
        // line up with.
        let mut votes = vec![0usize; n];
        for (pi, p) in walks.iter().enumerate() {
            let pf = &found.fields[pi];
            if pi == ui || pf.changes < 3 || p.frames.len() * 5 > u.frames.len() {
                continue;
            }
            let mut at = vec![0usize; n];
            for f in &p.frames {
                if let Some(&k) = u.step_at.get(f).or_else(|| u.step_at.get(&f.wrapping_sub(1))) {
                    at[k] += 1;
                }
            }
            let (k, best) = at.iter().copied().enumerate().max_by_key(|(_, c)| *c).unwrap();
            if best * 10 >= p.frames.len() * 8 {
                votes[k] += best;
            }
        }
        let (kstar, vote) = votes.iter().copied().enumerate().max_by_key(|(_, v)| *v).unwrap();
        let (cx, cy) = (uf.cells.iter().map(|c| c.1).min().unwrap(), uf.cells.iter().map(|c| c.2).min().unwrap());
        let tiles_of = |state: &State| -> Vec<(i32, i32, [u8; 64])> {
            uf.cells.iter().zip(state).filter_map(|(c, k)| Some((c.1 - cx, c.2 - cy, *pens.get(&(*k)?)?))).collect()
        };
        // The fit of an assignment: how much the states look like the
        // template digits they were given.
        let fit = |digit_of: &dyn Fn(usize) -> usize| -> f64 {
            (0..n).map(|k| template_score(&tiles_of(&u.cycle[k]), uf.size, digit_of(k))).sum::<f64>() / n as f64
        };
        let up = |zero: usize| move |k: usize| (k + n - zero) % n;
        let down = |nine: usize| move |k: usize| (9 + 2 * n - (k + n - nine) % n) % n;
        let (digit, evidence, confidence): (Box<dyn Fn(usize) -> usize>, String, f64) = if vote > 0 {
            // The state after the carry is the zero of a counter counting
            // up, the nine of one counting down.
            let after = (kstar + 1) % n;
            let (fu, fd) = (fit(&up(after)), fit(&down(after)));
            let shapes = (fu - fd).abs() >= 0.03;
            let dir = if shapes {
                if fu > fd {
                    Counts::Up
                } else {
                    Counts::Down
                }
            } else {
                counts
            };
            let (f, d): (f64, Box<dyn Fn(usize) -> usize>) = match dir {
                Counts::Up => (fu, Box::new(up(after))),
                Counts::Down => (fd, Box::new(down(after))),
            };
            let how = if shapes { "from the shapes" } else { "from --glyph-counts" };
            let word = if dir == Counts::Up { "up" } else { "down" };
            (d, format!("carry ({vote} carries; counts {word} {how}; shape fit {f:.2})"), 0.9)
        } else {
            let mut best = (-1.0f64, 0usize, false);
            let mut second = -1.0f64;
            for rev in [false, true] {
                for rot in 0..n {
                    let f = if rev { fit(&down(rot)) } else { fit(&up(rot)) };
                    if f > best.0 {
                        second = best.0;
                        best = (f, rot, rev);
                    } else if f > second {
                        second = f;
                    }
                }
            }
            let d: Box<dyn Fn(usize) -> usize> = if best.2 { Box::new(down(best.1)) } else { Box::new(up(best.1)) };
            (
                d,
                format!("shapes only (fit {:.2}, next {:.2}; no carry partner seen)", best.0, second),
                (best.0 - second).clamp(0.0, 0.5),
            )
        };
        let mut mine = Vec::new();
        for (k, state) in u.cycle.iter().enumerate() {
            for (c, key) in uf.cells.iter().zip(state) {
                let Some(key) = key else { continue };
                mine.push(Row {
                    tile: *key,
                    glyph: digit(k).to_string(),
                    quarter: quarter_name(c.1 - cx, c.2 - cy, uf.size),
                    evidence: evidence.clone(),
                    confidence,
                });
            }
        }
        cands.push((vote > 0, mine));
    }
    // A carry is evidence; shapes alone only stand where there is none.
    let any_carry = cands.iter().any(|c| c.0);
    let mut rows: BTreeMap<(u64, String), Row> = BTreeMap::new();
    for (_, mine) in cands.into_iter().filter(|c| c.0 || !any_carry) {
        for r in mine {
            rows.entry((r.tile, r.glyph.clone())).or_insert(r);
        }
    }
    found.rows = rows.into_values().collect();
    found
}

/// A block's cycle: its states in stepping order, whether the walk came
/// back to its start, how pure the steps are (each state's commonest
/// successor's share) and the frames of its changes (and which step each
/// frame made).
#[derive(Default, Debug)]
pub struct Walk {
    pub cycle: Vec<State>,
    pub closed: bool,
    pub purity: f64,
    pub frames: Vec<u64>,
    pub step_at: HashMap<u64, usize>,
}

/// The commonest successor of each state, as (state, count, total).
type Successors = HashMap<State, (State, usize, usize)>;

fn walk_cycle(seq: &[(u64, State)]) -> Walk {
    let mut succ: HashMap<&State, HashMap<&State, usize>> = HashMap::new();
    for w in seq.windows(2) {
        *succ.entry(&w[0].1).or_default().entry(&w[1].1).or_insert(0) += 1;
    }
    let mut next: Successors = HashMap::new();
    for (k, v) in &succ {
        let total: usize = v.values().sum();
        if let Some((s, n)) = v.iter().max_by_key(|(s, n)| (**n, std::cmp::Reverse(format!("{s:?}")))) {
            next.insert((*k).clone(), ((*s).clone(), *n, total));
        }
    }
    // The longest chain of dominant successors that returns to its start.
    let mut best: Vec<State> = Vec::new();
    let mut closed = false;
    for start in next.keys() {
        let mut chain: Vec<State> = vec![start.clone()];
        let mut back = false;
        while let Some((n, _, _)) = next.get(chain.last().unwrap()) {
            if n == start {
                back = true;
                break;
            }
            if chain.contains(n) {
                break;
            }
            chain.push(n.clone());
        }
        if (back, chain.len()) > (closed, best.len()) {
            best = chain;
            closed = back;
        }
    }
    let (num, den) = next.values().fold((0, 0), |a, t| (a.0 + t.1, a.1 + t.2));
    let index: HashMap<&State, usize> = best.iter().enumerate().map(|(i, s)| (s, i)).collect();
    let mut step_at = HashMap::new();
    for w in seq.windows(2) {
        if let (Some(&a), Some(&b)) = (index.get(&w[0].1), index.get(&w[1].1)) {
            if b == (a + 1) % best.len() {
                step_at.insert(w[1].0, a);
            }
        }
    }
    Walk {
        cycle: best,
        closed,
        purity: if den == 0 { 0.0 } else { num as f64 / den as f64 },
        frames: seq.iter().map(|s| s.0).collect(),
        step_at,
    }
}

fn quarter_name(dx: i32, dy: i32, size: (i32, i32)) -> String {
    match size {
        (1, 1) => "-".into(),
        (2, 2) => ["tl", "tr", "bl", "br"][(dy * 2 + dx) as usize].into(),
        _ => format!("r{dy}c{dx}"),
    }
}

impl Found {
    pub fn text(&self, game: &str, frames: u64, skipped: u64, pens: &HashMap<u64, [u8; 64]>) -> String {
        let mut s = format!(
            "glyph mapping for {game}: {frames} frames watched, {skipped} scene changes skipped\n\
             An inference from how the game's counters change (docs/uplift-studio.md, \"Glyph mapping\"); unconfirmed.\n\n"
        );
        for (i, f) in self.fields.iter().enumerate() {
            let c = f.cells[0];
            let _ = writeln!(
                s,
                "field {i}: layer {} at tile column {} row {}, {}x{} tiles, {} changes, {} states in its cycle",
                ["low", "high"][c.0 as usize],
                c.1,
                c.2,
                f.size.0,
                f.size.1,
                f.changes,
                f.cycle.len()
            );
        }
        s.push_str("\ntile key          glyph quarter evidence  confidence\n");
        for r in &self.rows {
            let _ =
                writeln!(s, "{:016x}  {:<5} {:<7} {:<9} {:.2}", r.tile, r.glyph, r.quarter, r.evidence, r.confidence);
        }
        s.push_str("\nthe glyphs as their tiles draw them (# is ink)\n");
        let mut by: BTreeMap<&str, Vec<&Row>> = BTreeMap::new();
        for r in &self.rows {
            by.entry(&r.glyph).or_default().push(r);
        }
        for (g, rows) in by {
            // Lay the quarters out: a 1x1 glyph is one tile, a 2x2 four.
            let at = |q: &str| match q {
                "-" | "tl" => Some((0, 0)),
                "tr" => Some((1, 0)),
                "bl" => Some((0, 1)),
                "br" => Some((1, 1)),
                _ => None,
            };
            let (w, h) = (
                rows.iter().filter_map(|r| at(&r.quarter)).map(|p| p.0 + 1).max().unwrap_or(1),
                rows.iter().filter_map(|r| at(&r.quarter)).map(|p| p.1 + 1).max().unwrap_or(1),
            );
            let mut canvas = vec![vec!['.'; w * 8]; h * 8];
            for r in &rows {
                let (Some((qx, qy)), Some(p)) = (at(&r.quarter), pens.get(&r.tile)) else { continue };
                for y in 0..8 {
                    for x in 0..8 {
                        if p[y * 8 + x] != 0 {
                            canvas[qy * 8 + y][qx * 8 + x] = '#';
                        }
                    }
                }
            }
            let _ = writeln!(s, "glyph {g}");
            for line in canvas {
                let _ = writeln!(s, "  {}", line.into_iter().collect::<String>());
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A digit drawn from the template font into a 2x2 block of tiles
    /// (16x16 pixels): the pens of its four tiles.
    fn quarters(d: usize) -> [[u8; 64]; 4] {
        let mut px = [[0u8; 16]; 16];
        for (y, row) in px.iter_mut().enumerate() {
            for (x, p) in row.iter_mut().enumerate() {
                if (2..14).contains(&x) && y < 15 {
                    *p = (FONT[d][y / 3] >> (2 - (x - 2) / 4) & 1) * 3;
                }
            }
        }
        let mut out = [[0u8; 64]; 4];
        for (q, tile) in out.iter_mut().enumerate() {
            let (ox, oy) = (q % 2 * 8, q / 2 * 8);
            for y in 0..8 {
                for x in 0..8 {
                    tile[y * 8 + x] = px[oy + y][ox + x];
                }
            }
        }
        out
    }

    fn key_of(p: &[u8; 64]) -> u64 {
        p.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &b| (h ^ b as u64).wrapping_mul(0x100_0000_01b3))
    }

    /// A clock `values` shown in two places (tens at column 8, units at
    /// column 10, row 4), one value every 10 frames, as the tile changes a
    /// watch would record, with the tiles' pens.
    fn clock(values: &[usize]) -> (Vec<Event>, HashMap<u64, [u8; 64]>) {
        let mut pens = HashMap::new();
        let mut shown: HashMap<Cell, u64> = HashMap::new();
        let mut events = Vec::new();
        for (i, &v) in values.iter().enumerate() {
            let frame = i as u64 * 10 + 5;
            for (col, d) in [(8, v / 10), (10, v % 10)] {
                for (q, p) in quarters(d).iter().enumerate() {
                    let cell: Cell = (1, col + (q % 2) as i32, 4 + (q / 2) as i32);
                    let now = p.iter().any(|&x| x != 0).then(|| key_of(p));
                    if let Some(k) = now {
                        pens.insert(k, *p);
                    }
                    let before = shown.get(&cell).copied();
                    if before != now {
                        events.push(Event { frame, cell, from: before, to: now });
                        match now {
                            Some(k) => shown.insert(cell, k),
                            None => shown.remove(&cell),
                        };
                    }
                }
            }
        }
        (events, pens)
    }

    /// The glyph the table gives the tile at `quarter` of digit `d`.
    fn glyph_of(found: &Found, d: usize, quarter: &str) -> Option<String> {
        let q = quarters(d);
        let at = ["tl", "tr", "bl", "br"].iter().position(|n| *n == quarter)?;
        let k = key_of(&q[at]);
        found
            .rows
            .iter()
            .find(|r| r.tile == k && r.quarter == quarter && r.glyph == d.to_string())
            .map(|r| r.glyph.clone())
    }

    #[test]
    fn a_counting_clock_gives_its_digits_by_the_carry() {
        let values: Vec<usize> = (0..60).collect();
        let (events, pens) = clock(&values);
        // Whichever way it is told the counter counts, the shapes and the
        // carry agree on up.
        for counts in [Counts::Up, Counts::Down] {
            let found = infer(&events, &pens, counts);
            assert!(!found.rows.is_empty(), "{counts:?}");
            for d in 0..10 {
                for q in ["tl", "tr", "bl", "br"] {
                    let k = key_of(&quarters(d)[["tl", "tr", "bl", "br"].iter().position(|n| *n == q).unwrap()]);
                    if pens.get(&k).is_some_and(|p| p.iter().any(|&x| x != 0)) {
                        assert_eq!(
                            glyph_of(&found, d, q).as_deref(),
                            Some(d.to_string().as_str()),
                            "digit {d} {q} ({counts:?})"
                        );
                    }
                }
            }
            assert!(found.rows.iter().all(|r| r.evidence.starts_with("carry")), "{:?}", found.rows[0]);
        }
    }

    #[test]
    fn a_counting_down_clock_is_read_down() {
        let values: Vec<usize> = (0..60).rev().collect();
        let (events, pens) = clock(&values);
        // Told it counts up, the shapes say down (the zero is not where an
        // up counter would put it).
        let found = infer(&events, &pens, Counts::Up);
        for d in 0..10 {
            if quarters(d)[0].iter().any(|&p| p != 0) {
                assert_eq!(glyph_of(&found, d, "tl").as_deref(), Some(d.to_string().as_str()), "digit {d}");
            }
        }
        let r = &found.rows[0];
        assert!(r.evidence.contains("counts down"), "{}", r.evidence);
    }

    #[test]
    fn a_blinking_tile_is_not_a_counter() {
        // One cell alternating between two tiles every frame: it has the
        // changes but not ten states.
        let mut events = Vec::new();
        for f in 0..200u64 {
            let (a, b) = if f % 2 == 0 { (Some(1), Some(2)) } else { (Some(2), Some(1)) };
            events.push(Event { frame: f, cell: (1, 3, 3), from: a, to: b });
        }
        let found = infer(&events, &HashMap::new(), Counts::Up);
        assert!(found.rows.is_empty());
    }

    #[test]
    fn the_watch_logs_only_what_changes() {
        let mut w = Watch::default();
        w.diff(1, HashMap::from([((1, 0, 0), 5u64), ((1, 1, 0), 6)]));
        assert!(w.events.is_empty(), "the first frame is a baseline");
        // One cell changes, one goes away, one appears.
        w.diff(2, HashMap::from([((1, 0, 0), 5u64), ((1, 1, 0), 7), ((1, 2, 0), 9)]));
        assert_eq!(w.events.len(), 2);
        assert_eq!((w.events[0].cell, w.events[0].from, w.events[0].to), ((1, 1, 0), Some(6), Some(7)));
        assert_eq!((w.events[1].cell, w.events[1].from, w.events[1].to), ((1, 2, 0), None, Some(9)));
        // A frame that changes everything (a new scene) logs nothing.
        w.diff(3, (0..100).map(|i| ((0u8, i, 0), 1u64)).collect());
        assert_eq!((w.events.len(), w.skipped), (2, 1));
    }

    #[test]
    fn template_scores_prefer_the_right_digit() {
        let q = quarters(7);
        let block: Vec<(i32, i32, [u8; 64])> = (0..4).map(|i| (i % 2, i / 2, q[i as usize])).collect();
        let best = (0..10)
            .max_by(|&a, &b| template_score(&block, (2, 2), a).total_cmp(&template_score(&block, (2, 2), b)))
            .unwrap();
        assert_eq!(best, 7);
    }
}
