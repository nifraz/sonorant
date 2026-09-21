//! Album art: decoding what a player hands over, and drawing the two pictures made
//! from it.
//!
//! One decode gives two pictures. The deck draws the cover at its own size, stretched
//! into the square the layout left for it. The immersive backdrop draws the same
//! picture reduced to [`BACKDROP_SIZE`] and stretched back over the whole window, which
//! is how Nostalgia+ blurred it: a real blur kernel at 1920x1080 is not worth its cost
//! for something deliberately out of focus, and the hardware's bilinear filter does the
//! upscale for nothing.
//!
//! The backdrop is drawn as the ground, into the visuals target before the spectrogram,
//! so it shows through wherever there is no data and is covered wherever there is. That
//! is what makes it read as light behind the analysis rather than a wash over it.

use crate::layout::Rect;

/// The backdrop's reduced picture, in pixels each way. Nostalgia+'s number.
pub const BACKDROP_SIZE: u32 = 40;

/// Decoded art is reduced to this on its longest side before it reaches the GPU. The
/// deck draws it at around a hundred pixels, so a 3000 pixel cover is nine megabytes of
/// texture to show nothing more.
const MAX_DECK_SIZE: u32 = 512;

/// A decoded picture, eight bits a channel, not premultiplied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Picture {
    pub width: u32,
    pub height: u32,
    /// `width * height * 4` bytes, RGBA, top row first.
    pub rgba: Vec<u8>,
}

impl Picture {
    /// A picture of a solid colour, for tests and for the empty frame.
    pub fn solid(width: u32, height: u32, rgba: [u8; 4]) -> Picture {
        Picture {
            width,
            height,
            rgba: rgba.repeat((width * height) as usize),
        }
    }

    fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * self.width + x) * 4) as usize;
        [
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ]
    }
}

/// Decodes a PNG or JPEG. Anything else is refused rather than guessed at.
///
/// These two cover what the platforms hand over: SMTC thumbnails are whatever the
/// player embedded, which in practice is JPEG for covers ripped from tags and PNG for
/// everything else, and MPRIS art files are the same.
pub fn decode(bytes: &[u8]) -> Option<Picture> {
    const PNG: &[u8] = &[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
    const JPEG: &[u8] = &[0xff, 0xd8, 0xff];
    if bytes.starts_with(PNG) {
        return decode_png(bytes);
    }
    if bytes.starts_with(JPEG) {
        return decode_jpeg(bytes);
    }
    log::debug!(
        "artwork: not a PNG or a JPEG (starts {:02x?})",
        &bytes[..bytes.len().min(4)]
    );
    None
}

fn decode_png(bytes: &[u8]) -> Option<Picture> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    // Palettes expanded, 16-bit samples cut to 8, so what comes out is grey, RGB or
    // RGBA and never anything needing a palette lookup here.
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let pixels = (info.width * info.height) as usize;
    let mut rgba = vec![255u8; pixels * 4];
    match info.color_type {
        png::ColorType::Rgba => rgba.copy_from_slice(&buf[..pixels * 4]),
        png::ColorType::Rgb => {
            for i in 0..pixels {
                rgba[i * 4..i * 4 + 3].copy_from_slice(&buf[i * 3..i * 3 + 3]);
            }
        }
        png::ColorType::Grayscale => {
            for i in 0..pixels {
                rgba[i * 4] = buf[i];
                rgba[i * 4 + 1] = buf[i];
                rgba[i * 4 + 2] = buf[i];
            }
        }
        png::ColorType::GrayscaleAlpha => {
            for i in 0..pixels {
                rgba[i * 4] = buf[i * 2];
                rgba[i * 4 + 1] = buf[i * 2];
                rgba[i * 4 + 2] = buf[i * 2];
                rgba[i * 4 + 3] = buf[i * 2 + 1];
            }
        }
        other => {
            log::debug!("artwork: a PNG that expanded to {other:?}");
            return None;
        }
    }
    Some(fit(Picture {
        width: info.width,
        height: info.height,
        rgba,
    }))
}

fn decode_jpeg(bytes: &[u8]) -> Option<Picture> {
    use zune_jpeg::zune_core::bytestream::ZCursor;
    use zune_jpeg::zune_core::colorspace::ColorSpace;
    use zune_jpeg::zune_core::options::DecoderOptions;

    let options = DecoderOptions::new_fast().jpeg_set_out_colorspace(ColorSpace::RGBA);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let rgba = decoder
        .decode()
        .map_err(|e| log::debug!("artwork: {e}"))
        .ok()?;
    let info = decoder.info()?;
    let (width, height) = (u32::from(info.width), u32::from(info.height));
    if rgba.len() < (width as usize * height as usize * 4) {
        return None;
    }
    Some(fit(Picture {
        width,
        height,
        rgba,
    }))
}

