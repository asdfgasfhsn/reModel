//! Texture dumps and texture packs for `run` snapshots (`--dump-textures`,
//! `--texture-pack`); docs/texture-packs.md describes the files.

use model2::enhance::{Enhance, Rgba, TexInfo, TileInfo};
use model2::raster::{texture_image, Sources};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

/// Set by `main` when either option is given; snapshots then render
/// through it.
pub static PACKS: Mutex<Option<Packs>> = Mutex::new(None);

/// One dumped texture or tile in the index.
struct Entry {
    about: String,
    first_frame: u64,
    /// Polygons drawn with it (textures) or frames it was on screen
    /// (tiles), over all snapshots.
    uses: u64,
}

pub struct Packs {
    dump: Option<PathBuf>,
    pack: Option<PathBuf>,
    /// Frame being snapshotted.
    pub frame: u64,
    /// Pack images by file stem, loaded on first use (None: not in the
    /// pack, or unreadable).
    cache: HashMap<String, Option<Arc<Rgba>>>,
    index: BTreeMap<String, Entry>,
    index_dirty: bool,
    /// Polygons and tile patterns replaced / looked up, over the run.
    replaced: [u64; 2],
    looked_up: [u64; 2],
}

impl Packs {
    pub fn new(dump: Option<PathBuf>, pack: Option<PathBuf>) -> Result<Self, String> {
        if let Some(d) = &dump {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        if let Some(p) = &pack {
            if !p.is_dir() {
                return Err(format!("{}: texture pack directory not found", p.display()));
            }
        }
        Ok(Packs {
            dump,
            pack,
            frame: 0,
            cache: HashMap::new(),
            index: BTreeMap::new(),
            index_dirty: false,
            replaced: [0; 2],
            looked_up: [0; 2],
        })
    }

    fn load(&mut self, name: &str) -> Option<Arc<Rgba>> {
        let dir = self.pack.as_ref()?;
        self.cache
            .entry(name.to_string())
            .or_insert_with(|| {
                let path = dir.join(format!("{name}.png"));
                let bytes = std::fs::read(&path).ok()?;
                match png::decode(&bytes) {
                    Ok(img) => Some(Arc::new(Rgba { width: img.width, height: img.height, data: img.rgba })),
                    Err(e) => {
                        eprintln!("texture pack: {}: {e}; using the original", path.display());
                        None
                    }
                }
            })
            .clone()
    }

    /// Count a use of `name`, writing its dump image the first time.
    fn record(&mut self, name: String, about: impl FnOnce() -> String, image: impl FnOnce() -> Rgba) {
        let Some(dir) = &self.dump else { return };
        let frame = self.frame;
        let e = self.index.entry(name).or_insert_with_key(|name| {
            let img = image();
            let _ = std::fs::write(dir.join(format!("{name}.png")), png::encode_rgba(img.width, img.height, &img.data));
            Entry { about: about(), first_frame: frame, uses: 0 }
        });
        e.uses += 1;
        self.index_dirty = true;
    }

    /// After a snapshot: rewrite the dump index if it changed.
    pub fn finish_snapshot(&mut self) {
        let (Some(dir), true) = (&self.dump, self.index_dirty) else { return };
        let mut text = String::from(
            "# Remodel texture dump: <name>.png, first frame seen, uses (textures: polygons drawn; tiles: frames\n\
             # on screen) over the snapshots, then what produced it. Put edited or upscaled copies, same names,\n\
             # in a directory and pass it to `run --texture-pack DIR` (docs/texture-packs.md).\n",
        );
        for (name, e) in &self.index {
            let _ = writeln!(text, "{name}\tframe={}\tuses={}\t{}", e.first_frame, e.uses, e.about);
        }
        let _ = std::fs::write(dir.join("index.txt"), text);
        self.index_dirty = false;
    }

    /// One line for the end-of-run report.
    pub fn summary(&self) -> String {
        let (tex, tiles) =
            self.index.keys().fold((0, 0), |(a, b), k| if k.starts_with("tile_") { (a, b + 1) } else { (a + 1, b) });
        let mut s = String::new();
        if let Some(d) = &self.dump {
            let _ = write!(s, "dumped {tex} textures and {tiles} tile patterns to {}", d.display());
        }
        if let Some(p) = &self.pack {
            if !s.is_empty() {
                s += "; ";
            }
            let _ = write!(
                s,
                "pack {}: replaced {} of {} textured polygons and {} of {} tile patterns",
                p.display(),
                self.replaced[0],
                self.looked_up[0],
                self.replaced[1],
                self.looked_up[1]
            );
        }
        s
    }
}

impl Enhance for Packs {
    fn texture(&mut self, info: &TexInfo) -> Option<Arc<Rgba>> {
        self.pack.as_ref()?;
        let img = self.load(&info.name());
        self.looked_up[0] += 1;
        self.replaced[0] += img.is_some() as u64;
        img
    }

