//! Golden renders: the passes drawn offscreen and compared with a committed picture.
//!
//! Software renderers give the same pixels on every machine, so the goldens are kept
//! per renderer (WARP on Windows, lavapipe on Ubuntu) and named after it. On a hardware
//! GPU, where the pixels legitimately differ, the test still renders everything and
//! checks the frame is the right size and isn't blank, which catches a pass that fails
//! to draw at all.
//!
//! A missing golden isn't a failure: the picture is written to `target/golden-out/` and
//! the test says so, so a new renderer can be adopted by copying the file in.

use std::path::{Path, PathBuf};

use sonorant_core::dsp::{FreqScale, FrequencyMap, LoudnessReadings};
use sonorant_core::engine::{GRID_BINS, GRID_FMAX, GRID_FMIN};
use sonorant_core::palette::{self, PaletteKind};
use sonorant_core::settings::Settings;
use sonorant_render::artwork::{ArtworkPass, Picture};
use sonorant_render::band::BandLayout;
use sonorant_render::bloom::{self, Visuals};
use sonorant_render::curves::{CurveData, CurveLook, CurvePass, CurveView};
use sonorant_render::deck::{DeckState, TrackInfo, WaveRing};
use sonorant_render::history::{HistoryStore, RowIn};
use sonorant_render::layout::{Rect, ScopeLayout};
use sonorant_render::overlay::{Face, Layer, Overlay};
use sonorant_render::readback::{Readback, write_png};
use sonorant_render::spectrogram::{PaneView, SpectrogramPass};
use sonorant_render::{axes, deck};
use half::f16;

const WIDTH: u32 = 960;
const HEIGHT: u32 = 540;
const ROWS: u32 = 512;
/// How far the backdrop comes forward in the scene, as the settings' percentage does.
const BACKDROP_STRENGTH: f32 = 0.18;
/// Average difference per channel a golden may drift by, out of 255.
const MEAN_TOLERANCE: f64 = 1.5;
/// The most any one channel may differ by.
const MAX_TOLERANCE: u8 = 72;

/// A GPU to draw on, preferring a software renderer so the pixels are reproducible.
fn open_gpu() -> Option<(wgpu::Device, wgpu::Queue, String, bool)> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));
    let adapter = adapters.into_iter().max_by_key(|a| {
        let info = a.get_info();
        // A software renderer first; failing that, whatever is here.
        u8::from(info.device_type == wgpu::DeviceType::Cpu) * 2
            + u8::from(info.backend != wgpu::Backend::Gl)
    })?;
    let info = adapter.get_info();
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("golden"),
        required_limits: wgpu::Limits::downlevel_defaults().using_resolution(adapter.limits()),
        ..Default::default()
    }))
    .ok()?;
    let software = info.device_type == wgpu::DeviceType::Cpu;
    Some((device, queue, info.name, software))
}

/// Levels for one history row of a deterministic scene: three harmonic stacks, a sweep
/// that moves with time, and a noise floor.
fn row_levels(row: u32) -> (Vec<f16>, Vec<f16>) {
    let t = row as f64 / ROWS as f64;
    let grid = FrequencyMap::new(FreqScale::Log, GRID_BINS, GRID_FMIN, GRID_FMAX);
    let mut a = Vec::with_capacity(GRID_BINS);
    let mut b = Vec::with_capacity(GRID_BINS);
    let sweep = 200.0 * 40.0f64.powf(t);
    for i in 0..GRID_BINS {
        let f = grid.centres[i];
        let mut level: f64 = -96.0 + 6.0 * ((i as f64 * 0.37).sin() * 0.5 + 0.5);
        for h in 1..=6 {
            for base in [110.0, 220.0, 330.0] {
                let centre = base * h as f64;
                let width = centre * 0.02;
                if (f - centre).abs() < width {
                    level = level.max(-12.0 - 4.0 * h as f64);
                }
            }
        }
        if (f - sweep).abs() < sweep * 0.01 {
            level = level.max(-6.0);
        }
        a.push(f16::from_f64(level));
        b.push(f16::from_f64(level - 4.0 * t));
    }
    (a, b)
}

