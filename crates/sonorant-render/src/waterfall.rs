//! The 3D waterfall: the history drawn as a landscape.
//!
//! The mesh never leaves the GPU and the CPU never rebuilds it. One index buffer over a
//! fixed grid is drawn every frame, and the vertex shader displaces each point straight
//! out of the history texture, so a new row changes the surface without anything being
//! uploaded. Frequency runs across the grid on the display's own axis and time runs away
//! from the near edge, with height on the same ramp the spectrogram colours with, so the
//! flat view and this one agree about what is loud.
//!
//! The camera orbits a point over the middle of the surface. Everything about it is here
//! rather than in the frame loop, including the matrices, because a camera that is
//! slightly wrong is much easier to find in a test than on a screen.

use bytemuck::{Pod, Zeroable};
use sonorant_core::dsp::FreqScale;
use sonorant_core::engine::{GRID_BINS, GRID_FMAX, GRID_FMIN};
use sonorant_core::settings::CameraView;

use crate::colour::Rgba;
use crate::history::HistoryStore;

/// The grid, across (frequency) and back (time), at each quality. Enough across to
/// resolve a semitone near the middle of the axis, and enough back that a ridge reads as
/// a ridge rather than a staircase.
pub const MESH: [(u32, u32); 3] = [(96, 64), (192, 128), (320, 192)];
/// What the camera is pointed at: over the middle of the surface, a little above it, so
/// the landscape sits in the frame rather than the horizon does.
const TARGET: [f64; 3] = [0.0, 0.15, -1.0];
/// How tall the loudest level stands. Much more and the ridges hide what is behind them;
/// much less and it is a flat picture drawn at an angle.
const RELIEF: f32 = 0.55;
/// How much light the surface has of its own, in linear light. The quiet end of every
/// palette is the background, so without this the flat ground is black on black and the
/// only thing on screen is the loudest ridge, hanging in nothing: you cannot see how far
/// the landscape reaches or which way it is tilted.
const SHEEN: f32 = 0.022;
/// How far the eye may be, and how far above the surface it may rise. The low end stops
/// short of the surface itself, where a height field has nothing to show.
pub const DISTANCE_RANGE: (f64, f64) = (0.8, 8.0);
pub const PITCH_RANGE: (f64, f64) = (0.04, 1.45);
const FOV_Y: f64 = std::f64::consts::FRAC_PI_4;

/// Where the eye is, as an orbit about [`TARGET`].
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Camera {
    /// Around the surface, in radians. 0 looks along the history from the front.
    pub yaw: f64,
    /// Above the surface, in radians.
    pub pitch: f64,
    /// How far the eye is from what it is looking at.
    pub distance: f64,
}

impl Default for Camera {
    fn default() -> Camera {
        Camera::of(CameraView::default())
    }
}

impl Camera {
    /// Where one of the menu's presets puts the eye.
    pub fn of(view: CameraView) -> Camera {
        let (yaw, pitch, distance) = match view {
            CameraView::Classic => (0.0, 0.45, 2.1),
            CameraView::Overhead => (0.0, 1.15, 1.9),
            CameraView::Side => (0.85, 0.32, 2.3),
            CameraView::Low => (0.15, 0.10, 2.5),
        };
        Camera {
            yaw,
            pitch,
            distance,
        }
    }

    /// Orbits by a drag of `dx` and `dy` pixels over a viewport `across` pixels wide.
    /// A drag right of the whole width turns the camera half way round.
    pub fn orbit(&mut self, dx: f64, dy: f64, across: f64) {
        let turn = std::f64::consts::PI / across.max(1.0);
        self.yaw -= dx * turn;
        self.pitch = (self.pitch + dy * turn).clamp(PITCH_RANGE.0, PITCH_RANGE.1);
        // Kept inside one turn, so the number stays readable and never drifts away.
        self.yaw = self.yaw.rem_euclid(std::f64::consts::TAU);
    }

    /// Moves the eye in or out by `notches` of the wheel.
    pub fn zoom(&mut self, notches: f64) {
        self.distance =
            (self.distance * 2f64.powf(-notches / 8.0)).clamp(DISTANCE_RANGE.0, DISTANCE_RANGE.1);
    }

    /// Where the eye is, in the surface's own space.
    pub fn eye(&self) -> [f64; 3] {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        [
            TARGET[0] + self.distance * cp * sy,
            TARGET[1] + self.distance * sp,
            TARGET[2] + self.distance * cp * cy,
        ]
    }

