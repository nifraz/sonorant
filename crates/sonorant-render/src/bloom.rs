//! The visuals' floating-point target and its glow.
//!
//! The spectrogram and the backdrop draw into a linear-light target rather than straight
//! onto the screen. The glow is thresholded and blurred at an eighth of the size, as
//! Nostalgia+'s was, and the composite puts both on the swapchain. The furniture is
//! drawn afterwards, on the swapchain itself, because it blends in encoded colour.

use bytemuck::{Pod, Zeroable};

/// The visuals render in linear light with room above white for the glow to work with.
pub const TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// How much smaller the glow chain is than the picture, at each visual quality.
///
/// Nostalgia+ built its glow at an eighth, which is Medium and what this has always
/// drawn. A sixteenth is a quarter of the pixels to threshold and blur, and spreads the
/// halo wider because the same nine taps cover twice the picture; a sixth is finer and
/// tighter. Both are a real change to the look as well as the cost, which is the honest
/// thing for a quality setting to be.
pub const GLOW_DIVISORS: [u32; 3] = [16, 8, 6];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct BloomUniform {
    texel: [f32; 2],
    vertical: u32,
    strength: f32,
}

#[derive(Debug)]
pub struct Visuals {
    threshold: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    stride: u64,
    target: wgpu::Texture,
    target_view: wgpu::TextureView,
    glow: [wgpu::Texture; 2],
    glow_views: [wgpu::TextureView; 2],
    /// Bind groups for the three passes: threshold, the two blurs, and the composite.
    binds: Vec<wgpu::BindGroup>,
    size: (u32, u32),
    divisor: u32,
}