/// Draws the whole scene the app draws, offscreen, and returns the pixels.
fn render(device: &wgpu::Device, queue: &wgpu::Queue) -> (u32, u32, Vec<u8>) {
    let settings = Settings {
        palette: PaletteKind::Magma,
        curve_width_pct: 45,
        mirror_left_pane: true,
        ..Settings::default()
    };
    let lut = palette::build_lut(settings.palette);

    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("golden"),
        size: wgpu::Extent3d {
            width: WIDTH,
            height: HEIGHT,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());

    // The history, filled with the scene, oldest first.
    let mut history = HistoryStore::new(device, GRID_BINS as u32, ROWS);
    for row in 0..ROWS {
        let (a, b) = row_levels(row);
        history.push(
            queue,
            &RowIn {
                index: row as u64,
                frames: row as u64 * 800,
                floor_db: -96.0,
                ceiling_db: -6.0,
                a: &a,
                b: &b,
            },
        );
    }

    let mut spectrogram = SpectrogramPass::new(device, bloom::TARGET_FORMAT);
    spectrogram.bind(device, &history);
    spectrogram.set_palette(queue, &lut);
    let visuals = Visuals::new(device, wgpu::TextureFormat::Rgba8Unorm, WIDTH, HEIGHT);
    let mut curves = CurvePass::new(device, wgpu::TextureFormat::Rgba8Unorm);
    curves.set_palette(queue, &lut);
    let mut overlay = Overlay::new(device, queue, wgpu::TextureFormat::Rgba8Unorm);
    let artwork = ArtworkPass::new(
        device,
        wgpu::TextureFormat::Rgba8Unorm,
        bloom::TARGET_FORMAT,
    );
    let cover = artwork.upload(device, queue, &test_cover());

    // The same layout the app builds: panes above, band below, colour bar at the edge.
    let client = Rect::new(0, 0, WIDTH as i32, HEIGHT as i32);
    let bar_w = 40;
    let colour_bar = Rect::new(client.right() - bar_w, 0, bar_w, client.h);
    let view_rect = Rect::new(0, 0, client.w - bar_w, client.h);
    let band_h = BandLayout::height_for(&settings, view_rect.h);
    let layout = ScopeLayout::new(
        Rect::new(0, 0, view_rect.w, view_rect.h - band_h),
        &settings,
    );
    let label_px = axes::label_px(&settings, 1.0);
    let mut measure = |text: &str| overlay.measure(text, Face::Sans, label_px).w;
    let band = BandLayout::new(view_rect, band_h, &settings, &layout.panes, &mut measure);

    let columns = layout.columns();
    let map = FrequencyMap::new(settings.scale, columns, settings.fmin, settings.fmax);
    let (floor_db, ceiling_db) = (-96.0, -6.0);
    // Curves straight from the newest row, so the picture is one consistent moment.
    let (newest, _) = row_levels(ROWS - 1);
    let display: Vec<f32> = (0..columns)
        .map(|x| {
            let f = map.centres[x];
            let pos =
                ((f / GRID_FMIN).ln() / (GRID_FMAX / GRID_FMIN).ln() * GRID_BINS as f64) as usize;
            newest[pos.min(GRID_BINS - 1)].to_f32()
        })
        .collect();
    let peak: Vec<f32> = display.iter().map(|v| v + 3.0).collect();
    let average: Vec<f32> = display.iter().map(|v| v - 6.0).collect();
    let minimum: Vec<f32> = display.iter().map(|v| v - 14.0).collect();

    let panes: Vec<PaneView> = layout
        .panes
        .iter()
        .enumerate()
        .map(|(i, p)| PaneView {
            rect: p.spectro.to_f32(),
            channel: i as u32,
            newest_left: p.curve_on_left,
            visible_rows: p.spectro.w as f32,
            frac: 0.0,
            scale: map.scale,
            fmin: map.fmin as f32,
            fmax: map.fmax as f32,
            global_range: None,
            smooth_time: false,
        })
        .collect();
    spectrogram.prepare(queue, &history, &panes, None);

    let look = CurveLook::new(&settings, &lut, 1.0);
    let strips: Vec<(CurveView, CurveData<'_>)> = layout
        .panes
        .iter()
        .map(|p| {
            (
                CurveView {
                    rect: p.curve,
                    curve_on_left: p.curve_on_left,
                    floor_db,
                    ceiling_db,
                },
                CurveData {
                    display: &display,
                    peak: &peak,
                    average: &average,
                    minimum: &minimum,
                    reference: None,
                },
            )
        })
        .collect();
    curves.prepare(device, queue, &look, &strips);

    overlay.begin(WIDTH, HEIGHT);
    axes::draw(
        &mut overlay,
        &axes::Scales {
            layout: &layout,
            settings: &settings,
            map: &map,
            floor_db,
            ceiling_db,
            px_per_second: settings.rows_per_second,
            scale: 1.0,
            alpha: 1.0,
            top_inset: 14.0,
            label_floor_y: 30.0,
        },
    );
    let mut waves = WaveRing::new(band.wave_a.w.max(8) as usize);
    for row in 0..ROWS {
        let t = row as f32 / ROWS as f32;
        let a = 0.2 + 0.6 * (t * 40.0).sin().abs();
        waves.push([-a, a, -a * 0.8, a * 0.8]);
    }
    let loudness = LoudnessReadings {
        momentary: -14.2,
        short_term: -13.8,
        integrated: -14.0,
        range: 6.5,
        true_peak_db: -0.8,
        crest_db: 12.3,
        correlation: 0.42,
        balance: -0.08,
        overs: 2,
        last_over_seconds: 75.0,
    };
    let scope: Vec<f32> = (0..2048).map(|i| (i as f32 * 0.05).sin() * 0.6).collect();
    let scope_r: Vec<f32> = (0..2048)
        .map(|i| (i as f32 * 0.05 + 0.4).sin() * 0.45)
        .collect();
    let track = TrackInfo {
        title: "Golden Render".into(),
        artist: "Sonorant".into(),
        album: "Test Scene".into(),
        composer: String::new(),
        year: "2026".into(),
    };
    deck::draw_band(
        &mut overlay,
        &band,
        &layout.panes,
        &settings,
        &lut,
        &waves,
        &DeckState {
            loudness: &loudness,
            bpm: 120.0,
            brightness_hz: 2400.0,
            scope_left: &scope,
            scope_right: &scope_r,
            track: &track,
            position: Some((62.0, 245.0)),
            playing: true,
            has_art: true,
        },
        1.0,
        label_px,
    );
    deck::draw_colour_bar(
        &mut overlay,
        colour_bar,
        &settings,
        &lut,
        floor_db,
        ceiling_db,
        1.0,
        label_px,
    );
    deck::draw_status(
        &mut overlay,
        &settings,
        "golden  |  16K / 4K / 1K  |  LeftRight  |  60 fps",
        0.0,
        1.0,
        label_px,
    );
    overlay.prepare(device, queue);
    artwork.prepare(
        queue,
        (WIDTH, HEIGHT),
        band.deck.art,
        1.0,
        BACKDROP_STRENGTH,
    );

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("golden"),
    });
    {
        let bg = palette::background(&lut);
        let linear = |c: u8| {
            let c = c as f64 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("visuals"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: visuals.view(),
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: linear(bg.r),
                            g: linear(bg.g),
                            b: linear(bg.b),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        artwork.draw_backdrop(&mut pass, &cover);
        spectrogram.draw(&mut pass, &panes);
    }
    visuals.build_glow(queue, &mut encoder, None);
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("composite"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        visuals.composite(queue, &mut pass, 1.0);
    }
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("furniture"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        overlay.draw(Layer::Under, &mut pass);
        curves.draw(&mut pass);
        overlay.draw(Layer::Over, &mut pass);
        // At this size the layout spends the deck's width on the track info instead,
        // as Nostalgia+'s does; `the_cover_fills_its_frame` covers the drawing.
        if band.deck.art.w > 0 {
            artwork.draw_deck(&mut pass, &cover);
        }
        overlay.draw(Layer::Top, &mut pass);
    }
    let shot = Readback::record(device, &mut encoder, &target);
    queue.submit([encoder.finish()]);
    shot.pixels(device).expect("the frame reads back")
}

/// A stand-in album cover: a two-way gradient with a dark border, so both the deck's
/// stretch and the backdrop's reduction show whether they ran.
fn test_cover() -> Picture {
    const SIZE: u32 = 128;
    let mut p = Picture::solid(SIZE, SIZE, [0, 0, 0, 255]);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let edge = x < 6 || y < 6 || x >= SIZE - 6 || y >= SIZE - 6;
            if edge {
                continue;
            }
            p.rgba[i] = (x * 255 / SIZE) as u8;
            p.rgba[i + 1] = (y * 255 / SIZE) as u8;
            p.rgba[i + 2] = 200 - (x * 120 / SIZE) as u8;
        }
    }
    p
}

