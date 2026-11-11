//! A texture pack directory for the GPU renderer (`gpu::Renderer::set_pack`):
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

use model2::enhance::{material_id, Found, FoundMaterial, Material, Rgba, TexInfo, TexturePack, TileInfo};
use model2::pack::{parse_name, resolve_maps, FileKind, Manifest};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Threads decoding images.
const WORKERS: usize = 2;

pub struct DirPack {
    /// File stems of the PNGs in the directory.
    names: HashSet<String>,
    /// File stems of the material maps (none unless the manifest
    /// `reads_maps`).
    maps: HashSet<String>,
    manifest: Arc<Manifest>,
    /// Images asked for and not yet handed over.
    asked: HashSet<String>,
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
/// if `maps` (other files are left alone).
fn list(dir: &Path, maps: bool) -> std::io::Result<(HashSet<String>, HashSet<String>)> {
    let (mut names, mut map_names) = (HashSet::new(), HashSet::new());
    for e in std::fs::read_dir(dir)? {
        let name = e?.file_name();
        let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".png")) else { continue };
        match parse_name(stem) {
            Ok(FileKind::Image { .. } | FileKind::Tile { .. }) => {
                names.insert(stem.to_string());
            }
            Ok(FileKind::Map { .. }) if maps => {
                map_names.insert(stem.to_string());
            }
            _ => {}
        }
    }
    Ok((names, map_names))
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
        let (names, maps) = list(dir, manifest.reads_maps()).map_err(|e| format!("{}: {e}", dir.display()))?;
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
            asked: HashSet::new(),
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
        self.collect();
        if let Some(img) = self.ready.remove(&name) {
            self.asked.remove(&name);
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
    fn find(&mut self, info: &TexInfo) -> Found {
        self.get(info.name())
    }

    fn has_tiles(&self) -> bool {
        self.names.iter().any(|n| n.starts_with("tile_"))
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

/// Where `game`'s pack is kept: `<data dir>/<set>/texpack/`, or for a clone
/// without one, its parent's (keys are the textures' content, so the same
/// images serve every version).
pub fn game_dir(game: &romset::Game) -> PathBuf {
    let dir = |set: &str| crate::settings::data_dir().join(set).join("texpack");
    romset::pack_sets(game).map(dir).find(|d| d.is_dir()).unwrap_or_else(|| dir(game.id))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_in_the_background() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../out/scratch/play-pack-test");
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
}
