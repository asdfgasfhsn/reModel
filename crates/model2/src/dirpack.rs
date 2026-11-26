//! A texture pack directory for the GPU renderer (`gpu::Renderer::set_pack`),
//! as the native player and `remodel pack-log` read it:
//! PNGs named `tex_<key>_<w>x<h>.png` and `tile_<key>.png`, and an optional
//! manifest `pack.txt` (docs/texture-packs.md, "Pack format"), decoded on
//! background threads so a new texture never holds up a frame. Until its
//! image is ready a texture is drawn from the original (`Found::Pending`).
//!
//! The renderer keeps the images it uploads, so an image is handed over once
//! and not kept here; one the renderer drops (over its memory budget, or the
//! filter setting changed) is decoded again when next asked for.
//!
//! Material maps (a format 2 pack's `.n`, `.s`, `.g` files) are decoded and
//! assembled on the same threads when the renderer asks for them, and handed
//! over the same way.

use crate::enhance::{
    material_id, Found, FoundMaterial, Material, PackChanges, PreloadKind, Rgba, TexInfo, TexturePack, TileInfo,
};
use crate::pack::{parse_name, resolve_maps, FileKind, Manifest};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Threads decoding images.
const WORKERS: usize = 2;
/// Preloaded images asked for and not yet handed over, at most, per kind.
const PRELOAD_AHEAD: usize = 6;

pub struct DirPack {
    /// File stems of the PNGs in the directory.
    names: HashSet<String>,
    /// The per-palette variants (`tex_<key>~<pal>_<w>x<h>`): the palettes
    /// each key has one for. Empty for a pack without any, which is then
    /// asked nothing more per draw (`TexturePack::variant`).
    variants: HashMap<u64, HashSet<u32>>,
    /// File stems of the material maps (none unless the manifest
    /// `reads_maps`).
    maps: HashSet<String>,
    manifest: Arc<Manifest>,
    /// The pack's region overrides (`uses regions`), loaded when it is
    /// opened (a few images, read here, not on the pack threads).
    regions: Option<Arc<crate::regions::RegionSet>>,
    /// Images asked for and not yet handed over.
    asked: HashSet<String>,
    /// Texture and tile images by key, in file name order (for `preload`),
    /// and how far preloading has got in each.
    preload_list: [Vec<(u64, String)>; 2],
    preload_cursor: [usize; 2],
    /// Names preloading asked for and has not handed over, and those
    /// handed over on demand (preloading skips them).
    preload_want: HashSet<String>,
    handed: HashSet<String>,
    /// Decoded tile images, kept (4 KB each at 4x): a tile the renderer
    /// gave up for room is asked for again constantly, and decoding it
    /// again shows the original meanwhile.
    tile_cache: HashMap<String, Arc<Rgba>>,
    /// Decoded images (None: unreadable), not yet handed over.
    ready: HashMap<String, Option<Arc<Rgba>>>,
    /// The same for materials, by id.
    asked_materials: HashSet<u64>,
    ready_materials: HashMap<u64, Option<Arc<Material>>>,
    to_workers: Sender<Job>,
    from_workers: Receiver<Done>,
    /// The directory watch (`watch`): files changed, as it settles them.
    from_watch: Receiver<Vec<Change>>,
    /// The watch thread ends when this is dropped.
    _alive: Arc<()>,
}

/// A file of the pack that was added, changed or removed, and when.
struct Change {
    /// The file name.
    file: String,
    /// Its modification time (the time of the removal if `!exists`).
    at: SystemTime,
    exists: bool,
}

/// How often the directory is looked at: this often, or when slower, eight
/// times as long as a look takes.
const WATCH_EVERY: Duration = Duration::from_millis(100);

/// A file's modification time and size.
type Stamp = (SystemTime, u64);
type Stamps = HashMap<String, Stamp>;

