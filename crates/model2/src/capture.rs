//! Captured frames: everything the renderers read for one frame, saved so
//! a renderer can be run and compared offline (`remodel run --capture`,
//! `remodel gpu-check`).
//!
//! A capture directory holds `texture-rom.bin` (the texture ROM, shared by
//! every frame) and one subdirectory per frame with the raster stream, the
//! video RAMs and the sync offsets, all little-endian. Derived from the
//! game: keep captures under `out/`.

use crate::raster::Sources;
use std::path::Path;

pub struct Frame {
    pub frame: u64,
    /// The geometrizer's raster stream.
    pub stream: Vec<u32>,
    pub texture_point_ram: Vec<u16>,
    pub render_ram: Vec<u8>,
    pub palette: Vec<u8>,
    pub tiles: Vec<u8>,
    pub xoff: f32,
    pub yoff: f32,
}

fn words16(b: &[u8]) -> Vec<u16> {
    b.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect()
}

fn bytes16(w: &[u16]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_le_bytes()).collect()
}

impl Frame {
    /// The current frame of `bus` (waits for the geometrizer to finish it).
    pub fn grab(bus: &mut crate::Model2Bus) -> Frame {
        bus.geo.sync();
        let (xoff, yoff) = bus.sync_offsets();
        let geo = bus.geo.engine();
        Frame {
            frame: bus.frames,
            stream: geo.stream.clone(),
            texture_point_ram: geo.texture_point_ram.clone(),
            render_ram: bus.render_ram().to_vec(),
            palette: bus.palette_ram().to_vec(),
            tiles: bus.tile_ram().to_vec(),
            xoff,
            yoff,
        }
    }

    /// The renderer's view of this frame.
    pub fn sources<'a>(&'a self, texture_rom: &'a [u16]) -> Sources<'a> {
        Sources {
            texture_rom,
            texture_point_ram: &self.texture_point_ram,
            render_ram: &self.render_ram,
            palette: &self.palette,
            xoff: self.xoff,
            yoff: self.yoff,
        }
    }

    /// Save as `root/NNNNNN/`, writing `root/texture-rom.bin` if missing.
    pub fn save(&self, root: &Path, texture_rom: &[u16]) -> std::io::Result<()> {
        let rom = root.join("texture-rom.bin");
        if !rom.exists() {
            std::fs::create_dir_all(root)?;
            std::fs::write(rom, bytes16(texture_rom))?;
        }
        let dir = root.join(format!("{:06}", self.frame));
        std::fs::create_dir_all(&dir)?;
        let stream: Vec<u8> = self.stream.iter().flat_map(|w| w.to_le_bytes()).collect();
        std::fs::write(dir.join("stream.bin"), stream)?;
        std::fs::write(dir.join("points.bin"), bytes16(&self.texture_point_ram))?;
        std::fs::write(dir.join("render.bin"), &self.render_ram)?;
        std::fs::write(dir.join("palette.bin"), &self.palette)?;
        std::fs::write(dir.join("tiles.bin"), &self.tiles)?;
        std::fs::write(dir.join("meta.txt"), format!("frame {}\nxoff {}\nyoff {}\n", self.frame, self.xoff, self.yoff))
    }

    pub fn load(dir: &Path) -> std::io::Result<Frame> {
        let read = |f: &str| std::fs::read(dir.join(f));
        let meta = std::fs::read_to_string(dir.join("meta.txt"))?;
        let field = |k: &str| {
            meta.lines().find_map(|l| l.strip_prefix(k).and_then(|v| v.trim().parse::<f64>().ok())).unwrap_or(0.0)
        };
        Ok(Frame {
            frame: field("frame ") as u64,
            stream: read("stream.bin")?.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect(),
            texture_point_ram: words16(&read("points.bin")?),
            render_ram: read("render.bin")?,
            palette: read("palette.bin")?,
            tiles: read("tiles.bin")?,
            xoff: field("xoff ") as f32,
            yoff: field("yoff ") as f32,
        })
    }
}

/// The texture ROM saved in capture directory `root`.
pub fn load_texture_rom(root: &Path) -> std::io::Result<Vec<u16>> {
    Ok(words16(&std::fs::read(root.join("texture-rom.bin"))?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let root = std::env::temp_dir().join(format!("remodel-capture-{}", std::process::id()));
        let f = Frame {
            frame: 42,
            stream: vec![0xff00_0003, 7],
            texture_point_ram: vec![1, 2],
            render_ram: vec![3; 8],
            palette: vec![4; 4],
            tiles: vec![5; 2],
            xoff: 84.0,
            yoff: 130.5,
        };
        f.save(&root, &[9, 10]).unwrap();
        let g = Frame::load(&root.join("000042")).unwrap();
        assert_eq!((g.frame, &g.stream, g.xoff, g.yoff), (42, &f.stream, 84.0, 130.5));
        assert_eq!(load_texture_rom(&root).unwrap(), [9, 10]);
        let _ = std::fs::remove_dir_all(root);
    }
}
