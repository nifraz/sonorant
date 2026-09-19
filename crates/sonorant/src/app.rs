//! The window, the event loop and the frame loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sonorant_core::engine::{AnalysisConfig, GRID_BINS};
use sonorant_core::palette;
use sonorant_core::settings::Settings;
use sonorant_core::store::{self, Store};
use sonorant_render::{HistoryStore, PaneView, Rect, RowIn, ScopeLayout, SpectrogramPass};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Fullscreen, Window, WindowId};

use crate::audio::{Audio, Input};
use crate::gpu::Gpu;
use crate::options::Options;
use crate::pacing::FramePacing;
use crate::ui::{self, Status, UiState};

/// How often the status line's numbers change, so they can be read.
const STATUS_EVERY: Duration = Duration::from_millis(250);

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
    window: Arc<Window>,
    gpu: Gpu,
    egui_ctx: egui::Context,
    egui_state: egui_winit::State,
    egui_renderer: egui_wgpu::Renderer,
    history: HistoryStore,
    spectrogram: SpectrogramPass,
    audio: Option<Audio>,
    settings: Settings,
    store: Option<Store>,
    layout: ScopeLayout,
    columns: usize,
    held: Option<u64>,
    pacing: FramePacing,
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
        let (settings, store) = load_settings();

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

        let mut desc = wgpu::InstanceDescriptor::new_with_display_handle_from_env(Box::new(
            event_loop.owned_display_handle(),
        ));
        if let Some(backends) = self.options.backends {
            desc.backends = backends;
        }
        let instance = wgpu::Instance::new(desc);
        let gpu = Gpu::new(&instance, window.clone(), &self.options)?;

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

        // Five minutes at the settings' scroll speed.
        let rows = (settings.rows_per_second.max(1.0) * 300.0) as u32;
        let history = HistoryStore::new(&gpu.device, GRID_BINS as u32, rows);
        let mut spectrogram = SpectrogramPass::new(&gpu.device, gpu.view_format);
        spectrogram.bind(&gpu.device, &history);
        spectrogram.set_palette(&gpu.queue, &palette::build_lut(settings.palette));

        let input = match (&self.options.wav, &self.options.app) {
            (Some(path), _) => Input::File(path.clone()),
            (None, Some(app)) => Input::App(app.clone()),
            (None, None) => Input::System,
        };
        let columns = 512;
        let audio = match Audio::start(&input, AnalysisConfig::from_settings(&settings, columns)) {
            Ok(a) => Some(a),
            Err(e) => {
                log::error!("{e}");
                None
            }
        };

        let mut ui = UiState::from_settings(&settings, self.options.fullscreen);
        ui.present_mode = gpu.config.present_mode;
        let refresh_hz = refresh_rate(&window);
        log::info!(
            "display: {} Hz, scale factor {:.2}; history holds {} rows",
            refresh_hz.map_or("unknown".to_owned(), |hz| format!("{hz:.2}")),
            window.scale_factor(),
            history.capacity()
        );
        window.set_visible(true);
        let now = Instant::now();
        log::info!(
            "first frame after {:.0} ms",
            now.duration_since(self.started).as_secs_f64() * 1000.0
        );
        Ok(Running {
            status: Status {
                adapter: gpu.adapter_info.name.clone(),
                backend: format!("{:?}", gpu.adapter_info.backend),
                present_mode: format!("{:?}", gpu.config.present_mode),
                ..Status::default()
            },
            window,
            gpu,
            egui_ctx,
            egui_state,
            egui_renderer,
            history,
            spectrogram,
            audio,
            settings,
            store,
            layout: ScopeLayout::default(),
            columns,
            held: None,
            pacing: FramePacing::new(refresh_hz),
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
        if let Some(path) = &self.options.pacing_log {
            match r.pacing.write_csv(path) {
                Ok(()) => log::info!("wrote frame intervals to {}", path.display()),
                Err(e) => log::error!("cannot write {}: {e}", path.display()),
            }
        }
        if let Some(store) = &r.store {
            if let Err(e) = store.save(&r.settings) {
                log::error!("cannot save settings in {}: {e}", store.dir().display());
            }
        }
    }
}

/// The saved settings, bringing Nostalgia+'s over on the first run.
fn load_settings() -> (Settings, Option<Store>) {
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
                if let Some(limit) = self.options.pacing_seconds {
                    if self.started.elapsed().as_secs_f64() >= limit {
                        event_loop.exit();
                        return;
                    }
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
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                self.pacing.break_sequence();
                return;
            }
            wgpu::CurrentSurfaceTexture::Validation => {
                log::error!("surface validation error");
                return;
            }
        };
        let now = Instant::now();
        self.pacing.record(now);
        self.apply_settings();

        // New rows and the newest analysis.
        let mut frames_now = 0.0;
        let mut rate = 48000.0;
        if let Some(audio) = &mut self.audio {
            audio.poll();
            let history = &mut self.history;
            let queue = &self.gpu.queue;
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
                )
            });
            let s = audio.latest(now);
            self.status.loudness = Some(s.loudness);
            self.status.bpm = s.bpm;
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
            self.status.size = [self.gpu.config.width, self.gpu.config.height];
            self.status.scale = self.window.scale_factor();
            self.status.rows_written = self.history.written();
            self.status.present_mode = format!("{:?}", self.gpu.config.present_mode);
        }

        // The UI first, so the visuals know the space left to them.
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut area = egui::Rect::NOTHING;
        let present_modes = self.gpu.present_modes.clone();
        let mut full_output = self.egui_ctx.run_ui(raw_input, |ui| {
            area = ui::show(ui, &mut self.ui, &self.status, &present_modes);
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
        let bounds = Rect::new(
            (area.min.x * ppp).round() as i32,
            (area.min.y * ppp).round() as i32,
            (area.width() * ppp).round() as i32,
            (area.height() * ppp).round() as i32,
        );
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
            let bg = palette::background(&palette::build_lut(self.settings.palette));
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
        self.gpu
            .queue
            .submit(extra.into_iter().chain(std::iter::once(encoder.finish())));
        self.window.pre_present_notify();
        self.gpu.queue.present(frame);

        for id in textures.free.drain() {
            self.egui_renderer.free_texture(&id);
        }
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
            self.spectrogram
                .set_palette(&self.gpu.queue, &palette::build_lut(new.palette));
        }
        let analysis_changed = new.rows_per_second != old.rows_per_second
            || new.pair_mode != old.pair_mode
            || new.scale != old.scale;
        self.settings.rows_per_second = new.rows_per_second as f64;
        self.settings.pair_mode = new.pair_mode;
        self.settings.scale = new.scale;
        if analysis_changed {
            if let Some(audio) = &self.audio {
                audio.set_config(AnalysisConfig::from_settings(&self.settings, self.columns));
            }
        }
        if new.present_mode != old.present_mode {
            self.gpu.set_present_mode(new.present_mode);
            self.pacing.break_sequence();
        }
        if new.fullscreen != old.fullscreen {
            self.window
                .set_fullscreen(new.fullscreen.then_some(Fullscreen::Borderless(None)));
            self.pacing.set_refresh_hz(refresh_rate(&self.window));
        }
        self.applied = self.ui.clone();
    }
}
