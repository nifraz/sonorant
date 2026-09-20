//! The phosphor screen: a trace whose light fades with real time rather than with the
//! frame.
//!
//! Nostalgia+ redrew the goniometer from scratch every frame, so the figure flickered
//! and a fast passage read as noise. Here the scope keeps a floating-point accumulator
//! between frames, fades it by how much real time has passed, and adds the new trace on
//! top. What comes out is a CRT: a quick sweep leaves a dim tail, a held note burns in,
//! and the brightness of a pixel says how long the trace dwelt on it.
//!
//! The light is frame-rate independent by construction. The caller hands over the
//! samples covering the time since the last frame rather than a fixed count, so a pixel
//! is crossed as many times as the audio really crossed it; the fade is an exponential
//! in real time. Halve the frame rate and each frame draws twice the trace and fades
//! twice as far, and the picture settles in the same place.

use bytemuck::{Pod, Zeroable};

use crate::colour::Rgba;
use crate::layout::Rect;

/// The accumulator's format: floating point, so light can pile up past white before the
/// composite brings it back down.
const ACCUMULATOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// How much smaller the glow is than the accumulator. Half, not the eighth the window's
/// bloom uses: the scope is a small square, and a halo blurred at an eighth of eighty
/// pixels reaches almost across it, which reads as fog rather than as phosphor.
const GLOW_DIVISOR: u32 = 2;
/// What a segment lays down at full intensity. Tuned by eye against the old
/// single-frame trace: at the default persistence a steady tone reads about as bright as
/// it did, and the tail is what's new.
const DEPOSIT: f32 = 0.055;
/// The fraction of the original light a trace has left once its persistence has passed.
/// A hundredth is about where a tail stops being visible, which is what the setting
/// should mean to someone reading it.
const FADED_TO: f64 = 0.01;

/// The uniform slots, one per pass, each at its own dynamic offset.
const SLOTS: u64 = 6;
const DECAY: u64 = 0;
const TRACE: u64 = 1;
const DOWN: u64 = 2;
const BLUR_ACROSS: u64 = 3;
const BLUR_DOWN: u64 = 4;
const COMPOSITE: u64 = 5;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct ScopeUniform {
    rect: [f32; 4],
    colour: [f32; 4],
    size: [f32; 2],
    texel: [f32; 2],
    screen: [f32; 2],
    half_width: f32,
    deposit: f32,
    glow: f32,
    alpha: f32,
    vertical: u32,
    _pad: u32,
}

/// How a phosphor trace looks and how long it lingers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PhosphorLook {
    /// How long the light takes to fade to a hundredth, in seconds.
    pub persistence: f64,
    /// How hard the trace is written, 0 to 1 and a little beyond.
    pub intensity: f64,
    /// The solid width of the trace in framebuffer pixels, before its edge is feathered.
    pub width: f32,
    /// How much of the blurred copy is added back over the trace.
    pub glow: f64,
    pub colour: Rgba,
}

impl Default for PhosphorLook {
    fn default() -> PhosphorLook {
        PhosphorLook {
            persistence: 0.5,
            intensity: 1.0,
            width: 1.1,
            glow: 0.8,
            colour: Rgba::argb(255, 120, 255, 190),
        }
    }
}

/// One frame's trace: where the scope goes, how long the frame was, and the points.
///
/// `points` are in the scope's own pixels, the origin at its top left, and should be
/// the samples covering `dt` rather than a fixed number of them. That is what keeps the
/// picture the same at any frame rate: see [`samples_for`].
#[derive(Clone, Copy, Debug)]
pub struct Sweep<'a> {
    pub rect: Rect,
    /// Real seconds since the last sweep.
    pub dt: f64,
    pub look: &'a PhosphorLook,
    pub points: &'a [[f32; 2]],
}

