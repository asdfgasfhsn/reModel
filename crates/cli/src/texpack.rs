//! Texture dumps and texture packs for `run` snapshots (`--dump-textures`,
//! `--texture-pack`); docs/texture-packs.md describes the files.

use model2::enhance::{Enhance, Fnv, Found, FoundMaterial, Material, Rgba, TexInfo, TexturePack, TileInfo};
use model2::pack::{parse_name, resolve_maps, FileKind, Manifest};
use model2::raster::{texture_image, texture_look, Sources};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
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
    /// Textures: the ways it was drawn, bit `mirror_u | mirror_v << 1`
    /// set for each (0 for tiles).
    wraps: u8,
    /// Uses by look (`raster::texture_look`, hashed; tiles have one, as
    /// their key holds their colours), and the look the dump image shows:
    /// the most used one so far.
    looks: HashMap<u64, u64>,
    look: u64,
}

/// The `wrap=` field of a texture's index line: each way it was drawn, u
/// then v, `r` repeat or `m` mirror (`rr,mr`: repeated, and mirrored in u).
fn wrap_field(wraps: u8) -> String {
    let ways: Vec<&str> = (0..4).filter(|b| wraps & 1 << b != 0).map(|b| ["rr", "mr", "rm", "mm"][b]).collect();
    format!("\twrap={}", ways.join(","))
}

pub struct Packs {
    dump: Option<PathBuf>,
    pack: Option<PathBuf>,
    /// Frame being snapshotted.
    pub frame: u64,
    /// Pack images by file stem, loaded on first use (None: not in the
    /// pack, or unreadable).
    cache: HashMap<String, Option<Arc<Rgba>>>,
    /// Mip chains of the images, by texture key (filtered snapshots).
    mips: HashMap<u64, Arc<Vec<Rgba>>>,
    index: BTreeMap<String, Entry>,
    index_dirty: bool,
    /// Polygons and tile patterns replaced / looked up, over the run.
    replaced: [u64; 2],
    looked_up: [u64; 2],
    /// The pack has tile images (`TexturePack::has_tiles`).
    has_tiles: bool,
    /// The pack's region overrides (`uses regions`; docs/texture-packs.md,
    /// "Regions"), loaded when the pack is opened.
    regions: Option<Arc<model2::regions::RegionSet>>,
    /// Why the pack given isn't drawn (`accept`).
    refused: Option<String>,
    /// The pack's manifest (`Manifest::NONE` without one).
    manifest: Manifest,
    /// Stems of the material maps in the pack (none unless the manifest
    /// `reads_maps`).
    maps: HashSet<String>,
    /// Materials assembled, by id (None: no file of it decoded).
    materials: HashMap<u64, Option<Arc<Material>>>,
    /// Their mip chains, by id (filtered snapshots).
    material_mips: HashMap<u64, Arc<Vec<Material>>>,
    /// Software drawing with materials (`Enhance::texture_material`) since
    /// the last `take_material_counts`: polygons drawn with a material and
    /// pixels it changed.
    material_counts: [u64; 2],
}

/// Whether the pack in `dir` may be drawn (`pack::Manifest::for_dir`),
/// printing what its manifest ignores or why it is refused: a refused pack
/// draws the original textures (docs/texture-packs.md, "Pack format").
pub fn accept(dir: &Path) -> Result<Manifest, String> {
    match Manifest::for_dir(dir) {
        Ok(m) => {
            if let Some(n) = m.notice() {
                eprintln!("texture pack {}: {n}", dir.display());
            }
            Ok(m)
        }
        Err(e) => {
            eprintln!("texture pack {}: {e}; drawing the original textures", dir.display());
            Err(e)
        }
    }
}