/// The mean and worst difference between two pictures, per channel out of 255.
fn difference(a: &[u8], b: &[u8]) -> (f64, u8) {
    let mut total = 0u64;
    let mut worst = 0u8;
    for (x, y) in a.iter().zip(b) {
        let d = x.abs_diff(*y);
        total += d as u64;
        worst = worst.max(d);
    }
    (total as f64 / a.len() as f64, worst)
}

fn read_png(path: &Path) -> Option<(u32, u32, Vec<u8>)> {
    let file = std::io::BufReader::new(std::fs::File::open(path).ok()?);
    let decoder = png::Decoder::new(file);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    buf.truncate(info.buffer_size());
    Some((info.width, info.height, buf))
}

/// The cover goes where the deck's frame is, and nowhere else.
///
/// The scene golden covers the backdrop, which is drawn over the whole window, but the
/// deck only has room for a cover at sizes the scene isn't rendered at. This draws the
/// same pass on its own and checks where the pixels landed.
#[test]
fn the_cover_fills_its_frame() {
    const SIDE: u32 = 64;
    let Some((device, queue, _, _)) = open_gpu() else {
        eprintln!("no GPU here; skipping the cover render");
        return;
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("cover"),
        size: wgpu::Extent3d {
            width: SIDE,
            height: SIDE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let view = target.create_view(&wgpu::TextureViewDescriptor::default());
    let artwork = ArtworkPass::new(
        &device,
        wgpu::TextureFormat::Rgba8Unorm,
        bloom::TARGET_FORMAT,
    );
    let cover = artwork.upload(&device, &queue, &Picture::solid(8, 8, [255, 0, 0, 255]));
    let frame = Rect::new(16, 8, 32, 24);
    artwork.prepare(&queue, (SIDE, SIDE), frame, 1.0, 0.0);

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("cover"),
    });
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("cover"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();
        artwork.draw_deck(&mut pass, &cover);
    }
    let shot = Readback::record(&device, &mut encoder, &target);
    queue.submit([encoder.finish()]);
    let (w, _h, pixels) = shot.pixels(&device).expect("the frame reads back");
    let at = |x: u32, y: u32| {
        let i = ((y * w + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2]]
    };
    // Inside the frame, the cover; a pixel out on every side, the black it was cleared
    // to. The corners catch a quad that is flipped or off by a row.
    assert_eq!(at(17, 9), [255, 0, 0], "the top left of the frame");
    assert_eq!(at(46, 30), [255, 0, 0], "the bottom right of the frame");
    assert_eq!(at(15, 20), [0, 0, 0], "left of the frame");
    assert_eq!(at(48, 20), [0, 0, 0], "right of the frame");
    assert_eq!(at(30, 6), [0, 0, 0], "above the frame");
    assert_eq!(at(30, 33), [0, 0, 0], "below the frame");
}