/// One phosphor screen: an accumulator over one rectangle, its glow, and the passes that
/// fade, draw and composite it.
#[derive(Debug)]
pub struct Phosphor {
    decay: wgpu::RenderPipeline,
    trace: wgpu::RenderPipeline,
    down: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    composite: wgpu::RenderPipeline,
    read_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    stride: u64,
    draw_bind: wgpu::BindGroup,
    /// The accumulator, its glow pair, and the bind groups that read them. `None` until
    /// the first frame gives the scope a size.
    screen: Option<Screen>,
    segments: wgpu::Buffer,
    /// Segments the buffer has room for.
    capacity: usize,
    /// Segments in this frame's trace.
    drawn: u32,
    /// Where the last sweep put the scope, and how much glow it asked for: the
    /// composite reads them back rather than being told twice.
    placed: Rect,
    glow_strength: f64,
    /// Set while the accumulator holds nothing yet, so the next pass clears rather than
    /// loads: a fresh texture's contents are undefined.
    blank: bool,
}

#[derive(Debug)]
struct Screen {
    size: (u32, u32),
    accumulator: wgpu::TextureView,
    glow: [wgpu::TextureView; 2],
    /// Down, blur across, blur down, composite.
    binds: [wgpu::BindGroup; 4],
}

impl Phosphor {
    /// A phosphor screen compositing into `output`, the furniture target's format.
    pub fn new(device: &wgpu::Device, output: wgpu::TextureFormat) -> Phosphor {
        let stride = (size_of::<ScopeUniform>() as u64)
            .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("phosphor"),
            size: stride * SLOTS,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("phosphor"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: true,
                min_binding_size: wgpu::BufferSize::new(size_of::<ScopeUniform>() as u64),
            },
            count: None,
        };
        // The fade and the trace write the accumulator, so they must not also bind it.
        let draw_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("phosphor draw"),
            entries: &[uniform_entry],
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
        let read_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("phosphor read"),
            entries: &[
                uniform_entry,
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
            label: Some("phosphor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("phosphor.wgsl").into()),
        });
        let make = |label: &str, bind: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(bind)],
                immediate_size: 0,
            })
        };
        let draw_pipes = make("phosphor draw", &draw_layout);
        let read_pipes = make("phosphor read", &read_layout);

        // Light is only ever added: the trace and the composite both sum into what is
        // already there, and the fade is the one pass that takes any away.
        let add = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        // The fade: nothing of the source, the destination scaled by the blend constant.
        // Scaling a texture in place, without a second copy to ping-pong through.
        let scale_by_constant = wgpu::BlendComponent {
            src_factor: wgpu::BlendFactor::Zero,
            dst_factor: wgpu::BlendFactor::Constant,
            operation: wgpu::BlendOperation::Add,
        };
        let fade = Some(wgpu::BlendState {
            color: scale_by_constant,
            alpha: scale_by_constant,
        });

        struct Pipe<'a> {
            label: &'a str,
            layout: &'a wgpu::PipelineLayout,
            vs: &'a str,
            fs: &'a str,
            format: wgpu::TextureFormat,
            blend: Option<wgpu::BlendState>,
            buffers: &'a [Option<wgpu::VertexBufferLayout<'a>>],
            strip: bool,
        }
        let pipeline = |p: Pipe<'_>| {
            let Pipe {
                label,
                layout,
                vs,
                fs,
                format,
                blend,
                buffers,
                strip,
            } = p;
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    compilation_options: Default::default(),
                    buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: if strip {
                        wgpu::PrimitiveTopology::TriangleStrip
                    } else {
                        wgpu::PrimitiveTopology::TriangleList
                    },
                    ..Default::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        // One instance per segment: its two ends, in accumulator pixels.
        let segment_layout = Some(wgpu::VertexBufferLayout {
            array_stride: size_of::<[f32; 4]>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 0,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: 8,
                    shader_location: 1,
                },
            ],
        });
        let draw_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("phosphor draw"),
            layout: &draw_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: &uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<ScopeUniform>() as u64),
                }),
            }],
        });
        Phosphor {
            decay: pipeline(Pipe {
                label: "phosphor decay",
                layout: &draw_pipes,
                vs: "vs_full",
                fs: "fs_decay",
                format: ACCUMULATOR_FORMAT,
                blend: fade,
                buffers: &[],
                strip: false,
            }),
            trace: pipeline(Pipe {
                label: "phosphor trace",
                layout: &draw_pipes,
                vs: "vs_trace",
                fs: "fs_trace",
                format: ACCUMULATOR_FORMAT,
                blend: add,
                buffers: std::slice::from_ref(&segment_layout),
                strip: true,
            }),
            down: pipeline(Pipe {
                label: "phosphor down",
                layout: &read_pipes,
                vs: "vs_full",
                fs: "fs_down",
                format: ACCUMULATOR_FORMAT,
                blend: None,
                buffers: &[],
                strip: false,
            }),
            blur: pipeline(Pipe {
                label: "phosphor blur",
                layout: &read_pipes,
                vs: "vs_full",
                fs: "fs_blur",
                format: ACCUMULATOR_FORMAT,
                blend: None,
                buffers: &[],
                strip: false,
            }),
            composite: pipeline(Pipe {
                label: "phosphor composite",
                layout: &read_pipes,
                vs: "vs_composite",
                fs: "fs_composite",
                format: output,
                blend: add,
                buffers: &[],
                strip: true,
            }),
            read_layout,
            sampler,
            uniforms,
            stride,
            draw_bind,
            screen: None,
            segments: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("phosphor segments"),
                size: size_of::<[f32; 4]>() as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            capacity: 1,
            drawn: 0,
            placed: Rect::new(0, 0, 0, 0),
            glow_strength: 0.0,
            blank: true,
        }
    }

    /// Throws away what the screen is holding, so the next frame starts from black.
    /// For a track change, or for the scope being switched off and on again.
    pub fn clear(&mut self) {
        self.blank = true;
        self.drawn = 0;
    }

    /// Fades the screen by the sweep's real time and lays its points over it.
    pub fn accumulate(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        sweep: &Sweep<'_>,
    ) {
        let Sweep {
            rect,
            dt,
            look,
            points,
        } = *sweep;
        self.placed = rect;
        self.glow_strength = look.glow;
        let size = (rect.w.max(0) as u32, rect.h.max(0) as u32);
        if size.0 == 0 || size.1 == 0 {
            self.screen = None;
            self.drawn = 0;
            return;
        }
        if self.screen.as_ref().is_none_or(|s| s.size != size) {
            self.screen = Some(self.build(device, size));
            self.blank = true;
        }
        let Some(screen) = &self.screen else {
            return;
        };

        // Rebuilding the segments every frame: a thousand of them is 16 KB, which is
        // cheaper to write than to work out what changed.
        let segments: Vec<[f32; 4]> = points
            .windows(2)
            .map(|w| [w[0][0], w[0][1], w[1][0], w[1][1]])
            .collect();
        self.drawn = segments.len() as u32;
        if segments.len() > self.capacity {
            self.capacity = segments.len().next_power_of_two();
            self.segments = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("phosphor segments"),
                size: (self.capacity * size_of::<[f32; 4]>()) as u64,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
        }
        if !segments.is_empty() {
            queue.write_buffer(&self.segments, 0, bytemuck::cast_slice(&segments));
        }

        let base = ScopeUniform {
            size: [size.0 as f32, size.1 as f32],
            ..Default::default()
        };
        let glow_size = glow_size(size);
        let glow_texel = [1.0 / glow_size.0 as f32, 1.0 / glow_size.1 as f32];
        let colour = look.colour.to_linear();
        self.write(
            TRACE,
            queue,
            &ScopeUniform {
                colour,
                half_width: look.width.max(0.1) * 0.5,
                deposit: DEPOSIT
                    * look.intensity.clamp(0.0, 4.0) as f32
                    * deposit_over(dt, look.persistence) as f32,
                ..base
            },
        );
        self.write(
            DOWN,
            queue,
            &ScopeUniform {
                texel: [1.0 / size.0 as f32, 1.0 / size.1 as f32],
                ..base
            },
        );
        for (slot, vertical) in [(BLUR_ACROSS, 0), (BLUR_DOWN, 1)] {
            self.write(
                slot,
                queue,
                &ScopeUniform {
                    texel: glow_texel,
                    vertical,
                    ..base
                },
            );
        }

        // The fade and the new trace in one pass over the accumulator.
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("phosphor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &screen.accumulator,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if self.blank {
                            wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if !self.blank {
                let decay = decay_over(dt, look.persistence) as f32;
                pass.set_pipeline(&self.decay);
                pass.set_blend_constant(wgpu::Color {
                    r: decay.into(),
                    g: decay.into(),
                    b: decay.into(),
                    a: decay.into(),
                });
                pass.set_bind_group(0, &self.draw_bind, &[(DECAY * self.stride) as u32]);
                pass.draw(0..3, 0..1);
            }
            if self.drawn > 0 {
                pass.set_pipeline(&self.trace);
                pass.set_bind_group(0, &self.draw_bind, &[(TRACE * self.stride) as u32]);
                pass.set_vertex_buffer(0, self.segments.slice(..));
                pass.draw(0..4, 0..self.drawn);
            }
        }
        self.blank = false;

        // The glow: the accumulator shrunk into the first glow texture, then blurred
        // across into the second and back down into the first.
        for (i, (pipeline, slot, target)) in [
            (&self.down, DOWN, 0),
            (&self.blur, BLUR_ACROSS, 1),
            (&self.blur, BLUR_DOWN, 0),
        ]
        .into_iter()
        .enumerate()
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("phosphor glow"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &screen.glow[target],
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &screen.binds[i], &[(slot * self.stride) as u32]);
            pass.draw(0..3, 0..1);
        }
    }

    /// Draws the screen and its glow into the pass, at the rectangle
    /// [`accumulate`](Self::accumulate) was given, faded to `alpha`.
    pub fn draw(
        &self,
        queue: &wgpu::Queue,
        pass: &mut wgpu::RenderPass<'_>,
        screen_size: (u32, u32),
        alpha: f64,
    ) {
        let Some(screen) = &self.screen else {
            return;
        };
        if alpha <= 0.0 {
            return;
        }
        let r = self.placed;
        self.write(
            COMPOSITE,
            queue,
            &ScopeUniform {
                rect: [r.x as f32, r.y as f32, r.w as f32, r.h as f32],
                screen: [screen_size.0 as f32, screen_size.1 as f32],
                glow: self.glow_strength.clamp(0.0, 4.0) as f32,
                alpha: alpha.clamp(0.0, 1.0) as f32,
                ..Default::default()
            },
        );
        pass.set_pipeline(&self.composite);
        pass.set_bind_group(0, &screen.binds[3], &[(COMPOSITE * self.stride) as u32]);
        pass.draw(0..4, 0..1);
    }

    fn write(&self, slot: u64, queue: &wgpu::Queue, u: &ScopeUniform) {
        queue.write_buffer(&self.uniforms, slot * self.stride, bytemuck::bytes_of(u));
    }

    fn build(&self, device: &wgpu::Device, size: (u32, u32)) -> Screen {
        let make = |label, w: u32, h: u32| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: w.max(1),
                        height: h.max(1),
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: ACCUMULATOR_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let (gw, gh) = glow_size(size);
        let accumulator = make("phosphor accumulator", size.0, size.1);
        let glow = [
            make("phosphor glow a", gw, gh),
            make("phosphor glow b", gw, gh),
        ];
        let bind = |accum: &wgpu::TextureView, halo: &wgpu::TextureView| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("phosphor read"),
                layout: &self.read_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                            buffer: &self.uniforms,
                            offset: 0,
                            size: wgpu::BufferSize::new(size_of::<ScopeUniform>() as u64),
                        }),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(accum),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: wgpu::BindingResource::TextureView(halo),
                    },
                    wgpu::BindGroupEntry {
                        binding: 3,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            })
        };
        // Down reads the accumulator, each blur the other glow texture, and the
        // composite the accumulator and the finished glow.
        let binds = [
            bind(&accumulator, &glow[1]),
            bind(&accumulator, &glow[0]),
            bind(&accumulator, &glow[1]),
            bind(&accumulator, &glow[0]),
        ];
        Screen {
            size,
            accumulator,
            glow,
            binds,
        }
    }
}