impl Visuals {
    /// A target and glow chain for `width` by `height` pixels, compositing into
    /// `output` (the swapchain's sRGB view format).
    pub fn new(
        device: &wgpu::Device,
        output: wgpu::TextureFormat,
        width: u32,
        height: u32,
    ) -> Visuals {
        let stride = (size_of::<BloomUniform>() as u64)
            .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("bloom"),
            size: stride * 4,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("bloom"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("bloom"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size_of::<BloomUniform>() as u64),
                    },
                    count: None,
                },
                tex(1),
                tex(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("bloom"),
            source: wgpu::ShaderSource::Wgsl(include_str!("bloom.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("bloom"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
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
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let divisor = GLOW_DIVISORS[1];
        let (target, target_view, glow, glow_views) = textures(device, width, height, divisor);
        let mut v = Visuals {
            threshold: pipeline("fs_threshold", TARGET_FORMAT),
            blur: pipeline("fs_blur", TARGET_FORMAT),
            composite: pipeline("fs_composite", output),
            layout,
            sampler,
            uniforms,
            stride,
            target,
            target_view,
            glow,
            glow_views,
            binds: Vec::new(),
            size: (width.max(1), height.max(1)),
            divisor,
        };
        v.rebind(device);
        v
    }

    /// Where the spectrogram and backdrop draw.
    pub fn view(&self) -> &wgpu::TextureView {
        &self.target_view
    }

    pub fn size(&self) -> (u32, u32) {
        self.size
    }

    /// Resizes the target and its glow to the window.
    pub fn resize(&mut self, device: &wgpu::Device, width: u32, height: u32) {
        let size = (width.max(1), height.max(1));
        if size == self.size {
            return;
        }
        self.size = size;
        let (target, target_view, glow, glow_views) =
            textures(device, size.0, size.1, self.divisor);
        self.target = target;
        self.target_view = target_view;
        self.glow = glow;
        self.glow_views = glow_views;
        self.rebind(device);
    }

    fn rebind(&mut self, device: &wgpu::Device) {
        let bind = |source: &wgpu::TextureView, glow: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("bloom"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(size_of::<BloomUniform>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(glow),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        // Threshold reads the picture, each blur reads the other glow texture, and the
        // composite reads the picture and the finished glow.
        self.binds = vec![
            bind(&self.target_view, &self.glow_views[1]),
            bind(&self.target_view, &self.glow_views[0]),
            bind(&self.target_view, &self.glow_views[1]),
            bind(&self.target_view, &self.glow_views[0]),
        ];
    }

    /// Builds the glow from what's in the target. Skip it when the glow is off.
    pub fn build_glow(
        &self,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        timestamps: Option<wgpu::RenderPassTimestampWrites<'_>>,
    ) {
        let (gw, gh) = self.glow_size();
        let texel = [1.0 / gw as f32, 1.0 / gh as f32];
        for (i, u) in [
            BloomUniform {
                texel: [1.0 / self.size.0 as f32, 1.0 / self.size.1 as f32],
                vertical: 0,
                strength: 0.0,
            },
            BloomUniform {
                texel,
                vertical: 0,
                strength: 0.0,
            },
            BloomUniform {
                texel,
                vertical: 1,
                strength: 0.0,
            },
        ]
        .into_iter()
        .enumerate()
        {
            queue.write_buffer(
                &self.uniforms,
                i as u64 * self.stride,
                bytemuck::bytes_of(&u),
            );
        }
        // Threshold the picture into the first glow texture, then blur across into the
        // second and back down into the first.
        // The three passes are timed as one: the first starts the clock, the last stops it.
        let last = 2;
        for (i, (pipeline, target)) in [(&self.threshold, 0), (&self.blur, 1), (&self.blur, 0)]
            .into_iter()
            .enumerate()
        {
            let writes = timestamps.as_ref().and_then(|w| {
                let (begin, end) = match i {
                    0 => (w.beginning_of_pass_write_index, None),
                    i if i == last => (None, w.end_of_pass_write_index),
                    _ => return None,
                };
                Some(wgpu::RenderPassTimestampWrites {
                    query_set: w.query_set,
                    beginning_of_pass_write_index: begin,
                    end_of_pass_write_index: end,
                })
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("glow"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.glow_views[target],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: writes,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &self.binds[i], &[(i as u64 * self.stride) as u32]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Draws the target, with the glow over it at `strength` (0 for none), into the
    /// pass's attachment.
    pub fn composite(&self, queue: &wgpu::Queue, pass: &mut wgpu::RenderPass<'_>, strength: f32) {
        let u = BloomUniform {
            texel: [1.0 / self.size.0 as f32, 1.0 / self.size.1 as f32],
            vertical: 0,
            strength,
        };
        queue.write_buffer(&self.uniforms, 3 * self.stride, bytemuck::bytes_of(&u));
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &self.binds[3], &[(3 * self.stride) as u32]);
        pass.draw(0..3, 0..1);
    }

    /// How small the glow chain is built. Rebuilds it, so only call it when it changes.
    pub fn set_divisor(&mut self, device: &wgpu::Device, divisor: u32) {
        let divisor = divisor.clamp(2, 64);
        if divisor == self.divisor {
            return;
        }
        self.divisor = divisor;
        let (target, target_view, glow, glow_views) =
            textures(device, self.size.0, self.size.1, divisor);
        self.target = target;
        self.target_view = target_view;
        self.glow = glow;
        self.glow_views = glow_views;
        self.rebind(device);
    }

    fn glow_size(&self) -> (u32, u32) {
        (
            (self.size.0 / self.divisor).max(8),
            (self.size.1 / self.divisor).max(8),
        )
    }
}

type Targets = (
    wgpu::Texture,
    wgpu::TextureView,
    [wgpu::Texture; 2],
    [wgpu::TextureView; 2],
);

fn textures(device: &wgpu::Device, width: u32, height: u32, divisor: u32) -> Targets {
    let make = |label, w: u32, h: u32| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: w.max(1),
                height: h.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        })
    };
    let view = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
    let target = make("visuals", width, height);
    let target_view = view(&target);
    let (gw, gh) = ((width / divisor).max(8), (height / divisor).max(8));
    let glow = [make("glow a", gw, gh), make("glow b", gw, gh)];
    let glow_views = [view(&glow[0]), view(&glow[1])];
    (target, target_view, glow, glow_views)
}
