//! SPIKE: screen-space ambient occlusion as GPU passes over the 3D layer
//! (`ao.wgsl`). The G-buffer is the look's ID pass (polygon index in the
//! ID target's z, exact 1/z), made for every polygon the colour pass draws
//! (blob shadows and checkerboards too, as ineligible entries), and a
//! per-polygon table of normals and window centres/focal lengths.

use crate::look::{ID_FORMAT, IZ_FORMAT};
use crate::{Renderer, FORMAT};
use model2::raster::{Placed, Poly, Sources};
use model2::video::View;
use std::sync::atomic::AtomicBool;

/// The ID pass draws every polygon, backgrounds included.
pub static GBUF_ALL: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Debug)]
pub struct AoSpike {
    /// Per placed polygon: (nx, ny, nz, takes), (cx, cy, fx, fy).
    pub table: Vec<[f32; 8]>,
    pub radius: f32,
    pub strength: f32,
    /// How often the two AO passes run (timing): 1 normally.
    pub reps: u32,
    /// Draw the G-buffer only (timing), no AO.
    pub gbuf_only: bool,
    /// 0 apply, 1 the term as grey, 2 the eligible pixels magenta.
    pub mode: f32,
}

pub struct AoPass {
    raw: wgpu::RenderPipeline,
    apply: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
}

fn tex(binding: u32, uint: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: if uint { wgpu::TextureSampleType::Uint } else { wgpu::TextureSampleType::Float { filterable: false } },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

impl AoPass {
    pub fn new(device: &wgpu::Device) -> Result<AoPass, String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("ao"),
            entries: &[
                crate::uniform(0, wgpu::ShaderStages::FRAGMENT),
                tex(1, true),
                tex(2, false),
                crate::storage(3),
                tex(4, false),
                tex(5, false),
            ],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ao"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let m = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("ao"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ao.wgsl").into()),
        });
        let mk = |fs: &str, format| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ao"),
                layout: Some(&pl),
                vertex: wgpu::VertexState { module: &m, entry_point: Some("vs"), compilation_options: Default::default(), buffers: &[] },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &m,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let raw = mk("fs_raw", wgpu::TextureFormat::R16Float);
        let apply = mk("fs_apply", FORMAT);
        match pollster::block_on(validation.pop()) {
            Some(e) => Err(format!("ao pass: {e}")),
            None => Ok(AoPass { raw, apply, layout }),
        }
    }
}

impl Renderer {
    /// SPIKE: from now on the 3D layer gets the occlusion pass (None: off).
    pub fn set_ao_spike(&mut self, s: Option<AoSpike>) -> Result<(), String> {
        GBUF_ALL.store(s.is_some(), std::sync::atomic::Ordering::Relaxed);
        if s.is_some() && self.ao_pass.is_none() {
            self.ao_pass = Some(AoPass::new(&self.device)?);
        }
        self.ao_spike = s;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn encode_ao_spike(
        &mut self,
        enc: &mut wgpu::CommandEncoder,
        polys: &[Poly],
        placed: &[Placed],
        src: &Sources,
        view: &View,
        versions: Option<(u64, u64)>,
        layer: wgpu::TextureView,
    ) -> wgpu::TextureView {
        let Some(spike) = self.ao_spike.clone() else { return layer };
        if placed.is_empty() {
            return layer;
        }
        let (w, h) = (view.width as u32, view.height as u32);
        let usage = Self::ID_USAGE;
        let ids = self.kept_target("ao ids", (w, h), ID_FORMAT, usage).0;
        let iz = self.kept_target("ao iz", (w, h), IZ_FORMAT, usage).0;
        let (idv, izv) = (ids.create_view(&Default::default()), iz.create_view(&Default::default()));
        self.share_ids = false;
        self.id_share = None;
        self.versions = versions;
        self.encode_3d(enc, polys, placed, src, view, &idv, None, Some(&izv));
        if spike.gbuf_only {
            return layer;
        }
        let rawt = self.kept_target("ao raw", (w, h), wgpu::TextureFormat::R16Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).0;
        let rawv = rawt.create_view(&Default::default());
        let out = self.kept_texture("3d ao", w, h, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
        let outv = out.create_view(&Default::default());
        let mut tb: Vec<u8> = spike.table.iter().flatten().flat_map(|f| f.to_le_bytes()).collect();
        if tb.is_empty() {
            tb.resize(32, 0);
        }
        let d = &self.device;
        let table = wgpu::util::DeviceExt::create_buffer_init(
            d,
            &wgpu::util::BufferInitDescriptor { label: Some("ao table"), contents: &tb, usage: wgpu::BufferUsages::STORAGE },
        );
        let params: Vec<u8> = [w as f32, h as f32, view.sx, view.sy, view.pad, spike.radius, spike.strength, spike.mode]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let params = wgpu::util::DeviceExt::create_buffer_init(
            d,
            &wgpu::util::BufferInitDescriptor { label: Some("ao params"), contents: &params, usage: wgpu::BufferUsages::UNIFORM },
        );
        let pass = self.ao_pass.as_ref().expect("ao pass");
        let bind = |picture: &wgpu::TextureView, raw: &wgpu::TextureView| {
            d.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("ao"),
                layout: &pass.layout,
                entries: &[
                    wgpu::BindGroupEntry { binding: 0, resource: params.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&idv) },
                    wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&izv) },
                    wgpu::BindGroupEntry { binding: 3, resource: table.as_entire_binding() },
                    wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(picture) },
                    wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(raw) },
                ],
            })
        };
        // The raw pass reads the picture and a stand-in for the raw term.
        let bind_raw = bind(&layer, &layer);
        let bind_apply = bind(&layer, &rawv);
        for _ in 0..spike.reps.max(1) {
            for (pipeline, bind, target, format_clear) in
                [(&pass.raw, &bind_raw, &rawv, 1.0), (&pass.apply, &bind_apply, &outv, 0.0)]
            {
                let mut rp = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ao"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: target,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: format_clear, g: 0.0, b: 0.0, a: 1.0 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    ..Default::default()
                });
                rp.set_pipeline(pipeline);
                rp.set_bind_group(0, bind, &[]);
                rp.draw(0..3, 0..1);
            }
        }
        outv
    }
}