fn glow_size(size: (u32, u32)) -> (u32, u32) {
    (
        (size.0 / GLOW_DIVISOR).max(4),
        (size.1 / GLOW_DIVISOR).max(4),
    )
}

/// What last frame's light is multiplied by after `dt` seconds, given a persistence of
/// `seconds`. Exponential, so the answer doesn't depend on how the time was cut up: two
/// half-steps fade exactly as far as one whole one.
pub fn decay_over(dt: f64, seconds: f64) -> f64 {
    if !dt.is_finite() || dt <= 0.0 {
        return 1.0;
    }
    if seconds <= 0.0 {
        return 0.0;
    }
    (FADED_TO.ln() * dt / seconds).exp()
}

/// What one frame's deposit is scaled by, so the settled brightness is the same at any
/// frame rate.
///
/// A frame lays its whole trace down at once and then the next fades it, where a real
/// phosphor is written continuously while it fades. The lump arrives early, so it
/// settles brighter, and the longer the frame the worse it is: at a half-second
/// persistence, 30 frames a second settles about 12% above 120. This is the ratio
/// between the two, `(1 - decay) / (decay rate times dt)`, which is exactly the light a
/// continuous deposit would have left. With no persistence the trace is wiped every
/// frame anyway, so there is nothing to correct.
pub fn deposit_over(dt: f64, seconds: f64) -> f64 {
    let decay = decay_over(dt, seconds);
    if !(0.0..1.0).contains(&decay) {
        return 1.0;
    }
    (1.0 - decay) / -decay.ln()
}