#[test]
fn the_scene_renders_as_it_did() {
    let Some((device, queue, adapter, software)) = open_gpu() else {
        eprintln!("no GPU here; skipping the golden render");
        return;
    };
    let (width, height, pixels) = render(&device, &queue);
    assert_eq!((width, height), (WIDTH, HEIGHT));
    // Every pass drew something: a blank or single-colour frame means one silently did
    // nothing.
    let distinct = {
        let mut seen = std::collections::HashSet::new();
        for px in pixels.as_chunks::<4>().0.iter().step_by(37) {
            seen.insert([px[0], px[1], px[2]]);
        }
        seen.len()
    };
    assert!(distinct > 200, "the frame has only {distinct} colours");

    let slug: String = adapter
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let golden = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/golden")
        .join(format!("scene.{}.png", slug.trim_matches('-')));
    if !software {
        eprintln!("{adapter} is a hardware GPU: checked the frame, not its pixels");
        return;
    }
    match read_png(&golden) {
        Some((w, h, want)) if (w, h) == (width, height) => {
            let (mean, worst) = difference(&pixels, &want);
            if mean > MEAN_TOLERANCE || worst > MAX_TOLERANCE {
                let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scene.png");
                let _ = write_png(&out, width, height, &pixels);
                panic!(
                    "the render drifted from {}: mean {mean:.2}, worst {worst} (this one is in {})",
                    golden.display(),
                    out.display()
                );
            }
        }
        _ => {
            let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("scene.png");
            write_png(&out, width, height, &pixels).expect("writing the new picture");
            eprintln!(
                "no golden for {adapter} yet; this render is in {}. Copy it to {} to adopt it.",
                out.display(),
                golden.display()
            );
        }
    }
}
