//! SPIKE: a pass over the finished picture whose fragment shader is a WGSL
//! file read at run time (a "shader pack" entry). Fixed interface, declared
//! by the author's file (see `shader-pack-spike` in the spike report):
//!
//! ```wgsl
//! struct Host { size: vec2f, game: vec2f, frame: f32, _a: f32, _b: f32, _c: f32, params: array<vec4f, 2> }
//! @group(0) @binding(0) var<uniform> host: Host;
//! @group(0) @binding(1) var picture: texture_2d<f32>;
//! @group(0) @binding(2) var smp: sampler;
//! @fragment fn fs_main(@builtin(position) frag: vec4f) -> @location(0) vec4f { ... }
//! ```
//!
//! Parameters are `// @param NAME DEFAULT MIN MAX` comment lines.

const VERTEX: &str = "@vertex fn vs(@builtin(vertex_index) i: u32) -> @builtin(position) vec4f {
    let x = f32((i << 1u) & 2u);
    let y = f32(i & 2u);
    return vec4f(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
}";

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
}

/// The parameters a shader file declares, without compiling it.
pub fn parse_params(src: &str) -> Vec<Param> {
    let mut out = Vec::new();
    for l in src.lines() {
        let Some(rest) = l.trim().strip_prefix("// @param ") else { continue };
        let f: Vec<&str> = rest.split_whitespace().collect();
        if let [n, d, lo, hi] = f[..] {
            if let (Ok(d), Ok(lo), Ok(hi)) = (d.parse(), lo.parse(), hi.parse()) {
                out.push(Param { name: n.into(), default: d, min: lo, max: hi });
            }
        }
    }
    out
}

pub struct UserPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
}

impl UserPass {
    /// Compile `source`; every error (parse, validation, entry point,
    /// binding mismatch) comes back as text, never a panic.
    pub fn new(device: &wgpu::Device, source: &str, format: wgpu::TextureFormat) -> Result<UserPass, String> {
        let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
        let memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("user pass"),
            entries: &[crate::uniform(0, wgpu::ShaderStages::FRAGMENT), crate::filterable(1), crate::sampler(2)],
        });
        let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("user pass"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let vs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("user pass vertex"),
            source: wgpu::ShaderSource::Wgsl(VERTEX.into()),
        });
        let fs = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("user pass"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("user pass"),
            layout: Some(&pl),
            vertex: wgpu::VertexState {
                module: &vs,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &fs,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let info = pollster::block_on(fs.get_compilation_info());
        let oom = pollster::block_on(memory.pop());
        let err = pollster::block_on(validation.pop());
        if oom.is_some() || err.is_some() {
            let mut t = String::new();
            for m in &info.messages {
                let at = m.location.map_or(String::new(), |l| format!(" (line {}, col {})", l.line_number, l.line_position));
                t += &format!("[{:?}]{at} {}\n", m.message_type, m.message);
            }
            if let Some(e) = oom {
                t += &format!("out of memory: {e}\n");
            }
            if let Some(e) = err {
                t += &format!("wgpu: {e}\n");
            }
            return Err(t);
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("user pass"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("user pass"),
            size: 64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Ok(UserPass { pipeline, layout, sampler, uniforms })
    }

    /// Draw the pass: `picture` (`size` pixels) into `target`.
    pub fn encode(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        picture: &wgpu::TextureView,
        target: &wgpu::TextureView,
        size: (u32, u32),
        game: (f32, f32),
        frame: u32,
        params: [f32; 8],
    ) {
        let mut v = vec![size.0 as f32, size.1 as f32, game.0, game.1, frame as f32, 0.0, 0.0, 0.0];
        v.extend_from_slice(&params);
        let bytes: Vec<u8> = v.iter().flat_map(|f| f.to_le_bytes()).collect();
        queue.write_buffer(&self.uniforms, 0, &bytes);
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("user pass"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: self.uniforms.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(picture) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.sampler) },
            ],
        });
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("user pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store },
            })],
            ..Default::default()
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}
