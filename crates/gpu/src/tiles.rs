//! Live tile replacement: a texture pack's `tile_<key>.png` images drawn in
//! place of the 2D layers' patterns at output resolution
//! (docs/texture-packs.md, "Tiles"), by the same rules as the software
//! snapshots (`model2::video::compose_enhanced`).
//!
//! The CPU works out, per frame, which tile is under each game pixel on
//! each plane and where its replacement is (`video::layers_2d_replaced`);
//! replacements live in one atlas texture of equal square cells, and
//! compose.wgsl's `fs_tiles` samples them at each output pixel's place in
//! its game pixel, bilinear and clamped to the tile, so tiles never blend
//! into their neighbours. Only a pack with tile images (or regions) does any
//! of this.
//!
//! Region overrides (docs/texture-packs.md, "Regions") ride the same path:
//! each tile cell of a matched region's image is a cell of the atlas, found
//! per map cell instead of per pattern (`video::layers_2d_replaced`), so
//! the shader is unchanged.

use model2::enhance::{Address, Found, Rgba, TexturePack, TileInfo};
use model2::regions::Slice;
use model2::video::{self, Layers2d, TileMap, View, HEIGHT, NO_CELL, WIDTH};
use std::cell::RefCell;
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// Cells along each side of the atlas, at most (fewer for large cells).
const CELLS_PER_ROW: u32 = 64;
/// Largest atlas side, texels.
const MAX_ATLAS: u32 = 4096;
/// Largest cell: larger images are scaled down to it.
const MAX_CELL: u32 = 256;

/// Replacement images in one texture, a cell each, by tile key; the least
/// recently drawn give way when it is full.
struct Atlas {
    view: wgpu::TextureView,
    texture: wgpu::Texture,
    /// Cell side and cells per row and column.
    cell: u32,
    per_row: u32,
    /// Per cell: the key it holds and the frame it was last drawn in.
    owner: Vec<Option<u64>>,
    used: Vec<u64>,
}

impl Atlas {
    /// An atlas of `cell`-texel cells.
    fn new(device: &wgpu::Device, cell: u32) -> Atlas {
        let max = device.limits().max_texture_dimension_2d.min(MAX_ATLAS);
        let per_row = (max / cell).min(CELLS_PER_ROW);
        let side = per_row * cell;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile atlas"),
            size: wgpu::Extent3d { width: side, height: side, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Cell NO_CELL is "none" in the hits.
        let cells = ((per_row * per_row) as usize).min(NO_CELL as usize);
        Atlas {
            view: texture.create_view(&Default::default()),
            texture,
            cell,
            per_row,
            owner: vec![None; cells],
            used: vec![0; cells],
        }
    }

    fn bytes(&self) -> u64 {
        (self.per_row * self.cell) as u64 * (self.per_row * self.cell) as u64 * 4
    }

    /// A free cell, or the least recently drawn one not drawn this frame
    /// (its key is returned to be forgotten); None when every cell is in
    /// use this frame.
    fn take(&mut self, frame: u64) -> Option<(u16, Option<u64>)> {
        if let Some(c) = self.owner.iter().position(Option::is_none) {
            return Some((c as u16, None));
        }
        let c = (0..self.used.len()).filter(|&c| self.used[c] < frame).min_by_key(|&c| self.used[c])?;
        Some((c as u16, self.owner[c]))
    }

    /// Put `img` in cell `c`, scaled to the cell if it is another size.
    fn upload(&self, queue: &wgpu::Queue, c: u16, img: &Rgba) {
        let n = self.cell as usize;
        let scaled;
        let img = if (img.width, img.height) == (n, n) {
            img
        } else {
            scaled = resample(img, n);
            &scaled
        };
        let (x, y) = (c as u32 % self.per_row * self.cell, c as u32 / self.per_row * self.cell);
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x, y, z: 0 },
                aspect: wgpu::TextureAspect::All,
            },
            &img.data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(self.cell * 4), rows_per_image: None },
            wgpu::Extent3d { width: self.cell, height: self.cell, depth_or_array_layers: 1 },
        );
    }
}

