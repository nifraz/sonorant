//! The curve strips: each pane's spectrum drawn sideways beside its spectrogram, with
//! the peak, average, minimum and reference traces over it.
//!
//! One fragment-shader pass per strip does all of it (see `curves.wgsl`); the values go
//! up as a small float texture each frame, five rows per pane.

use bytemuck::{Pod, Zeroable};
use sonorant_core::palette::{self, Lut};
use sonorant_core::settings::{CurveStyle, GraphBackground, Settings};

use crate::colour::{Rgba, pick, pick_keep_alpha};
use crate::layout::Rect;

/// Rows of values per pane: display, peak, average, minimum, reference.
const SERIES: u32 = 5;
const MAX_PANES: usize = 2;

/// One pane's values, lowest frequency first, in dBFS. All the same length: the strip's
/// height in pixels.
#[derive(Clone, Copy, Debug)]
pub struct CurveData<'a> {
    pub display: &'a [f32],
    pub peak: &'a [f32],
    pub average: &'a [f32],
    pub minimum: &'a [f32],
    /// The held reference curve, if one is held.
    pub reference: Option<&'a [f32]>,
}

/// Where one strip goes and the level range it spans.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveView {
    /// The strip, in framebuffer pixels.
    pub rect: Rect,
    /// Level grows rightwards from the left edge when true, leftwards from the right
    /// edge otherwise: always from the pane's outer edge towards the spectrogram.
    pub curve_on_left: bool,
    pub floor_db: f64,
    pub ceiling_db: f64,
}

/// The settings' say on every strip, resolved to colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CurveLook {
    pub style: CurveStyle,
    pub bar_size: i32,
    pub led_segment: i32,
    pub fill: bool,
    pub show_peak: bool,
    pub show_average: bool,
    pub show_minimum: bool,
    pub background: GraphBackground,
    /// The curve's colour, and the darker one its fill fades to.
    pub hi: Rgba,
    pub lo: Rgba,
    pub peak: Rgba,
    pub average: Rgba,
    pub minimum: Rgba,
    pub reference: Rgba,
    /// The level scale's lines and the chessboard's squares, already faded.
    pub grid: Rgba,
    pub chess: Rgba,
    /// Whether the phosphor screen is drawing the display curve. The fill, the
    /// background and the other traces are drawn either way; only the line moves.
    pub phosphor: bool,
}

impl CurveLook {
    /// The look `settings` give strips drawn from `lut`, with the furniture faded to
    /// `alpha` (0 to 1).
    pub fn new(s: &Settings, lut: &Lut, alpha: f64) -> CurveLook {
        let t = &s.theme;
        let hi = pick(t.curve, Rgba::rgb(palette::color_at(lut, 0.85), 255));
        let lo = match t.curve {
            Some(c) => Rgba::from_argb(c).halved(),
            None => Rgba::rgb(palette::color_at(lut, 0.35), 255),
        };
        let grid = pick_keep_alpha(t.grid_major, Rgba::argb(34, 255, 255, 255));
        CurveLook {
            style: s.style,
            bar_size: s.bar_size,
            led_segment: s.led_segment,
            fill: s.solid_fill,
            show_peak: s.show_max,
            show_average: s.show_avg,
            show_minimum: s.show_min,
            background: s.background,
            hi,
            lo,
            peak: pick_keep_alpha(t.peak_trace, Rgba::argb(180, 255, 255, 255)),
            average: pick_keep_alpha(t.average_trace, Rgba::argb(170, 130, 200, 255)),
            minimum: pick_keep_alpha(t.minimum_trace, Rgba::argb(140, 120, 120, 140)),
            reference: pick_keep_alpha(t.snapshot, Rgba::argb(215, 255, 205, 90)),
            grid: grid.faded(alpha),
            chess: Rgba::argb(26, 255, 255, 255).faded(alpha),
            // Only the line style has a line for a phosphor to draw; bars and LED are
            // blocks, and smearing those would just be a dimmer block.
            phosphor: s.curve_phosphor && s.style == CurveStyle::Line,
        }
    }
}

