//! The 2D furniture over the visuals: rectangles, lines and text, in layers.
//!
//! Everything here is in framebuffer pixels and sRGB-encoded colour, blended as GDI+
//! blended (see [`crate::colour`]). Each frame the drawing code queues shapes and text
//! into a [`Layer`]; the layers are drawn in order around the curve strips, so a label
//! can sit on a chip that sits on a curve.
//!
//! Text is shaped by cosmic-text and drawn by glyphon from the bundled IBM Plex faces.
//! A label is shaped once and kept while it's in use, so the axis's fifty labels cost
//! nothing after the first frame.

use std::collections::HashMap;

use bytemuck::{Pod, Zeroable};
use glyphon::{
    Attrs, Buffer, Cache, Color, ColorMode, Family, FontSystem, Metrics, Resolution, Shaping,
    SwashCache, TextArea, TextAtlas, TextBounds, TextRenderer, Viewport, Weight, fontdb,
};

use crate::colour::Rgba;

/// Where a shape or label goes in the stack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Layer {
    /// Under the curve strips: the scale lane's ground, the time marks.
    Under = 0,
    /// Over them: gridlines, axis labels and their chips.
    Over = 1,
    /// Over everything else: the hover readout.
    Top = 2,
}

const LAYERS: usize = 3;

/// The bundled faces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Sans,
    SansLight,
    SansMedium,
    Mono,
}

impl Face {
    fn attrs(self) -> Attrs<'static> {
        let (family, weight) = match self {
            Face::Sans => (SANS, Weight::NORMAL),
            Face::SansLight => (SANS, Weight::LIGHT),
            Face::SansMedium => (SANS, Weight::MEDIUM),
            Face::Mono => (MONO, Weight::NORMAL),
        };
        Attrs::new().family(Family::Name(family)).weight(weight)
    }
}

const SANS: &str = "IBM Plex Sans";
const MONO: &str = "IBM Plex Mono";

const FONTS: [&[u8]; 4] = [
    include_bytes!("../fonts/IBMPlexSans-Regular.ttf"),
    include_bytes!("../fonts/IBMPlexSans-Light.ttf"),
    include_bytes!("../fonts/IBMPlexSans-Medium.ttf"),
    include_bytes!("../fonts/IBMPlexMono-Regular.ttf"),
];

/// Line height as a multiple of the font size, close to what GDI+ measured for Segoe UI
/// so layouts ported from Nostalgia+ keep their spacing.
const LINE_HEIGHT: f32 = 1.3;

/// Frames a shaped label is kept after it was last drawn or measured.
const KEEP_FRAMES: u64 = 120;

/// A label's size in pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TextSize {
    pub w: f32,
    pub h: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct Vertex {
    /// Clip space, worked out when the shape is queued.
    clip: [f32; 2],
    /// The same point in pixels, for the line coverage.
    pos: [f32; 2],
    colour: [f32; 4],
    segment: [f32; 4],
    half_width: f32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct TextKey {
    text: String,
    face: Face,
    /// The size in 1/64 px.
    size: u32,
}

struct Shaped {
    buffer: Buffer,
    size: TextSize,
    used: u64,
}

struct Placed {
    key: TextKey,
    x: f32,
    y: f32,
    colour: Rgba,
    clip: Option<[f32; 4]>,
}

pub struct Overlay {
    pipeline: wgpu::RenderPipeline,
    vertex_buffer: wgpu::Buffer,
    vertex_capacity: u64,
    shapes: [Vec<Vertex>; LAYERS],
    ranges: [std::ops::Range<u32>; LAYERS],

    fonts: FontSystem,
    swash: SwashCache,
    atlas: TextAtlas,
    viewport: Viewport,
    renderers: Vec<TextRenderer>,
    shaped: HashMap<TextKey, Shaped>,
    placed: [Vec<Placed>; LAYERS],
    frame: u64,
    size: [u32; 2],
}

impl std::fmt::Debug for Overlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Overlay")
            .field("shaped", &self.shaped.len())
            .field("frame", &self.frame)
            .finish_non_exhaustive()
    }
}