impl Packs {
    pub fn new(dump: Option<PathBuf>, pack: Option<PathBuf>) -> Result<Self, String> {
        if let Some(d) = &dump {
            std::fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
        }
        let (manifest, refused) = match &pack {
            Some(p) => {
                if !p.is_dir() {
                    return Err(format!("{}: texture pack directory not found", p.display()));
                }
                match accept(p) {
                    Ok(m) => (m, None),
                    Err(e) => (Manifest::NONE, Some(e)),
                }
            }
            None => (Manifest::NONE, None),
        };
        let pack = pack.filter(|_| refused.is_none());
        let (mut has_tiles, mut maps) = (false, HashSet::new());
        for e in pack.iter().flat_map(std::fs::read_dir).flatten().flatten() {
            let name = e.file_name();
            let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(".png")) else { continue };
            match parse_name(stem) {
                Ok(FileKind::Tile { .. }) => has_tiles = true,
                Ok(FileKind::Map { .. }) if manifest.reads_maps() => {
                    maps.insert(stem.to_string());
                }
                _ => {}
            }
        }
        let regions = match (&pack, manifest.reads_regions()) {
            (Some(dir), true) => match model2::regions::RegionSet::load_dir(dir, &mut |p| decode(p)) {
                Ok(set) => {
                    for (name, why) in &set.skipped {
                        eprintln!("texture pack {}: region `{name}` not drawn: {why}", dir.display());
                    }
                    (!set.is_empty()).then(|| Arc::new(set))
                }
                Err(e) => {
                    eprintln!("texture pack {}: {e}; drawing without regions", dir.display());
                    None
                }
            },
            _ => None,
        };
        Ok(Packs {
            dump,
            pack,
            refused,
            manifest,
            regions,
            maps,
            materials: HashMap::new(),
            material_mips: HashMap::new(),
            material_counts: [0; 2],
            frame: 0,
            cache: HashMap::new(),
            mips: HashMap::new(),
            index: BTreeMap::new(),
            index_dirty: false,
            replaced: [0; 2],
            looked_up: [0; 2],
            has_tiles,
        })
    }

    fn load(&mut self, name: &str) -> Option<Arc<Rgba>> {
        let dir = self.pack.as_ref()?;
        self.cache.entry(name.to_string()).or_insert_with(|| decode(&dir.join(format!("{name}.png")))).clone()
    }

    /// Material maps of the pack (`TexturePack::has_materials`).
    pub fn material_maps(&self) -> usize {
        self.maps.len()
    }

    /// Count a use of `name` drawn in `look` (see `Entry::looks`), writing
    /// its dump image the first time and again whenever another look has
    /// become the most used: a key's texels can be drawn in several looks
    /// (fades and tints in the colour translation tables), and the one the
    /// game shows most is what a replacement should stand for, not
    /// whichever came first (srallyc's race textures first appear in its
    /// fade from white). `wrap`: for a texture, bit `mirror_u | mirror_v <<
    /// 1` of this use.
    fn record(
        &mut self,
        name: String,
        wrap: u8,
        look: impl FnOnce() -> u64,
        about: impl FnOnce() -> String,
        image: impl FnOnce() -> Rgba,
    ) {
        let Some(dir) = &self.dump else { return };
        let frame = self.frame;
        let look = look();
        let e = self.index.entry(name.clone()).or_insert_with(|| Entry {
            about: about(),
            first_frame: frame,
            uses: 0,
            wraps: 0,
            looks: HashMap::new(),
            look,
        });
        let n = e.looks.entry(look).or_insert(0);
        *n += 1;
        let n = *n;
        if e.uses == 0 || (look != e.look && n > e.looks[&e.look]) {
            let img = image();
            let _ = std::fs::write(dir.join(format!("{name}.png")), png::encode_rgba(img.width, img.height, &img.data));
            e.look = look;
        }
        e.uses += 1;
        e.wraps |= wrap;
        self.index_dirty = true;
    }

    /// After a snapshot: rewrite the dump index if it changed.
    pub fn finish_snapshot(&mut self) {
        let (Some(dir), true) = (&self.dump, self.index_dirty) else { return };
        let mut text = String::from(
            "# reModel texture dump: <name>.png, first frame seen, uses (textures: polygons drawn; tiles: frames\n\
             # on screen) over the snapshots, then what produced it (textures: header and the rest of the first\n\
             # use, surf= the surface key that material maps may be named by, then wrap= every way it was drawn,\n\
             # u then v, r repeat or m mirror). Put edited or upscaled copies, same names, in a directory and pass\n\
             # it to `run --texture-pack DIR` (docs/texture-packs.md).\n",
        );
        for (name, e) in &self.index {
            let wrap = if e.wraps != 0 { wrap_field(e.wraps) } else { String::new() };
            let looks = if e.looks.len() > 1 { format!("\tlooks={}", e.looks.len()) } else { String::new() };
            let _ = writeln!(text, "{name}\tframe={}\tuses={}\t{}{wrap}{looks}", e.first_frame, e.uses, e.about);
        }
        let _ = std::fs::write(dir.join("index.txt"), text);
        self.index_dirty = false;
    }

    /// Polygons drawn with a material and the pixels materials changed
    /// since the last call (software snapshots).
    pub fn take_material_counts(&mut self) -> [u64; 2] {
        std::mem::take(&mut self.material_counts)
    }

    /// One line for the end-of-run report.
    pub fn summary(&self) -> String {
        let (tex, tiles) =
            self.index.keys().fold((0, 0), |(a, b), k| if k.starts_with("tile_") { (a, b + 1) } else { (a + 1, b) });
        let mut s = String::new();
        if let Some(d) = &self.dump {
            let _ = write!(s, "dumped {tex} textures and {tiles} tile patterns to {}", d.display());
            let looks = self.index.values().filter(|e| e.looks.len() > 1).count();
            if looks > 0 {
                let _ = write!(s, " ({looks} textures drawn in several looks: each dump shows its most used)");
            }
        }
        if let Some(e) = &self.refused {
            if !s.is_empty() {
                s += "; ";
            }
            let _ = write!(s, "texture pack refused ({e}): original textures drawn");
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

    fn texture_mips(&mut self, info: &TexInfo) -> Option<Arc<Vec<Rgba>>> {
        let img = self.texture(info)?;
        let mips = self.mips.entry(info.key).or_insert_with(|| Arc::new(img.mip_chain()));
        Some(mips.clone())
    }

    fn texture_drawn(&mut self, info: &TexInfo, src: &Sources, pixels: usize) {
        if pixels == 0 {
            return;
        }
        let about = || {
            format!(
                "header={:04x},{:04x},{:04x},{:04x}\tcolour={:#06x}\tbank={}\tat={},{}\tluma_table={}\ttranslucent={}\tsurf={:016x}",
                info.header[0],
                info.header[1],
                info.header[2],
                info.header[3],
                info.colour,
                info.bank,
                info.x0,
                info.y0,
                info.luma_table,
                info.translucent as u8,
                info.surface
            )
        };
        let wrap = 1 << ((info.header[0] >> 8) & 3);
        let look = || {
            let mut f = Fnv::default();
            f.bytes(texture_look(src, info).as_flattened());
            f.0
        };
        self.record(info.name(), wrap, look, about, || texture_image(src, info));
    }

    fn texture_material(&mut self, info: &TexInfo) -> Option<Arc<Material>> {
        match TexturePack::material(self, info) {
            FoundMaterial::Maps(m) => Some(m),
            _ => None,
        }
    }

    fn texture_material_mips(&mut self, info: &TexInfo) -> Option<Arc<Vec<Material>>> {
        let m = self.texture_material(info)?;
        Some(self.material_mips.entry(m.id).or_insert_with(|| Arc::new(m.mip_chain())).clone())
    }

    fn material_drawn(&mut self, _info: &TexInfo, changed: usize) {
        self.material_counts[0] += 1;
        self.material_counts[1] += changed as u64;
    }

    fn regions(&mut self) -> Option<Arc<model2::regions::RegionSet>> {
        self.regions.clone()
    }

    fn tile(&mut self, info: &TileInfo) -> Option<Arc<Rgba>> {
        let about = || format!("tile={:#06x}\tpalette_bank={:#04x}", info.tile, info.bank);
        self.record(info.name(), 0, || 0, about, || info.image());
        self.pack.as_ref()?;
        let img = self.load(&info.name());
        self.looked_up[1] += 1;
        self.replaced[1] += img.is_some() as u64;
        img
    }
}

/// Synchronous: images load on first use, so never `Pending` (snapshots and
/// gpu-check stay deterministic).
impl TexturePack for Packs {
    fn find(&mut self, info: &TexInfo) -> Found {
        match self.texture(info) {
            Some(img) => Found::Image(img),
            None => Found::Missing,
        }
    }

    fn has_tiles(&self) -> bool {
        self.has_tiles || self.regions.is_some()
    }

    fn regions(&mut self) -> Option<Arc<model2::regions::RegionSet>> {
        self.regions.clone()
    }

    fn find_tile(&mut self, info: &TileInfo) -> Found {
        match self.load(&info.name()) {
            Some(img) => Found::Image(img),
            None => Found::Missing,
        }
    }

    fn has_materials(&self) -> bool {
        !self.maps.is_empty()
    }

    /// Loaded on first use and kept, like the images (never `Pending`).
    fn material(&mut self, info: &TexInfo) -> FoundMaterial {
        let Some(dir) = &self.pack else { return FoundMaterial::Missing };
        let stems = resolve_maps(info, |s| self.maps.contains(s));
        if stems.iter().all(Option::is_none) {
            return FoundMaterial::Missing;
        }
        let stems = stems.each_ref().map(Option::as_deref);
        let id = model2::enhance::material_id(stems);
        let m = self.materials.entry(id).or_insert_with(|| {
            let files = stems.map(|s| s.and_then(|s| decode(&dir.join(format!("{s}.png")))));
            files
                .iter()
                .any(Option::is_some)
                .then(|| Arc::new(Material::assemble(files.each_ref().map(Option::as_deref), &self.manifest, stems)))
        });
        m.clone().map_or(FoundMaterial::Missing, FoundMaterial::Maps)
    }

    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
}

/// A pack image or map file (None: missing, or unreadable, said once).
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
    fn index_lists_every_way_a_texture_is_drawn() {
        assert_eq!(wrap_field(0b0001), "\twrap=rr");
        assert_eq!(wrap_field(0b1011), "\twrap=rr,mr,mm");
    }

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
        let mut p = Packs::new(None, Some(pack.clone())).unwrap();
        assert_eq!(p.tile(&info).as_deref(), Some(&big));
        assert_eq!(png::decode(&dumped).unwrap().width, 8);
        // Textures through the pack interface: Missing, then the image.
        let tex = TexInfo {
            key: 0xabcd,
            surface: 0,
            width: 8,
            height: 8,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        assert!(matches!(p.find(&tex), Found::Missing));
        std::fs::write(pack.join(format!("{}.png", tex.name())), png::encode_rgba(32, 32, &big.data)).unwrap();
        let mut p = Packs::new(None, Some(pack.clone())).unwrap();
        assert!(matches!(p.find(&tex), Found::Image(img) if *img == big));
        // A manifest this build reads changes nothing; one it can't (a newer
        // format, a feature it needs and can't show) draws the originals.
        for m in ["format 2\nuses materials\n", "format 2\nneeds materials\n"] {
            std::fs::write(pack.join("pack.txt"), m).unwrap();
            assert!(matches!(Packs::new(None, Some(pack.clone())).unwrap().find(&tex), Found::Image(_)), "{m}");
        }
        for m in ["format 3\n", "format 2\nneeds models\n", "not a manifest\n"] {
            std::fs::write(pack.join("pack.txt"), m).unwrap();
            let mut p = Packs::new(None, Some(pack.clone())).unwrap();
            assert!(matches!(p.find(&tex), Found::Missing) && !p.has_tiles(), "{m}");
            assert!(p.tile(&info).is_none());
            assert!(p.summary().contains("refused"), "{}", p.summary());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loads_material_maps_from_format_2_packs() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../out/scratch/remodel-texpack-mat-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let tex = TexInfo {
            key: 0xabcd,
            surface: 0x1234,
            width: 8,
            height: 8,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        let px = |v: [u8; 4], n| v.repeat(n);
        // A normal map by image key, a 2x2 specular map by surface key.
        std::fs::write(
            dir.join(format!("{}.n.png", tex.name())),
            png::encode_rgba(4, 4, &px([140, 128, 255, 255], 16)),
        )
        .unwrap();
        std::fs::write(
            dir.join(format!("{}.s.png", tex.surface_name())),
            png::encode_rgba(2, 2, &px([64, 0, 0, 255], 4)),
        )
        .unwrap();
        // No manifest: a format 1 pack, maps not read.
        let mut p = Packs::new(None, Some(dir.clone())).unwrap();
        assert!(!p.has_materials() && matches!(p.material(&tex), FoundMaterial::Missing));
        std::fs::write(dir.join("pack.txt"), "format 2\nuses materials\nspec_scale 2\n").unwrap();
        let mut p = Packs::new(None, Some(dir.clone())).unwrap();
        assert_eq!(p.material_maps(), 2);
        let FoundMaterial::Maps(m) = p.material(&tex) else { panic!("no material") };
        assert_eq!((m.width, m.height, m.has, m.spec_scale), (4, 4, 3, 2.0));
        assert!(m.data.chunks(4).all(|t| t == [140, 128, 64, 128]));
        // Another colour of the texels: the surface's specular map alone.
        let FoundMaterial::Maps(m) = p.material(&TexInfo { key: 1, ..tex.clone() }) else { panic!("no material") };
        assert_eq!((m.width, m.height, m.has), (2, 2, 2));
        // No replacement image is needed, and none is found.
        assert!(matches!(p.find(&tex), Found::Missing));
        assert!(matches!(p.material(&TexInfo { surface: 9, ..tex.clone() }), FoundMaterial::Maps(_)));
        assert!(matches!(p.material(&TexInfo { key: 2, surface: 9, ..tex.clone() }), FoundMaterial::Missing));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn dumps_the_most_used_look() {
        // One 32x32 texture of texel 0 in colour (1, 1, 1), luma table 0
        // entry 0 = 0, so it maps through the translation tables at luma 0.
        let dir = std::env::temp_dir().join(format!("remodel-texpack-look-{}", std::process::id()));
        let mut p = Packs::new(Some(dir.clone()), None).unwrap();
        let ram = vec![0u8; 0x40_0000 + 0x8000];
        let tex = TexInfo {
            key: 0x77,
            surface: 0,
            width: 32,
            height: 32,
            header: [0x4000, 0, 0, 0],
            colour: 1 | 1 << 5 | 1 << 10,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        };
        let palette = |v: u8| {
            let mut pal = vec![0u8; 0x1_c000];
            for ch in 0..3 {
                pal[0x1_0000 + ch * 0x4000 + 256 * 2] = v;
            }
            pal
        };
        let (faded, normal) = (palette(255), palette(90));
        let draw = |p: &mut Packs, pal: &[u8], n: usize| {
            let src = Sources {
                texture_rom: &[],
                texture_point_ram: &[],
                render_ram: &ram,
                palette: pal,
                xoff: 0.0,
                yoff: 0.0,
            };
            for _ in 0..n {
                p.texture_drawn(&tex, &src, 10);
            }
            p.finish_snapshot();
        };
        let dumped = || png::decode(&std::fs::read(dir.join(format!("{}.png", tex.name()))).unwrap()).unwrap().rgba;
        // First seen in a fade: dumped as it looks then...
        draw(&mut p, &faded, 2);
        assert_eq!(dumped()[..4], [255, 255, 255, 255]);
        // ...then drawn more in its usual look: that is the dump.
        draw(&mut p, &normal, 3);
        assert_eq!(dumped()[..4], [90, 90, 90, 255]);
        let index = std::fs::read_to_string(dir.join("index.txt")).unwrap();
        assert!(index.contains("uses=5\t") && index.contains("\tlooks=2\n"), "{index}");
        // The surface key, before wrap=: what parse_index-style readers find by k=v.
        assert!(index.contains("\ttranslucent=0\tsurf=0000000000000000\twrap=rr\t"), "{index}");
        assert!(p.summary().contains("1 textures drawn in several looks"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
