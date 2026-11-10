//! A texture pack directory for the GPU renderer (`gpu::Renderer::set_pack`):
//! PNGs named `tex_<key>_<w>x<h>.png` (docs/texture-packs.md), decoded on
//! background threads so a new texture never holds up a frame. Until its
//! image is ready a texture is drawn from the original (`Found::Pending`).
//!
//! The renderer keeps the images it uploads, so an image is handed over once
//! and not kept here; one the renderer drops (over its memory budget, or the
//! filter setting changed) is decoded again when next asked for.

use model2::enhance::{Found, Rgba, TexInfo, TexturePack};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};

/// Threads decoding images.
const WORKERS: usize = 2;

pub struct DirPack {
    /// File stems of the PNGs in the directory.
    names: HashSet<String>,
    /// Images asked for and not yet handed over.
    asked: HashSet<String>,
    /// Decoded images (None: unreadable), not yet handed over.
    ready: HashMap<String, Option<Arc<Rgba>>>,
    to_workers: Sender<String>,
    from_workers: Receiver<(String, Option<Arc<Rgba>>)>,
}

/// The texture PNGs in `dir` by file stem.
fn list(dir: &Path) -> std::io::Result<HashSet<String>> {
    let mut names = HashSet::new();
    for e in std::fs::read_dir(dir)? {
        let name = e?.file_name();
        let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".png")) else { continue };
        if stem.starts_with("tex_") {
            names.insert(stem.to_string());
        }
    }
    Ok(names)
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
    /// The pack in `dir`: an error if it can't be read.
    pub fn open(dir: &Path) -> Result<DirPack, String> {
        let names = list(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let (to_workers, jobs) = channel::<String>();
        let (done, from_workers) = channel();
        let jobs = Arc::new(Mutex::new(jobs));
        for i in 0..WORKERS {
            let (jobs, done, dir): (_, Sender<_>, PathBuf) = (jobs.clone(), done.clone(), dir.to_path_buf());
            let spawned = std::thread::Builder::new().name(format!("texpack {i}")).spawn(move || loop {
                // Ends when the pack is dropped (the sender goes).
                let Ok(name) = jobs.lock().map_err(|_| ()).and_then(|j| j.recv().map_err(|_| ())) else { return };
                let img = decode(&dir.join(format!("{name}.png")));
                if done.send((name, img)).is_err() {
                    return;
                }
            });
            spawned.map_err(|e| format!("texture pack: {e}"))?;
        }
        Ok(DirPack { names, asked: HashSet::new(), ready: HashMap::new(), to_workers, from_workers })
    }

    /// Images in the pack.
    pub fn len(&self) -> usize {
        self.names.len()
    }
}

impl TexturePack for DirPack {
    fn find(&mut self, info: &TexInfo) -> Found {
        let name = info.name();
        if !self.names.contains(&name) {
            return Found::Missing;
        }
        self.ready.extend(self.from_workers.try_iter());
        if let Some(img) = self.ready.remove(&name) {
            self.asked.remove(&name);
            return img.map_or(Found::Missing, Found::Image);
        }
        if !self.asked.contains(&name) {
            if self.to_workers.send(name.clone()).is_err() {
                return Found::Missing;
            }
            self.asked.insert(name);
        }
        Found::Pending
    }
}

/// Where `game`'s pack is kept: `<data dir>/<set>/texpack/`, or for a clone
/// without one, its parent's (keys are the textures' content, so the same
/// images serve every version).
pub fn game_dir(game: &romset::Game) -> PathBuf {
    let dir = |set: &str| crate::settings::data_dir().join(set).join("texpack");
    std::iter::once(game.id).chain(game.parent).map(dir).find(|d| d.is_dir()).unwrap_or_else(|| dir(game.id))
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
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