/// The level scale's step: labels roughly 55 px apart along a strip `pixels` long.
pub fn db_step(span_db: f64, pixels: i32) -> f64 {
    const STEPS: [f64; 6] = [3.0, 6.0, 12.0, 20.0, 30.0, 40.0];
    if pixels <= 0 || span_db <= 0.0 {
        return 12.0;
    }
    STEPS
        .into_iter()
        .find(|&step| pixels as f64 * step / span_db >= 55.0)
        .unwrap_or(40.0)
}

/// The display curve as a polyline in the strip's own pixels, the origin at its top
/// left, for the phosphor screen.
///
/// The same map `curves.wgsl` uses: frequency runs up the strip with the lowest value at
/// the bottom, and level grows from the strip's outer edge towards the image.
pub fn phosphor_trace(view: &CurveView, display: &[f32], into: &mut Vec<[f32; 2]>) {
    into.clear();
    let n = display.len();
    if n < 2 || view.rect.w <= 0 || view.rect.h <= 0 {
        return;
    }
    let span = (view.ceiling_db - view.floor_db).max(1.0);
    let w = view.rect.w as f64;
    let (base, dir) = if view.curve_on_left {
        (0.0, 1.0)
    } else {
        (w, -1.0)
    };
    into.extend(display.iter().enumerate().map(|(k, &db)| {
        let t = ((f64::from(db) - view.floor_db) / span).clamp(0.0, 1.0);
        [(base + dir * t * w + 0.5) as f32, (n - 1 - k) as f32 + 0.5]
    }));
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct CurveUniform {
    rect: [f32; 4],
    base_x: f32,
    dir: f32,
    amp: f32,
    n: u32,
    first_row: u32,
    style: u32,
    bar_size: u32,
    led_segment: u32,
    floor_db: f32,
    span_db: f32,
    flags: u32,
    background: u32,
    db_step: f32,
    line_width: f32,
    _pad: [u32; 2],
    fill_left: [f32; 4],
    fill_right: [f32; 4],
    line: [f32; 4],
    peak: [f32; 4],
    average: [f32; 4],
    minimum: [f32; 4],
    reference: [f32; 4],
    grid: [f32; 4],
    chess: [f32; 4],
}

#[derive(Debug)]
pub struct CurvePass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    stride: u64,
    series: wgpu::Texture,
    palette: wgpu::Texture,
    bind_group: wgpu::BindGroup,
    /// The strips prepared for this frame.
    rects: Vec<Rect>,
    scratch: Vec<f32>,
}

