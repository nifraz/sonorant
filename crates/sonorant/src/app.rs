//! The window, the event loop and the frame loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sonorant_core::engine::{AnalysisConfig, GRID_BINS};
use sonorant_core::palette::{self, Lut};
use sonorant_core::runtime::PaneCurves;
use sonorant_core::settings::{CurveStyle, Settings};
use sonorant_core::store::{self, Store};
use sonorant_render::{
    BandLayout, CurveData, CurveLook, CurvePass, CurveView, DeckState, HistoryStore, Layer,
    Overlay, PaneView, Readback, Rect, RowIn, ScopeLayout, SpectrogramPass, TrackInfo, WaveRing,
    axes, deck,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::audio::{Audio, Input};
use crate::gpu::Gpu;
use crate::options::Options;
use crate::pacing::FramePacing;
use crate::present::PresentMonitor;
use crate::ui::{self, Status, UiState};

/// How often the status line's numbers change, so they can be read.
const STATUS_EVERY: Duration = Duration::from_millis(250);
/// The colour bar's column down the right edge, in pixels.
const COLOUR_BAR_WIDTH: i32 = 40;
/// A frame interval this long is logged with where the time went.
const STALL: Duration = Duration::from_millis(100);

/// Events sent to the loop from other threads.
#[derive(Debug)]
pub enum UserEvent {
    AccessKit(egui_winit::accesskit_winit::Event),
}

impl From<egui_winit::accesskit_winit::Event> for UserEvent {
    fn from(e: egui_winit::accesskit_winit::Event) -> Self {
        UserEvent::AccessKit(e)
    }
}

pub struct App {
    options: Options,
    proxy: EventLoopProxy<UserEvent>,
    running: Option<Running>,
    error: Option<String>,
    started: Instant,
}

struct Running {
    /// Start-up's timings, until the first frame is on its way to the screen.
    steps: Option<Steps>,
    started: Instant,
    /// Where to save a screenshot, and when.
    screenshot: Option<(std::path::PathBuf, Duration)>,
    window: Arc<Window>,
    gpu: Gpu,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    history: HistoryStore,
    spectrogram: SpectrogramPass,
    curves: CurvePass,
    overlay: Overlay,
    lut: Lut,
    /// The newest curves and the range they were measured against.
    latest: Vec<PaneCurves>,
    range: (f64, f64),
    analysis: f64,
    bpm: f64,
    /// The spectral centroid, 0 to 1 along the display axis.
    brightness: f64,
    scope: (Vec<f32>, Vec<f32>),
    loudness: sonorant_core::dsp::LoudnessReadings,
    /// The average spectrum of each pane when the reference was taken, drawn in amber
    /// until it's dropped.
    reference: Option<Vec<Vec<f32>>>,
    audio: Option<Audio>,
    settings: Settings,
    store: Option<Store>,
    layout: ScopeLayout,
    /// The bottom band: waveform lanes and the centre deck.
    band: BandLayout,
    last_band: BandLayout,
    /// The colour bar down the right edge, or empty.
    colour_bar: Rect,
    waves: WaveRing,
    track: TrackInfo,
    columns: usize,
    held: Option<u64>,
    pacing: FramePacing,
    presented: PresentMonitor,
    /// How long the last frame's CPU work took, from acquire to present.
    last_work: Duration,
    ui: UiState,
    applied: UiState,
    status: Status,
    status_at: Instant,
    occluded: bool,
}

impl App {
    pub fn new(options: Options, proxy: EventLoopProxy<UserEvent>) -> App {
        App {
            options,
            proxy,
            running: None,
            error: None,
            started: Instant::now(),
        }
    }

    /// Why the app stopped, if it was an error.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    fn start(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, String> {
        let mut steps = Steps::new(self.started);
        let (settings, store) = load_settings(self.options.settings_dir.as_deref());
        steps.mark("settings");

        // Capture opens on a thread of its own while the window and the GPU do.
        let input = match (&self.options.wav, &self.options.app) {
            (Some(path), _) => Input::File(path.clone()),
            (None, Some(app)) => Input::App(app.clone()),
            (None, None) => Input::System,
        };
        let columns = 512;
        let audio_config = AnalysisConfig::from_settings(&settings, columns);
        let opening = std::thread::Builder::new()
            .name("sonorant-open-capture".into())
            .spawn(move || Audio::start(&input, audio_config))
            .map_err(|e| format!("cannot start a thread: {e}"))?;

        let mut attributes = Window::default_attributes()
            .with_title("Sonorant")
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(320.0, 200.0))
            // AccessKit has to be attached before the window is first shown.
            .with_visible(false);
        if self.options.fullscreen {
            attributes = attributes.with_fullscreen(Some(Fullscreen::Borderless(None)));
        }
        let window = Arc::new(
            event_loop
                .create_window(attributes)
                .map_err(|e| e.to_string())?,
        );
        steps.mark("window");

        let gpu = Gpu::open(event_loop, window.clone(), &self.options)?;
        steps.mark("gpu");

        let egui_ctx = egui::Context::default();
        let mut egui_state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            event_loop,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        egui_state.init_accesskit(event_loop, &window, self.proxy.clone());
        let egui_renderer = egui_wgpu::Renderer::new(
            &gpu.device,
            gpu.config.format,
            egui_wgpu::RendererOptions::default(),
        );
        steps.mark("ui");

        // Five minutes at the settings' scroll speed.
        let rows = (settings.rows_per_second.max(1.0) * 300.0) as u32;
        let history = HistoryStore::new(&gpu.device, GRID_BINS as u32, rows);
        let mut spectrogram = SpectrogramPass::new(&gpu.device, gpu.view_format);
        spectrogram.bind(&gpu.device, &history);
        let lut = palette::build_lut(settings.palette);
        spectrogram.set_palette(&gpu.queue, &lut);
        let curves = CurvePass::new(&gpu.device, gpu.config.format);
        curves.set_palette(&gpu.queue, &lut);
        let overlay = Overlay::new(&gpu.device, &gpu.queue, gpu.config.format);
        steps.mark("renderer");

        let audio = match opening.join() {
            Ok(Ok(a)) => Some(a),
            Ok(Err(e)) => {
                log::error!("{e}");
                None
            }
            Err(_) => {
                log::error!("opening the capture failed");
                None
            }
        };
        steps.mark("capture");

        let mut ui = UiState::from_settings(&settings, self.options.fullscreen);
        ui.present_mode = gpu.config.present_mode;
        let refresh_hz = crate::present::compositor_refresh_hz().or_else(|| refresh_rate(&window));
        log::info!(
            "display: {} Hz, scale factor {:.2}; history holds {} rows",
            refresh_hz.map_or("unknown".to_owned(), |hz| format!("{hz:.2}")),
            window.scale_factor(),
            history.capacity()
        );
        window.set_visible(true);
        steps.mark("show");
        let now = Instant::now();
        Ok(Running {
            steps: Some(steps),
            started: self.started,
            screenshot: self.options.screenshot.clone().map(|path| {
                (
                    path,
                    Duration::from_secs_f64(self.options.screenshot_seconds),
                )
            }),
            status: Status {
                ..Status::default()
            },
            window,
            gpu,
            egui_ctx,
            egui_state,
            egui_renderer,
            history,
            spectrogram,
            curves,
            overlay,
            lut,
            latest: Vec::new(),
            range: (-95.0, -5.0),
            analysis: 0.0,
            bpm: 0.0,
            brightness: 0.0,
            scope: (Vec::new(), Vec::new()),
            loudness: sonorant_core::dsp::LoudnessReadings::default(),
            reference: None,
            audio,
            settings,
            store,
            layout: ScopeLayout::default(),
            band: BandLayout::default(),
            last_band: BandLayout::default(),
            colour_bar: Rect::EMPTY,
            waves: WaveRing::new(4096),
            track: TrackInfo::default(),
            columns,
            held: None,
            pacing: FramePacing::new(refresh_hz),
            presented: PresentMonitor::default(),
            last_work: Duration::ZERO,
            applied: ui.clone(),
            ui,
            status_at: now,
            occluded: false,
        })
    }

    fn finish(&mut self) {
        let Some(r) = &self.running else { return };
        let overall = r.pacing.overall();
        log::info!("frame pacing: {overall}");
        if let Some(counts) = r.presented.counts() {
            log::info!("presentation: {counts}");
        }
        if let Some(path) = &self.options.pacing_log {
            match r.pacing.write_csv(path) {
                Ok(()) => log::info!("wrote frame intervals to {}", path.display()),
                Err(e) => log::error!("cannot write {}: {e}", path.display()),
            }
        }
        if let Some(store) = &r.store
            && let Err(e) = store.save(&r.settings)
        {
            log::error!("cannot save settings in {}: {e}", store.dir().display());
        }
    }
}

