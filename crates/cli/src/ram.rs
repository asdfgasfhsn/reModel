//! `remodel ram search | show | pokes`: find work-RAM variables by running
//! a tour, as a cheat finder does, and list a game's pokes
//! (docs/uplift/content-coverage.md, "Pokes"; docs/uplift-studio.md,
//! "Tours and coverage").
//!
//! `ram search TOUR --at F1,F2,F3 --rule R1,R2` runs the tour from power-on
//! (the EEPROM as the board is made, nothing read from a folder), copies
//! work RAM as frames F1, F2 and F3 start, and keeps the addresses whose
//! values follow the rules between consecutive snapshots: `same`, `diff`,
//! `inc`, `dec`, `inc=N`, `dec=N`, and conditions on the later value
//! (`eq=V`, `ne=V`, `lt=V`, `gt=V`), joined with `/` (`inc/lt=100`).
//! `--first COND` constrains the first snapshot. Reads only: the machine is
//! never written, so state hashes are the same as a plain tour's.

use crate::answer::Answer;
use crate::tour::{self, answer, fail, Finish};
use romset::RomSet;
use serde_json::{json, Value};
use std::process::ExitCode;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Cond {
    Same,
    Diff,
    Inc(Option<u32>),
    Dec(Option<u32>),
    Eq(u32),
    Ne(u32),
    Lt(u32),
    Gt(u32),
}

/// One rule: conditions that all hold.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Rule(Vec<Cond>);

fn number(s: &str) -> Option<u32> {
    match s.strip_prefix("0x") {
        Some(h) => u32::from_str_radix(h, 16).ok(),
        None => s.parse().ok(),
    }
}

impl Rule {
    fn parse(s: &str) -> Result<Rule, String> {
        let mut conds = Vec::new();
        for t in s.split('/') {
            let (k, v) = match t.split_once('=') {
                Some((k, v)) => (k, Some(number(v).ok_or_else(|| format!("rule {t}: {v:?} is not a number"))?)),
                None => (t, None),
            };
            conds.push(match (k, v) {
                ("same", None) => Cond::Same,
                ("diff", None) => Cond::Diff,
                ("inc", v) => Cond::Inc(v),
                ("dec", v) => Cond::Dec(v),
                ("eq", Some(v)) => Cond::Eq(v),
                ("ne", Some(v)) => Cond::Ne(v),
                ("lt", Some(v)) => Cond::Lt(v),
                ("gt", Some(v)) => Cond::Gt(v),
                ("any", None) => continue,
                _ => {
                    return Err(format!(
                        "rule {t:?}: want same, diff, inc[=N], dec[=N], eq=V, ne=V, lt=V, gt=V (joined with /)"
                    ))
                }
            });
        }
        Ok(Rule(conds))
    }

    /// Whether `now` (after `before`, if there is one) passes.
    fn holds(&self, before: Option<u32>, now: u32) -> bool {
        self.0.iter().all(|c| match (c, before) {
            (Cond::Eq(v), _) => now == *v,
            (Cond::Ne(v), _) => now != *v,
            (Cond::Lt(v), _) => now < *v,
            (Cond::Gt(v), _) => now > *v,
            (_, None) => false,
            (Cond::Same, Some(b)) => now == b,
            (Cond::Diff, Some(b)) => now != b,
            (Cond::Inc(None), Some(b)) => now > b,
            (Cond::Dec(None), Some(b)) => now < b,
            (Cond::Inc(Some(n)), Some(b)) => now == b.wrapping_add(*n),
            (Cond::Dec(Some(n)), Some(b)) => now == b.wrapping_sub(*n),
        })
    }
}

/// The addresses of `snaps` (work RAM copies, `(base, bytes)` per RAM) at
/// `width` bytes that pass `first` on the first snapshot and `rules[i]`
/// between snapshot i and i+1, within `region`; each with its values.
fn search(
    snaps: &[Vec<(u32, Vec<u8>)>],
    width: usize,
    first: &Rule,
    rules: &[Rule],
    region: (u32, u32),
) -> Vec<(u32, Vec<u32>)> {
    let read = |s: &Vec<(u32, Vec<u8>)>, ram: usize, o: usize| -> u32 {
        let b = &s[ram].1;
        (0..width).fold(0u32, |v, i| v | (b[o + i] as u32) << (8 * i))
    };
    let mut out = Vec::new();
    for (ram, (base, bytes)) in snaps[0].iter().enumerate() {
        let mut o = 0;
        while o + width <= bytes.len() {
            let addr = base + o as u32;
            if addr >= region.0 && addr <= region.1 {
                let vals: Vec<u32> = snaps.iter().map(|s| read(s, ram, o)).collect();
                let ok = first.holds(None, vals[0])
                    && rules.iter().enumerate().all(|(i, r)| r.holds(Some(vals[i]), vals[i + 1]));
                if ok {
                    out.push((addr, vals));
                }
            }
            o += width;
        }
    }
    out
}

