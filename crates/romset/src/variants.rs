//! Variants: another release of a supported title (a revision, a region, a
//! prototype, a hack, a clone on another board), written as a thin manifest
//! that inherits from its parent and resolved once into a full [`Game`], so
//! everything downstream (the board model, the player, `pack_sets`, the CLI)
//! keeps using `Game` unchanged (docs/variants.md).
//!
//! A [`Variant`] names its parent, says which of the parent's chips it
//! swaps for its own (looked up as `<variant id>/<chip>` in folders and in
//! the parent's zip before the bare name, as a merged set keeps them) and
//! overrides only the per-game data that differed in practice: the idle
//! loops, the attract frame, the protection answers' ROM addresses, the
//! backup RAM presets, the board and packing of a cross-board clone, the
//! link play data. Everything else (controls, drive and link boards, input
//! lag, shading, `recommended`) is the parent's. **Scenes and tours are not
//! inherited**: they are frame counts from power-on, and another program's
//! timing differs; a variant has its own or none.
//!
//! The data is in `games/variants.rs`.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use crate::{Board, Game, IdleLoop, LinkPlay, Protection, Recommended, Role, RomFile, Scene, Shading};

/// What sort of release a variant is. The launcher lists every kind in a
/// title's Version list, a prototype and a hack tagged as such.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum VariantKind {
    /// Another official revision of the program.
    Revision,
    /// The same game for another market (Japan, USA).
    Region,
    /// The same program on another board (a cross-board clone).
    CrossBoard,
    /// A prototype or location-test build.
    Prototype,
    /// A modified set (a fan or third-party hack of an official one).
    Hack,
}

impl VariantKind {
    /// The kind's name, as the launcher's badge shows it.
    pub fn label(self) -> &'static str {
        match self {
            VariantKind::Revision => "Revision",
            VariantKind::Region => "Region",
            VariantKind::CrossBoard => "Other board",
            VariantKind::Prototype => "Prototype",
            VariantKind::Hack => "Hack",
        }
    }
}

/// One chip of a variant in place of one of its parent's: `chip` takes the
/// place (the role and socket order) of the parent's chip named `parent`.
#[derive(Clone, Copy, Debug)]
pub struct ChipSwap {
    pub parent: &'static str,
    pub chip: RomFile,
}

/// Where a variant's link play data comes from.
#[derive(Clone, Copy, Debug)]
pub enum VariantLink {
    /// The parent's (the variant's settings record was checked to be the
    /// parent's: `srallycb`, `srallyp`, ...).
    Parent,
    /// None: not surveyed or another layout.
    None,
    /// Its own.
    Own(&'static LinkPlay),
}

/// A thin manifest over a parent's [`Game`]. `None` and empty fields inherit.
#[derive(Clone, Copy, Debug)]
pub struct Variant {
    /// The variant's set id: what `--game` takes, the folder of its EEPROM,
    /// saves and settings.
    pub id: &'static str,
    /// The title's parent set.
    pub parent: &'static str,
    /// A short name for the version list ("Rev B", "Japan", "Twin").
    pub label: &'static str,
    pub kind: VariantKind,
    /// Whether `kind` rests on more than the set's name and chips (the
    /// survey's reading is in docs/variants.md; false: unconfirmed).
    pub kind_confirmed: bool,
    /// The game's own title, where it differs from the parent's.
    pub title: Option<&'static str>,
    pub year: Option<u16>,
    /// Chips swapped for the parent's.
    pub swaps: &'static [ChipSwap],
    /// Parent chips this variant lacks.
    pub drop: &'static [&'static str],
    /// Chips the variant has that its parent lacks, after the parent's (a
    /// cross-board clone's own program chips and the other board's extras).
    pub add: &'static [RomFile],
    /// Its textures are its own: texture packs do not fall back to the
    /// parent's (`dayton93`).
    pub own_textures: bool,
    pub board: Option<Board>,
    pub packed: Option<&'static [Role]>,
    pub attract_frame: Option<u64>,
    pub idle_loops: Option<&'static [IdleLoop]>,
    /// The protection answers at other ROM addresses: (reply index, address).
    pub protection_rom: &'static [(usize, u32)],
    /// The `coin-start` script where the parent's does not reach play (the program
    /// takes a start press later: `indy500to`).
    pub coin_start: Option<&'static str>,
    pub backup_ram: Option<&'static [(usize, &'static [u8])]>,
    pub volume_db: Option<f32>,
    pub texture_ram_mirrored: Option<bool>,
    pub shading: Option<Shading>,
    pub recommended: Option<Recommended>,
    pub input_lag: Option<u8>,
    pub run_ahead_max: Option<u8>,
    /// The variant's own scenes (never the parent's).
    pub scenes: &'static [Scene],
    pub link: VariantLink,
    /// It boots, attracts and reaches play. False: not listed in the launcher or the web build
    /// (`--game ID` still takes it, for the work of finishing it), as `games::IN_PROGRESS` does
    /// for titles (docs/variants.md, class C).
    pub plays: bool,
}