impl CurvePass {
    /// A pass drawing into `target`, which should be the swapchain's plain (non-sRGB)
    /// format: see [`crate::colour`].
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> CurvePass {
        let stride = (size_of::<CurveUniform>() as u64)
            .next_multiple_of(device.limits().min_uniform_buffer_offset_alignment as u64);
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("curve strips"),
            size: stride * MAX_PANES as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("curve palette"),
            size: wgpu::Extent3d {
                width: 256,
                height: 1,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            // Plain, not sRGB: the shader works in encoded values.
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let series = series_texture(device, 1024);

        let tex = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: false },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("curve strips"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: true,
                        min_binding_size: wgpu::BufferSize::new(size_of::<CurveUniform>() as u64),
                    },
                    count: None,
                },
                tex(1),
                tex(2),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("curve strips"),
            source: wgpu::ShaderSource::Wgsl(include_str!("curves.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("curve strips"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("curve strips"),
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
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::COLOR,
                })],
            }),
            multiview_mask: None,
            cache: None,
        });
        let bind_group = bind(device, &layout, &uniforms, &series, &palette);
        CurvePass {
            pipeline,
            layout,
            uniforms,
            stride,
            series,
            palette,
            bind_group,
            rects: Vec::new(),
            scratch: Vec::new(),
        }
    }

    /// Replaces the palette the bars are coloured from.
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

    /// Uploads this frame's strips. Call before the render pass.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        look: &CurveLook,
        strips: &[(CurveView, CurveData<'_>)],
    ) {
        self.rects.clear();
        let longest = strips
            .iter()
            .map(|(_, d)| d.display.len())
            .max()
            .unwrap_or(0) as u32;
        if longest > self.series.width() {
            let width = longest
                .next_power_of_two()
                .min(device.limits().max_texture_dimension_2d);
            self.series = series_texture(device, width);
            self.bind_group = bind(
                device,
                &self.layout,
                &self.uniforms,
                &self.series,
                &self.palette,
            );
        }

        for (i, (view, data)) in strips.iter().take(MAX_PANES).enumerate() {
            let n = data.display.len().min(self.series.width() as usize);
            if n < 2 || view.rect.w <= 2 || view.rect.h <= 0 {
                continue;
            }
            let rows = [
                Some(data.display),
                Some(data.peak),
                Some(data.average),
                Some(data.minimum),
                data.reference,
            ];
            self.scratch.clear();
            for row in rows {
                let start = self.scratch.len();
                self.scratch.resize(start + n, -140.0);
                if let Some(values) = row {
                    let m = values.len().min(n);
                    self.scratch[start..start + m].copy_from_slice(&values[..m]);
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.series,
                    mip_level: 0,
                    origin: wgpu::Origin3d {
                        x: 0,
                        y: i as u32 * SERIES,
                        z: 0,
                    },
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&self.scratch),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(n as u32 * 4),
                    rows_per_image: None,
                },
                wgpu::Extent3d {
                    width: n as u32,
                    height: SERIES,
                    depth_or_array_layers: 1,
                },
            );

            let r = view.rect;
            let span = (view.ceiling_db - view.floor_db).max(1.0);
            // Brightest at the base the level grows from, fading towards the image.
            let (base_x, dir, fill_left, fill_right) = if view.curve_on_left {
                (r.x, 1.0, look.hi.with_alpha(190), look.lo.with_alpha(25))
            } else {
                (
                    r.right(),
                    -1.0,
                    look.lo.with_alpha(25),
                    look.hi.with_alpha(190),
                )
            };
            let flags = look.fill as u32
                | (look.show_peak as u32) << 1
                | (look.show_average as u32) << 2
                | (look.show_minimum as u32) << 3
                | (data.reference.is_some() as u32) << 4
                | (look.phosphor as u32) << 5;
            let u = CurveUniform {
                rect: r.to_f32(),
                base_x: base_x as f32,
                dir,
                amp: r.w as f32,
                n: n as u32,
                first_row: i as u32 * SERIES,
                style: match look.style {
                    CurveStyle::Line => 0,
                    CurveStyle::Bars => 1,
                    CurveStyle::Led => 2,
                },
                bar_size: look.bar_size.max(2) as u32,
                led_segment: look.led_segment.max(3) as u32,
                floor_db: view.floor_db as f32,
                span_db: span as f32,
                flags,
                background: match look.background {
                    GraphBackground::Plain => 0,
                    GraphBackground::Lines => 1,
                    GraphBackground::Grid => 2,
                    GraphBackground::Chessboard => 3,
                },
                db_step: db_step(span, r.w) as f32,
                line_width: 1.3,
                _pad: [0; 2],
                fill_left: fill_left.to_array(),
                fill_right: fill_right.to_array(),
                line: look.hi.with_alpha(235).to_array(),
                peak: look.peak.to_array(),
                average: look.average.to_array(),
                minimum: look.minimum.to_array(),
                reference: look.reference.to_array(),
                grid: look.grid.to_array(),
                chess: look.chess.to_array(),
            };
            queue.write_buffer(
                &self.uniforms,
                i as u64 * self.stride,
                bytemuck::bytes_of(&u),
            );
            self.rects.push(r);
        }
    }

    /// Draws the prepared strips.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        for (i, r) in self.rects.iter().enumerate() {
            let [x, y, w, h] = r.to_f32();
            pass.set_viewport(x, y, w, h, 0.0, 1.0);
            pass.set_bind_group(0, &self.bind_group, &[(i as u64 * self.stride) as u32]);
            pass.draw(0..3, 0..1);
        }
    }
}

