//! Bloom (docs/enhancements.md, "Bloom"): the bright parts of the 3D layer
//! blurred through a chain of halvings and added back by the compose pass.
//!
//! It runs on the finished 3D layer (after the multisample resolve, or the
//! temporal accumulation: at output resolution either way), so the 2D
//! layers are not in it: tile backgrounds don't glow, and the HUD, which
//! the compose pass draws over the sum, is never smeared. The chain is
//! 16-bit float ([`AUX_FORMAT`], the temporal path's requirement too).

use crate::temporal::AUX_FORMAT;

/// Levels of the chain (each half the size of the one before), in play; a
/// photo larger than play adds some (`encode`'s `extra`).
const LEVELS: usize = 5;
/// Brightness (largest channel, 0 to 1) where glow starts, and the width of
/// the soft knee around it.
const THRESHOLD: f32 = 0.85;
const KNEE: f32 = 0.15;
/// Weight of each level's blur added to the next larger one.
const UP: f32 = 0.5;

pub(crate) struct Bloom {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// First (threshold and halve), down, up (added to its target).
    pipelines: [wgpu::RenderPipeline; 3],
    /// The chain for the size it was made for.
    levels: Vec<wgpu::TextureView>,
    size: (u32, u32),
}

impl Bloom {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bloom.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[crate::uniform(0, wgpu::ShaderStages::FRAGMENT), crate::filterable(1), crate::sampler(2)],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let additive = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::One,
            dst_factor: wgpu::BlendFactor::One,
            operation: wgpu::BlendOperation::Add,
        };
        let pipeline = |entry: &str, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("bloom"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: AUX_FORMAT,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipelines = [
            pipeline("fs_first", None),
            pipeline("fs_down", None),
            pipeline("fs_up", Some(wgpu::BlendState { color: additive, alpha: additive })),
        ];
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Bloom { layout, sampler, pipelines, levels: Vec::new(), size: (0, 0) }
    }

    /// The bloom of `src` (a `w` x `h` filterable image, the 3D layer): the
    /// chain's largest level, half size, in `AUX_FORMAT`, to be sampled
    /// bilinearly over the frame.
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        src: &wgpu::TextureView,
        w: u32,
        h: u32,
        extra: u32,
    ) -> wgpu::TextureView {
        let levels = LEVELS + extra.min(3) as usize;
        if self.size != (w, h) || self.levels.len() != levels {
            self.size = (w, h);
            self.levels = (1..=levels as u32)
                .map(|i| {
                    device
                        .create_texture(&wgpu::TextureDescriptor {
                            label: Some("bloom level"),
                            size: wgpu::Extent3d {
                                width: (w >> i).max(1),
                                height: (h >> i).max(1),
                                depth_or_array_layers: 1,
                            },
                            mip_level_count: 1,
                            sample_count: 1,
                            dimension: wgpu::TextureDimension::D2,
                            format: AUX_FORMAT,
                            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                            view_formats: &[],
                        })
                        .create_view(&Default::default())
                })
                .collect();
        }
        let mut pass =
            |pipeline: usize, from: &wgpu::TextureView, to: &wgpu::TextureView, dst: (u32, u32), load: bool| {
                let params: Vec<u8> = [THRESHOLD, KNEE, UP, 0.0, dst.0 as f32, dst.1 as f32, 0.0, 0.0]
                    .iter()
                    .flat_map(|f| f.to_le_bytes())
                    .collect();
                let params = wgpu::util::DeviceExt::create_buffer_init(
                    device,
                    &wgpu::util::BufferInitDescriptor {
                        label: Some("bloom params"),
                        contents: &params,
                        usage: wgpu::BufferUsages::UNIFORM,
                    },
                );
                let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("bloom"),
                    layout: &self.layout,
                    entries: &[
                        wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(from) },
                        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
                    ],
                });
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("bloom"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: to,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if load { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::BLACK) },
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                rp.set_pipeline(&self.pipelines[pipeline]);
                rp.set_bind_group(0, &bind, &[]);
                rp.draw(0..3, 0..1);
            };
        let dims = |i: usize| ((w >> (i + 1)).max(1), (h >> (i + 1)).max(1));
        pass(0, src, &self.levels[0], dims(0), false);
        for i in 1..levels {
            pass(1, &self.levels[i - 1], &self.levels[i], dims(i), false);
        }
        for i in (0..levels - 1).rev() {
            pass(2, &self.levels[i + 1], &self.levels[i], dims(i), true);
        }
        self.levels[0].clone()
    }
}