impl Variant {
    /// The defaults of a variant: inherit everything. Write `Variant { id,
    /// parent, label, kind, swaps, ..Variant::BASE }`.
    pub const BASE: Variant = Variant {
        id: "",
        parent: "",
        label: "",
        kind: VariantKind::Revision,
        kind_confirmed: false,
        title: None,
        year: None,
        swaps: &[],
        drop: &[],
        add: &[],
        own_textures: false,
        board: None,
        packed: None,
        attract_frame: None,
        idle_loops: None,
        protection_rom: &[],
        coin_start: None,
        backup_ram: None,
        volume_db: None,
        texture_ram_mirrored: None,
        shading: None,
        recommended: None,
        input_lag: None,
        run_ahead_max: None,
        scenes: &[],
        link: VariantLink::None,
        plays: true,
    };

    /// The chips of `game` (this variant resolved) that are the variant's
    /// own: the swapped ones, which a set that has the parent's chips
    /// must add.
    pub fn own_chips<'a>(&'a self, game: &'a Game) -> impl Iterator<Item = &'a RomFile> + 'a {
        game.files.iter().filter(|f| self.swaps.iter().any(|s| s.chip.name == f.name && s.chip.sha1 == f.sha1))
    }

    /// The variant's [`Game`]: the parent's with the swaps and overrides
    /// applied. None if the parent is unknown or a swap names a chip the
    /// parent lacks (a manifest error that the tests catch).
    fn build(&'static self, parent: &'static Game) -> Option<Game> {
        let mut files: Vec<RomFile> = Vec::with_capacity(parent.files.len());
        for f in parent.files {
            if self.drop.contains(&f.name) {
                continue;
            }
            files.push(self.swaps.iter().find(|s| s.parent == f.name).map_or(*f, |s| s.chip));
        }
        files.extend_from_slice(self.add);
        if self.swaps.iter().any(|s| !parent.files.iter().any(|f| f.name == s.parent)) {
            return None;
        }
        let mut g = parent.clone();
        g.id = self.id;
        g.parent = Some(parent.id);
        g.files = Box::leak(files.into_boxed_slice());
        g.scenes = self.scenes;
        if let Some(t) = self.title {
            g.title = t;
        }
        if let Some(y) = self.year {
            g.year = y;
        }
        if let Some(b) = self.board {
            g.board = b;
        }
        if let Some(p) = self.packed {
            g.packed = p;
        }
        if let Some(a) = self.attract_frame {
            g.attract_frame = a;
        }
        if let Some(c) = self.coin_start {
            g.controls.coin_start = c;
        }
        if let Some(i) = self.idle_loops {
            g.idle_loops = i;
        }
        if let Some(b) = self.backup_ram {
            g.backup_ram = b;
        }
        if let Some(v) = self.volume_db {
            g.volume_db = v;
        }
        if let Some(m) = self.texture_ram_mirrored {
            g.texture_ram_mirrored = m;
        }
        if let Some(s) = self.shading {
            g.shading = s;
        }
        if let Some(r) = self.recommended {
            g.recommended = r;
        }
        if let Some(l) = self.input_lag {
            g.input_lag = l;
        }
        if let Some(r) = self.run_ahead_max {
            g.run_ahead_max = r;
        }
        if !self.protection_rom.is_empty() {
            let p = parent.protection.as_ref()?;
            let mut replies = p.replies.to_vec();
            for &(i, rom) in self.protection_rom {
                replies.get_mut(i)?.rom = rom;
            }
            g.protection = Some(Protection { replies: Box::leak(replies.into_boxed_slice()), ..*p });
        }
        Some(g)
    }
}

