//! The spectrogram pass: draws panes of the history through a palette.

use bytemuck::{Pod, Zeroable};
use sonorant_core::dsp::FreqScale;
use sonorant_core::engine::{GRID_BINS, GRID_FMAX, GRID_FMIN};
use sonorant_core::palette::Lut;

use crate::history::HistoryStore;

/// Up to this many panes per frame.
const MAX_PANES: usize = 2;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct PaneUniform {
    newest: u32,
    available: u32,
    layer_rows: u32,
    capacity: u32,
    visible_rows: f32,
    frac: f32,
    newest_left: u32,
    channel: u32,
    scale: u32,
    fmin: f32,
    fmax: f32,
    grid_at: f32,
    grid_span: f32,
    grid_bins: u32,
    grid_fmin: f32,
    grid_fmax: f32,
    global_range: u32,
    smooth_time: u32,
    floor_db: f32,
    ceiling_db: f32,
    _pad: [u32; 4],
}

/// How one pane is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaneView {
    /// The spectrogram area, in physical pixels: x, y, width, height.
    pub rect: [f32; 4],
    /// Which pane's levels: 0 or 1.
    pub channel: u32,
    /// True when the newest row is at the left edge.
    pub newest_left: bool,
    /// Rows across the width.
    pub visible_rows: f32,
    /// The audio clock's progress into the row after the newest, 0 to 1.
    pub frac: f32,
    pub scale: FreqScale,
    pub fmin: f32,
    pub fmax: f32,
    /// `Some((floor, ceiling))` colours every row with that range instead of its own.
    pub global_range: Option<(f32, f32)>,
    pub smooth_time: bool,
}

#[derive(Debug)]
pub struct SpectrogramPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    palette: wgpu::Texture,
    palette_view: wgpu::TextureView,
    uniforms: wgpu::Buffer,
    stride: u64,
    bind_group: Option<wgpu::BindGroup>,
    panes: usize,
}

impl SpectrogramPass {
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> SpectrogramPass {
        let stride = (size_of::<PaneUniform>() as u64)
            .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("spectrogram panes"),
            size: stride * MAX_PANES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("spectrogram palette"),
            size: wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let palette_view = palette.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("palette"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let tex = |binding, dim, filterable| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable },
                view_dimension: dim,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("spectrogram"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size_of::<PaneUniform>() as u64),
                    },
                    count: None,
                },
                tex(1, wgpu::TextureViewDimension::D2Array, false),
                tex(2, wgpu::TextureViewDimension::D2, false),
                tex(3, wgpu::TextureViewDimension::D2, true),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("spectrogram"),
            source: wgpu::ShaderSource::Wgsl(include_str!("spectrogram.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("spectrogram"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("spectrogram"),
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
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        SpectrogramPass {
            pipeline,
            layout,
            sampler,
            palette,
            palette_view,
            uniforms,
            stride,
            bind_group: None,
            panes: 0,
        }
    }

    /// Replaces the palette. The whole history recolours on the next draw.
    pub fn set_palette(&self, queue: &wgpu::Queue, lut: &Lut) {
        let texels: Vec<u8> = lut.iter().flat_map(|c| c.to_rgba8()).collect();
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.palette,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &texels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256 * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Points the pass at a history store. Call again after replacing the store.
    pub fn bind(&mut self, device: &wgpu::Device, history: &HistoryStore) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("spectrogram"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                        buffer: &self.uniforms,
                        offset: 0,
                        size: wgpu::BufferSize::new(size_of::<PaneUniform>() as u64),
                    }),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(history.levels_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(history.ranges_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&self.palette_view),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        }));
    }

    /// Writes this frame's pane parameters. Call before the render pass.
    ///
    /// `held` freezes the picture at the moment the history had that many rows: the view
    /// stays put while new rows keep arriving behind it, until they wrap round.
    pub fn prepare(
        &mut self,
        queue: &wgpu::Queue,
        history: &HistoryStore,
        panes: &[PaneView],
        held: Option<u64>,
    ) {
        self.panes = panes.len().min(MAX_PANES);
        let written = history.written();
        let cap = history.capacity();
        let (newest, available) = match held {
            Some(h) if h > 0 && h <= written => {
                let overwritten = (written - h).min(cap);
                ((h - 1) % cap, h.min(cap).saturating_sub(overwritten))
            }
            _ => (history.newest_slot(), history.available()),
        };
        let grid_decades = (GRID_FMAX / GRID_FMIN).ln();
        for (i, p) in panes.iter().take(MAX_PANES).enumerate() {
            // pos = log(f / grid_fmin) / log(grid_fmax / grid_fmin) * bins - 0.5, with
            // f = fmin (fmax/fmin)^v, which is linear in v.
            let (fmin, fmax) = (p.fmin.max(1e-3) as f64, p.fmax.max(1e-3) as f64);
            let bins = GRID_BINS as f64;
            let grid_at = (fmin / GRID_FMIN).ln() / grid_decades * bins - 0.5;
            let grid_span = (fmax / fmin).ln() / grid_decades * bins;
            let (global, floor, ceiling) = match p.global_range {
                Some((f, c)) => (1, f, c),
                None => (0, 0.0, 0.0),
            };
            let u = PaneUniform {
                newest: newest as u32,
                available: available as u32,
                layer_rows: history.layer_rows(),
                capacity: history.capacity() as u32,
                visible_rows: p.visible_rows.max(1.0),
                frac: p.frac.clamp(0.0, 1.0),
                newest_left: p.newest_left as u32,
                channel: p.channel,
                scale: (p.scale != FreqScale::Linear) as u32,
                fmin: p.fmin,
                fmax: p.fmax,
                grid_at: grid_at as f32,
                grid_span: grid_span as f32,
                grid_bins: GRID_BINS as u32,
                grid_fmin: GRID_FMIN as f32,
                grid_fmax: GRID_FMAX as f32,
                global_range: global,
                smooth_time: p.smooth_time as u32,
                floor_db: floor,
                ceiling_db: ceiling,
                _pad: [0; 4],
            };
            queue.write_buffer(
                &self.uniforms,
                i as u64 * self.stride,
                bytemuck::bytes_of(&u),
            );
        }
    }

    /// Draws the prepared panes, each confined to its rectangle.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>, panes: &[PaneView]) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        pass.set_pipeline(&self.pipeline);
        for (i, p) in panes.iter().take(self.panes).enumerate() {
            let [x, y, w, h] = p.rect;
            if w < 1.0 || h < 1.0 {
                continue;
            }
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_bind_group(0, bind_group, &[(i as u64 * self.stride) as u32]);
            pass.draw(0..3, 0..1);
        }
    }
}
