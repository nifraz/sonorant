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

use half::f16;
use sonorant_core::dsp::{FreqScale, FrequencyMap, LoudnessReadings};
use sonorant_core::engine::{GRID_BINS, GRID_FMAX, GRID_FMIN};
use sonorant_core::palette::{self, PaletteKind};
use sonorant_core::settings::{CameraView, Settings};
use sonorant_render::artwork::{ArtworkPass, Picture};
use sonorant_render::backdrop::{BackdropPass, FieldView};
use sonorant_render::band::BandLayout;
use sonorant_render::bloom::{self, Visuals};
use sonorant_render::colour::Rgba;
use sonorant_render::curves::{self, CurveData, CurveLook, CurvePass, CurveView};
use sonorant_render::deck::{DeckState, TrackInfo, WaveRing};
use sonorant_render::history::{HistoryStore, RowIn};
use sonorant_render::layout::{Rect, ScopeLayout};
use sonorant_render::overlay::{Face, Layer, Overlay};
use sonorant_render::phosphor::{Deposit, Phosphor, PhosphorLook, Sweep};
use sonorant_render::readback::{Readback, write_png};
use sonorant_render::spectrogram::{PaneView, SpectrogramPass};
use sonorant_render::waterfall::{self, Camera, Landscape, WaterfallPass};
use sonorant_render::{axes, deck};

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

    let backdrop = BackdropPass::new(device, bloom::TARGET_FORMAT);
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
    let band_h = BandLayout::height_for(&settings, view_rect.h, 1.0);
    let layout = ScopeLayout::new(
        Rect::new(0, 0, view_rect.w, view_rect.h - band_h),
        &settings,
        1.0,
    );
    let label_px = axes::label_px(&settings, 1.0);
    let mut measure = |text: &str| overlay.measure(text, Face::Sans, label_px).w;
    let band = BandLayout::new(
        view_rect,
        band_h,
        &settings,
        &layout.panes,
        1.0,
        &mut measure,
    );

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
    // The right channel's phase walks across the buffer, so the two halves the phosphor
    // is swept with trace different figures. Without that they are the same ellipse and
    // the golden cannot tell a fade from a redraw.
    let scope_r: Vec<f32> = (0..2048)
        .map(|i| {
            let along = i as f32 / 2048.0;
            (i as f32 * 0.05 + 0.4 + along * 1.6).sin() * 0.45
        })
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
            phosphor: true,
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

    // The phosphor screen, swept twice at a sixtieth of a second: the first sweep starts
    // it from black and the second fades that and writes over it, so the golden covers
    // the fade as well as the trace, the glow and the composite. A submit each, because
    // the points only reach the GPU at one (see `Phosphor::accumulate`).
    let mut phosphor = Phosphor::new(device, wgpu::TextureFormat::Rgba8Unorm);
    let look = PhosphorLook {
        colour: Rgba::rgb(palette::color_at(&lut, 0.80), 255),
        ..PhosphorLook::default()
    };
    let mut trace = Vec::new();
    for half in 0..2 {
        let from = half * scope.len() / 2;
        let to = from + scope.len() / 2;
        deck::goniometer_trace(
            band.deck.goniometer,
            &scope[from..to],
            &scope_r[from..to],
            scope.len() / 2,
            &mut trace,
        );
        let mut sweep = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("golden phosphor"),
        });
        phosphor.accumulate(
            device,
            queue,
            &mut sweep,
            &Sweep {
                rect: band.deck.goniometer,
                dt: 1.0 / 60.0,
                look: &look,
                points: &trace,
                deposit: Deposit::AlongThePath,
            },
        );
        queue.submit([sweep.finish()]);
    }

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
        // Over the cover and under the analysis, as the app draws it. Fixed numbers
        // rather than a clock, so the golden is the same picture every run.
        backdrop.draw(
            queue,
            &mut pass,
            &FieldView {
                size: (WIDTH, HEIGHT),
                time: 12.5,
                phase: 0.35,
                pulse: 0.8,
                brightness: 0.55,
                strength: f64::from(BACKDROP_STRENGTH),
                reactive: true,
                deep: Rgba::rgb(palette::color_at(&lut, 0.30), 255),
                hot: Rgba::rgb(palette::color_at(&lut, 0.92), 255),
            },
        );
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
        phosphor.draw(queue, &mut pass, (WIDTH, HEIGHT), 1.0);
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