/// Run `script` from power-on and copy work RAM as each of `at`'s frames
/// start (frames in ascending order).
fn snapshots(
    roms: &RomSet,
    script: &model2::input::InputScript,
    at: &[u64],
) -> Result<Vec<Vec<(u32, Vec<u8>)>>, String> {
    let mut m = crate::machine(roms);
    if crate::GEO_MODE.get().is_none() {
        m.bus.geo.set_mode(model2::geo::GeoMode::Native);
    }
    let mut out = Vec::new();
    let mut next = 0;
    let take = |m: &model2::Machine| m.bus.work_ram().iter().map(|(a, b)| (*a, b.to_vec())).collect::<Vec<_>>();
    // Frame 0, then each frame as it starts, before its scripted inputs.
    script.apply(0, &mut m.bus);
    let mut last = 0;
    loop {
        while next < at.len() && at[next] == m.frames {
            out.push(take(&m));
            next += 1;
        }
        if next >= at.len() {
            break;
        }
        if let Err(stop) = m.run_slice(u64::MAX / 2) {
            return Err(format!("the machine stopped at frame {}: {stop:?}", m.frames));
        }
        if m.frames != last {
            last = m.frames;
            if at.get(next).is_some_and(|&f| f > m.frames) {
                script.apply(m.frames, &mut m.bus);
            } else if at.get(next).is_some_and(|&f| f == m.frames) {
                // Snapshot first (loop top), then the frame's inputs.
                while next < at.len() && at[next] == m.frames {
                    out.push(take(&m));
                    next += 1;
                }
                script.apply(m.frames, &mut m.bus);
            }
        }
    }
    Ok(out)
}

fn frames_list(s: &str) -> Result<Vec<u64>, String> {
    let v: Result<Vec<u64>, _> = s.split(',').map(|f| number(f.trim()).map(u64::from).ok_or(f)).collect();
    let v = v.map_err(|f| format!("--at: {f:?} is not a frame number"))?;
    if v.windows(2).any(|w| w[0] > w[1]) {
        return Err("--at: frames in ascending order".into());
    }
    Ok(v)
}

fn width(s: &str) -> Result<usize, String> {
    match s {
        "b" => Ok(1),
        "w" => Ok(2),
        "l" => Ok(4),
        _ => Err(format!("--width {s:?}: want b, w or l")),
    }
}

/// `remodel ram pokes`: the game's table (no ROM needed).
pub fn pokes(o: &crate::Opts) -> ExitCode {
    let mut args = o.args[2..].to_vec();
    let json = tour::take_flag(&mut args, "--json");
    let a = (|| -> Answer {
        let game = match romset::games::find(&o.game_id) {
            Ok(g) => g,
            Err(e) => return fail(answer::BAD_REQUEST, e.to_string(), Some("remodel games")),
        };
        match tour::poke_table(romset::games::pokes(game)) {
            Ok(t) => {
                let rows: Vec<Value> =
                    t.iter().map(|(n, s, note)| json!({"name": n, "poke": s, "note": note})).collect();
                let mut a = Answer::done(json!({"game": game.id, "pokes": rows}));
                if t.is_empty() {
                    a = a.warn(format!("{} has no pokes yet: `remodel ram search` finds them", game.id));
                }
                a.next("poke=NAME@FROM-TO in a tour (remodel tour run)")
            }
            Err(e) => fail(answer::INTERNAL, format!("{} {e}", game.id), None),
        }
    })();
    a.finish(json, |d| {
        let mut s = String::new();
        for p in d["pokes"].as_array().into_iter().flatten() {
            s.push_str(&format!(
                "{:<18} {:<22} {}\n",
                p["name"].as_str().unwrap_or(""),
                p["poke"].as_str().unwrap_or(""),
                p["note"].as_str().unwrap_or("")
            ));
        }
        s
    })
}

