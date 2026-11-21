//! What the launcher lists, apart from how it draws it: the two views, the
//! grid's filters and sort, and the quick resume (the game and state the Y
//! button goes straight back to). Pure functions of the playable games, the
//! recently played and the save files, so they're tested without a window.

use romset::{variants, Game, Kind, VariantKind};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::SystemTime;

/// How many of the recently played the "Recent" filter shows.
pub const RECENT_SHOWN: usize = 8;

/// Whether the set `played` (a title or one of its versions) is a version of
/// the card `game`: a title's card stands for its variants too.
pub fn is_version_of(played: &str, game: &Game) -> bool {
    played == game.id || variants::get(played).is_some_and(|v| v.parent == game.id)
}

/// Where the card `game` is in `recent` (0 the latest played), by any of its
/// versions.
pub fn played_rank(recent: &[String], game: &Game) -> Option<usize> {
    recent.iter().position(|r| is_version_of(r, game))
}

/// The cards to list among `games`: the titles, and a variant whose title is
/// not among them (only its own ROMs are there).
pub fn cards(games: &[&'static Game]) -> Vec<&'static Game> {
    games
        .iter()
        .copied()
        .filter(|g| variants::get(g.id).is_none_or(|v| !games.iter().any(|t| t.id == v.parent)))
        .collect()
}

/// The versions of the card `card` among `games`, in the version list's
/// order: the title itself, then official revisions, regions, other boards,
/// prototypes and hacks (each kind in the table's order): every version
/// there is, none hidden.
pub fn versions(card: &Game, games: &[&'static Game]) -> Vec<&'static Game> {
    let mut out: Vec<(VariantKind, usize, &'static Game)> = Vec::new();
    for g in games {
        if g.id == card.id {
            continue;
        }
        let Some(v) = variants::get(g.id).filter(|v| v.parent == card.id) else { continue };
        let order = variants::all().iter().position(|x| x.id == g.id).unwrap_or(0);
        out.push((v.kind, order, g));
    }
    out.sort_by_key(|&(k, order, _)| (k, order));
    games.iter().copied().filter(|g| g.id == card.id).chain(out.into_iter().map(|o| o.2)).collect()
}

/// A version's name in the list: its label, or the title's own label ("Export").
pub fn version_label(g: &Game) -> String {
    match variants::get(g.id) {
        Some(v) => v.label.to_string(),
        None => variants::parent_label(g.id).unwrap_or("Original").to_string(),
    }
}

/// The version of `card` the player chose (`chosen`: title to set), when it is
/// listed in `versions`; else the card's own.
pub fn chosen_version(
    card: &'static Game,
    versions: &[&'static Game],
    chosen: &BTreeMap<String, String>,
) -> &'static Game {
    chosen.get(card.id).and_then(|set| versions.iter().copied().find(|g| g.id == set)).unwrap_or_else(|| {
        versions.iter().copied().find(|g| g.id == card.id).or_else(|| versions.first().copied()).unwrap_or(card)
    })
}

/// The launcher's two looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ViewMode {
    /// One game large, the next ones beside it.
    #[default]
    Carousel,
    /// Every game as a poster, with filters.
    Grid,
}

impl ViewMode {
    pub const ALL: [ViewMode; 2] = [ViewMode::Carousel, ViewMode::Grid];

    pub fn label(self) -> &'static str {
        match self {
            ViewMode::Carousel => "Carousel",
            ViewMode::Grid => "Grid",
        }
    }

    /// The name the settings file keeps it under.
    pub fn name(self) -> &'static str {
        match self {
            ViewMode::Carousel => "carousel",
            ViewMode::Grid => "grid",
        }
    }

    pub fn parse(text: &str) -> Option<ViewMode> {
        ViewMode::ALL.into_iter().find(|v| v.name() == text.trim().to_lowercase())
    }

    /// The other view (LB and RB switch between the two).
    pub fn other(self) -> ViewMode {
        match self {
            ViewMode::Carousel => ViewMode::Grid,
            ViewMode::Grid => ViewMode::Carousel,
        }
    }
}

/// The grid's filter chips.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    /// The games played lately (`RECENT_SHOWN`).
    Recent,
    Kind(Kind),
}

impl Filter {
    pub fn label(self) -> &'static str {
        match self {
            Filter::All => "All",
            Filter::Recent => "Recent",
            Filter::Kind(k) => k.label(),
        }
    }

    /// The chips to offer for `games`: All, Recent (when something has been
    /// played), then each kind that has a game.
    pub fn chips(games: &[&Game], recent: &[String]) -> Vec<Filter> {
        let mut chips = vec![Filter::All];
        if games.iter().any(|g| played_rank(recent, g).is_some_and(|i| i < RECENT_SHOWN)) {
            chips.push(Filter::Recent);
        }
        chips.extend(Kind::ALL.into_iter().filter(|k| games.iter().any(|g| g.kind == *k)).map(Filter::Kind));
        chips
    }

    fn keeps(self, game: &Game, recent: &[String]) -> bool {
        match self {
            Filter::All => true,
            Filter::Recent => played_rank(recent, game).is_some_and(|i| i < RECENT_SHOWN),
            Filter::Kind(k) => game.kind == k,
        }
    }
}