/// The files the pack reads, by name.
fn stamps(dir: &Path) -> Stamps {
    let mut out = Stamps::new();
    let Ok(rd) = std::fs::read_dir(dir) else { return out };
    for e in rd.flatten() {
        let Some(name) = e.file_name().to_str().map(str::to_string) else { continue };
        if !(name.ends_with(".png") || name == crate::pack::MANIFEST || name == crate::regions::FILE) {
            continue;
        }
        if let Ok(m) = e.metadata() {
            out.insert(name, (m.modified().unwrap_or(UNIX_EPOCH), m.len()));
        }
    }
    out
}

/// Watch `dir` from the state `known` on a thread of its own, until `alive`
/// is gone. A change is reported once its file has stayed the same for a
/// whole look, so an editor that writes in pieces is not read half way (a
/// file that then does not decode keeps the old image).
fn watch(dir: PathBuf, mut known: Stamps, alive: &Arc<()>) -> Receiver<Vec<Change>> {
    let (tx, rx) = channel();
    let alive = Arc::downgrade(alive);
    let shown = dir.display().to_string();
    let spawned = std::thread::Builder::new().name("texpack watch".into()).spawn(move || {
        let mut waiting: HashMap<String, Option<Stamp>> = HashMap::new();
        let mut every = WATCH_EVERY;
        loop {
            std::thread::sleep(every);
            if alive.upgrade().is_none() {
                return;
            }
            let t = Instant::now();
            let now = stamps(&dir);
            let mut seen: HashMap<String, Option<Stamp>> = HashMap::new();
            for (f, s) in &now {
                if known.get(f) != Some(s) {
                    seen.insert(f.clone(), Some(*s));
                }
            }
            for f in known.keys().filter(|f| !now.contains_key(*f)) {
                seen.insert(f.clone(), None);
            }
            let mut out = Vec::new();
            for (f, s) in &seen {
                if waiting.get(f) == Some(s) {
                    out.push(Change {
                        file: f.clone(),
                        at: s.map_or_else(SystemTime::now, |s| s.0),
                        exists: s.is_some(),
                    });
                    match s {
                        Some(s) => known.insert(f.clone(), *s),
                        None => known.remove(f),
                    };
                }
            }
            waiting = seen.into_iter().filter(|(f, _)| !out.iter().any(|c| &c.file == f)).collect();
            if !out.is_empty() && tx.send(out).is_err() {
                return;
            }
            every = WATCH_EVERY.max(t.elapsed() * 8);
        }
    });
    if let Err(e) = spawned {
        eprintln!("texture pack: no watch on {shown}: {e}");
    }
    rx
}

/// Work for the decoding threads.
enum Job {
    /// The image with this file stem.
    Image(String),
    /// A material: its id and the stems of its normal, specular and gloss
    /// maps.
    Material(u64, [Option<String>; 3]),
}

enum Done {
    Image(String, Option<Arc<Rgba>>),
    Material(u64, Option<Arc<Material>>),
}

/// The texture and tile images in `dir` by file stem, and the material maps
/// if `maps` (other files are left alone), and the images by key for
/// preloading (textures, tiles).
type Variants = HashMap<u64, HashSet<u32>>;
type Listing = (HashSet<String>, HashSet<String>, [Vec<(u64, String)>; 2], Variants);
fn list(dir: &Path, maps: bool) -> std::io::Result<Listing> {
    let (mut names, mut map_names) = (HashSet::new(), HashSet::new());
    let mut variants = Variants::new();
    let mut by_key: [Vec<(u64, String)>; 2] = Default::default();
    for e in std::fs::read_dir(dir)? {
        let name = e?.file_name();
        let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".png")) else { continue };
        match parse_name(stem) {
            Ok(FileKind::Image { key, .. }) => {
                names.insert(stem.to_string());
                by_key[0].push((key, stem.to_string()));
            }
            Ok(FileKind::Art { .. }) => {
                // Found by surface, asked for on demand: not preloaded.
                names.insert(stem.to_string());
            }
            Ok(FileKind::Variant { key, pal, .. }) => {
                // Found by key and palette, asked for on demand: not
                // preloaded (which palette is drawn is known only when it is).
                names.insert(stem.to_string());
                variants.entry(key).or_default().insert(pal);
            }
            Ok(FileKind::Tile { key }) => {
                names.insert(stem.to_string());
                by_key[1].push((key, stem.to_string()));
            }
            Ok(FileKind::Map { .. }) if maps => {
                map_names.insert(stem.to_string());
            }
            _ => {}
        }
    }
    for l in &mut by_key {
        l.sort_by(|a, b| a.1.cmp(&b.1));
    }
    Ok((names, map_names, by_key, variants))
}