    /// World space to clip space, for a viewport of that aspect ratio. Column-major, as
    /// WGSL wants it, and with clip z running 0 to 1, as wgpu wants it.
    pub fn view_projection(&self, aspect: f64) -> [[f32; 4]; 4] {
        let eye = self.eye();
        let forward = normalise(sub(TARGET, eye));
        let right = normalise(cross(forward, [0.0, 1.0, 0.0]));
        let up = cross(right, forward);
        // Right-handed, looking down -Z.
        let view = [
            [right[0], up[0], -forward[0], 0.0],
            [right[1], up[1], -forward[1], 0.0],
            [right[2], up[2], -forward[2], 0.0],
            [-dot(right, eye), -dot(up, eye), dot(forward, eye), 1.0],
        ];
        let (near, far) = (0.05, 40.0);
        let f = 1.0 / (FOV_Y / 2.0).tan();
        let projection = [
            [f / aspect.max(1e-3), 0.0, 0.0, 0.0],
            [0.0, f, 0.0, 0.0],
            [0.0, 0.0, far / (near - far), -1.0],
            [0.0, 0.0, near * far / (near - far), 0.0],
        ];
        let product = multiply(projection, view);
        product.map(|column| column.map(|v| v as f32))
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalise(v: [f64; 3]) -> [f64; 3] {
    let len = dot(v, v).sqrt();
    if len < 1e-12 {
        return [0.0, 0.0, -1.0];
    }
    [v[0] / len, v[1] / len, v[2] / len]
}

/// `a` times `b`, both column-major.
fn multiply(a: [[f64; 4]; 4], b: [[f64; 4]; 4]) -> [[f64; 4]; 4] {
    let mut out = [[0.0; 4]; 4];
    for (c, column) in out.iter_mut().enumerate() {
        for (r, cell) in column.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[k][r] * b[c][k]).sum();
        }
    }
    out
}

/// Where the landscape goes, and whether this frame is being timed.
#[derive(Debug)]
pub struct Target<'a> {
    pub view: &'a wgpu::TextureView,
    pub timestamps: Option<wgpu::RenderPassTimestampWrites<'a>>,
}