/// The phosphor holds its light between frames and lets it go on a clock.
///
/// The golden covers one sweep and one fade; this covers a long run of them, which is
/// where the fade being exponential in real time either holds or drifts. A trace is
/// swept until it settles, then the sweeps stop and exactly one persistence passes: what
/// is left should be the hundredth the setting promises.
#[test]
fn the_phosphor_fades_on_a_clock() {
    const SIDE: u32 = 64;
    const PERSISTENCE: f64 = 0.5;
    let Some((device, queue, _, _)) = open_gpu() else {
        eprintln!("no GPU here; skipping the phosphor fade");
        return;
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("phosphor"),
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
    let mut phosphor = Phosphor::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let rect = Rect::new(0, 0, SIDE as i32, SIDE as i32);
    let look = PhosphorLook {
        persistence: PERSISTENCE,
        // No halo: this is about what the accumulator holds, and a blur would mix the
        // lit row into the reading taken beside it.
        glow: 0.0,
        colour: Rgba::argb(255, 255, 255, 255),
        ..PhosphorLook::default()
    };
    // A line straight across the middle, swept every sixtieth of a second.
    let line: Vec<[f32; 2]> = (0..SIDE).map(|x| [x as f32, 32.0]).collect();
    let dt = 1.0 / 60.0;

    let mut sweep_and_read = |points: &[[f32; 2]], sweeps: usize| {
        // One submit a sweep, as a frame does: the points only reach the GPU at one.
        for _ in 0..sweeps {
            let mut one = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("phosphor"),
            });
            phosphor.accumulate(
                &device,
                &queue,
                &mut one,
                &Sweep {
                    rect,
                    dt,
                    look: &look,
                    points,
                    deposit: Deposit::AlongThePath,
                },
            );
            queue.submit([one.finish()]);
        }
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("phosphor"),
        });
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("phosphor"),
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
            phosphor.draw(&queue, &mut pass, (SIDE, SIDE), 1.0);
        }
        let shot = Readback::record(&device, &mut encoder, &target);
        queue.submit([encoder.finish()]);
        let (w, _h, pixels) = shot.pixels(&device).expect("the frame reads back");
        // The lit row, as encoded light: the composite writes sRGB, so undo that to
        // compare two readings as amounts of light rather than as pixel values.
        let i = ((32 * w + 32) * 4) as usize;
        let v = f64::from(pixels[i]) / 255.0;
        let lit = if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        };
        // Nothing should have spilled off the line.
        let j = ((10 * w + 32) * 4) as usize;
        assert_eq!(
            pixels[j], 0,
            "the phosphor lit a row the trace never crossed"
        );
        lit
    };

    // Two persistences of sweeping is well past settled.
    let settled = sweep_and_read(&line, (2.0 * PERSISTENCE / dt) as usize);
    assert!(settled > 0.05, "the trace barely registered: {settled}");
    // Then exactly one persistence with nothing drawn.
    let left = sweep_and_read(&[], (PERSISTENCE / dt).round() as usize);
    let ratio = left / settled;
    assert!(
        (0.005..0.02).contains(&ratio),
        "a hundredth should be left after one persistence, not {ratio} ({left} of {settled})"
    );
}