/// Every variant of every supported title.
pub fn all() -> &'static [Variant] {
    crate::games::VARIANTS
}

/// The variant with set id `id`, if there is one.
pub fn get(id: &str) -> Option<&'static Variant> {
    all().iter().find(|v| v.id == id)
}

/// The variants of title `parent`, in the table's order.
pub fn of(parent: &str) -> impl Iterator<Item = &'static Variant> + '_ {
    all().iter().filter(move |v| v.parent == parent)
}

/// The title's own set name for `id`: the parent of a variant, else `id`.
pub fn title_of(id: &str) -> &str {
    get(id).map_or(id, |v| v.parent)
}

/// The title's own version name in the version list ("Export", "Rev C"), if
/// the title has variants.
pub fn parent_label(title: &str) -> Option<&'static str> {
    crate::games::PARENT_LABELS.iter().find(|p| p.0 == title).map(|p| p.1)
}

/// A set as a message names it: the title, and the version when the title has
/// variants ("Top Skater (Japan)", "Top Skater (Export)"); the set id for an
/// unknown one.
pub fn display_name(id: &str) -> String {
    let title = |t: &str| crate::games::title(t).map_or_else(|| t.to_string(), |g| g.title.to_string());
    match (get(id), parent_label(id)) {
        (Some(v), _) => format!("{} ({})", title(v.parent), v.label),
        (None, Some(label)) => format!("{} ({label})", title(id)),
        (None, None) => title(id),
    }
}