/// What the waterfall draws this frame.
#[derive(Clone, Copy, Debug)]
pub struct Landscape {
    pub camera: Camera,
    /// The viewport, in framebuffer pixels, and the whole target it sits in, which is
    /// what the depth buffer has to match.
    pub rect: crate::layout::Rect,
    pub framebuffer: (u32, u32),
    /// Freezes the landscape at the moment the history had that many rows, as the flat
    /// view's anchor does.
    pub held: Option<u64>,
    /// Whether the surface has its own light where the palette gives it none, so a flat
    /// ground still shows its shape. Off is the plainer picture.
    pub sheen: bool,
    /// Which pane's levels: 0 or 1.
    pub channel: u32,
    /// Rows of history the grid spans, and the clock's progress into the next one.
    pub span_rows: f32,
    pub frac: f32,
    pub scale: FreqScale,
    pub fmin: f32,
    pub fmax: f32,
    /// `Some((floor, ceiling))` colours every row with that range instead of its own.
    pub global_range: Option<(f32, f32)>,
    /// What the far rows fade into.
    pub background: Rgba,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
struct SceneUniform {
    view_projection: [[f32; 4]; 4],
    fog_colour: [f32; 4],
    eye: [f32; 3],
    columns: u32,
    rows: u32,
    newest: u32,
    available: u32,
    layer_rows: u32,
    capacity: u32,
    span_rows: f32,
    frac: f32,
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
    floor_db: f32,
    ceiling_db: f32,
    relief: f32,
    fog_near: f32,
    fog_far: f32,
    sheen: f32,
    _pad: u32,
}

#[derive(Debug)]
pub struct WaterfallPass {
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    palette: wgpu::Texture,
    palette_view: wgpu::TextureView,
    uniforms: wgpu::Buffer,
    bind_group: Option<wgpu::BindGroup>,
    indices: wgpu::Buffer,
    index_count: u32,
    mesh: (u32, u32),
    depth: Option<(wgpu::TextureView, (u32, u32))>,
}

/// The depth format the surface sorts itself with. A height field seen from anywhere has
/// no reliable back-to-front order, so it is sorted per pixel rather than per row.
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

impl WaterfallPass {
    /// A waterfall drawing into `target`, the visuals' floating-point format.
    pub fn new(device: &wgpu::Device, target: wgpu::TextureFormat) -> WaterfallPass {
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("waterfall"),
            size: size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let palette = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("waterfall palette"),
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
            label: Some("waterfall palette"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // The vertex stage reads the history, because that is where the mesh is made.
        let both = wgpu::ShaderStages::VERTEX_FRAGMENT;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("waterfall"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: both,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: wgpu::BufferSize::new(size_of::<SceneUniform>() as u64),
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("waterfall"),
            source: wgpu::ShaderSource::Wgsl(include_str!("waterfall.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("waterfall"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let mesh = MESH[1];
        let (indices, index_count) = index_buffer(device, mesh);
        WaterfallPass {
            pipeline: device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("waterfall"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    // Both faces: from underneath, a landscape should still be there.
                    cull_mode: None,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(true),
                    depth_compare: Some(wgpu::CompareFunction::Less),
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),
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
            }),
            layout,
            sampler,
            palette,
            palette_view,
            uniforms,
            bind_group: None,
            indices,
            index_count,
            mesh,
            depth: None,
        }
    }

    /// Points the pass at a history store. Call again when the store is rebuilt.
    pub fn bind(&mut self, device: &wgpu::Device, history: &HistoryStore) {
        self.bind_group = Some(device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("waterfall"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.uniforms.as_entire_binding(),
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

    /// Replaces the palette, as [`crate::SpectrogramPass::set_palette`] does, so the two
    /// views are coloured alike.
    pub fn set_palette(&self, queue: &wgpu::Queue, lut: &sonorant_core::palette::Lut) {
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

    /// How dense the mesh is. Changing it rebuilds the index buffer, and nothing else.
    pub fn set_mesh(&mut self, device: &wgpu::Device, mesh: (u32, u32)) {
        if mesh == self.mesh {
            return;
        }
        self.mesh = mesh;
        let (indices, count) = index_buffer(device, mesh);
        self.indices = indices;
        self.index_count = count;
    }

    pub fn mesh(&self) -> (u32, u32) {
        self.mesh
    }

    /// Draws the landscape into `target`, in its own pass, with its own depth buffer.
    /// The colour already there is kept: the backdrop under it is the sky.
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        target: Target<'_>,
        history: &HistoryStore,
        view: &Landscape,
    ) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        if view.rect.w <= 0 || view.rect.h <= 0 {
            return;
        }
        let size = view.framebuffer;
        if self.depth.as_ref().is_none_or(|(_, had)| *had != size) {
            self.depth = Some((depth_texture(device, size), size));
        }
        let Some((depth, _)) = &self.depth else {
            return;
        };

        let written = history.written();
        let cap = history.capacity();
        let (newest, available) = match view.held {
            Some(h) if h > 0 && h <= written => {
                let overwritten = (written - h).min(cap);
                ((h - 1) % cap, h.min(cap).saturating_sub(overwritten))
            }
            _ => (history.newest_slot(), history.available()),
        };
        let grid_decades = (GRID_FMAX / GRID_FMIN).ln();
        let (fmin, fmax) = (
            f64::from(view.fmin).max(1e-3),
            f64::from(view.fmax).max(1e-3),
        );
        let bins = GRID_BINS as f64;
        let grid_at = (fmin / GRID_FMIN).ln() / grid_decades * bins - 0.5;
        let grid_span = (fmax / fmin).ln() / grid_decades * bins;
        let (global, floor, ceiling) = match view.global_range {
            Some((f, c)) => (1, f, c),
            None => (0, 0.0, 0.0),
        };
        let aspect = f64::from(view.rect.w) / f64::from(view.rect.h.max(1));
        let eye = view.camera.eye();
        // The fog reaches from a little beyond the near edge to a little past the far
        // one, measured from the eye, so it is the distance into the history that fades
        // rather than the camera's own distance.
        let fog_near = (view.camera.distance - 0.6) as f32;
        let fog_far = (view.camera.distance + 2.2) as f32;
        let u = SceneUniform {
            view_projection: view.camera.view_projection(aspect),
            fog_colour: view.background.to_linear(),
            eye: [eye[0] as f32, eye[1] as f32, eye[2] as f32],
            columns: self.mesh.0,
            rows: self.mesh.1,
            newest: newest as u32,
            available: available as u32,
            layer_rows: history.layer_rows(),
            capacity: cap as u32,
            span_rows: view.span_rows.max(1.0),
            frac: view.frac.clamp(0.0, 1.0),
            channel: view.channel,
            scale: u32::from(view.scale != FreqScale::Linear),
            fmin: view.fmin,
            fmax: view.fmax,
            grid_at: grid_at as f32,
            grid_span: grid_span as f32,
            grid_bins: GRID_BINS as u32,
            grid_fmin: GRID_FMIN as f32,
            grid_fmax: GRID_FMAX as f32,
            global_range: global,
            floor_db: floor,
            ceiling_db: ceiling,
            relief: RELIEF,
            fog_near: fog_near.max(0.1),
            fog_far: fog_far.max(fog_near + 0.2),
            sheen: if view.sheen { SHEEN } else { 0.0 },
            _pad: 0,
        };
        queue.write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));

        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("waterfall"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: target.timestamps,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        let [x, y, w, h] = view.rect.to_f32();
        pass.set_viewport(x, y, w, h, 0.0, 1.0);
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, bind_group, &[]);
        pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
        pass.draw_indexed(0..self.index_count, 0, 0..1);
    }
}

fn depth_texture(device: &wgpu::Device, size: (u32, u32)) -> wgpu::TextureView {
    device
        .create_texture(&wgpu::TextureDescriptor {
            label: Some("waterfall depth"),
            size: wgpu::Extent3d {
                width: size.0.max(1),
                height: size.1.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        })
        .create_view(&wgpu::TextureViewDescriptor::default())
}

/// Two triangles a cell, over a grid `columns` by `rows`. Built once: the surface
/// changes every frame but the way its points are joined up never does.
fn index_buffer(device: &wgpu::Device, mesh: (u32, u32)) -> (wgpu::Buffer, u32) {
    let (columns, rows) = (mesh.0.max(2), mesh.1.max(2));
    let mut indices = Vec::with_capacity(((columns - 1) * (rows - 1) * 6) as usize);
    for row in 0..rows - 1 {
        for column in 0..columns - 1 {
            let a = row * columns + column;
            let (b, c, d) = (a + 1, a + columns, a + columns + 1);
            indices.extend([a, c, b, b, c, d]);
        }
    }
    let count = indices.len() as u32;
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("waterfall mesh"),
        size: (indices.len() * size_of::<u32>()) as u64,
        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: true,
    });
    buffer
        .slice(..)
        .get_mapped_range_mut()
        .expect("a buffer mapped at creation")
        .copy_from_slice(bytemuck::cast_slice(&indices));
    buffer.unmap();
    (buffer, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where a point lands on the screen, -1 to 1 either way, or `None` when it is
    /// behind the eye.
    fn project(camera: &Camera, aspect: f64, p: [f64; 3]) -> Option<(f64, f64)> {
        let m = camera.view_projection(aspect);
        let at = |r: usize| {
            f64::from(m[0][r]) * p[0]
                + f64::from(m[1][r]) * p[1]
                + f64::from(m[2][r]) * p[2]
                + f64::from(m[3][r])
        };
        let w = at(3);
        (w > 1e-6).then(|| (at(0) / w, at(1) / w))
    }

    #[test]
    fn the_camera_looks_at_what_it_is_pointed_at() {
        for &view in CameraView::ALL {
            let camera = Camera::of(view);
            let (x, y) = project(&camera, 16.0 / 9.0, TARGET).expect("in front of the eye");
            // A pixel is about a thousandth of the screen; the matrix is handed over as
            // f32, so a ten-millionth is as close as it can be asked to come.
            assert!(
                x.abs() < 1e-5 && y.abs() < 1e-5,
                "{view:?} put its own target at {x}, {y}"
            );
        }
    }

    /// Looking down the history from above it: the newest row is lower on the screen
    /// than the oldest, and the surface is the right way up.
    #[test]
    fn the_history_runs_away_from_the_viewer() {
        let camera = Camera::of(CameraView::Classic);
        let near = project(&camera, 16.0 / 9.0, [0.0, 0.0, 0.0]).expect("in view");
        let far = project(&camera, 16.0 / 9.0, [0.0, 0.0, -2.0]).expect("in view");
        assert!(near.1 < far.1, "the newest row should be nearer the bottom");
        let high = project(&camera, 16.0 / 9.0, [0.0, 0.5, 0.0]).expect("in view");
        assert!(high.1 > near.1, "a loud row should stand up, not down");
    }

    /// Left is left, whichever side the camera has swung round to, because the whole
    /// surface turns with it.
    #[test]
    fn the_axis_runs_across_the_screen() {
        let camera = Camera::of(CameraView::Classic);
        let left = project(&camera, 16.0 / 9.0, [-1.0, 0.0, -1.0]).expect("in view");
        let right = project(&camera, 16.0 / 9.0, [1.0, 0.0, -1.0]).expect("in view");
        assert!(left.0 < right.0, "the low end should be on the left");
    }

    /// Something behind the eye has no place on the screen, which is what the w test in
    /// `project` is for; without it a point behind would appear mirrored in front.
    #[test]
    fn what_is_behind_the_eye_does_not_project() {
        let camera = Camera::of(CameraView::Classic);
        let eye = camera.eye();
        let behind = [eye[0] * 2.0, eye[1] * 2.0, eye[2] * 2.0 + 4.0];
        assert!(project(&camera, 16.0 / 9.0, behind).is_none());
    }

    #[test]
    fn orbiting_turns_the_camera_and_stops_it_going_under_the_surface() {
        // A drag of a quarter of the width turns the camera an eighth of the way round,
        // the other way, because the surface follows the pointer.
        let mut camera = Camera {
            yaw: 2.0,
            ..Camera::of(CameraView::Classic)
        };
        camera.orbit(200.0, 0.0, 800.0);
        assert!(
            (camera.yaw - (2.0 - std::f64::consts::FRAC_PI_4)).abs() < 1e-9,
            "{}",
            camera.yaw
        );

        // Dragged far past either end, the pitch stops rather than tumbling.
        camera.orbit(0.0, 10_000.0, 800.0);
        assert_eq!(camera.pitch, PITCH_RANGE.1);
        camera.orbit(0.0, -10_000.0, 800.0);
        assert_eq!(camera.pitch, PITCH_RANGE.0);
        // And the yaw stays inside one turn however far it is dragged, which is what
        // keeps a long drag from running the number off into the distance.
        for _ in 0..40 {
            camera.orbit(500.0, 0.0, 800.0);
            assert!((0.0..std::f64::consts::TAU).contains(&camera.yaw));
        }
    }

    /// Turning right round comes back to the same view, wrap or no wrap.
    #[test]
    fn a_whole_turn_is_where_it_started() {
        let start = Camera::of(CameraView::Classic);
        let mut camera = start;
        // Sixteen drags of an eighth of the width each: two full turns.
        for _ in 0..32 {
            camera.orbit(100.0, 0.0, 800.0);
        }
        let a = start.eye();
        let b = camera.eye();
        for (x, y) in a.iter().zip(&b) {
            assert!((x - y).abs() < 1e-9, "{a:?} against {b:?}");
        }
    }

    #[test]
    fn the_wheel_moves_the_eye_in_and_out_between_stops() {
        // From the middle of the range, so halving and doubling both have room.
        let mut camera = Camera {
            distance: 2.4,
            ..Camera::of(CameraView::Classic)
        };
        let was = camera.distance;
        camera.zoom(8.0);
        assert!(
            (camera.distance - was / 2.0).abs() < 1e-9,
            "{}",
            camera.distance
        );
        camera.zoom(-8.0);
        assert!((camera.distance - was).abs() < 1e-9);
        camera.zoom(1000.0);
        assert_eq!(camera.distance, DISTANCE_RANGE.0);
        camera.zoom(-1000.0);
        assert_eq!(camera.distance, DISTANCE_RANGE.1);
    }

    /// A wider viewport spreads the surface further across rather than stretching it.
    #[test]
    fn the_aspect_ratio_spreads_the_picture_rather_than_stretching_it() {
        let camera = Camera::of(CameraView::Classic);
        let p = [0.8, 0.0, -1.0];
        let wide = project(&camera, 21.0 / 9.0, p).expect("in view");
        let narrow = project(&camera, 4.0 / 3.0, p).expect("in view");
        assert!(wide.0.abs() < narrow.0.abs(), "{wide:?} {narrow:?}");
        assert!(
            (wide.1 - narrow.1).abs() < 1e-9,
            "the height should not move"
        );
    }

    #[test]
    fn the_uniform_matches_the_shader_layout() {
        // 64 for the matrix, 16 for the fog, 12 for the eye, then twenty-one more words
        // and two of padding, which is 192 in all and a multiple of 16.
        assert_eq!(size_of::<SceneUniform>(), 192);
        assert_eq!(size_of::<SceneUniform>() % 16, 0);
    }

    #[test]
    fn every_mesh_joins_its_points_up_into_whole_cells() {
        for (columns, rows) in MESH {
            assert!(columns >= 2 && rows >= 2);
            let cells = (columns - 1) * (rows - 1);
            assert_eq!(
                cells * 6 % 3,
                0,
                "{columns} by {rows} is not whole triangles"
            );
        }
        // Denser is denser, at every step.
        for pair in MESH.windows(2) {
            assert!(pair[0].0 < pair[1].0 && pair[0].1 < pair[1].1);
        }
    }
}