/// A curve that moves leaves a trail behind it, dimmest where it has been longest.
///
/// This is the whole point of the phosphor on the strips, and it is the one thing a
/// still picture cannot show. The curve is walked across the strip a pixel or so a
/// frame; afterwards the light at each place it stood should fall away with how long
/// ago it stood there, and nowhere it never reached should be lit at all.
#[test]
fn a_moving_curve_leaves_a_trail() {
    const SIDE: u32 = 64;
    const FRAMES: usize = 10;
    let Some((device, queue, _, _)) = open_gpu() else {
        eprintln!("no GPU here; skipping the curve trail");
        return;
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("trail"),
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
    let mut phosphor = Phosphor::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let rect = Rect::new(0, 0, SIDE as i32, SIDE as i32);
    let look = PhosphorLook {
        persistence: 0.5,
        // No halo, so a reading at one column is that column's own light.
        glow: 0.0,
        colour: Rgba::argb(255, 255, 255, 255),
        ..PhosphorLook::default()
    };
    let strip = CurveView {
        rect,
        curve_on_left: true,
        floor_db: -100.0,
        ceiling_db: -20.0,
    };
    // A flat curve, walked from a quarter across to three quarters, four pixels a frame.
    let at_column = |frame: usize| 16 + frame * 4;
    let mut trace = Vec::new();
    for frame in 0..FRAMES {
        let db = -100.0 + 80.0 * at_column(frame) as f32 / SIDE as f32;
        let display = vec![db; SIDE as usize];
        curves::phosphor_trace(&strip, &display, &mut trace);
        // A submit a frame: the points only reach the GPU at one, so recording all ten
        // into one command buffer would draw the tenth curve ten times over.
        let mut one = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("trail"),
        });
        phosphor.accumulate(
            &device,
            &queue,
            &mut one,
            &Sweep {
                rect,
                dt: 1.0 / 60.0,
                look: &look,
                points: &trace,
                deposit: Deposit::OncePerFrame,
            },
        );
        queue.submit([one.finish()]);
    }
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("trail"),
    });
    {
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("trail"),
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
        phosphor.draw(&queue, &mut pass, (SIDE, SIDE), 1.0);
    }
    let shot = Readback::record(&device, &mut encoder, &target);
    queue.submit([encoder.finish()]);
    let (w, _h, pixels) = shot.pixels(&device).expect("the frame reads back");
    let column = |x: usize| pixels[(32 * w as usize + x) * 4];

    // Newest first: every place the curve stood, in the order it left them.
    let trail: Vec<u8> = (0..FRAMES).rev().map(|f| column(at_column(f))).collect();
    eprintln!("the trail, newest first: {trail:?}");
    assert!(
        trail[0] > 40,
        "the curve itself barely registered: {trail:?}"
    );
    for (newer, older) in trail.iter().zip(&trail[1..]) {
        assert!(
            newer > older,
            "the trail should fade away behind the curve, not {trail:?}"
        );
    }
    assert!(
        *trail.last().unwrap() > 0,
        "half a persistence of trail should still be visible: {trail:?}"
    );
    // Ahead of the curve and behind the trail, nothing.
    assert_eq!(column(at_column(FRAMES - 1) + 8), 0, "ahead of the curve");
    assert_eq!(column(at_column(0) - 8), 0, "behind the trail");
}