/// A material from its files in `dir` (None: none of them decoded).
fn assemble(dir: &Path, manifest: &Manifest, stems: &[Option<String>; 3]) -> Option<Arc<Material>> {
    let files = stems.each_ref().map(|s| s.as_ref().and_then(|s| decode(&dir.join(format!("{s}.png")))));
    files.iter().any(Option::is_some).then(|| {
        Arc::new(Material::assemble(
            files.each_ref().map(Option::as_deref),
            manifest,
            stems.each_ref().map(Option::as_deref),
        ))
    })
}

fn decode(path: &Path) -> Option<Arc<Rgba>> {
    let bytes = std::fs::read(path).ok()?;
    match png::decode(&bytes) {
        Ok(img) => Some(Arc::new(Rgba { width: img.width, height: img.height, data: img.rgba })),
        Err(e) => {
            eprintln!("texture pack: {}: {e}; using the original", path.display());
            None
        }
    }
}

impl DirPack {
    /// The pack in `dir`: an error if it can't be read, or if its manifest
    /// says this build can't show it (the original textures are drawn).
    pub fn open(dir: &Path) -> Result<DirPack, String> {
        let manifest = Manifest::for_dir(dir)
            .map_err(|e| format!("texture pack {}: {e}; drawing the original textures", dir.display()))?;
        if let Some(n) = manifest.notice() {
            println!("texture pack  {}: {n}", dir.display());
        }
        let (names, maps, preload_list, variants) =
            list(dir, manifest.reads_maps()).map_err(|e| format!("{}: {e}", dir.display()))?;
        let regions = if manifest.reads_regions() {
            match crate::regions::RegionSet::load_dir(dir, &mut |p| decode(p)) {
                Ok(set) => {
                    for (name, why) in &set.skipped {
                        println!("texture pack  {}: region `{name}` not drawn: {why}", dir.display());
                    }
                    (!set.is_empty()).then(|| Arc::new(set))
                }
                Err(e) => {
                    println!("texture pack  {}: {e}; drawing without regions", dir.display());
                    None
                }
            }
        } else {
            None
        };
        let manifest = Arc::new(manifest);
        let (to_workers, jobs) = channel::<Job>();
        let (done, from_workers) = channel();
        let jobs = Arc::new(Mutex::new(jobs));
        for i in 0..WORKERS {
            let (jobs, done, dir): (_, Sender<_>, PathBuf) = (jobs.clone(), done.clone(), dir.to_path_buf());
            let manifest = manifest.clone();
            let spawned = std::thread::Builder::new().name(format!("texpack {i}")).spawn(move || loop {
                // Ends when the pack is dropped (the sender goes).
                let Ok(job) = jobs.lock().map_err(|_| ()).and_then(|j| j.recv().map_err(|_| ())) else { return };
                let out = match job {
                    Job::Image(name) => {
                        let img = decode(&dir.join(format!("{name}.png")));
                        Done::Image(name, img)
                    }
                    Job::Material(id, stems) => Done::Material(id, assemble(&dir, &manifest, &stems)),
                };
                if done.send(out).is_err() {
                    return;
                }
            });
            spawned.map_err(|e| format!("texture pack: {e}"))?;
        }
        let alive = Arc::new(());
        let from_watch = watch(dir.to_path_buf(), stamps(dir), &alive);
        Ok(DirPack {
            from_watch,
            _alive: alive,
            names,
            variants,
            maps,
            manifest,
            regions,
            asked: HashSet::new(),
            preload_list,
            preload_cursor: [0; 2],
            preload_want: HashSet::new(),
            handed: HashSet::new(),
            tile_cache: HashMap::new(),
            ready: HashMap::new(),
            asked_materials: HashSet::new(),
            ready_materials: HashMap::new(),
            to_workers,
            from_workers,
        })
    }