/// `remodel ram search ...` and `remodel ram show ...`.
pub fn run(o: &crate::Opts, roms: &RomSet) -> ExitCode {
    let mut args = o.args[1..].to_vec();
    let verb = if args.is_empty() { String::new() } else { args.remove(0) };
    let c = match tour::common(&mut args) {
        Ok(c) => c,
        Err(e) => return fail(answer::BAD_REQUEST, e, None).finish(false, |_| String::new()),
    };
    let json = c.json;
    let a = match verb.as_str() {
        "search" | "show" => work(&verb, &mut args, &c, roms),
        "pokes" => return pokes(o),
        v => fail(
            answer::BAD_REQUEST,
            format!("ram {v}: no such verb{}", tour::near(v, &["search", "show", "pokes"])),
            Some("remodel ram search TOUR --at F1,F2 --rule diff"),
        ),
    };
    a.finish(json, |d| {
        let mut s = String::new();
        if let Some(n) = d["count"].as_u64() {
            s.push_str(&format!(
                "{n} candidate(s) at frames {}, {} byte(s) wide; first {}:\n",
                d["frames"],
                d["width"],
                d["candidates"].as_array().map_or(0, |c| c.len())
            ));
        }
        for c in d["candidates"].as_array().into_iter().flatten() {
            let vals: Vec<String> = c["values"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|v| format!("{:#x}", v.as_u64().unwrap_or(0)))
                .collect();
            s.push_str(&format!("{}  {}\n", c["addr"].as_str().unwrap_or(""), vals.join(" -> ")));
        }
        s
    })
}

fn work(verb: &str, args: &mut Vec<String>, c: &tour::Common, roms: &RomSet) -> Answer {
    let usage = if verb == "search" {
        "remodel ram search TOUR --at F1,F2,.. --rule R1,R2,.. [--first COND] [--width b|w|l] [--region LO-HI] [--top N]"
    } else {
        "remodel ram show TOUR --at F ADDR[:b|:w|:l]..."
    };
    let bad = |e: String| fail(answer::BAD_REQUEST, e, Some(usage));
    let at = match tour::take_opt(args, "--at").map(|s| frames_list(&s)) {
        Some(Ok(a)) if !a.is_empty() => a,
        Some(Err(e)) => return bad(e),
        _ => return bad("--at F1,F2,...: the frames to look at".into()),
    };
    let rules = tour::take_opt(args, "--rule");
    let first = tour::take_opt(args, "--first");
    let w = tour::take_opt(args, "--width");
    let region = tour::take_opt(args, "--region");
    let top = tour::take_opt(args, "--top");
    let [label] = &args[..] else {
        if verb == "show" && args.len() > 1 {
            return show(args, &at, c, roms);
        }
        return bad("name one tour".into());
    };
    if verb == "show" {
        return bad("name the tour, then the addresses".into());
    }
    let game = roms.game;
    let width = match w.as_deref().map(width).transpose() {
        Ok(w) => w.unwrap_or(1),
        Err(e) => return bad(e),
    };
    let rules: Vec<Rule> = match rules {
        Some(r) => match r.split(',').map(Rule::parse).collect() {
            Ok(r) => r,
            Err(e) => return bad(e),
        },
        None => return bad("--rule R1,R2,..: one rule between each pair of frames".into()),
    };
    if rules.len() + 1 != at.len() {
        return bad(format!("{} frame(s) need {} rule(s), one between each pair", at.len(), at.len() - 1));
    }
    let first = match first.as_deref().map(Rule::parse).transpose() {
        Ok(f) => f.unwrap_or(Rule(vec![])),
        Err(e) => return bad(e),
    };
    let region = match region.as_deref().map(|r| {
        let (a, b) = r.split_once('-').ok_or("--region LO-HI")?;
        Ok::<_, String>((number(a).ok_or("--region: LO is a number")?, number(b).ok_or("--region: HI is a number")?))
    }) {
        Some(Ok(r)) => r,
        Some(Err(e)) => return bad(e),
        None => (0, u32::MAX),
    };
    let top: usize = top.and_then(|t| t.parse().ok()).unwrap_or(40);
    let script = match script_of(label, c, game) {
        Ok(s) => s,
        Err(a) => return a,
    };
    let snaps = match snapshots(roms, &script, &at) {
        Ok(s) => s,
        Err(e) => return fail(answer::INTERNAL, e, None),
    };
    let found = search(&snaps, width, &first, &rules, region);
    let rows: Vec<Value> =
        found.iter().take(top).map(|(a, v)| json!({"addr": format!("{a:#x}"), "values": v})).collect();
    Answer::done(
        json!({"game": game.id, "tour": label, "frames": at, "width": width, "count": found.len(), "candidates": rows}),
    )
    .next("narrow with another --at frame and rule, then poke=ADDR:VALUE[:w]@from-to in a tour")
}

