//! Several texture packs stacked: for each texture, tile or material the
//! highest pack that has it wins (docs/texture-packs.md, "Layers"). The
//! renderer takes a pack as one `TexturePack`, so a stack is a wrapper over
//! several of them and the renderer does not change. Draw-only: nothing here
//! touches emulation.
//!
//! A layer that answers `Pending` (its image is still decoding) holds the
//! answer: the layers below are not asked, so a texture never shows a lower
//! layer's image for a frame and then jumps to the higher one's.

use crate::enhance::{Found, FoundMaterial, PreloadKind, Rgba, TexInfo, TexturePack, TileInfo};
use crate::pack::{Manifest, NO_MANIFEST};
use crate::regions::RegionSet;
use std::collections::HashSet;
use std::sync::Arc;

pub struct LayeredPack {
    /// Highest first.
    layers: Vec<Box<dyn TexturePack + Send>>,
}

impl LayeredPack {
    /// `layers`, highest first. Zero layers draws the original.
    pub fn new(layers: Vec<Box<dyn TexturePack + Send>>) -> LayeredPack {
        LayeredPack { layers }
    }

    pub fn len(&self) -> usize {
        self.layers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.layers.is_empty()
    }
}

impl TexturePack for LayeredPack {
    fn find(&mut self, info: &TexInfo) -> Found {
        for l in &mut self.layers {
            match l.find(info) {
                Found::Missing => continue,
                other => return other,
            }
        }
        Found::Missing
    }

    fn preload(&mut self, kind: PreloadKind, max: usize) -> Vec<(u64, Arc<Rgba>)> {
        // A key a higher layer handed over is not handed over again from a
        // lower one in the same call.
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for l in &mut self.layers {
            for (k, img) in l.preload(kind, max.saturating_sub(out.len())) {
                if seen.insert(k) {
                    out.push((k, img));
                }
            }
        }
        out
    }

    fn preload_restart(&mut self, kind: PreloadKind) {
        for l in &mut self.layers {
            l.preload_restart(kind);
        }
    }

    fn has_tiles(&self) -> bool {
        self.layers.iter().any(|l| l.has_tiles())
    }

    fn find_tile(&mut self, info: &TileInfo) -> Found {
        for l in &mut self.layers {
            match l.find_tile(info) {
                Found::Missing => continue,
                other => return other,
            }
        }
        Found::Missing
    }

    /// The highest layer's regions: region sets are not merged, so a layer
    /// with regions hides every lower layer's.
    fn regions(&mut self) -> Option<Arc<RegionSet>> {
        self.layers.iter_mut().find_map(|l| l.regions())
    }

    fn has_materials(&self) -> bool {
        self.layers.iter().any(|l| l.has_materials())
    }

    fn material(&mut self, info: &TexInfo) -> FoundMaterial {
        for l in &mut self.layers {
            match l.material(info) {
                FoundMaterial::Missing => continue,
                other => return other,
            }
        }
        FoundMaterial::Missing
    }

    /// The highest layer's manifest.
    fn manifest(&self) -> &Manifest {
        self.layers.first().map_or(&NO_MANIFEST, |l| l.manifest())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pack that has the textures whose key is listed, as a flat colour,
    /// or answers `Pending` to everything.
    struct Flat(u8, Vec<u64>, bool);

    impl TexturePack for Flat {
        fn find(&mut self, info: &TexInfo) -> Found {
            if self.2 {
                return Found::Pending;
            }
            if self.1.contains(&info.key) {
                Found::Image(Arc::new(Rgba { width: 1, height: 1, data: vec![self.0, 0, 0, 255] }))
            } else {
                Found::Missing
            }
        }
    }

    fn info(key: u64) -> TexInfo {
        TexInfo {
            key,
            surface: key,
            width: 8,
            height: 8,
            header: [0; 4],
            colour: 0,
            bank: 0,
            x0: 0,
            y0: 0,
            luma_table: 0,
            translucent: false,
        }
    }

    fn red(f: Found) -> Option<u8> {
        match f {
            Found::Image(i) => Some(i.data[0]),
            _ => None,
        }
    }

    #[test]
    fn the_highest_layer_with_the_texture_wins() {
        let mut l = LayeredPack::new(vec![Box::new(Flat(1, vec![10], false)), Box::new(Flat(2, vec![10, 20], false))]);
        assert_eq!(red(l.find(&info(10))), Some(1), "both have it: the top one");
        assert_eq!(red(l.find(&info(20))), Some(2), "only the lower has it");
        assert!(matches!(l.find(&info(30)), Found::Missing));
        assert!(LayeredPack::new(vec![]).is_empty());
    }

    #[test]
    fn a_pending_top_layer_holds_the_answer() {
        let mut l = LayeredPack::new(vec![Box::new(Flat(1, vec![], true)), Box::new(Flat(2, vec![10], false))]);
        assert!(matches!(l.find(&info(10)), Found::Pending));
    }
}