/// The saved settings, bringing Nostalgia+'s over on the first run. A folder given on
/// the command line is used as it is, without the import.
fn load_settings(dir: Option<&std::path::Path>) -> (Settings, Option<Store>) {
    if let Some(dir) = dir {
        let store = Store::new(dir.to_path_buf());
        return (store.load(), Some(store));
    }
    let Some(dir) = store::default_dir() else {
        log::warn!("no settings folder; settings won't be kept");
        return (Settings::default(), None);
    };
    let store = Store::new(dir);
    if !store.exists() {
        for from in sonorant_core::legacy::nostalgia_plus_dirs() {
            match store.import_nostalgia_plus(&from) {
                Ok(Some(s)) => {
                    log::info!("brought Nostalgia+ settings over from {}", from.display());
                    return (s, Some(store));
                }
                Ok(None) => {}
                Err(e) => log::warn!("cannot import from {}: {e}", from.display()),
            }
        }
    }
    (store.load(), Some(store))
}

/// How long each part of start-up took, for the log.
#[derive(Debug)]
struct Steps {
    started: Instant,
    last: Instant,
    parts: Vec<(&'static str, Duration)>,
}

impl Steps {
    fn new(started: Instant) -> Steps {
        Steps {
            started,
            last: started,
            parts: Vec::new(),
        }
    }