    fn texture_drawn(&mut self, info: &TexInfo, src: &Sources, pixels: usize) {
        if pixels == 0 {
            return;
        }
        let about = || {
            format!(
                "header={:04x},{:04x},{:04x},{:04x}\tcolour={:#06x}\tbank={}\tat={},{}\tluma_table={}\ttranslucent={}",
                info.header[0],
                info.header[1],
                info.header[2],
                info.header[3],
                info.colour,
                info.bank,
                info.x0,
                info.y0,
                info.luma_table,
                info.translucent as u8
            )
        };
        self.record(info.name(), about, || texture_image(src, info));
    }

    fn tile(&mut self, info: &TileInfo) -> Option<Arc<Rgba>> {
        let about = || format!("tile={:#06x}\tpalette_bank={:#04x}", info.tile, info.bank);
        self.record(info.name(), about, || info.image());
        self.pack.as_ref()?;
        let img = self.load(&info.name());
        self.looked_up[1] += 1;
        self.replaced[1] += img.is_some() as u64;
        img
    }
}

/// Parse `--dump-textures` / `--texture-pack` into `PACKS`.
pub fn configure(dump: Option<String>, pack: Option<String>) -> Result<(), String> {
    if dump.is_none() && pack.is_none() {
        return Ok(());
    }
    let p = Packs::new(dump.map(PathBuf::from), pack.map(PathBuf::from))?;
    *PACKS.lock().unwrap() = Some(p);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dumps_once_and_loads_replacements() {
        let dir = std::env::temp_dir().join(format!("remodel-texpack-test-{}", std::process::id()));
        let (dump, pack) = (dir.join("dump"), dir.join("pack"));
        std::fs::create_dir_all(&pack).unwrap();
        let mut p = Packs::new(Some(dump.clone()), Some(pack.clone())).unwrap();
        let info = TileInfo { key: 0x1234, tile: 5, bank: 2, pens: [1; 64], colours: [0x7fff; 16] };
        assert!(p.tile(&info).is_none());
        p.tile(&info);
        p.finish_snapshot();
        let dumped = std::fs::read(dump.join("tile_0000000000001234.png")).unwrap();
        let index = std::fs::read_to_string(dump.join("index.txt")).unwrap();
        assert!(index.contains("tile_0000000000001234\tframe=0\tuses=2\ttile=0x0005"));
        // A 4x replacement is picked up (by a fresh instance: misses are cached).
        let big = Rgba { width: 32, height: 32, data: vec![200; 32 * 32 * 4] };
        std::fs::write(pack.join("tile_0000000000001234.png"), png::encode_rgba(32, 32, &big.data)).unwrap();
        let mut p = Packs::new(None, Some(pack)).unwrap();
        assert_eq!(p.tile(&info).as_deref(), Some(&big));
        assert_eq!(png::decode(&dumped).unwrap().width, 8);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