/// The grid's sort.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sort {
    /// The recently played first (latest first), then the rest A to Z.
    #[default]
    Recent,
    AtoZ,
}

impl Sort {
    pub fn label(self) -> &'static str {
        match self {
            Sort::Recent => "Recently played",
            Sort::AtoZ => "A to Z",
        }
    }

    pub fn other(self) -> Sort {
        match self {
            Sort::Recent => Sort::AtoZ,
            Sort::AtoZ => Sort::Recent,
        }
    }
}

/// The games to list: those of `games` (the playable ones, whose ROMs are
/// there) that `filter` keeps, in `sort`'s order.
pub fn list(games: &[&'static Game], recent: &[String], filter: Filter, sort: Sort) -> Vec<&'static Game> {
    let mut out: Vec<&'static Game> = games.iter().copied().filter(|g| filter.keeps(g, recent)).collect();
    let rank = |g: &Game| played_rank(recent, g).unwrap_or(usize::MAX);
    out.sort_by_key(|g| match sort {
        Sort::Recent => (rank(g), g.title, g.id),
        Sort::AtoZ => (0, g.title, g.id),
    });
    out
}

/// Where a game starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Start {
    /// Power-on.
    New,
    /// Where it was left (`saves::resume_file`).
    Resume,
    /// A save slot: 0 the quick save.
    Slot(u8),
}

impl Start {
    /// What to call it: "where you left off", "quick save", "slot 2".
    pub fn label(self) -> String {
        match self {
            Start::New => "from the start".into(),
            Start::Resume => "where you left off".into(),
            Start::Slot(0) => "quick save".into(),
            Start::Slot(n) => format!("slot {n}"),
        }
    }
}

/// The states saved in `dir` (a game's `saves` folder) and when each was
/// written: the resume state, the quick save and the numbered slots.
pub fn saved_in(dir: &Path) -> Vec<(Start, SystemTime)> {
    let files = std::iter::once(("resume.save".to_string(), Start::Resume)).chain(
        (0..crate::menu::SLOTS)
            .map(|n| (if n == 0 { "quick.save".to_string() } else { format!("slot{n}.save") }, Start::Slot(n))),
    );
    files
        .filter_map(|(name, start)| {
            let time = std::fs::metadata(dir.join(name)).and_then(|m| m.modified()).ok()?;
            Some((start, time))
        })
        .collect()
}

/// The latest of `saved`.
pub fn latest(saved: &[(Start, SystemTime)]) -> Option<(Start, SystemTime)> {
    saved.iter().copied().reduce(|a, b| if b.1 > a.1 { b } else { a })
}

/// The quick resume: the game last played and the state to pick it up from.
#[derive(Clone, Copy)]
pub struct Continue {
    pub game: &'static Game,
    pub from: Start,
    /// When that state was saved (none: it starts from the start).
    pub saved: Option<SystemTime>,
}

/// The game last played among `games` (`recent` is most recent first) and
/// its latest state (`saved` lists a game's), else from the start; none if
/// nothing has been played.
pub fn quick_resume(
    games: &[&'static Game],
    recent: &[String],
    saved: impl Fn(&str) -> Vec<(Start, SystemTime)>,
) -> Option<Continue> {
    let game = recent.iter().find_map(|r| games.iter().copied().find(|g| g.id == r))?;
    Some(match latest(&saved(game.id)) {
        Some((from, at)) => Continue { game, from, saved: Some(at) },
        None => Continue { game, from: Start::New, saved: None },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn games(ids: &[&str]) -> Vec<&'static Game> {
        ids.iter().map(|id| romset::games::find(id).unwrap()).collect()
    }

    fn ids(list: &[&Game]) -> Vec<&'static str> {
        list.iter().map(|g| g.id).collect()
    }

    fn recent(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    const SET: [&str; 6] = ["topskatr", "hotd", "vf2", "stcc", "srallyc", "vcop"];

    #[test]
    fn views_round_trip_through_their_names() {
        for v in ViewMode::ALL {
            assert_eq!(ViewMode::parse(v.name()), Some(v));
            assert_eq!(v.other().other(), v);
        }
        assert_eq!(ViewMode::parse(" Grid "), Some(ViewMode::Grid));
        assert_eq!(ViewMode::parse("list"), None);
        assert_eq!(ViewMode::default(), ViewMode::Carousel);
    }

    #[test]
    fn chips_are_the_kinds_present() {
        let none = recent(&[]);
        let chips =
            |ids: &[&str], r: &[String]| Filter::chips(&games(ids), r).iter().map(|f| f.label()).collect::<Vec<_>>();
        assert_eq!(chips(&SET, &none), ["All", "Racing", "Fighting", "Shooting", "Sports"]);
        // Nothing of a kind, no chip for it; Recent once something is played.
        assert_eq!(chips(&["hotd", "vcop"], &none), ["All", "Shooting"]);
        assert_eq!(chips(&["hotd", "vcop"], &recent(&["vcop"])), ["All", "Recent", "Shooting"]);
        // A played game that isn't here doesn't make Recent.
        assert_eq!(chips(&["hotd"], &recent(&["topskatr"])), ["All", "Shooting"]);
    }

    #[test]
    fn filters_keep_what_they_name() {
        let g = games(&SET);
        let r = recent(&["vf2", "hotd"]);
        let by = |f| ids(&list(&g, &r, f, Sort::AtoZ));
        assert_eq!(by(Filter::All).len(), 6);
        assert_eq!(by(Filter::Kind(Kind::Racing)), ["srallyc", "stcc"]);
        assert_eq!(by(Filter::Kind(Kind::Fighting)), ["vf2"]);
        assert_eq!(by(Filter::Kind(Kind::Shooting)), ["hotd", "vcop"]);
        assert_eq!(by(Filter::Kind(Kind::Sports)), ["topskatr"]);
        assert_eq!(by(Filter::Recent), ["hotd", "vf2"], "A to Z, even among the recent");
    }

    #[test]
    fn recent_sort_puts_the_latest_played_first_then_a_to_z() {
        let g = games(&SET);
        let r = recent(&["stcc", "gone", "hotd"]);
        assert_eq!(
            ids(&list(&g, &r, Filter::All, Sort::Recent)),
            ["stcc", "hotd", "srallyc", "topskatr", "vcop", "vf2"]
        );
        assert_eq!(ids(&list(&g, &r, Filter::Recent, Sort::Recent)), ["stcc", "hotd"]);
        // By title: Sega Rally, Sega Touring Car, The House of the Dead...
        assert_eq!(ids(&list(&g, &r, Filter::All, Sort::AtoZ)), ["srallyc", "stcc", "hotd", "topskatr", "vcop", "vf2"]);
    }

    #[test]
    fn only_games_given_are_listed() {
        // The ROMs a player has are the games: a set that isn't passed in
        // never shows, whatever the played list says.
        let g = games(&["hotd"]);
        let r = recent(&["topskatr", "hotd"]);
        for f in [Filter::All, Filter::Recent, Filter::Kind(Kind::Sports)] {
            assert!(!ids(&list(&g, &r, f, Sort::Recent)).contains(&"topskatr"));
        }
    }

    #[test]
    fn every_game_has_a_genre_and_every_kind_a_game() {
        for g in romset::games::ALL {
            assert!(!g.genre.is_empty(), "{}", g.id);
        }
        for k in Kind::ALL {
            assert!(romset::games::ALL.iter().any(|g| g.kind == k), "{k:?}");
        }
    }

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn the_latest_state_wins() {
        assert_eq!(latest(&[]), None);
        let s = [(Start::Resume, at(10)), (Start::Slot(0), at(30)), (Start::Slot(2), at(20))];
        assert_eq!(latest(&s), Some((Start::Slot(0), at(30))));
    }

    #[test]
    fn quick_resume_is_the_last_game_and_its_latest_state() {
        let g = games(&SET);
        let states = |set: &str| match set {
            "hotd" => vec![(Start::Resume, at(100)), (Start::Slot(3), at(200))],
            _ => Vec::new(),
        };
        // The latest played among the games present (the unlisted skipped).
        let c = quick_resume(&g, &recent(&["gone", "hotd", "vf2"]), states).unwrap();
        assert_eq!((c.game.id, c.from, c.saved), ("hotd", Start::Slot(3), Some(at(200))));
        // With no save, from the start.
        let c = quick_resume(&g, &recent(&["vf2", "hotd"]), states).unwrap();
        assert_eq!((c.game.id, c.from, c.saved), ("vf2", Start::New, None));
        // Nothing played (or nothing played is here): no quick resume.
        assert!(quick_resume(&g, &recent(&[]), states).is_none());
        assert!(quick_resume(&g, &recent(&["gone"]), states).is_none());
    }

    #[test]
    fn saved_states_are_read_from_the_folder() {
        let dir = std::env::temp_dir().join(format!("remodel-library-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(saved_in(&dir).is_empty());
        for name in ["resume.save", "slot2.save", "slot9.save", "notes.txt"] {
            std::fs::write(dir.join(name), b"x").unwrap();
        }
        let found: Vec<Start> = saved_in(&dir).iter().map(|s| s.0).collect();
        assert_eq!(found, [Start::Resume, Start::Slot(2)], "slot 9 is past the slots listed");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn states_are_named() {
        assert_eq!(Start::Slot(0).label(), "quick save");
        assert_eq!(Start::Slot(3).label(), "slot 3");
        assert_eq!(Start::Resume.label(), "where you left off");
    }
}