impl Overlay {
    /// An overlay drawing into `target`, the swapchain's plain (non-sRGB) format.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, target: wgpu::TextureFormat) -> Overlay {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("overlay"),
            source: wgpu::ShaderSource::Wgsl(include_str!("overlay.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("overlay"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("overlay"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x2, 1 => Float32x2, 2 => Float32x4, 3 => Float32x4,
                        4 => Float32
                    ],
                })],
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
        let vertex_capacity = 4096;
        let vertex_buffer = vertex_buffer(device, vertex_capacity);

        let mut db = fontdb::Database::new();
        for font in FONTS {
            db.load_font_data(font.to_vec());
        }
        let fonts = FontSystem::new_with_locale_and_db("en-US".into(), db);
        let cache = Cache::new(device);
        // Web mode: colours pass through as sRGB, as the rest of the overlay's do.
        let mut atlas = TextAtlas::with_color_mode(device, queue, &cache, target, ColorMode::Web);
        let viewport = Viewport::new(device, &cache);
        let renderers = (0..LAYERS)
            .map(|_| TextRenderer::new(&mut atlas, device, wgpu::MultisampleState::default(), None))
            .collect();

        Overlay {
            pipeline,
            vertex_buffer,
            vertex_capacity,
            shapes: Default::default(),
            ranges: Default::default(),
            fonts,
            swash: SwashCache::new(),
            atlas,
            viewport,
            renderers,
            shaped: HashMap::new(),
            placed: Default::default(),
            frame: 0,
            size: [1, 1],
        }
    }

    /// Starts a frame `width` by `height` pixels: forgets last frame's shapes and labels.
    pub fn begin(&mut self, width: u32, height: u32) {
        self.frame += 1;
        self.size = [width.max(1), height.max(1)];
        for layer in &mut self.shapes {
            layer.clear();
        }
        for layer in &mut self.placed {
            layer.clear();
        }
    }

    /// A filled rectangle.
    pub fn rect(&mut self, layer: Layer, x: f32, y: f32, w: f32, h: f32, colour: Rgba) {
        self.gradient(layer, x, y, w, h, colour, colour);
    }

    /// A rectangle shading from `left` at its left edge to `right` at its right.
    #[allow(clippy::too_many_arguments)]
    pub fn gradient(
        &mut self,
        layer: Layer,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        left: Rgba,
        right: Rgba,
    ) {
        if w <= 0.0 || h <= 0.0 || (left.a <= 0.0 && right.a <= 0.0) {
            return;
        }
        let v = |px: f32, py: f32, c: Rgba| self.vertex(px, py, c);
        let (x1, y1) = (x + w, y + h);
        self.shapes[layer as usize].extend([
            v(x, y, left),
            v(x1, y, right),
            v(x, y1, left),
            v(x, y1, left),
            v(x1, y, right),
            v(x1, y1, right),
        ]);
    }

    /// A rectangle shading from `top` at its top edge to `bottom` at its bottom.
    #[allow(clippy::too_many_arguments)]
    pub fn gradient_v(
        &mut self,
        layer: Layer,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        top: Rgba,
        bottom: Rgba,
    ) {
        if w <= 0.0 || h <= 0.0 || (top.a <= 0.0 && bottom.a <= 0.0) {
            return;
        }
        let v = |px: f32, py: f32, c: Rgba| self.vertex(px, py, c);
        let (x1, y1) = (x + w, y + h);
        self.shapes[layer as usize].extend([
            v(x, y, top),
            v(x1, y, top),
            v(x, y1, bottom),
            v(x, y1, bottom),
            v(x1, y, top),
            v(x1, y1, bottom),
        ]);
    }

    /// A filled triangle through three pixel positions.
    pub fn triangle(&mut self, layer: Layer, points: [(f32, f32); 3], colour: Rgba) {
        if colour.a <= 0.0 {
            return;
        }
        self.shapes[layer as usize].extend(points.map(|(x, y)| self.vertex(x, y, colour)));
    }

    /// A one-pixel horizontal line across pixel row `y`, from column `x0` up to `x1`.
    pub fn hline(&mut self, layer: Layer, x0: f32, x1: f32, y: f32, colour: Rgba) {
        self.rect(layer, x0.min(x1), y, (x1 - x0).abs(), 1.0, colour);
    }

    /// A one-pixel vertical line down pixel column `x`, from row `y0` up to `y1`.
    pub fn vline(&mut self, layer: Layer, x: f32, y0: f32, y1: f32, colour: Rgba) {
        self.rect(layer, x, y0.min(y1), 1.0, (y1 - y0).abs(), colour);
    }

    /// The one-pixel outline of a rectangle.
    pub fn outline(&mut self, layer: Layer, x: f32, y: f32, w: f32, h: f32, colour: Rgba) {
        self.hline(layer, x, x + w, y, colour);
        self.hline(layer, x, x + w, y + h - 1.0, colour);
        self.vline(layer, x, y + 1.0, y + h - 1.0, colour);
        self.vline(layer, x + w - 1.0, y + 1.0, y + h - 1.0, colour);
    }

    /// An anti-aliased line of `width` pixels between two points, which are pixel
    /// positions: (10.5, 3.5) is the centre of pixel (10, 3).
    #[allow(clippy::too_many_arguments)]
    pub fn line(
        &mut self,
        layer: Layer,
        x0: f32,
        y0: f32,
        x1: f32,
        y1: f32,
        width: f32,
        colour: Rgba,
    ) {
        if colour.a <= 0.0 {
            return;
        }
        let half = (width * 0.5).max(0.01);
        // The quad reaches a pixel past the line's edge for the anti-aliased fringe.
        let pad = half + 1.0;
        let (dx, dy) = (x1 - x0, y1 - y0);
        let len = (dx * dx + dy * dy).sqrt().max(1e-6);
        let (ux, uy) = (dx / len * pad, dy / len * pad);
        let (nx, ny) = (-uy, ux);
        let segment = [x0, y0, x1, y1];
        let v = |px: f32, py: f32| Vertex {
            segment,
            half_width: half,
            ..self.vertex(px, py, colour)
        };
        let a = v(x0 - ux + nx, y0 - uy + ny);
        let b = v(x1 + ux + nx, y1 + uy + ny);
        let c = v(x0 - ux - nx, y0 - uy - ny);
        let d = v(x1 + ux - nx, y1 + uy - ny);
        self.shapes[layer as usize].extend([a, b, c, c, b, d]);
    }

    /// A vertex at a pixel position, in clip space as the shader wants it.
    fn vertex(&self, x: f32, y: f32, colour: Rgba) -> Vertex {
        let [w, h] = self.size;
        Vertex {
            clip: [x / w as f32 * 2.0 - 1.0, 1.0 - y / h as f32 * 2.0],
            pos: [x, y],
            colour: colour.to_array(),
            segment: [0.0; 4],
            half_width: 0.0,
        }
    }

    /// The size `text` takes in `face` at `size` pixels.
    pub fn measure(&mut self, text: &str, face: Face, size: f32) -> TextSize {
        let key = TextKey {
            text: text.to_owned(),
            face,
            size: (size * 64.0).round() as u32,
        };
        self.shape(&key)
    }

    /// Draws `text` with its top-left corner at (`x`, `y`) and returns its size.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        layer: Layer,
        text: &str,
        face: Face,
        size: f32,
        x: f32,
        y: f32,
        colour: Rgba,
    ) -> TextSize {
        self.text_clipped(layer, text, face, size, x, y, colour, None)
    }

    /// As [`Overlay::text`], cut off outside `clip` (x, y, width, height).
    #[allow(clippy::too_many_arguments)]
    pub fn text_clipped(
        &mut self,
        layer: Layer,
        text: &str,
        face: Face,
        size: f32,
        x: f32,
        y: f32,
        colour: Rgba,
        clip: Option<[f32; 4]>,
    ) -> TextSize {
        let key = TextKey {
            text: text.to_owned(),
            face,
            size: (size * 64.0).round() as u32,
        };
        let measured = self.shape(&key);
        if colour.a > 0.0 && !text.is_empty() {
            self.placed[layer as usize].push(Placed {
                key,
                x,
                y,
                colour,
                clip,
            });
        }
        measured
    }

    fn shape(&mut self, key: &TextKey) -> TextSize {
        let frame = self.frame;
        if let Some(s) = self.shaped.get_mut(key) {
            s.used = frame;
            return s.size;
        }
        let px = key.size as f32 / 64.0;
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(px, (px * LINE_HEIGHT).ceil()));
        buffer.set_size(None, None);
        buffer.set_text(&key.text, &key.face.attrs(), Shaping::Advanced, None);
        buffer.shape_until_scroll(&mut self.fonts, false);
        let (mut w, mut h) = (0.0f32, 0.0f32);
        for run in buffer.layout_runs() {
            w = w.max(run.line_w);
            h = h.max(run.line_top + run.line_height);
        }
        let size = TextSize { w, h };
        self.shaped.insert(
            key.clone(),
            Shaped {
                buffer,
                size,
                used: frame,
            },
        );
        size
    }

    /// Uploads the frame's shapes and lays out its text. Call before the render pass.
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        let [w, h] = self.size;
        let total: usize = self.shapes.iter().map(Vec::len).sum();
        if total as u64 > self.vertex_capacity {
            self.vertex_capacity = (total as u64).next_power_of_two();
            self.vertex_buffer = vertex_buffer(device, self.vertex_capacity);
        }
        let mut start = 0u32;
        for (i, layer) in self.shapes.iter().enumerate() {
            let end = start + layer.len() as u32;
            if !layer.is_empty() {
                queue.write_buffer(
                    &self.vertex_buffer,
                    start as u64 * size_of::<Vertex>() as u64,
                    bytemuck::cast_slice(layer),
                );
            }
            self.ranges[i] = start..end;
            start = end;
        }

        self.viewport.update(
            queue,
            Resolution {
                width: w,
                height: h,
            },
        );
        for (i, placed) in self.placed.iter().enumerate() {
            let areas = placed.iter().filter_map(|p| {
                let s = self.shaped.get(&p.key)?;
                let bounds = match p.clip {
                    Some([cx, cy, cw, ch]) => TextBounds {
                        left: cx.floor() as i32,
                        top: cy.floor() as i32,
                        right: (cx + cw).ceil() as i32,
                        bottom: (cy + ch).ceil() as i32,
                    },
                    None => TextBounds::default(),
                };
                let c = p.colour;
                let byte = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                Some(TextArea {
                    buffer: &s.buffer,
                    left: p.x.round(),
                    top: p.y.round(),
                    scale: 1.0,
                    bounds,
                    default_color: Color::rgba(byte(c.r), byte(c.g), byte(c.b), byte(c.a)),
                    custom_glyphs: &[],
                })
            });
            if let Err(e) = self.renderers[i].prepare(
                device,
                queue,
                &mut self.fonts,
                &mut self.atlas,
                &self.viewport,
                areas,
                &mut self.swash,
            ) {
                log::warn!("text: {e}");
            }
        }
    }

    /// Draws one layer: its shapes, then its text.
    pub fn draw(&self, layer: Layer, pass: &mut wgpu::RenderPass<'_>) {
        let [w, h] = self.size;
        pass.set_viewport(0.0, 0.0, w as f32, h as f32, 0.0, 1.0);
        let range = self.ranges[layer as usize].clone();
        if !range.is_empty() {
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.draw(range, 0..1);
        }
        if !self.placed[layer as usize].is_empty()
            && let Err(e) = self.renderers[layer as usize].render(&self.atlas, &self.viewport, pass)
        {
            log::warn!("text: {e}");
        }
    }

    /// Ends the frame: lets go of glyphs and labels that are no longer drawn.
    pub fn finish(&mut self) {
        self.atlas.trim();
        let frame = self.frame;
        if frame.is_multiple_of(60) {
            self.shaped.retain(|_, s| frame - s.used <= KEEP_FRAMES);
        }
    }
}

fn vertex_buffer(device: &wgpu::Device, vertices: u64) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("overlay shapes"),
        size: vertices * size_of::<Vertex>() as u64,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
