//! A Model 2 board's frame as the renderer reads it: borrowed from a bus
//! ([`BoardRef`]), or held apart from the machine ([`BoardFrame`]) so it
//! can be drawn on another thread while the machine runs its next frame.

use model2::raster::Sources;
use model2::Model2Bus;
use std::sync::Arc;

/// The memory a frame is drawn from, and its versions
/// (`Model2Bus::video_ram_versions`) and 3D screen offsets.
#[derive(Clone, Copy)]
pub(crate) struct BoardRef<'a> {
    pub texture_rom: &'a [u16],
    pub render_ram: &'a [u8],
    pub palette: &'a [u8],
    pub tiles: &'a [u8],
    pub sync: (f32, f32),
    pub versions: (u64, u64),
    /// `Model2Bus::black_level`.
    pub black: f32,
}

impl<'a> BoardRef<'a> {
    pub fn of(b: &'a Model2Bus) -> Self {
        BoardRef {
            texture_rom: &b.texture_rom,
            render_ram: b.render_ram(),
            palette: b.palette_ram(),
            tiles: b.tile_ram(),
            sync: b.sync_offsets(),
            versions: b.video_ram_versions(),
            black: b.black_level(),
        }
    }

    /// The rasteriser's sources with texture point RAM `points`.
    pub fn sources(&self, points: &'a [u16]) -> Sources<'a> {
        Sources {
            texture_rom: self.texture_rom,
            texture_point_ram: points,
            render_ram: self.render_ram,
            palette: self.palette,
            xoff: self.sync.0,
            yoff: self.sync.1,
        }
    }
}

/// One frame of a board, copied out of its bus by [`BoardFrame::capture`]
/// so the machine can run on while it is drawn
/// ([`crate::Renderer::present_board`]): what the `present_bus*` calls
/// read from the bus and its geometrizer. The texture ROM is shared, render
/// and palette RAM are copied only when their versions changed since the
/// last capture (render RAM is 7 MB; most frames don't touch it), tile RAM
/// (1 MB) and the raster stream every time. Kept and reused from frame to
/// frame, so it allocates once.
#[derive(Default)]
pub struct BoardFrame {
    texture_rom: Option<Arc<[u16]>>,
    render_ram: Vec<u8>,
    palette: Vec<u8>,
    tiles: Vec<u8>,
    /// Versions of render and palette RAM as copied (None: not yet, or
    /// forgotten).
    copied: Option<(u64, u64)>,
    sync: (f32, f32),
    versions: (u64, u64),
    black: f32,
    /// The geometrizer's output drawn: its raster stream and texture point
    /// RAM, and its smooth motion frame and lighting side channel if it
    /// recorded them.
    stream: Vec<u32>,
    points: Vec<u16>,
    smooth: Option<Arc<model2::smooth::Frame>>,
    light: Option<Arc<model2::light::Frame>>,
    late: bool,
    /// Bytes the last capture copied (diagnostics).
    pub bytes: usize,
}

impl BoardFrame {
    /// Copy `bus`'s current frame: the geometrizer's frame just finished
    /// (as `present_bus` draws), or with `late` the one before it (as
    /// `present_bus_last` draws, while the newest is still being worked
    /// out). Waits for the geometrizer unless `late`.
    pub fn capture(&mut self, bus: &mut Model2Bus, late: bool) {
        if !late {
            bus.geo.sync();
        }
        let b = BoardRef::of(bus);
        if self.texture_rom.as_ref().is_none_or(|t| !Arc::ptr_eq(t, &bus.texture_rom)) {
            self.texture_rom = Some(bus.texture_rom.clone());
        }
        let mut bytes = 0;
        let (old_render, old_palette) = self.copied.map_or((None, None), |(r, p)| (Some(r), Some(p)));
        if old_render != Some(b.versions.0) || self.render_ram.len() != b.render_ram.len() {
            copy(&mut self.render_ram, b.render_ram);
            bytes += b.render_ram.len();
        }
        if old_palette != Some(b.versions.1) || self.palette.len() != b.palette.len() {
            copy(&mut self.palette, b.palette);
            bytes += b.palette.len();
        }
        copy(&mut self.tiles, b.tiles);
        bytes += b.tiles.len();
        self.copied = Some(b.versions);
        self.sync = b.sync;
        self.versions = b.versions;
        self.black = b.black;
        let geo = &bus.geo;
        let ((stream, points), smooth, light) = if late {
            (geo.last_frame(), geo.last_smooth().cloned(), geo.last_light().cloned())
        } else {
            let e = geo.engine();
            ((&e.stream[..], &e.texture_point_ram[..]), e.smooth_frame(), e.light_frame())
        };
        copy(&mut self.stream, stream);
        copy(&mut self.points, points);
        bytes += stream.len() * 4 + points.len() * 2;
        (self.smooth, self.light, self.late) = (smooth, light, late);
        self.bytes = bytes;
    }

    /// Forget what was copied, so the next capture copies everything: for
    /// another machine (run-ahead's, a new game) or a loaded state, whose
    /// RAM versions say nothing about what this holds.
    pub fn forget(&mut self) {
        self.copied = None;
    }

    /// Whether this frame holds the geometrizer's previous frame's
    /// polygons (captured `late`).
    pub fn late(&self) -> bool {
        self.late
    }

    /// The smooth motion frame recorded with the polygons drawn, if any:
    /// its in-betweens and motion data.
    pub fn smooth(&self) -> Option<&Arc<model2::smooth::Frame>> {
        self.smooth.as_ref()
    }

    /// The raster stream drawn (the game frame's own polygons).
    pub fn stream(&self) -> &[u32] {
        &self.stream
    }

    /// Whether anything was captured yet.
    pub fn is_empty(&self) -> bool {
        self.texture_rom.is_none()
    }

    pub(crate) fn board(&self) -> BoardRef<'_> {
        BoardRef {
            texture_rom: self.texture_rom.as_deref().unwrap_or(&[]),
            render_ram: &self.render_ram,
            palette: &self.palette,
            tiles: &self.tiles,
            sync: self.sync,
            versions: self.versions,
            black: self.black,
        }
    }

    /// The texture point RAM drawn with the frame's polygons.
    pub fn points(&self) -> &[u16] {
        &self.points
    }

    /// The lighting side channel recorded with the frame's polygons.
    pub fn light_frame(&self) -> Option<&Arc<model2::light::Frame>> {
        self.light.as_ref()
    }

    pub(crate) fn engine(&self) -> (&[u32], &[u16]) {
        (&self.stream, &self.points)
    }

    pub(crate) fn light(&self) -> Option<&Arc<model2::light::Frame>> {
        self.light.as_ref()
    }
}

/// `dst` = `src`, reusing `dst`'s allocation.
fn copy<T: Copy>(dst: &mut Vec<T>, src: &[T]) {
    if dst.len() == src.len() {
        dst.copy_from_slice(src);
    } else {
        dst.clear();
        dst.extend_from_slice(src);
    }
}