/// Zooming and panning are nothing but what the pane asks the history for.
///
/// That is the claim the whole long-history view rests on, so it is worth pinning:
/// fewer rows across the same pane stretches time, and holding the view at an older row
/// shows what was there then. One loud row in a quiet history says exactly where the
/// image is looking.
#[test]
fn what_a_pane_asks_for_is_where_the_image_looks() {
    const WIDE: u32 = 64;
    const PUSHED: usize = 200;
    const LOUD: usize = 150;
    let Some((device, queue, _, _)) = open_gpu() else {
        eprintln!("no GPU here; skipping the history view");
        return;
    };
    let mut history = HistoryStore::new(&device, GRID_BINS as u32, 256);
    let quiet = vec![f16::from_f32(-96.0); GRID_BINS];
    let loud = vec![f16::from_f32(-6.0); GRID_BINS];
    for i in 0..PUSHED {
        let levels = if i == LOUD { &loud } else { &quiet };
        history.push(
            &queue,
            &RowIn {
                index: i as u64,
                frames: i as u64 * 800,
                floor_db: -96.0,
                ceiling_db: -6.0,
                a: levels,
                b: levels,
            },
        );
    }
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("history view"),
        size: wgpu::Extent3d {
            width: WIDE,
            height: 8,
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
    let mut spectrogram = SpectrogramPass::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    spectrogram.bind(&device, &history);
    spectrogram.set_palette(&queue, &palette::build_lut(PaletteKind::Magma));

    // The brightest column, or `None` when the loud row is nowhere in view.
    let mut brightest = |visible_rows: f32, held: Option<u64>| {
        let panes = [PaneView {
            rect: [0.0, 0.0, WIDE as f32, 8.0],
            channel: 0,
            newest_left: true,
            visible_rows,
            frac: 0.0,
            scale: FreqScale::Note,
            fmin: 20.0,
            fmax: 20000.0,
            global_range: None,
            smooth_time: false,
        }];
        spectrogram.prepare(&queue, &history, &panes, held);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("history view"),
        });
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("history view"),
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
            spectrogram.draw(&mut pass, &panes);
        }
        let shot = Readback::record(&device, &mut encoder, &target);
        queue.submit([encoder.finish()]);
        let (w, _h, pixels) = shot.pixels(&device).expect("the frame reads back");
        let row = 4 * w as usize * 4;
        let (mut best, mut at) = (0u8, 0usize);
        for x in 0..WIDE as usize {
            let v = pixels[row + x * 4 + 1];
            if v > best {
                (best, at) = (v, x);
            }
        }
        (best > 40).then_some(at)
    };

    // A row a pixel, live: the loud row is 49 rows old, so it lands about there.
    let live = brightest(WIDE as f32, None).expect("the loud row is in view");
    let age = PUSHED - 1 - LOUD;
    assert!(
        live.abs_diff(age) <= 2,
        "the loud row is {age} old and drew at {live}"
    );

    // Zoomed in to two pixels a row, half the history fits and the loud row falls off
    // the far edge.
    assert_eq!(
        brightest(WIDE as f32 / 2.0, None),
        None,
        "zooming in should have taken the loud row off the pane"
    );

    // Parked twenty rows past it, it comes back in near the newest edge.
    let parked = brightest(WIDE as f32, Some((LOUD + 20) as u64)).expect("in view when parked");
    assert!(
        parked <= 21,
        "parked twenty rows past it, it drew at {parked}"
    );

    // Zoomed in and parked together: the same row, twice as far across.
    let both = brightest(WIDE as f32 / 2.0, Some((LOUD + 20) as u64)).expect("in view");
    assert!(
        both.abs_diff(parked * 2) <= 3,
        "zoomed in it should be about twice as far across: {both} against {parked}"
    );
}