fn series_texture(device: &wgpu::Device, width: u32) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("curve values"),
        size: wgpu::Extent3d {
            width,
            height: SERIES * MAX_PANES as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

fn bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniforms: &wgpu::Buffer,
    series: &wgpu::Texture,
    palette: &wgpu::Texture,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("curve strips"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                    buffer: uniforms,
                    offset: 0,
                    size: wgpu::BufferSize::new(size_of::<CurveUniform>() as u64),
                }),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(
                    &series.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::TextureView(
                    &palette.create_view(&wgpu::TextureViewDescriptor::default()),
                ),
            },
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The phosphor's polyline has to land on the line the shader would have drawn,
    /// because it replaces it: `curves.wgsl` puts the lowest value at the bottom and
    /// grows level from the strip's outer edge, and so must this.
    #[test]
    fn the_phosphor_trace_runs_where_the_shader_draws_the_line() {
        let view = |curve_on_left| CurveView {
            rect: Rect::new(100, 40, 60, 4),
            curve_on_left,
            floor_db: -100.0,
            ceiling_db: -20.0,
        };
        // Floor, a quarter up, three quarters up, ceiling; lowest frequency first.
        let display = [-100.0, -80.0, -40.0, -20.0];
        let mut trace = Vec::new();

        phosphor_trace(&view(true), &display, &mut trace);
        // Frequency runs up the strip, so the first value is the bottom row.
        let ys: Vec<f32> = trace.iter().map(|p| p[1]).collect();
        assert_eq!(ys, [3.5, 2.5, 1.5, 0.5]);
        // Level grows rightwards from the strip's left edge.
        let xs: Vec<f32> = trace.iter().map(|p| p[0]).collect();
        assert_eq!(xs, [0.5, 15.5, 45.5, 60.5]);

        // On the right, level grows leftwards from the right edge: mirrored, same rows.
        phosphor_trace(&view(false), &display, &mut trace);
        let xs: Vec<f32> = trace.iter().map(|p| p[0]).collect();
        assert_eq!(xs, [60.5, 45.5, 15.5, 0.5]);
        assert_eq!(
            trace.iter().map(|p| p[1]).collect::<Vec<_>>(),
            [3.5, 2.5, 1.5, 0.5]
        );
    }

    /// A strip with no room, or a curve with nothing to join up, draws nothing rather
    /// than a point at the origin.
    #[test]
    fn a_curve_with_nothing_in_it_traces_nothing() {
        let mut trace = vec![[9.0, 9.0]];
        let empty = CurveView {
            rect: Rect::new(0, 0, 0, 0),
            curve_on_left: true,
            floor_db: -100.0,
            ceiling_db: -20.0,
        };
        phosphor_trace(&empty, &[-50.0, -50.0], &mut trace);
        assert!(trace.is_empty());
        let real = CurveView {
            rect: Rect::new(0, 0, 10, 1),
            ..empty
        };
        phosphor_trace(&real, &[-50.0], &mut trace);
        assert!(trace.is_empty(), "one point is not a line");
    }

    #[test]
    fn uniform_matches_the_shader_layout() {
        // Nine vec4s after five words of scalars and padding, as the WGSL struct.
        assert_eq!(size_of::<CurveUniform>(), 224);
    }

    #[test]
    fn level_steps_spread_labels_out() {
        assert_eq!(db_step(90.0, 300), 20.0);
        assert_eq!(db_step(90.0, 1000), 6.0);
        assert_eq!(db_step(90.0, 40), 40.0);
    }
}