/// `img` resampled to `n` x `n` (bilinear, clamped, as tiles are drawn).
fn resample(img: &Rgba, n: usize) -> Rgba {
    let mut data = Vec::with_capacity(n * n * 4);
    for y in 0..n {
        for x in 0..n {
            let sx = (x as f32 + 0.5) * img.width as f32 / n as f32;
            let sy = (y as f32 + 0.5) * img.height as f32 / n as f32;
            let (c, a) = img.sample(sx, sy, Address::Clamp, Address::Clamp);
            let c = c.map(|v| v.round().clamp(0.0, 255.0) as u8);
            data.extend_from_slice(&[c[0], c[1], c[2], (a * 255.0).round() as u8]);
        }
    }
    Rgba { width: n, height: n, data }
}

/// The renderer's tile replacement state (see the module comment).
pub(crate) struct Tiles {
    atlas: Option<Atlas>,
    /// Cells by tile key (None: not in the pack or unusable).
    by_key: HashMap<u64, Option<u16>>,
    /// Frames composed with tiles looked up, for eviction.
    frame: u64,
    layout: wgpu::BindGroupLayout,
    /// The `fs_tiles` compose pipeline for the last output format used.
    pipeline: Option<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    /// The hits and HUD source textures, kept while their size holds.
    hits: Option<wgpu::Texture>,
    hud_src: Option<wgpu::Texture>,
}