/// The backdrop adds light, answers the beat, and does nothing when told not to.
///
/// The scene golden has it in the picture, but at a backdrop's strength it moves the
/// mean by a third of a level out of 255, which is well inside the tolerance a golden
/// has to allow: switching the whole pass off would not fail it. This is the test that
/// would.
#[test]
fn the_backdrop_lights_the_ground_and_rides_the_beat() {
    const SIDE: u32 = 128;
    /// Where the ring's front is, as `backdrop.wgsl` puts it: the phase over the rings
    /// per unit of distance from the centre.
    const RINGS_PER_UNIT: f64 = 1.6;
    let Some((device, queue, _, _)) = open_gpu() else {
        eprintln!("no GPU here; skipping the backdrop");
        return;
    };
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("backdrop"),
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
    let attachment = target.create_view(&wgpu::TextureViewDescriptor::default());
    let backdrop = BackdropPass::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    let lut = palette::build_lut(PaletteKind::Magma);
    // Halfway round the beat, so the ring's front is well inside the picture.
    const PHASE: f64 = 0.5;
    let field = |strength: f64, pulse: f64, reactive: bool| FieldView {
        size: (SIDE, SIDE),
        time: 4.0,
        phase: PHASE,
        pulse,
        brightness: 0.5,
        strength,
        reactive,
        deep: Rgba::rgb(palette::color_at(&lut, 0.30), 255),
        hot: Rgba::rgb(palette::color_at(&lut, 0.92), 255),
    };
    // The light over the whole picture, and the row through the middle, so the ring can
    // be found as well as counted.
    let render = |view: &FieldView| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("backdrop"),
        });
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("backdrop"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &attachment,
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
            backdrop.draw(&queue, &mut pass, view);
        }
        let shot = Readback::record(&device, &mut encoder, &target);
        queue.submit([encoder.finish()]);
        let (w, _h, pixels) = shot.pixels(&device).expect("the frame reads back");
        let total: u64 = (0..SIDE as usize)
            .flat_map(|y| (0..SIDE as usize).map(move |x| (x, y)))
            .map(|(x, y)| u64::from(pixels[(y * w as usize + x) * 4]))
            .sum();
        // The middle row, right of centre, where the ring crosses it.
        let half = SIDE as usize / 2;
        let across: Vec<u8> = (half..SIDE as usize)
            .map(|x| pixels[(half * w as usize + x) * 4])
            .collect();
        (total, across)
    };

    // Switched off, nothing at all reaches the target.
    let (none, _) = render(&field(0.0, 1.0, true));
    assert_eq!(none, 0, "a backdrop of no strength drew something");

    // Switched on, light where there was none, and more of it for more strength.
    let (dim, _) = render(&field(0.2, 0.0, true));
    let (quiet, still) = render(&field(0.4, 0.0, true));
    let (bright, _) = render(&field(0.6, 0.0, true));
    assert!(
        0 < dim && dim < quiet && quiet < bright,
        "{dim} {quiet} {bright}"
    );

    // A hit lights the ring. A ring is a band, not the whole picture, so a fifth more
    // light over everything is a strong signal rather than a weak one.
    let (hit_total, hit) = render(&field(0.4, 1.0, true));
    assert!(
        hit_total > quiet * 6 / 5,
        "a hit barely showed: {quiet} to {hit_total}"
    );
    // Where the ring is, against the same row with no hit in it. The difference is the
    // ring and nothing else; the brightest pixel on its own would as likely be a crest
    // of the drift, which is there either way.
    let added: Vec<i32> = hit
        .iter()
        .zip(&still)
        .map(|(&a, &b)| i32::from(a) - i32::from(b))
        .collect();
    // The front is where the phase reaches a whole ring, in half-widths from the centre.
    // Inside it the wave's light; outside it, nothing yet. That edge is the thing worth
    // pinning: it is what makes the ring read as travelling rather than as a circle
    // being drawn, and it was the wrong way round the first time.
    let front = (PHASE / RINGS_PER_UNIT * f64::from(SIDE)) as usize;
    // The levels are small because this target is linear and eight bits; in the app the
    // composite encodes them and the same ring is four times brighter on screen.
    assert!(
        added[front - 1] >= 10,
        "no light behind the ring's front: {added:?}"
    );
    assert!(
        added[front] * 3 < added[front - 1],
        "light ahead of the ring's front, so it is not travelling: {added:?}"
    );
    // And the light falls away going back from the front, rather than being a band.
    assert!(added[front - 1] > added[front - 6] * 2, "{added:?}");

    // With the beat switched off the same hit changes nothing: the field only drifts.
    let (deaf, _) = render(&field(0.4, 1.0, false));
    assert_eq!(
        deaf, quiet,
        "the field answered a beat it was told to ignore"
    );
}