/// How many of the newest samples cover `dt` seconds at `rate`, held to what there is.
///
/// Asking for the samples that really passed, rather than a fixed number, is what makes
/// the picture the same at 30 and 60 frames a second: the trace covers the audio once,
/// no more and no less.
pub fn samples_for(dt: f64, rate: f64, available: usize) -> usize {
    if !dt.is_finite() || dt <= 0.0 || rate <= 0.0 {
        return 0;
    }
    ((dt * rate).ceil() as usize + 1).min(available)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fade_does_not_depend_on_how_the_time_is_cut_up() {
        let whole = decay_over(1.0 / 30.0, 0.5);
        let halves = decay_over(1.0 / 60.0, 0.5) * decay_over(1.0 / 60.0, 0.5);
        assert!((whole - halves).abs() < 1e-12, "{whole} against {halves}");
    }

    #[test]
    fn a_persistence_is_the_time_to_fade_to_a_hundredth() {
        assert!((decay_over(0.5, 0.5) - 0.01).abs() < 1e-12);
        assert!((decay_over(2.0, 2.0) - 0.01).abs() < 1e-12);
    }

    #[test]
    fn no_persistence_leaves_nothing_and_no_time_takes_nothing() {
        assert_eq!(decay_over(1.0 / 60.0, 0.0), 0.0);
        assert_eq!(decay_over(0.0, 0.5), 1.0);
        assert_eq!(decay_over(-1.0, 0.5), 1.0);
        assert_eq!(decay_over(f64::NAN, 0.5), 1.0);
    }

    #[test]
    fn the_trace_covers_the_frame_and_no_more() {
        // 48 kHz, a frame at 60 and one at 30: twice the frame, twice the samples.
        assert_eq!(samples_for(1.0 / 60.0, 48_000.0, 4096), 801);
        assert_eq!(samples_for(1.0 / 30.0, 48_000.0, 4096), 1601);
        // Never more than the ring holds, however long the frame took.
        assert_eq!(samples_for(1.0, 48_000.0, 4096), 4096);
        assert_eq!(samples_for(0.0, 48_000.0, 4096), 0);
    }

    /// A pixel the trace crosses settles at the same brightness whatever the frame rate:
    /// halve it and each frame deposits twice as much and fades twice as far.
    #[test]
    fn the_settled_brightness_does_not_depend_on_the_frame_rate() {
        let settle = |fps: f64| {
            let dt = 1.0 / fps;
            let decay = decay_over(dt, 0.5);
            // Crossings scale with the frame, as `samples_for` makes them.
            let deposit = f64::from(DEPOSIT) * dt * 400.0 * deposit_over(dt, 0.5);
            let mut light = 0.0;
            for _ in 0..2000 {
                light = light * decay + deposit;
            }
            light
        };
        let (fast, slow) = (settle(120.0), settle(30.0));
        assert!(
            (fast - slow).abs() / fast < 1e-9,
            "{fast} at 120 against {slow} at 30"
        );
    }
}