fn script_of(label: &str, c: &tour::Common, game: &romset::Game) -> Result<model2::input::InputScript, Answer> {
    let tours = tour::load(game, c.tours.as_deref()).map_err(|e| fail(answer::BAD_REQUEST, e, None))?;
    let names: Vec<&str> = tours.iter().map(|t| t.label.as_str()).collect();
    if !names.contains(&label) {
        return Err(fail(
            answer::BAD_REQUEST,
            format!("no tour {label} for {}{}", game.id, tour::near(label, &names)),
            Some("remodel tour list"),
        ));
    }
    let table = tour::poke_table(romset::games::pokes(game)).map_err(|e| fail(answer::INTERNAL, e, None))?;
    let items = tour::inputs(&tours, label, game.controls.coin_start)
        .and_then(|i| tour::resolve_pokes(i, &table))
        .map_err(|e| fail(answer::BAD_REQUEST, e, None))?;
    model2::input::InputScript::parse(&items.join(","), &game.controls)
        .map_err(|e| fail(answer::BAD_REQUEST, format!("tour {label}: {e}"), None))
}

/// `ram show TOUR --at F ADDR[:w]...`: the values at those addresses.
fn show(args: &[String], at: &[u64], c: &tour::Common, roms: &RomSet) -> Answer {
    let usage = "remodel ram show TOUR --at F ADDR[:b|:w|:l]...";
    let game = roms.game;
    let script = match script_of(&args[0], c, game) {
        Ok(s) => s,
        Err(a) => return a,
    };
    let snaps = match snapshots(roms, &script, at) {
        Ok(s) => s,
        Err(e) => return fail(answer::INTERNAL, e, None),
    };
    let mut rows = Vec::new();
    for spec in &args[1..] {
        let (a, w) = match spec.split_once(':') {
            Some((a, w)) => (a, width(w)),
            None => (spec.as_str(), Ok(1)),
        };
        let (Some(addr), Ok(w)) = (number(a), w) else {
            return fail(answer::BAD_REQUEST, format!("{spec}: want ADDR[:b|:w|:l]"), Some(usage));
        };
        let mut vals = Vec::new();
        for s in &snaps {
            let v = s.iter().find_map(|(base, b)| {
                let o = addr.checked_sub(*base)? as usize;
                (o + w <= b.len()).then(|| (0..w).fold(0u32, |v, i| v | (b[o + i] as u32) << (8 * i)))
            });
            match v {
                Some(v) => vals.push(v),
                None => {
                    return fail(
                        answer::BAD_REQUEST,
                        format!("{spec}: not in work RAM (0x200000, 0x500000)"),
                        Some(usage),
                    )
                }
            }
        }
        rows.push(json!({"addr": format!("{addr:#x}"), "values": vals}));
    }
    Answer::done(json!({"game": game.id, "frames": at, "width": 0, "count": rows.len(), "candidates": rows}))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(a: &[u8], b: &[u8]) -> Vec<(u32, Vec<u8>)> {
        vec![(0x500000, a.to_vec()), (0x200000, b.to_vec())]
    }

    /// A counter that rises while the rest stays or jitters is found by
    /// `inc` then `same`, and only it.
    #[test]
    fn search_narrows_to_the_variable_that_follows_the_rules() {
        let snaps = [snap(&[1, 5, 9, 7], &[0, 0]), snap(&[2, 5, 3, 7], &[0, 1]), snap(&[2, 5, 4, 7], &[0, 2])];
        let rules = [Rule::parse("inc").unwrap(), Rule::parse("same").unwrap()];
        let got = search(&snaps, 1, &Rule(vec![]), &rules, (0, u32::MAX));
        assert_eq!(got, vec![(0x500000, vec![1, 2, 2])]);
        let rules = [Rule::parse("diff").unwrap(), Rule::parse("inc=1/lt=10").unwrap()];
        let got = search(&snaps, 1, &Rule(vec![]), &rules, (0, u32::MAX));
        assert_eq!(got.iter().map(|g| g.0).collect::<Vec<_>>(), vec![0x500002, 0x200001]);
        // A word reads little-endian; a region limits the addresses.
        let w = search(
            &snaps,
            2,
            &Rule(vec![]),
            &[Rule::parse("any").unwrap(), Rule::parse("any").unwrap()],
            (0x500002, 0x500002),
        );
        assert_eq!(w, vec![(0x500002, vec![0x709, 0x703, 0x704])]);
        assert!(Rule::parse("bogus").is_err());
        assert!(Rule::parse("eq=zz").is_err());
    }
}
