//! The player's cheats (docs/play.md, "Cheats"): which of the game's poke
//! table's labelled lines are on. The session holds the set as
//! `Model2Bus::holds` (written into RAM before every frame, in the game's
//! frames, run-ahead's and photo mode's replays alike; the save header
//! records the names), and everything here is the set's bookkeeping, apart
//! from the machine so it can be tested without one.

use crate::menu::CheatRow;
use model2::bus::Hold;
use romset::pokes::Poke;
use std::sync::Arc;

/// The holds for the cheats of `defs` named in `names`, in the table's order
/// (a name the table does not have is ignored).
pub fn holds(defs: &[Poke], names: &[String]) -> Arc<[Hold]> {
    defs.iter()
        .filter(|p| names.contains(&p.name))
        .map(|p| Hold { name: p.name.clone(), addr: p.addr, value: p.value, width: p.width })
        .collect()
}

/// `on` with `name` turned over: its new set, and whether it is on now.
pub fn toggled(on: &[String], name: &str) -> (Vec<String>, bool) {
    let mut names = on.to_vec();
    let now = !names.iter().any(|n| n == name);
    if now {
        names.push(name.to_string());
    } else {
        names.retain(|n| n != name);
    }
    (names, now)
}

/// The Cheats page's rows for `defs`, with the cheats in `held` on.
pub fn rows(defs: &[Poke], held: &[Hold]) -> Vec<CheatRow> {
    defs.iter()
        .map(|p| CheatRow {
            label: p.cheat.clone().unwrap_or_else(|| p.name.clone()),
            note: p.short_note().to_string(),
            on: held.iter().any(|h| h.name == p.name),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::saves;

    fn table() -> Vec<Poke> {
        romset::pokes::parse(
            "clock|0x500010:0x0e10:w|Held at 60 s. More.|cheat: Time never runs out\n\
             free|0x500020:1|Free continues.|cheat: Free continues\n",
        )
        .unwrap()
    }

    #[test]
    fn the_set_is_toggled_and_its_holds_follow_the_table() {
        let defs = table();
        let none: Vec<String> = Vec::new();
        assert!(holds(&defs, &none).is_empty());
        let (a, now) = toggled(&none, "free");
        assert_eq!((a.as_slice(), now), (&["free".to_string()][..], true));
        let (b, _) = toggled(&a, "clock");
        let h = holds(&defs, &b);
        assert_eq!(h.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), ["clock", "free"], "the table's order");
        assert_eq!((h[0].addr, h[0].value, h[0].width), (0x500010, 0x0e10, 2));
        let (c, now) = toggled(&b, "free");
        assert_eq!((c.as_slice(), now), (&["clock".to_string()][..], false));
        // A name the table doesn't have (a save from another table) is ignored.
        assert_eq!(holds(&defs, &["gone".to_string()]).len(), 0);
        let r = rows(&defs, &h);
        assert_eq!((r[0].label.as_str(), r[0].note.as_str(), r[0].on), ("Time never runs out", "Held at 60 s.", true));
        assert_eq!((r[1].label.as_str(), r[1].on), ("Free continues", true));
        assert!(!rows(&defs, &[])[0].on);
    }

    /// A save made with cheats on carries their names in its header, the
    /// slots page reads them back as labels, and a save made with none (or
    /// by an older reModel, format 3) reads as none.
    #[test]
    fn a_marked_save_reads_back_its_cheats_and_an_old_one_none() {
        let game = romset::games::find("topskatr").unwrap();
        let roms = romset::RomSet { game, program: vec![0; 0x1000], chips: vec![Vec::new(); game.files.len()] };
        let mut m = model2::Machine::new(&roms);
        let defs = romset::pokes::cheats(game);
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/cheats-test");
        std::fs::create_dir_all(&dir).unwrap();
        let plain = dir.join("plain.save");
        std::fs::write(&plain, model2::save_session(&mut m, None)).unwrap();
        let info = saves::read_info(&plain).unwrap();
        assert!(info.cheats.is_empty() && info.loads);
        m.bus.holds = holds(&defs, &["hold-clock".to_string()]);
        let marked = dir.join("marked.save");
        std::fs::write(&marked, model2::save_session(&mut m, None)).unwrap();
        let info = saves::read_info(&marked).unwrap();
        assert_eq!(info.cheats, ["Time never runs out"], "the label, as the slots page shows it");
        assert!(info.loads);
        assert!(saves::when_line(&info).ends_with(" · Time never runs out"));
        // What a load restores: the names, from the head of the file.
        let data = std::fs::read(&marked).unwrap();
        let names = model2::save_info(&data[..data.len().min(512)]).unwrap().cheats;
        assert_eq!(names, ["hold-clock"]);
        let mut other = model2::Machine::new(&roms);
        model2::load_session(&mut other, None, &data).unwrap();
        other.bus.holds = holds(&defs, &names);
        assert_eq!(other.bus.holds, m.bus.holds);
        // A save from an older reModel (format 3, a header as it always was) loads, with no cheats.
        let old = std::fs::read(&plain).unwrap();
        assert!(old.starts_with(b"REMODEL-SAVE-3\n"));
        assert!(model2::save_info(&old[..old.len().min(512)]).unwrap().cheats.is_empty());
        model2::load_session(&mut other, None, &old).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }
}