/// Brings a decoded picture down to a size worth uploading.
fn fit(p: Picture) -> Picture {
    let longest = p.width.max(p.height);
    if longest <= MAX_DECK_SIZE || longest == 0 {
        return p;
    }
    let scale = f64::from(MAX_DECK_SIZE) / f64::from(longest);
    let w = ((f64::from(p.width) * scale).round() as u32).max(1);
    let h = ((f64::from(p.height) * scale).round() as u32).max(1);
    reduce(&p, w, h)
}

/// A smaller copy, averaging the source pixels that fall under each new one.
///
/// The average is taken on premultiplied values, so a transparent corner doesn't drag
/// its colour into the pixels beside it. Aspect ratio is not kept: both the deck's
/// square and the backdrop's are stretched to, which is what Nostalgia+ drew.
pub fn reduce(p: &Picture, width: u32, height: u32) -> Picture {
    let (width, height) = (width.max(1), height.max(1));
    if p.width == 0 || p.height == 0 {
        return Picture::solid(width, height, [0, 0, 0, 0]);
    }
    let mut rgba = vec![0u8; (width * height * 4) as usize];
    for y in 0..height {
        // The source rows this row covers, always at least one.
        let y0 = y * p.height / height;
        let y1 = (((y + 1) * p.height).div_ceil(height))
            .max(y0 + 1)
            .min(p.height);
        for x in 0..width {
            let x0 = x * p.width / width;
            let x1 = (((x + 1) * p.width).div_ceil(width))
                .max(x0 + 1)
                .min(p.width);
            let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
            let mut n = 0u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let [pr, pg, pb, pa] = p.pixel(sx, sy);
                    let alpha = u64::from(pa);
                    r += u64::from(pr) * alpha;
                    g += u64::from(pg) * alpha;
                    b += u64::from(pb) * alpha;
                    a += alpha;
                    n += 1;
                }
            }
            let i = ((y * width + x) * 4) as usize;
            if a == 0 || n == 0 {
                rgba[i..i + 4].copy_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // Back out of premultiplied: the sums divide by the alpha they carried.
            rgba[i] = (r / a) as u8;
            rgba[i + 1] = (g / a) as u8;
            rgba[i + 2] = (b / a) as u8;
            rgba[i + 3] = (a / n) as u8;
        }
    }
    Picture {
        width,
        height,
        rgba,
    }
}

/// One track's pictures on the GPU.
#[derive(Debug)]
pub struct Artwork {
    deck: Bound,
    backdrop: Bound,
}

#[derive(Debug)]
struct Bound {
    #[allow(dead_code)] // the bind group holds the view, which holds this
    texture: wgpu::Texture,
    bind: wgpu::BindGroup,
}

/// Draws pictures as stretched quads.
///
/// Two pipelines for the two places art is drawn: over the furniture, in the
/// swapchain's own format and sRGB-encoded colour, and under the visuals, in the
/// floating-point target and linear light. The same picture is uploaded twice, viewed
/// once each way, so neither pass has to know what the other is doing.
#[derive(Debug)]
pub struct ArtworkPass {
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    over: wgpu::RenderPipeline,
    under: wgpu::RenderPipeline,
    vertices: wgpu::Buffer,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    clip: [f32; 2],
    uv: [f32; 2],
    alpha: f32,
    _pad: [f32; 3],
}

/// Where each quad's six vertices live in the buffer.
const DECK: std::ops::Range<u32> = 0..6;
const BACKDROP: std::ops::Range<u32> = 6..12;