    fn mark(&mut self, name: &'static str) {
        let now = Instant::now();
        self.parts.push((name, now - self.last));
        self.last = now;
    }
}

impl std::fmt::Display for Steps {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let ms = |d: Duration| d.as_secs_f64() * 1000.0;
        write!(f, "{:.0} ms (", ms(self.last - self.started))?;
        for (i, (name, d)) in self.parts.iter().enumerate() {
            let sep = if i == 0 { "" } else { ", " };
            write!(f, "{sep}{name} {:.0}", ms(*d))?;
        }
        write!(f, ")")
    }
}

fn refresh_rate(window: &Window) -> Option<f64> {
    window
        .current_monitor()
        .and_then(|m| m.refresh_rate_millihertz())
        .map(|mhz| mhz as f64 / 1000.0)
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match self.start(event_loop) {
            Ok(r) => {
                r.window.request_redraw();
                self.running = Some(r);
            }
            Err(e) => {
                log::error!("{e}");
                self.error = Some(e);
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
        let Some(r) = &mut self.running else { return };
        match event {
            UserEvent::AccessKit(e) => {
                use egui_winit::accesskit_winit::WindowEvent as A;
                match e.window_event {
                    A::InitialTreeRequested => r.egui_ctx.enable_accesskit(),
                    A::ActionRequested(request) => {
                        r.egui_state.on_accesskit_action_request(request)
                    }
                    A::AccessibilityDeactivated => r.egui_ctx.disable_accesskit(),
                }
                r.window.request_redraw();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        let Some(r) = &mut self.running else { return };
        let response = r.egui_state.on_window_event(&r.window, &event);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                r.gpu.resize(size.width, size.height);
                r.window.request_redraw();
            }
            WindowEvent::Occluded(occluded) => {
                // Nothing is drawn while covered; analysis carries on.
                r.occluded = occluded;
                r.pacing.break_sequence();
                if !occluded {
                    r.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        logical_key,
                        state: ElementState::Pressed,
                        repeat: false,
                        ..
                    },
                ..
            } if !response.consumed => match logical_key {
                Key::Named(NamedKey::Space) => r.ui.frozen = !r.ui.frozen,
                Key::Character(c) if c.eq_ignore_ascii_case("a") => r.toggle_reference(),
                Key::Character(c) if c.eq_ignore_ascii_case("b") => {
                    r.settings.style = match r.settings.style {
                        CurveStyle::Line => CurveStyle::Bars,
                        CurveStyle::Bars => CurveStyle::Led,
                        CurveStyle::Led => CurveStyle::Line,
                    }
                }
                Key::Named(NamedKey::F11) => r.ui.fullscreen = !r.ui.fullscreen,
                Key::Named(NamedKey::Escape) if r.ui.fullscreen => r.ui.fullscreen = false,
                _ => {}
            },
            WindowEvent::RedrawRequested => {
                if r.occluded {
                    return;
                }
                r.frame();
                if r.ui.quit {
                    event_loop.exit();
                    return;
                }
                if let Some(limit) = self.options.pacing_seconds
                    && self.started.elapsed().as_secs_f64() >= limit
                {
                    event_loop.exit();
                    return;
                }
                r.window.request_redraw();
            }
            _ => {}
        }
        if response.repaint {
            r.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // Frames are driven by redraw requests; while the window is covered, look again
        // every so often in case the platform doesn't say when it's uncovered.
        let occluded = self.running.as_ref().is_some_and(|r| r.occluded);
        event_loop.set_control_flow(if occluded {
            ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250))
        } else {
            ControlFlow::Wait
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.finish();
    }
}

fn srgb_to_linear(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

impl Running {
    fn frame(&mut self) {
        let asked = Instant::now();
        let frame = match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) => f,
            wgpu::CurrentSurfaceTexture::Suboptimal(f) => {
                // Draw this one, then match the surface again.
                self.gpu.reconfigure();
                f
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.gpu.reconfigure();
                self.pacing.break_sequence();
                self.presented.reset();
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.pacing.break_sequence();
                self.presented.reset();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error");
                return;
            }
        };
        let now = Instant::now();
        if let Some(interval) = self.pacing.record(now)
            && interval >= STALL
        {
            let ms = |d: Duration| d.as_secs_f64() * 1000.0;
            log::warn!(
                "stall at {:.1} s: {:.0} ms between frames; the last frame's work took {:.1} \
                 ms, then {:.1} ms waiting for the swapchain",
                self.started.elapsed().as_secs_f64(),
                ms(interval),
                ms(self.last_work),
                ms(now - asked)
            );
        }
        self.apply_settings();

        // New rows and the newest analysis.
        let mut frames_now = 0.0;
        let mut rate = 48000.0;
        if let Some(audio) = &mut self.audio {
            audio.poll();
            let history = &mut self.history;
            let queue = &self.gpu.queue;
            let waves = &mut self.waves;
            audio.take_rows(|r| {
                history.push(
                    queue,
                    &RowIn {
                        index: r.index,
                        frames: r.frames,
                        floor_db: r.floor_db,
                        ceiling_db: r.ceiling_db,
                        a: &r.a,
                        b: &r.b,
                    },
                );
                waves.push(r.wave);
            });
            let s = audio.latest(now);
            self.latest.clone_from(&s.panes);
            self.range = (s.floor_db, s.ceiling_db);
            self.analysis = s.analysis_seconds;
            self.bpm = s.bpm;
            self.brightness = s.centroid;
            self.scope.0.clone_from(&s.scope_left);
            self.scope.1.clone_from(&s.scope_right);
            self.loudness = s.loudness;
            self.status.dropped = (s.dropped_frames, s.dropped_rows);
            rate = if s.sample_rate > 0.0 {
                s.sample_rate
            } else {
                audio.sample_rate
            };
            frames_now = audio.frames_now(now);
            self.status.capture = audio.status.to_string();
        } else {
            self.status.capture = "no capture".into();
        }

        if now.duration_since(self.status_at) >= STATUS_EVERY {
            self.status_at = now;
            self.status.pacing = self.pacing.recent();
            self.status.presented = self.presented.counts();
        }

        // The UI first, so the visuals know the space left to them.
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut area = egui::Rect::NOTHING;
        let present_modes = self.gpu.present_modes.clone();
        let mut full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            area = ui::show(ui, &mut self.ui, &present_modes);
        });
        let mut textures = std::mem::take(&mut full_output.textures_delta);
        self.egui_state
            .handle_platform_output(&self.window, full_output.platform_output);
        let ppp = full_output.pixels_per_point;
        let paint_jobs = self.egui_ctx.tessellate(full_output.shapes, ppp);
        let screen = egui_wgpu::ScreenDescriptor {
            size_in_pixels: [self.gpu.config.width, self.gpu.config.height],
            pixels_per_point: ppp,
        };
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.egui_renderer
                    .update_texture(&self.gpu.device, &self.gpu.queue, id, delta);
            }
        }

        // The panes, in physical pixels so the history keeps its detail at any scaling.
        let client = Rect::new(
            (area.min.x * ppp).round() as i32,
            (area.min.y * ppp).round() as i32,
            (area.width() * ppp).round() as i32,
            (area.height() * ppp).round() as i32,
        );
        let bar_w = if self.settings.show_color_bar {
            COLOUR_BAR_WIDTH
        } else {
            0
        };
        self.colour_bar = Rect::new(client.right() - bar_w, client.y, bar_w, client.h);
        let view = Rect::new(client.x, client.y, (client.w - bar_w).max(16), client.h);
        let band_h = BandLayout::height_for(&self.settings, view.h);
        let bounds = Rect::new(view.x, view.y, view.w, (view.h - band_h).max(16));
        self.layout = ScopeLayout::new(bounds, &self.settings);
        if self.layout.columns() != self.columns {
            self.columns = self.layout.columns();
            if let Some(audio) = &self.audio {
                audio.set_config(AnalysisConfig::from_settings(&self.settings, self.columns));
            }
        }
        let config = AnalysisConfig::from_settings(&self.settings, self.columns);
        let map = config.map(rate);
        let rps = self.settings.effective_rows_per_second();
        let frac = match (self.held, self.history.meta(0)) {
            (None, Some(newest)) => {
                ((frames_now - newest.frames as f64) / rate * rps).clamp(0.0, 1.0)
            }
            _ => 0.0,
        };
        let panes: Vec<PaneView> = self
            .layout
            .panes
            .iter()
            .enumerate()
            .map(|(i, p)| PaneView {
                rect: p.spectro.to_f32(),
                channel: i as u32,
                newest_left: p.curve_on_left,
                visible_rows: (p.spectro.w as f32 / self.ui.px_per_row as f32).max(1.0),
                frac: frac as f32,
                scale: map.scale,
                fmin: map.fmin as f32,
                fmax: map.fmax as f32,
                global_range: None,
                smooth_time: self.ui.smooth_time,
            })
            .collect();
        self.spectrogram
            .prepare(&self.gpu.queue, &self.history, &panes, self.held);
        self.prepare_curves();
        self.overlay
            .begin(self.gpu.config.width, self.gpu.config.height);
        let label_px = axes::label_px(&self.settings, ppp);
        // The deck's geometry depends on how wide a few strings are.
        let overlay = &mut self.overlay;
        let mut measure = |text: &str| {
            overlay
                .measure(text, sonorant_render::Face::Sans, label_px)
                .w
        };
        self.band = BandLayout::new(
            Rect::new(
                self.layout.bounds.x,
                self.layout.bounds.y,
                self.layout.bounds.w,
                self.layout.bounds.h + band_h,
            ),
            band_h,
            &self.settings,
            &self.layout.panes,
            &mut measure,
        );
        self.waves
            .resize(self.band.wave_a.w.max(self.band.wave_b.w).max(8) as usize);
        if log::log_enabled!(log::Level::Debug) && self.band != self.last_band {
            self.last_band = self.band;
            log::debug!("band {:?}", self.band);
        }

        // The status line is chrome over the image: it starts below the scale lane, and
        // labels drawn over the image step below it in turn.
        let chrome_top = self
            .layout
            .panes
            .first()
            .filter(|p| p.lane.h > 0 && p.lane.y <= p.bounds.y)
            .map_or(0.0, |p| p.lane.h as f32);
        let inset = if self.settings.show_status { 14.0 } else { 0.0 };
        let scales = axes::Scales {
            layout: &self.layout,
            settings: &self.settings,
            map: &map,
            floor_db: self.range.0,
            ceiling_db: self.range.1,
            px_per_second: rps * self.ui.px_per_row as f64,
            scale: ppp,
            alpha: 1.0,
            top_inset: inset,
            label_floor_y: if self.settings.show_status {
                chrome_top + inset + 16.0
            } else {
                0.0
            },
        };
        axes::draw(&mut self.overlay, &scales);
        let state = DeckState {
            loudness: &self.loudness,
            bpm: self.bpm,
            brightness_hz: map.x_to_freq(self.brightness * map.width as f64),
            scope_left: &self.scope.0,
            scope_right: &self.scope.1,
            track: &self.track,
            position: None,
            playing: false,
        };
        deck::draw_band(
            &mut self.overlay,
            &self.band,
            &self.layout.panes,
            &self.settings,
            &self.lut,
            &self.waves,
            &state,
            1.0,
            label_px,
        );
        if self.settings.show_color_bar {
            deck::draw_colour_bar(
                &mut self.overlay,
                self.colour_bar,
                &self.settings,
                &self.lut,
                self.range.0,
                self.range.1,
                1.0,
                label_px,
            );
        }
        if self.settings.show_status {
            let status = self.status_line();
            deck::draw_status(
                &mut self.overlay,
                &self.settings,
                &status,
                chrome_top,
                1.0,
                label_px,
            );
        }
        self.overlay.prepare(&self.gpu.device, &self.gpu.queue);

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        let extra = self.egui_renderer.update_buffers(
            &self.gpu.device,
            &self.gpu.queue,
            &mut encoder,
            &paint_jobs,
            &screen,
        );
        // The visuals through the sRGB view, then egui on the plain one.
        let linear = frame.texture.create_view(&wgpu::TextureViewDescriptor {
            format: Some(self.gpu.view_format),
            ..Default::default()
        });
        let plain = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        {
            let bg = palette::background(&self.lut);
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("visuals"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &linear,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: srgb_to_linear(bg.r),
                                g: srgb_to_linear(bg.g),
                                b: srgb_to_linear(bg.b),
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
            self.spectrogram.draw(&mut pass, &panes);
        }
        {
            // The furniture over the visuals, blended as GDI+ did (see colour.rs).
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("furniture"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &plain,
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
            self.overlay.draw(Layer::Under, &mut pass);
            self.curves.draw(&mut pass);
            self.overlay.draw(Layer::Over, &mut pass);
            self.overlay.draw(Layer::Top, &mut pass);
        }
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("ui"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &plain,
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
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }
        let shot = match &self.screenshot {
            Some((_, after)) if self.started.elapsed() >= *after => Some(Readback::record(
                &self.gpu.device,
                &mut encoder,
                &frame.texture,
            )),
            _ => None,
        };
        self.gpu
            .queue
            .submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);
        if let Some(shot) = shot
            && let Some((path, _)) = self.screenshot.take()
        {
            match shot.save(&self.gpu.device, &path) {
                Ok(()) => log::info!("saved the picture to {}", path.display()),
                Err(e) => log::error!("cannot save a screenshot: {e}"),
            }
            self.ui.quit = true;
        }
        self.overlay.finish();
        self.last_work = now.elapsed();
        self.presented.sample(&self.gpu.surface);
        if let Some(mut steps) = self.steps.take() {
            steps.mark("first frame");
            log::info!("start-up: {steps}");
        }

        for id in textures.free.drain() {
            self.egui_renderer.free_texture(&id);
        }
    }

    /// What the status line says: the source, the transform sizes, the channel pair,
    /// the frame rate, what analysis costs and the preset.
    fn status_line(&self) -> String {
        let (sizes, _) = self.settings.quality.profile();
        let resolution: Vec<String> = sizes
            .iter()
            .map(|&n| {
                if n >= 1024 {
                    format!("{}K", n / 1024)
                } else {
                    n.to_string()
                }
            })
            .collect();
        let pacing = self.pacing.recent();
        let missed = self.status.presented.map_or(pacing.missed, |c| c.repeated);
        let mut line = format!(
            "{}  |  {}  |  {}  |  {:.0} fps  |  {:.1} ms  |  {}",
            self.status.capture,
            resolution.join(" / "),
            self.settings.pair_mode.name(),
            pacing.fps,
            self.analysis * 1000.0,
            self.settings.preset.name(),
        );
        if missed > 0 {
            line += &format!("  |  {missed} missed");
        }
        let (frames, rows) = self.status.dropped;
        if frames > 0 || rows > 0 {
            line += &format!("  |  dropped {frames} frames, {rows} rows");
        }
        if self.ui.frozen {
            line += "  |  FROZEN";
        }
        line
    }

    /// Uploads the curve strips for this frame's layout.
    fn prepare_curves(&mut self) {
        // A reference taken at another size no longer lines up with the rows.
        if let Some(r) = &self.reference
            && r.iter()
                .zip(&self.latest)
                .any(|(r, p)| r.len() != p.display.len())
        {
            self.reference = None;
        }
        let look = CurveLook::new(&self.settings, &self.lut, 1.0);
        let (floor_db, ceiling_db) = self.range;
        let strips: Vec<(CurveView, CurveData<'_>)> = self
            .layout
            .panes
            .iter()
            .zip(&self.latest)
            .enumerate()
            .filter(|(_, (p, c))| c.display.len() == p.curve.h as usize)
            .map(|(i, (p, c))| {
                let view = CurveView {
                    rect: p.curve,
                    curve_on_left: p.curve_on_left,
                    floor_db,
                    ceiling_db,
                };
                let data = CurveData {
                    display: &c.display,
                    peak: &c.max,
                    average: &c.average,
                    minimum: &c.min,
                    reference: self
                        .reference
                        .as_ref()
                        .and_then(|r| r.get(i))
                        .map(Vec::as_slice),
                };
                (view, data)
            })
            .collect();
        self.curves
            .prepare(&self.gpu.device, &self.gpu.queue, &look, &strips);
    }

    /// Holds each pane's average spectrum as an amber reference, or drops the one held.
    fn toggle_reference(&mut self) {
        self.reference = match self.reference {
            Some(_) => None,
            None if !self.latest.is_empty() => {
                Some(self.latest.iter().map(|p| p.average.clone()).collect())
            }
            None => None,
        };
    }

    /// Applies whatever the menu or keys changed since the last frame.
    fn apply_settings(&mut self) {
        if self.ui == self.applied {
            return;
        }
        let (new, old) = (self.ui.clone(), self.applied.clone());
        if new.frozen != old.frozen {
            self.held = new.frozen.then(|| self.history.written());
        }
        if new.palette != old.palette {
            self.settings.palette = new.palette;
            self.lut = palette::build_lut(new.palette);
            self.spectrogram.set_palette(&self.gpu.queue, &self.lut);
            self.curves.set_palette(&self.gpu.queue, &self.lut);
        }
        let analysis_changed = new.rows_per_second != old.rows_per_second
            || new.pair_mode != old.pair_mode
            || new.scale != old.scale;
        self.settings.rows_per_second = new.rows_per_second as f64;
        self.settings.pair_mode = new.pair_mode;
        self.settings.scale = new.scale;
        if analysis_changed && let Some(audio) = &self.audio {
            audio.set_config(AnalysisConfig::from_settings(&self.settings, self.columns));
        }
        if new.present_mode != old.present_mode {
            self.gpu.set_present_mode(new.present_mode);
            self.pacing.break_sequence();
            self.presented.reset();
        }
        if new.fullscreen != old.fullscreen {
            self.window
                .set_fullscreen(new.fullscreen.then_some(Fullscreen::Borderless(None)));
            self.pacing.set_refresh_hz(
                crate::present::compositor_refresh_hz().or_else(|| refresh_rate(&self.window)),
            );
            self.presented.reset();
        }
        self.applied = self.ui.clone();
    }
}