    /// Take what the threads have finished.
    fn collect(&mut self) {
        for d in self.from_workers.try_iter() {
            match d {
                Done::Image(name, img) => {
                    self.ready.insert(name, img);
                }
                Done::Material(id, m) => {
                    self.ready_materials.insert(id, m);
                }
            }
        }
    }

    /// Images in the pack.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Material map files in it (0 unless its manifest is format 2 or
    /// later).
    pub fn maps(&self) -> usize {
        self.maps.len()
    }

    /// The image named `name` (a file stem): see `TexturePack::find`.
    fn get(&mut self, name: String) -> Found {
        if !self.names.contains(&name) {
            return Found::Missing;
        }
        if let Some(img) = self.tile_cache.get(&name) {
            return Found::Image(img.clone());
        }
        self.collect();
        if let Some(img) = self.ready.remove(&name) {
            self.asked.remove(&name);
            self.preload_want.remove(&name);
            // Tiles and artwork images serve many asks (every colouring of an
            // artwork): keep them.
            if let (true, Some(i)) = (name.starts_with("tile_") || name.starts_with("art_"), &img) {
                self.tile_cache.insert(name.clone(), i.clone());
            }
            self.handed.insert(name);
            return img.map_or(Found::Missing, Found::Image);
        }
        if !self.asked.contains(&name) {
            if self.to_workers.send(Job::Image(name.clone())).is_err() {
                return Found::Missing;
            }
            self.asked.insert(name);
        }
        Found::Pending
    }
}

impl TexturePack for DirPack {
    /// Changes the watch has settled: the lists are brought up to date and
    /// what was kept of the changed files is forgotten, so `find` reads
    /// them again.
    fn take_changes(&mut self) -> PackChanges {
        let mut out = PackChanges::default();
        let reads_maps = self.manifest.reads_maps();
        for c in self.from_watch.try_iter().flatten().collect::<Vec<_>>() {
            let Some(stem) = c.file.strip_suffix(".png") else {
                out.reopen = true;
                continue;
            };
            // Texture and tile images are also on the preload lists.
            let listed = match parse_name(stem) {
                Ok(FileKind::Image { key, .. }) => Some((0, key)),
                Ok(FileKind::Tile { key }) => Some((1, key)),
                Ok(FileKind::Art { .. }) => None,
                Ok(FileKind::Variant { key, pal, .. }) => {
                    if c.exists {
                        self.variants.entry(key).or_default().insert(pal);
                    } else if let Some(v) = self.variants.get_mut(&key) {
                        v.remove(&pal);
                        if v.is_empty() {
                            self.variants.remove(&key);
                        }
                    }
                    None
                }
                Ok(FileKind::Map { .. }) if reads_maps => {
                    if c.exists {
                        self.maps.insert(stem.to_string());
                    } else {
                        self.maps.remove(stem);
                    }
                    self.asked_materials.clear();
                    self.ready_materials.clear();
                    out.stems.push((stem.to_string(), c.at));
                    continue;
                }
                Ok(FileKind::Region) => {
                    out.reopen = true;
                    continue;
                }
                _ => continue,
            };
            if let Some((k, key)) = listed {
                let list = &mut self.preload_list[k];
                match (c.exists, list.iter().position(|e| e.1 == stem)) {
                    (true, None) => list.push((key, stem.to_string())),
                    (false, Some(i)) => {
                        list.remove(i);
                        if i < self.preload_cursor[k] {
                            self.preload_cursor[k] -= 1;
                        }
                    }
                    _ => {}
                }
            }
            if c.exists {
                self.names.insert(stem.to_string());
            } else {
                self.names.remove(stem);
            }
            self.tile_cache.remove(stem);
            self.ready.remove(stem);
            self.asked.remove(stem);
            self.preload_want.remove(stem);
            self.handed.remove(stem);
            out.stems.push((stem.to_string(), c.at));
        }
        out
    }