/// The landscape stands where the history says it should, and turns with the camera.
///
/// One loud band in a quiet history is a ridge; from the front it should stand near the
/// middle of the picture and above the flat ground, and swinging the camera round should
/// move it without the picture going empty. What this is really checking is that the
/// mesh reads the same history the flat view does, that the displacement goes upwards,
/// and that the camera reaches the shader at all.
#[test]
fn the_landscape_stands_where_the_history_says() {
    const WIDE: u32 = 192;
    const TALL: u32 = 144;
    const PUSHED: usize = 240;
    let Some((device, queue, _, _)) = open_gpu() else {
        eprintln!("no GPU here; skipping the landscape");
        return;
    };
    let mut history = HistoryStore::new(&device, GRID_BINS as u32, 256);
    // Quiet everywhere but a band a third of the way up the axis, in every row, so the
    // surface is a ridge running away from the viewer.
    let quiet = vec![f16::from_f32(-96.0); GRID_BINS];
    let mut ridged = quiet.clone();
    let middle = GRID_BINS / 3;
    for level in &mut ridged[middle - 24..middle + 24] {
        *level = f16::from_f32(-8.0);
    }
    for i in 0..PUSHED {
        history.push(
            &queue,
            &RowIn {
                index: i as u64,
                frames: i as u64 * 800,
                floor_db: -96.0,
                ceiling_db: -6.0,
                a: &ridged,
                b: &quiet,
            },
        );
    }
    let target = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("landscape"),
        size: wgpu::Extent3d {
            width: WIDE,
            height: TALL,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let attachment = target.create_view(&wgpu::TextureViewDescriptor::default());
    let mut waterfall = WaterfallPass::new(&device, wgpu::TextureFormat::Rgba8Unorm);
    waterfall.bind(&device, &history);
    waterfall.set_palette(&queue, &palette::build_lut(PaletteKind::Magma));

    // The brightest pixel, where it is, and how much of the picture is lit at all.
    let mut render = |camera: Camera, channel: u32| {
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("landscape"),
        });
        // The landscape loads what is there rather than clearing it, because in the app
        // the backdrop under it is its sky. Here there is no sky, so the last render
        // would still be showing through this one.
        encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("sky"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &attachment,
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
        waterfall.draw(
            &device,
            &queue,
            &mut encoder,
            waterfall::Target {
                view: &attachment,
                timestamps: None,
            },
            &history,
            &Landscape {
                camera,
                rect: Rect::new(0, 0, WIDE as i32, TALL as i32),
                framebuffer: (WIDE, TALL),
                held: None,
                sheen: true,
                channel,
                span_rows: 200.0,
                frac: 0.0,
                scale: FreqScale::Note,
                fmin: 20.0,
                fmax: 20000.0,
                global_range: None,
                background: Rgba::argb(255, 0, 0, 0),
            },
        );
        let shot = Readback::record(&device, &mut encoder, &target);
        queue.submit([encoder.finish()]);
        let (w, _h, pixels) = shot.pixels(&device).expect("the frame reads back");
        let mut best = (0u32, 0usize, 0usize);
        let mut lit = 0usize;
        for y in 0..TALL as usize {
            for x in 0..WIDE as usize {
                let i = (y * w as usize + x) * 4;
                let v = u32::from(pixels[i]) + u32::from(pixels[i + 1]) + u32::from(pixels[i + 2]);
                if v > best.0 {
                    best = (v, x, y);
                }
                // Anything at all: the sheen over flat ground is a handful of levels
                // in a linear eight-bit target, where in the app the composite encodes
                // it into something you can see.
                if v > 3 {
                    lit += 1;
                }
            }
        }
        (best, lit)
    };

    // Nothing is drawn before the pass is told where to look, so the cleared target has
    // to be the thing that changed.
    let ((bright, x, y), lit) = render(Camera::of(CameraView::Classic), 0);
    assert!(bright > 150, "the ridge barely showed: {bright}");
    // The ground is lit too, so a good part of the picture is the surface rather than
    // the sky behind it.
    let all = (WIDE * TALL) as usize;
    assert!(
        lit > all / 4 && lit < all,
        "{lit} of {all} pixels are surface"
    );
    // The band is a third of the way up a note axis, so left of centre and above the
    // bottom of the ground, which is what says the displacement went upwards.
    assert!(
        x < WIDE as usize / 2,
        "the ridge should be left of centre, not at {x}"
    );
    assert!(
        (TALL as usize / 8..TALL as usize * 3 / 4).contains(&y),
        "the ridge should stand above the ground, not at {y}"
    );

    // The other channel is quiet everywhere, so it is ground and no ridge.
    let ((quiet_peak, _, _), _) = render(Camera::of(CameraView::Classic), 1);
    assert!(
        quiet_peak * 3 < bright,
        "a silent channel drew a ridge: {quiet_peak} against {bright}"
    );

    // Swung round to the side, the ridge moves but the landscape is still there.
    let ((side, side_x, _), side_lit) = render(Camera::of(CameraView::Side), 0);
    assert!(side > 150 && side_lit > all / 10, "{side} {side_lit}");
    assert!(
        side_x.abs_diff(x) > WIDE as usize / 20,
        "turning the camera moved nothing: {x} to {side_x}"
    );
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
