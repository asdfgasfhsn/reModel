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

use crate::enhance::{material_id, Found, FoundMaterial, Material, PreloadKind, Rgba, TexInfo, TexturePack, TileInfo};
use crate::pack::{parse_name, resolve_maps, FileKind, Manifest};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Threads decoding images.
const WORKERS: usize = 2;
/// Preloaded images asked for and not yet handed over, at most, per kind.
const PRELOAD_AHEAD: usize = 6;

pub struct DirPack {
    /// File stems of the PNGs in the directory.
    names: HashSet<String>,
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
type Listing = (HashSet<String>, HashSet<String>, [Vec<(u64, String)>; 2]);
fn list(dir: &Path, maps: bool) -> std::io::Result<Listing> {
    let (mut names, mut map_names) = (HashSet::new(), HashSet::new());
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
    Ok((names, map_names, by_key))
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
        let (names, maps, preload_list) =
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
        Ok(DirPack {
            names,
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
    /// The image of this colouring, else the artwork's (`art_`), which
    /// every colouring without an image of its own is drawn with.
    fn find(&mut self, info: &TexInfo) -> Found {
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
}