    /// This colouring's variant for the palette it is drawn with
    /// (`tex_<key>~<pal>_..`), else its own image, else the artwork's
    /// (`art_`), which
    /// every colouring without an image of its own is drawn with.
    fn find(&mut self, info: &TexInfo) -> Found {
        if info.pal != 0 {
            let name = info.variant_name();
            if self.names.contains(&name) {
                return self.get(name);
            }
        }
        let own = info.name();
        if self.names.contains(&own) {
            return self.get(own);
        }
        self.get(info.art_name())
    }

    /// Images in the order of their file names, decoded a few at a time in
    /// the background (the demand for others goes first: the workers take
    /// jobs in order, and at most `PRELOAD_AHEAD` of these are queued).
    fn preload(&mut self, kind: PreloadKind, max: usize) -> Vec<(u64, std::sync::Arc<Rgba>)> {
        let k = (kind == PreloadKind::Tile) as usize;
        self.collect();
        let mut out = Vec::new();
        // Finished ones first, in list order.
        let mut i = 0;
        while out.len() < max && i < self.preload_cursor[k] {
            let (key, name) = &self.preload_list[k][i];
            i += 1;
            if !self.preload_want.contains(name) || !self.ready.contains_key(name) {
                continue;
            }
            let (key, name) = (*key, name.clone());
            self.preload_want.remove(&name);
            self.asked.remove(&name);
            if let Some(Some(img)) = self.ready.remove(&name) {
                if k == 1 {
                    self.tile_cache.insert(name.clone(), img.clone());
                }
                out.push((key, img));
            }
        }
        // Queue more.
        let mut ahead =
            self.preload_want.iter().filter(|n| n.starts_with(if k == 1 { "tile_" } else { "tex_" })).count();
        while ahead < PRELOAD_AHEAD && self.preload_cursor[k] < self.preload_list[k].len() {
            let name = self.preload_list[k][self.preload_cursor[k]].1.clone();
            self.preload_cursor[k] += 1;
            if self.asked.contains(&name) || self.handed.contains(&name) {
                continue;
            }
            if self.to_workers.send(Job::Image(name.clone())).is_err() {
                break;
            }
            self.asked.insert(name.clone());
            self.preload_want.insert(name);
            ahead += 1;
        }
        out
    }

    fn preload_restart(&mut self, kind: PreloadKind) {
        let k = (kind == PreloadKind::Tile) as usize;
        self.preload_cursor[k] = 0;
        self.handed.retain(|n| n.starts_with("tile_") != (k == 0));
    }

    fn has_tiles(&self) -> bool {
        self.regions.is_some() || self.names.iter().any(|n| n.starts_with("tile_"))
    }

    fn regions(&mut self) -> Option<Arc<crate::regions::RegionSet>> {
        self.regions.clone()
    }

    fn find_tile(&mut self, info: &TileInfo) -> Found {
        self.get(info.name())
    }

    fn has_materials(&self) -> bool {
        !self.maps.is_empty()
    }