/// The variant resolved into a full [`Game`], once: later calls return the
/// same one. None if its parent is not a supported game or the manifest is
/// inconsistent.
pub fn resolve(v: &'static Variant) -> Option<&'static Game> {
    static CACHE: OnceLock<Mutex<HashMap<&'static str, &'static Game>>> = OnceLock::new();
    let mut cache = CACHE.get_or_init(Default::default).lock().unwrap();
    if let Some(g) = cache.get(v.id) {
        return Some(g);
    }
    let parent = crate::games::title(v.parent)?;
    let g: &'static Game = Box::leak(Box::new(v.build(parent)?));
    cache.insert(v.id, g);
    Some(g)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games;

    #[test]
    fn every_variant_resolves_and_is_consistent() {
        let mut ids = std::collections::HashSet::new();
        for v in all() {
            assert!(ids.insert(v.id), "{}: listed twice", v.id);
            assert!(games::ALL.iter().all(|g| g.id != v.id), "{}: also a title", v.id);
            let parent = games::title(v.parent).unwrap_or_else(|| panic!("{}: parent {} unknown", v.id, v.parent));
            let g = resolve(v).unwrap_or_else(|| panic!("{}: does not resolve", v.id));
            assert_eq!((g.id, g.parent), (v.id, Some(parent.id)));
            assert!(!v.label.is_empty());
            // Same pairing as a title: chips of a role come in pairs.
            for role in [Role::Program, Role::MainData, Role::Polygons, Role::Textures, Role::CoproData] {
                assert!(g.files.iter().filter(|f| f.role == role).count() % 2 == 0, "{}: {role:?}", v.id);
            }
            // Its own chips are named: at least one swap, or it is a cross-board clone.
            assert!(!v.swaps.is_empty(), "{}: swaps nothing", v.id);
            // A swap keeps the chip's role unless it says otherwise: same role as the parent's.
            for s in v.swaps {
                let p = parent.files.iter().find(|f| f.name == s.parent).unwrap();
                assert_eq!(p.role, s.chip.role, "{}: {} -> {}", v.id, s.parent, s.chip.name);
            }
            // Scenes are never inherited.
            assert!(g.scenes.iter().all(|s| v.scenes.contains(s)), "{}", v.id);
            // The variant's own set ids do not collide: the resolved game is the same one next time.
            assert!(std::ptr::eq(g, resolve(v).unwrap()));
        }
    }

    #[test]
    fn a_variant_inherits_what_it_does_not_override() {
        let v = get("topskatrj").unwrap();
        let (g, p) = (resolve(v).unwrap(), games::find("topskatr").unwrap());
        assert_eq!(g.title, p.title);
        assert_eq!(g.idle_loops, p.idle_loops);
        assert_eq!(g.controls.coin_start, p.controls.coin_start);
        assert_ne!(g.files[0].sha1, p.files[0].sha1, "its program chip is its own");
        assert_eq!(g.files[2].sha1, p.files[2].sha1, "the rest are the parent's");
        assert!(g.scenes.is_empty() && !p.scenes.is_empty(), "scenes are not inherited");
        assert!(games::tours(g).is_empty() && !games::tours(p).is_empty(), "tours are not inherited");
    }

    #[test]
    fn the_overrides_a_variant_may_make_take_effect() {
        // Its own textures: no fall back to the parent's pack; the others fall back.
        let sets = |id| crate::pack_sets(games::find(id).unwrap()).collect::<Vec<_>>();
        assert_eq!(sets("dayton93"), ["dayton93"]);
        assert_eq!(sets("daytonat"), ["daytonat", "daytona"]);
        // A start script of its own, only where the variant gives one.
        let (to, p) = (games::find("indy500to").unwrap(), games::find("indy500").unwrap());
        assert_ne!(to.controls.coin_start, p.controls.coin_start);
        assert_eq!(games::find("indy500d").unwrap().controls.coin_start, p.controls.coin_start);
        // A cross-board clone: the other board, its extra chips after the parent's, the protection
        // answer at its own ROM address and the rest of the protection the parent's.
        let (a, b) = (games::find("doaa").unwrap(), games::find("doa").unwrap());
        assert_eq!(a.board, Board::Model2A);
        assert_eq!(a.files.len(), b.files.len() + 6);
        assert!(a.files[b.files.len()..].iter().all(|f| f.role == Role::CoproData));
        let (pa, pb) = (a.protection.as_ref().unwrap(), b.protection.as_ref().unwrap());
        assert_eq!((pa.replies[0].rom, pb.replies[0].rom), (0x2830, 0x2810));
        assert_eq!((pa.replies[0].command, pa.replies[0].len), (pb.replies[0].command, pb.replies[0].len));
        // The 2A clones of 2B sets: the other board, the parent's chips in the parent's roles and
        // the six 2A geometry programs after them.
        let (z, p) = (games::find("zeroguna").unwrap(), games::find("zerogun").unwrap());
        assert_eq!(z.board, Board::Model2A);
        assert_eq!(z.files.len(), p.files.len() + 6);
        assert_eq!(z.protection.as_ref().unwrap().replies[0].rom, p.protection.as_ref().unwrap().replies[0].rom);
        // Class C clones that do not play yet resolve (`--game` takes them) but are not listed.
        let b = get("dynmcopb").unwrap();
        assert!(!b.plays && resolve(b).is_some());
        assert!(all().iter().filter(|v| v.plays).count() < all().len());
        // The preset settings record of a clone whose program rejects the parent's.
        let (s, p) = (games::find("daytonas").unwrap(), games::find("daytona").unwrap());
        assert_ne!(s.backup_ram, p.backup_ram);
        assert_eq!(s.backup_ram[0].1[0x0b], 0, "SINGLE");
    }
}
