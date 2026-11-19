//! The CRT look and the picture menu's before/after wipe, both drawn over a
//! finished frame (docs/enhancements.md, "CRT").
//!
//! The CRT pass takes the composed frame from a kept texture (the compose
//! pass draws into it instead of the target when [`crate::Quality::crt`] is
//! on) and writes it to the target warped, with scanlines, a mask, glow and
//! a vignette (`crt.wgsl`). The wipe copies part of a kept picture over the
//! target (`Renderer::wipe`): the player draws the paused frame once as the
//! arcade original into that picture, once as chosen into the target, and
//! shows the first on one side of a divider.

/// A kept picture: its texture, for a size and format, and its view.
struct Kept {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub(crate) struct Crt {
    /// Draw over what `target` holds instead of clearing it first (a
    /// picture in part of a shared target).
    pub keep: bool,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    shader: wgpu::ShaderModule,
    /// The CRT and copy pipelines, for each format they were made for.
    pipelines: Vec<(wgpu::TextureFormat, [wgpu::RenderPipeline; 2])>,
    /// The compose pass's output for the CRT pass to read, and the picture
    /// the wipe draws from.
    input: Option<Kept>,
    before: Option<Kept>,
}

/// Which pipeline.
const CRT: usize = 0;
const COPY: usize = 1;

impl Crt {
    pub fn new(device: &wgpu::Device) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("crt"),
            source: wgpu::ShaderSource::Wgsl(include_str!("crt.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("crt"),
            entries: &[crate::uniform(0, wgpu::ShaderStages::FRAGMENT), crate::filterable(1), crate::sampler(2)],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("crt"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Crt { keep: false, layout, sampler, shader, pipelines: Vec::new(), input: None, before: None }
    }

    /// Drop the kept pictures (nothing has used them for a while).
    pub fn release(&mut self) {
        (self.input, self.before) = (None, None);
    }

    fn pipelines(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) -> [wgpu::RenderPipeline; 2] {
        if let Some(p) = self.pipelines.iter().find(|p| p.0 == format) {
            return p.1.clone();
        }
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("crt"),
            bind_group_layouts: &[Some(&self.layout)],
            immediate_size: 0,
        });
        let make = |entry: &str| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("crt"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &self.shader,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &self.shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let both = [make("fs"), make("fs_copy")];
        self.pipelines.push((format, both.clone()));
        both
    }

    fn kept(
        slot: &mut Option<Kept>,
        device: &wgpu::Device,
        w: u32,
        h: u32,
        format: wgpu::TextureFormat,
    ) -> wgpu::TextureView {
        let same = |k: &&Kept| k.texture.width() == w && k.texture.height() == h && k.texture.format() == format;
        if let Some(k) = slot.as_ref().filter(same) {
            return k.view.clone();
        }
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("crt picture"),
            size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        *slot = Some(Kept { texture, view: view.clone() });
        view
    }

    /// The picture the compose pass draws into for [`Crt::encode`] to read:
    /// `w` x `h` in `format`.
    pub fn input(&mut self, device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
        Self::kept(&mut self.input, device, w, h, format)
    }

    /// The picture a wipe draws from, for the caller to draw into: `w` x
    /// `h` in `format`.
    pub fn before(&mut self, device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
        Self::kept(&mut self.before, device, w, h, format)
    }

    fn bind(&self, device: &wgpu::Device, params: &[f32; 8], src: &wgpu::TextureView) -> wgpu::BindGroup {
        let bytes: Vec<u8> = params.iter().flat_map(|f| f.to_le_bytes()).collect();
        let buffer = wgpu::util::DeviceExt::create_buffer_init(
            device,
            &wgpu::util::BufferInitDescriptor {
                label: Some("crt params"),
                contents: &bytes,
                usage: wgpu::BufferUsages::UNIFORM,
            },
        );
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crt"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(src) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        })
    }

    /// Draw the input picture into `target` (a `size` picture at `origin`;
    /// the rest of `target` goes black) with the CRT look at `strength` (0
    /// to 1) over the game's `lines`.
    #[allow(clippy::too_many_arguments)]
    pub fn encode(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        origin: (u32, u32),
        size: (u32, u32),
        strength: f32,
        lines: f32,
    ) {
        let Some(input) = self.input.as_ref().map(|k| k.view.clone()) else { return };
        let params = [strength, lines, size.0 as f32, size.1 as f32, origin.0 as f32, origin.1 as f32, 0.0, 0.0];
        let bind = self.bind(device, &params, &input);
        let pipeline = self.pipelines(device, format)[CRT].clone();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("crt"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: if self.keep { wgpu::LoadOp::Load } else { wgpu::LoadOp::Clear(wgpu::Color::BLACK) },
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });
        pass.set_viewport(origin.0 as f32, origin.1 as f32, size.0 as f32, size.1 as f32, 0.0, 1.0);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }

    /// Copy the rectangle `rect` (x, y, width, height in pixels) of the
    /// `before` picture over the same place in `target`.
    pub fn wipe(
        &mut self,
        device: &wgpu::Device,
        enc: &mut wgpu::CommandEncoder,
        target: &wgpu::TextureView,
        format: wgpu::TextureFormat,
        rect: (u32, u32, u32, u32),
    ) {
        let Some((before, (w, h))) =
            self.before.as_ref().map(|k| (k.view.clone(), (k.texture.width(), k.texture.height())))
        else {
            return;
        };
        let (x, y) = (rect.0.min(w), rect.1.min(h));
        let (rw, rh) = (rect.2.min(w - x), rect.3.min(h - y));
        if rw == 0 || rh == 0 {
            return;
        }
        let bind = self.bind(device, &[0.0; 8], &before);
        let pipeline = self.pipelines(device, format)[COPY].clone();
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("wipe"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_scissor_rect(x, y, rw, rh);
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

#[cfg(test)]
#[path = "crt_tests.rs"]
mod tests;