fn uint_texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Uint,
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl Tiles {
    pub fn new(device: &wgpu::Device) -> Tiles {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("compose tiles"),
            entries: &[
                uint_texture(0),
                uint_texture(1),
                super::texture(2),
                super::storage(3),
                super::uniform(4, wgpu::ShaderStages::FRAGMENT),
            ],
        });
        Tiles { atlas: None, by_key: HashMap::new(), frame: 0, layout, pipeline: None, hits: None, hud_src: None }
    }

    /// Forget every image (another pack).
    pub fn reset(&mut self) {
        self.atlas = None;
        self.by_key.clear();
    }

    /// Replacement images held, and the atlas's size in bytes.
    pub fn held(&self) -> (u32, u64) {
        let n = self.atlas.as_ref().map_or(0, |a| a.owner.iter().flatten().count());
        (n as u32, self.atlas.as_ref().map_or(0, Atlas::bytes))
    }

    /// The 2D layers of a frame with `pack`'s tile replacements: looks the
    /// patterns on screen up (once per key; `Pending` ones again next
    /// frame) and uploads new images into the atlas.
    #[allow(clippy::too_many_arguments)]
    pub fn layers(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pack: &mut dyn TexturePack,
        tiles: &[u8],
        palette: &[u8],
        view: &View,
        has_3d: bool,
    ) -> Layers2d {
        self.frame += 1;
        let frame = self.frame;
        let set = pack.regions();
        // The atlas's cells are as large as the largest image it will hold,
        // regions' included (smaller images are scaled up to a cell).
        let want = set.as_ref().map_or(0, |s| s.max_cell());
        let state = RefCell::new((&mut self.atlas, &mut self.by_key));
        // Put `img` in the atlas under `key` (None: unusable or no room this frame).
        let put = |key: u64, img: &Rgba| -> Option<u16> {
            let mut st = state.borrow_mut();
            let (atlas, by_key) = &mut *st;
            let side = (img.width.max(want) as u32).clamp(8, MAX_CELL);
            let a = atlas.get_or_insert_with(|| Atlas::new(device, side));
            // Full of this frame's tiles: the original this time.
            let (c, old) = a.take(frame)?;
            if let Some(old) = old {
                by_key.remove(&old);
            }
            a.upload(queue, c, img);
            a.owner[c as usize] = Some(key);
            a.used[c as usize] = frame;
            by_key.insert(key, Some(c));
            Some(c)
        };
        // A cached cell for `key`, touched for this frame (Some(None): known to have none).
        let cached = |key: u64| -> Option<Option<u16>> {
            let mut st = state.borrow_mut();
            let (atlas, by_key) = &mut *st;
            let c = *by_key.get(&key)?;
            if let (Some(c), Some(a)) = (c, atlas.as_mut()) {
                a.used[c as usize] = frame;
            }
            Some(c)
        };
        let mut cell = |info: &TileInfo| -> Option<u16> {
            if let Some(c) = cached(info.key) {
                return c;
            }
            let img = match pack.find_tile(info) {
                Found::Pending => return None,
                Found::Missing => {
                    state.borrow_mut().1.insert(info.key, None);
                    return None;
                }
                Found::Image(img) => img,
            };
            // As the software: smaller than a tile is unusable.
            if img.width < 8 || img.height < 8 {
                state.borrow_mut().1.insert(info.key, None);
                return None;
            }
            put(info.key, &img)
        };
        let mut slice_cell = |s: &Slice| -> Option<u16> {
            if let Some(c) = cached(s.key) {
                return c;
            }
            put(s.key, &s.image)
        };
        let regions = set.as_deref().map(|s| (s, &mut slice_cell as &mut dyn FnMut(&Slice) -> Option<u16>));
        video::layers_2d_replaced(tiles, palette, view, has_3d, &mut cell, regions)
    }

    /// Upload `map` (of layers `ext_w` columns wide) and bind it with the
    /// atlas for `fs_tiles`; None without an atlas.
    pub fn bind(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        map: &TileMap,
        ext_w: usize,
    ) -> Option<wgpu::BindGroup> {
        let atlas = self.atlas.as_ref()?;
        let kept = |slot: &mut Option<wgpu::Texture>, label, w: u32, format| {
            let t = slot.take().filter(|t| t.width() == w && t.format() == format).unwrap_or_else(|| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: w, height: HEIGHT as u32, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                    view_formats: &[],
                })
            });
            *slot = Some(t.clone());
            t
        };
        let hits = kept(&mut self.hits, "tile hits", WIDTH as u32, wgpu::TextureFormat::Rg32Uint);
        let hud_src = kept(&mut self.hud_src, "tile hud source", ext_w as u32, wgpu::TextureFormat::R32Uint);
        let write = |t: &wgpu::Texture, words: &[u32], per_row: u32| {
            let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
            queue.write_texture(
                t.as_image_copy(),
                &bytes,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(per_row * 4), rows_per_image: None },
                wgpu::Extent3d { width: t.width(), height: HEIGHT as u32, depth_or_array_layers: 1 },
            );
        };
        write(&hits, &map.hits, 2 * WIDTH as u32);
        write(&hud_src, &map.hud_src, ext_w as u32);
        let palette = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tile palette"),
            contents: &super::le(map.palette.iter().map(|c| c.to_le_bytes())),
            usage: wgpu::BufferUsages::STORAGE,
        });
        let params = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tile params"),
            contents: &super::le([atlas.cell, atlas.per_row, 0, 0].map(u32::to_le_bytes)),
            usage: wgpu::BufferUsages::UNIFORM,
        });
        let views = [hits.create_view(&Default::default()), hud_src.create_view(&Default::default())];
        Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("compose tiles"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&views[0]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&views[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&atlas.view) },
                wgpu::BindGroupEntry { binding: 3, resource: palette.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: params.as_entire_binding() },
            ],
        }))
    }

    /// The compose pipeline with replaced tiles (`fs_tiles`) for `format`.
    pub fn pipeline(
        &mut self,
        device: &wgpu::Device,
        shader: &wgpu::ShaderModule,
        compose_layout: &wgpu::BindGroupLayout,
        format: wgpu::TextureFormat,
    ) -> wgpu::RenderPipeline {
        if let Some((f, p)) = &self.pipeline {
            if *f == format {
                return p.clone();
            }
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compose tiles"),
            bind_group_layouts: &[Some(compose_layout), Some(&self.layout)],
            immediate_size: 0,
        });
        let p = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compose tiles"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some("fs_tiles"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        self.pipeline = Some((format, p.clone()));
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resampling_keeps_flat_images_and_cut_outs() {
        let img = Rgba { width: 16, height: 16, data: [10, 20, 30, 255].repeat(256) };
        assert_eq!(resample(&img, 32).data, [10, 20, 30, 255].repeat(1024));
        // Left half opaque, right half transparent: the cut stays in the
        // middle.
        let data = (0..16 * 16).flat_map(|i| if i % 16 < 8 { [200, 0, 0, 255] } else { [0, 0, 0, 0] }).collect();
        let r = resample(&Rgba { width: 16, height: 16, data }, 8);
        assert_eq!(r.data[3 * 4 + 3], 255);
        assert_eq!(r.data[4 * 4 + 3], 0);
        assert_eq!(r.data[..3], [200, 0, 0]);
    }
}