impl ArtworkPass {
    /// `over` is the swapchain's plain format, `under` the visuals' float target.
    pub fn new(
        device: &wgpu::Device,
        over: wgpu::TextureFormat,
        under: wgpu::TextureFormat,
    ) -> ArtworkPass {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("artwork"),
            source: wgpu::ShaderSource::Wgsl(include_str!("artwork.wgsl").into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("artwork"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("artwork"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("artwork"),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Vertex>() as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32],
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
                        format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::COLOR,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("artwork"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            ..Default::default()
        });
        let vertices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("artwork quads"),
            size: (BACKDROP.end as usize * size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        ArtworkPass {
            over: pipeline(over),
            under: pipeline(under),
            layout,
            sampler,
            vertices,
        }
    }

    /// Uploads a decoded picture, making the deck's copy and the backdrop's.
    ///
    /// The deck's is kept in the swapchain's own encoding, because the furniture writes
    /// sRGB-encoded values straight out; the backdrop's is an sRGB texture so the
    /// hardware converts it to the linear light the visuals target works in.
    ///
    /// Two textures rather than one texture with two views, although the bytes are the
    /// same both times. A view in another format is something a downlevel device may
    /// not have, and OpenGL here is exactly that device: it is the only way to reach an
    /// Intel GPU too old for Vulkan, and asking it for one is a validation error rather
    /// than a warning. Declaring the format on the texture costs a second copy of a
    /// cover thumbnail and works everywhere.
    pub fn upload(&self, device: &wgpu::Device, queue: &wgpu::Queue, picture: &Picture) -> Artwork {
        let small = reduce(picture, BACKDROP_SIZE, BACKDROP_SIZE);
        Artwork {
            deck: self.bind(device, queue, picture, wgpu::TextureFormat::Rgba8Unorm),
            backdrop: self.bind(device, queue, &small, wgpu::TextureFormat::Rgba8UnormSrgb),
        }
    }

    fn bind(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        picture: &Picture,
        format: wgpu::TextureFormat,
    ) -> Bound {
        let size = wgpu::Extent3d {
            width: picture.width.max(1),
            height: picture.height.max(1),
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("artwork"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &picture.rgba,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(size.width * 4),
                rows_per_image: Some(size.height),
            },
            size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("artwork"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        Bound { texture, bind }
    }

    /// Works out both quads for this frame. Call once, before the passes.
    ///
    /// `deck` is where the cover goes, `strength` how far the backdrop comes forward,
    /// from 0 to 1. A zero alpha leaves the quad off screen rather than drawing
    /// nothing at cost.
    pub fn prepare(
        &self,
        queue: &wgpu::Queue,
        screen: (u32, u32),
        deck: Rect,
        deck_alpha: f32,
        strength: f32,
    ) {
        let (w, h) = (screen.0.max(1) as f32, screen.1.max(1) as f32);
        let mut vertices = [Vertex::default(); BACKDROP.end as usize];
        quad(
            &mut vertices[DECK.start as usize..DECK.end as usize],
            [
                deck.x as f32 / w * 2.0 - 1.0,
                1.0 - deck.y as f32 / h * 2.0,
                deck.right() as f32 / w * 2.0 - 1.0,
                1.0 - deck.bottom() as f32 / h * 2.0,
            ],
            deck_alpha,
        );
        quad(
            &mut vertices[BACKDROP.start as usize..BACKDROP.end as usize],
            [-1.0, 1.0, 1.0, -1.0],
            strength,
        );
        queue.write_buffer(&self.vertices, 0, bytemuck::cast_slice(&vertices));
    }

    /// The cover, over the furniture.
    pub fn draw_deck(&self, pass: &mut wgpu::RenderPass<'_>, art: &Artwork) {
        pass.set_pipeline(&self.over);
        pass.set_bind_group(0, &art.deck.bind, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(DECK, 0..1);
    }

    /// The reduced copy over the whole window, under the visuals.
    pub fn draw_backdrop(&self, pass: &mut wgpu::RenderPass<'_>, art: &Artwork) {
        pass.set_pipeline(&self.under);
        pass.set_bind_group(0, &art.backdrop.bind, &[]);
        pass.set_vertex_buffer(0, self.vertices.slice(..));
        pass.draw(BACKDROP, 0..1);
    }
}

/// Two triangles for a rectangle given as left, top, right, bottom in clip space.
fn quad(out: &mut [Vertex], [l, t, r, b]: [f32; 4], alpha: f32) {
    let corner = |clip: [f32; 2], uv: [f32; 2]| Vertex {
        clip,
        uv,
        alpha,
        _pad: [0.0; 3],
    };
    out[0] = corner([l, t], [0.0, 0.0]);
    out[1] = corner([l, b], [0.0, 1.0]);
    out[2] = corner([r, t], [1.0, 0.0]);
    out[3] = corner([r, t], [1.0, 0.0]);
    out[4] = corner([l, b], [0.0, 1.0]);
    out[5] = corner([r, b], [1.0, 1.0]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reducing_averages_the_pixels_that_fall_under_each_one() {
        // Four quadrants of a 2x2 block each, reduced back to 2x2.
        let mut p = Picture::solid(4, 4, [0, 0, 0, 255]);
        for y in 0..4 {
            for x in 0..4 {
                let i = ((y * 4 + x) * 4) as usize;
                p.rgba[i] = if x < 2 { 0 } else { 200 };
                p.rgba[i + 1] = if y < 2 { 0 } else { 100 };
            }
        }
        let small = reduce(&p, 2, 2);
        assert_eq!(small.width, 2);
        assert_eq!(small.pixel(0, 0), [0, 0, 0, 255]);
        assert_eq!(small.pixel(1, 0), [200, 0, 0, 255]);
        assert_eq!(small.pixel(0, 1), [0, 100, 0, 255]);
        assert_eq!(small.pixel(1, 1), [200, 100, 0, 255]);
    }

    #[test]
    fn reducing_stretches_rather_than_keeping_the_aspect() {
        let wide = Picture::solid(100, 10, [1, 2, 3, 255]);
        let small = reduce(&wide, BACKDROP_SIZE, BACKDROP_SIZE);
        assert_eq!((small.width, small.height), (BACKDROP_SIZE, BACKDROP_SIZE));
        assert_eq!(small.pixel(20, 20), [1, 2, 3, 255]);
    }

    #[test]
    fn growing_is_allowed_and_keeps_the_colour() {
        let one = Picture::solid(1, 1, [9, 8, 7, 255]);
        let big = reduce(&one, 4, 4);
        assert_eq!(big.rgba.len(), 4 * 4 * 4);
        assert_eq!(big.pixel(3, 3), [9, 8, 7, 255]);
    }

    #[test]
    fn a_transparent_neighbour_does_not_bleed_into_the_average() {
        // One opaque red pixel and three clear ones: the colour stays red and only
        // the alpha falls, rather than the red being dragged towards black.
        let mut p = Picture::solid(2, 2, [0, 0, 0, 0]);
        p.rgba[0..4].copy_from_slice(&[255, 0, 0, 255]);
        let one = reduce(&p, 1, 1);
        assert_eq!(one.pixel(0, 0), [255, 0, 0, 63]);
    }

    #[test]
    fn a_picture_too_big_for_the_deck_is_brought_down() {
        let huge = Picture::solid(2000, 1000, [40, 50, 60, 255]);
        let fitted = fit(huge);
        assert_eq!((fitted.width, fitted.height), (MAX_DECK_SIZE, 256));
        assert_eq!(fitted.pixel(10, 10), [40, 50, 60, 255]);
    }

    #[test]
    fn a_picture_small_enough_is_left_exactly_as_it_was() {
        let small = Picture::solid(300, 300, [1, 1, 1, 255]);
        assert_eq!(fit(small.clone()), small);
    }

    #[test]
    fn png_round_trips_through_the_decoder() {
        let source = Picture::solid(8, 6, [10, 120, 250, 255]);
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, source.width, source.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&source.rgba).unwrap();
        }
        assert_eq!(decode(&bytes), Some(source));
    }

    #[test]
    fn an_rgb_png_comes_back_opaque() {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 2, 2);
            encoder.set_color(png::ColorType::Rgb);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&[7, 8, 9].repeat(4)).unwrap();
        }
        let out = decode(&bytes).expect("an RGB PNG decodes");
        assert_eq!(out.pixel(1, 1), [7, 8, 9, 255]);
    }

    #[test]
    fn a_jpeg_decodes_to_roughly_the_colour_it_was_saved_as() {
        // 8x8 of one colour, saved by GDI+. JPEG is lossy and subsamples chroma, so
        // the test is that the colour survives, not that the bytes match.
        let bytes = include_bytes!("../tests/data/solid.jpg");
        let out = decode(bytes).expect("a JPEG decodes");
        assert_eq!((out.width, out.height), (8, 8));
        let [r, g, b, a] = out.pixel(4, 4);
        assert_eq!(a, 255, "a JPEG has no alpha, so it comes back opaque");
        for (got, want) in [(r, 20u8), (g, 140), (b, 220)] {
            assert!(got.abs_diff(want) <= 6, "channel {got} is not near {want}");
        }
    }

    #[test]
    fn anything_that_is_not_a_picture_is_refused_rather_than_guessed_at() {
        assert_eq!(decode(b""), None);
        assert_eq!(decode(b"GIF89a and then some"), None);
        assert_eq!(decode(b"\x89PNG\r\n\x1a\n truncated"), None);
    }
}