    fn material(&mut self, info: &TexInfo) -> FoundMaterial {
        if self.maps.is_empty() {
            return FoundMaterial::Missing;
        }
        let stems = resolve_maps(info, |s| self.maps.contains(s));
        if stems.iter().all(Option::is_none) {
            return FoundMaterial::Missing;
        }
        let id = material_id(stems.each_ref().map(Option::as_deref));
        self.collect();
        if let Some(m) = self.ready_materials.remove(&id) {
            self.asked_materials.remove(&id);
            return m.map_or(FoundMaterial::Missing, FoundMaterial::Maps);
        }
        if !self.asked_materials.contains(&id) {
            if self.to_workers.send(Job::Material(id, stems)).is_err() {
                return FoundMaterial::Missing;
            }
            self.asked_materials.insert(id);
        }
        FoundMaterial::Pending
    }

    fn variant(&mut self, info: &TexInfo, palette: &mut dyn FnMut() -> u32) -> u32 {
        if self.variants.is_empty() {
            return 0;
        }
        match self.variants.get(&info.key) {
            Some(pals) => Some(palette()).filter(|p| pals.contains(p)).unwrap_or(0),
            None => 0,
        }
    }

    /// A variant is `selflit` as its key's `material` line says.
    fn image_selflit(&mut self, info: &TexInfo) -> Option<bool> {
        let own = self.names.contains(&info.name()) || (info.pal != 0 && self.names.contains(&info.variant_name()));
        (own || self.names.contains(&info.art_name())).then(|| self.manifest.image_selflit(info, own))
    }

    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_in_the_background() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/dirpack-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let info = TexInfo {
            key: 0x1234,
            surface: 0,
            width: 2,
            height: 1,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
            pal: 0,
        };
        let data = [255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 9, 9, 9, 255];
        std::fs::write(dir.join(format!("{}.png", info.name())), png::encode_rgba(4, 1, &data)).unwrap();
        // Material maps aren't images; a manifest of this format is read.
        std::fs::write(dir.join(format!("{}.n.png", info.name())), png::encode_rgba(4, 1, &data)).unwrap();
        std::fs::write(dir.join("pack.txt"), "format 2\nuses materials\n").unwrap();
        let mut pack = DirPack::open(&dir).unwrap();
        assert_eq!(pack.len(), 1);
        assert!(matches!(pack.find(&TexInfo { key: 1, ..info.clone() }), Found::Missing));
        let start = std::time::Instant::now();
        let img = loop {
            match pack.find(&info) {
                Found::Image(img) => break img,
                Found::Pending => assert!(start.elapsed().as_secs() < 10, "never decoded"),
                Found::Missing => panic!("missing"),
            }
            std::thread::yield_now();
        };
        assert_eq!((img.width, img.height), (4, 1));
        assert_eq!(&img.data[..], &data[..]);
        // Handed over once: asked again, it's decoded again.
        assert!(matches!(pack.find(&info), Found::Pending));
        // Its normal map, assembled on the threads: the size of the map,
        // neutral specular and gloss.
        assert!(pack.has_materials());
        let start = std::time::Instant::now();
        let m = loop {
            match pack.material(&info) {
                FoundMaterial::Maps(m) => break m,
                FoundMaterial::Pending => assert!(start.elapsed().as_secs() < 10, "never assembled"),
                FoundMaterial::Missing => panic!("missing"),
            }
            std::thread::yield_now();
        };
        assert_eq!((m.width, m.height, m.has), (4, 1, 1));
        // Red 255, green 0: (1, -1), y flipped (normal_y up) and scaled to
        // length 1.
        assert_eq!(m.data[..4], [218, 218, 128, 128]);
        assert!(matches!(pack.material(&TexInfo { key: 1, ..info.clone() }), FoundMaterial::Missing));
        // Without a manifest the maps aren't read (a format 1 pack, as before).
        std::fs::remove_file(dir.join("pack.txt")).unwrap();
        let mut plain = DirPack::open(&dir).unwrap();
        assert!(!plain.has_materials() && matches!(plain.material(&info), FoundMaterial::Missing));
        // A pack that needs material maps is shown (with the maps when the
        // player's setting is on).
        std::fs::write(dir.join("pack.txt"), "format 2\nneeds materials\n").unwrap();
        let needs = DirPack::open(&dir).unwrap();
        assert!(needs.has_materials() && needs.maps() == 1 && needs.len() == 1);
        // A pack of a newer format, or one needing what this build can't
        // show, is refused (the player then draws the originals).
        for m in ["format 3\n", "format 2\nneeds models\n"] {
            std::fs::write(dir.join("pack.txt"), m).unwrap();
            let e = DirPack::open(&dir).err().unwrap();
            assert!(e.contains("can't show") || e.contains("newer reModel"), "{e}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The watch reports an image added, rewritten and removed once its file
    /// has settled, and the pack then answers `find` with the new file.
    #[test]
    fn the_watch_reports_changed_files_and_the_pack_reads_them_again() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/dirpack-watch-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let info = TexInfo {
            key: 0x77,
            surface: 5,
            width: 1,
            height: 1,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
            pal: 0,
        };
        let path = dir.join(format!("{}.png", info.name()));
        let px = |c: u8| png::encode_rgba(1, 1, &[c, c, c, 255]);
        let mut pack = DirPack::open(&dir).unwrap();
        assert!(matches!(pack.find(&info), Found::Missing));
        let wait = |pack: &mut DirPack| {
            let start = Instant::now();
            loop {
                let c = pack.take_changes();
                if !c.is_empty() {
                    return (c, start.elapsed());
                }
                assert!(start.elapsed().as_secs() < 10, "no change reported");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let image = |pack: &mut DirPack| {
            let start = Instant::now();
            loop {
                match pack.find(&info) {
                    Found::Image(i) => return Some(i.data[0]),
                    Found::Missing => return None,
                    Found::Pending => assert!(start.elapsed().as_secs() < 10, "never decoded"),
                }
                std::thread::yield_now();
            }
        };
        // Added.
        std::fs::write(&path, px(10)).unwrap();
        let (c, took) = wait(&mut pack);
        assert_eq!(c.stems.iter().map(|s| s.0.as_str()).collect::<Vec<_>>(), [info.name()]);
        assert!(!c.reopen && took.as_millis() < 1000, "{took:?}");
        assert_eq!(pack.len(), 1);
        assert_eq!(image(&mut pack), Some(10));
        // Rewritten.
        std::fs::write(&path, px(200)).unwrap();
        assert_eq!(wait(&mut pack).0.stems.len(), 1);
        assert_eq!(image(&mut pack), Some(200));
        // The manifest changed: the pack is to be opened again.
        std::fs::write(dir.join("pack.txt"), "format 2\n").unwrap();
        assert!(wait(&mut pack).0.reopen);
        // Removed.
        std::fs::remove_file(&path).unwrap();
        assert_eq!(wait(&mut pack).0.stems.len(), 1);
        assert_eq!(pack.len(), 0);
        assert_eq!(image(&mut pack), None);
        // The watch ends with the pack.
        drop(pack);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_artwork_image_is_drawn_for_every_colouring_without_its_own() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/dirpack-art-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let info = |key| TexInfo {
            key,
            surface: 0xaa,
            width: 2,
            height: 1,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
            pal: 0,
        };
        let (art, own) = ([1u8, 2, 3, 255, 4, 5, 6, 255], [9u8, 9, 9, 255, 8, 8, 8, 255]);
        std::fs::write(dir.join(format!("{}.png", info(1).art_name())), png::encode_rgba(2, 1, &art)).unwrap();
        std::fs::write(dir.join(format!("{}.png", info(2).name())), png::encode_rgba(2, 1, &own)).unwrap();
        std::fs::write(dir.join("pack.txt"), "format 2\nneeds artworks\n").unwrap();
        let mut pack = DirPack::open(&dir).unwrap();
        let mut get = |k| {
            let start = std::time::Instant::now();
            loop {
                match pack.find(&info(k)) {
                    Found::Image(img) => break img.data.clone(),
                    Found::Pending => assert!(start.elapsed().as_secs() < 10, "never decoded"),
                    Found::Missing => panic!("missing"),
                }
                std::thread::yield_now();
            }
        };
        // Two colourings with no image of their own both get the artwork's;
        // a third with its own keeps it.
        assert_eq!(get(1), art);
        assert_eq!(get(3), art);
        assert_eq!(get(2), own);
        // Another artwork has none.
        let mut other = info(4);
        other.surface = 0xbb;
        assert!(matches!(pack.find(&other), Found::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Per-palette variants on disk: listed from the names, asked for the
    /// palette only for a key that has one, drawn in preference to the plain
    /// image, kept out of the preload list, and followed by the watch.
    #[test]
    fn variants_win_over_the_plain_image_and_follow_the_watch() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/dirpack-variant-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut info = TexInfo {
            key: 0x55,
            surface: 5,
            width: 1,
            height: 1,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
            pal: 0,
        };
        let px = |c: u8| png::encode_rgba(1, 1, &[c, c, c, 255]);
        let with_pal = |pal: u32| TexInfo { pal, ..info.clone() };
        std::fs::write(dir.join(format!("{}.png", info.name())), px(10)).unwrap();
        std::fs::write(dir.join(format!("{}.png", with_pal(7).variant_name())), px(70)).unwrap();
        let mut pack = DirPack::open(&dir).unwrap();
        let asked = std::cell::Cell::new(0);
        let ask = |pack: &mut DirPack, key: u64, pal: u32| {
            let i = TexInfo { key, ..info.clone() };
            TexturePack::variant(pack, &i, &mut || {
                asked.set(asked.get() + 1);
                pal
            })
        };
        assert_eq!(ask(&mut pack, 0x55, 7), 7);
        assert_eq!(ask(&mut pack, 0x55, 8), 0, "a palette with no variant");
        assert_eq!(asked.get(), 2);
        assert_eq!(ask(&mut pack, 0x66, 7), 0, "a key with none");
        assert_eq!(asked.get(), 2, "the palette is not worked out for it");
        let image = |pack: &mut DirPack, i: &TexInfo| {
            let start = Instant::now();
            loop {
                match pack.find(i) {
                    Found::Image(img) => return Some(img.data[0]),
                    Found::Missing => return None,
                    Found::Pending => assert!(start.elapsed().as_secs() < 10, "never decoded"),
                }
                std::thread::yield_now();
            }
        };
        assert_eq!(image(&mut pack, &with_pal(7)), Some(70));
        assert_eq!(image(&mut pack, &with_pal(0)), Some(10), "no palette asked: the plain image");
        assert_eq!(image(&mut pack, &with_pal(9)), Some(10), "a palette with no file: the plain image");
        // Variants are not preloaded: only the plain image is on the list.
        assert_eq!(pack.preload_list[0].len(), 1);
        // The watch: a variant added, then removed.
        let wait = |pack: &mut DirPack| {
            let start = Instant::now();
            loop {
                let c = pack.take_changes();
                if !c.is_empty() {
                    return c;
                }
                assert!(start.elapsed().as_secs() < 10, "no change reported");
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let path9 = dir.join(format!("{}.png", with_pal(9).variant_name()));
        std::fs::write(&path9, px(90)).unwrap();
        assert_eq!(wait(&mut pack).stems.len(), 1);
        assert_eq!(ask(&mut pack, 0x55, 9), 9);
        assert_eq!(image(&mut pack, &with_pal(9)), Some(90));
        std::fs::remove_file(&path9).unwrap();
        assert_eq!(wait(&mut pack).stems.len(), 1);
        assert_eq!(ask(&mut pack, 0x55, 9), 0);
        info.pal = 0;
        let _ = info;
        drop(pack);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
