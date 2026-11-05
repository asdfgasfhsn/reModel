//! Model 2 3D rasteriser on the GPU, through wgpu (Vulkan on Linux and
//! Android, Metal on macOS and iOS, DirectX 12 on Windows, OpenGL as a
//! fallback).
//!
//! It draws the same placed polygons as the software rasteriser
//! (`model2::raster::place`) and shades them with a port of its shading,
//! reading texels, luma tables and colour translation from the game's
//! video RAM. The software rasteriser stays the reference: `remodel
//! gpu-check` draws captured frames both ways and compares them.
//!
//! Polygons are drawn in order in a single pass with no depth buffer:
//! GPUs keep primitive order for overlapping pixels, which gives the
//! painter's algorithm the software renderer uses.

use model2::raster::{Placed, Poly, Sources};
use model2::video::View;
use wgpu::util::DeviceExt;

pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    adapter: String,
}

/// Bytes per vertex: x, y, 1/z, u/z, v/z, polygon index.
const VERTEX_BYTES: u64 = 24;
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn storage(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only: true },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// `bytes` padded to whole u32 words (storage buffers are read as u32).
fn words(bytes: &[u8]) -> Vec<u8> {
    let mut v = bytes.to_vec();
    v.resize(bytes.len().div_ceil(4).max(1) * 4, 0);
    v
}

impl Renderer {
    /// A headless renderer on the best GPU available.
    pub fn new() -> Result<Self, String> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .map_err(|e| format!("no GPU adapter: {e}"))?;
        let info = adapter.get_info();
        let name = format!("{} ({:?}, {})", info.name, info.backend, info.driver);
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("remodel"),
            required_limits: wgpu::Limits {
                // Render RAM is 4+ MB; ask for what this adapter allows.
                max_storage_buffer_binding_size: adapter.limits().max_storage_buffer_binding_size,
                ..wgpu::Limits::default()
            },
            ..Default::default()
        }))
        .map_err(|e| format!("{name}: {e}"))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("raster"),
            source: wgpu::ShaderSource::Wgsl(include_str!("raster.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("raster"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                storage(1),
                storage(2),
                storage(3),
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("raster"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("raster"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_BYTES,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x3, 2 => Uint32],
                })],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        Ok(Renderer { device, queue, pipeline, layout, adapter: name })
    }

    /// The GPU in use: name, backend and driver.
    pub fn adapter(&self) -> &str {
        &self.adapter
    }

    /// Draw `placed` (from `model2::raster::place` over `polys`) at the size
    /// of `view`. Returns RGB and a coverage mask, like the software
    /// rasteriser's `draw_view`.
    pub fn draw(&self, polys: &[Poly], placed: &[Placed], src: &Sources, view: &View) -> (Vec<u8>, Vec<bool>) {
        let (w, h) = (view.width as u32, view.height as u32);
        let mut verts: Vec<u8> = Vec::new();
        let mut table: Vec<u8> = Vec::new();
        for (i, pl) in placed.iter().enumerate() {
            let p = &polys[pl.poly];
            let t = p.texture;
            let flags = t.map_or(0, |t| {
                1 | (t.mirror_x as u32) << 1
                    | (t.mirror_y as u32) << 2
                    | (t.translucent as u32) << 3
                    | (t.checker as u32) << 4
            });
            let (x0, y0, x1, y1) = pl.clip;
            let tex = t.map_or([0; 4], |t| [t.x0, t.y0, t.w, t.h]);
            let (bank, luma_table) = t.map_or((0, 0), |t| (t.bank, t.luma_table));
            for f in [x0, y0, x1, y1] {
                table.extend(f.to_le_bytes());
            }
            for v in tex.into_iter().chain([flags, bank, luma_table, p.luma]).chain(p.colour).chain([0]) {
                table.extend(v.to_le_bytes());
            }
            // Convex polygon as a triangle fan.
            for k in 1..pl.points.len().saturating_sub(1) {
                for v in [pl.points[0], pl.points[k], pl.points[k + 1]] {
                    for f in [v.x, v.y, v.iz, v.uz, v.vz] {
                        verts.extend(f.to_le_bytes());
                    }
                    verts.extend((i as u32).to_le_bytes());
                }
            }
        }
        if table.is_empty() {
            table.resize(64, 0);
        }

        let d = &self.device;
        let buffer = |label, contents: &[u8], usage| {
            d.create_buffer_init(&wgpu::util::BufferInitDescriptor { label: Some(label), contents, usage })
        };
        let mut screen = Vec::new();
        for f in [w as f32, h as f32, view.sx, view.sy] {
            screen.extend(f.to_le_bytes());
        }
        let screen = buffer("screen", &screen, wgpu::BufferUsages::UNIFORM);
        let table = buffer("polys", &table, wgpu::BufferUsages::STORAGE);
        let render = buffer("render ram", &words(src.render_ram), wgpu::BufferUsages::STORAGE);
        let palette = buffer("palette", &words(src.palette), wgpu::BufferUsages::STORAGE);
        let vertices = buffer("vertices", &words(&verts), wgpu::BufferUsages::VERTEX);
        let bind = d.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("raster"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: screen.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: table.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: render.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: palette.as_entire_binding() },
            ],
        });

        let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
        let target = d.create_texture(&wgpu::TextureDescriptor {
            label: Some("3d layer"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view_tex = target.create_view(&Default::default());
        let row = (w * 4).div_ceil(256) * 256;
        let readback = d.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (row * h) as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut enc = d.create_command_encoder(&Default::default());
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("3d layer"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view_tex,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.set_vertex_buffer(0, vertices.slice(..));
            pass.draw(0..(verts.len() as u64 / VERTEX_BYTES) as u32, 0..1);
        }
        enc.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &readback,
                layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: None },
            },
            size,
        );
        self.queue.submit([enc.finish()]);

        let slice = readback.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        let _ = d.poll(wgpu::PollType::Wait { submission_index: None, timeout: None });
        let data = slice.get_mapped_range().expect("readback buffer is mapped");
        let mut rgb = vec![0u8; (w * h * 3) as usize];
        let mut covered = vec![false; (w * h) as usize];
        for y in 0..h as usize {
            for x in 0..w as usize {
                let s = y * row as usize + x * 4;
                let o = y * w as usize + x;
                rgb[o * 3..o * 3 + 3].copy_from_slice(&data[s..s + 3]);
                covered[o] = data[s + 3] != 0;
            }
        }
        (rgb, covered)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(include_str!("raster.wgsl")).expect("WGSL parses");
        naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::default())
            .validate(&module)
            .expect("WGSL validates");
    }
}
