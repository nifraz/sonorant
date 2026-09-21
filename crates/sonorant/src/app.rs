//! The window, the event loop and the frame loop.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sonorant_core::engine::{AnalysisConfig, GRID_BINS};
use sonorant_core::media::Transport;
use sonorant_core::menu::{self, Capture, Presentation};
use sonorant_core::palette::{self, Lut};
use sonorant_core::runtime::PaneCurves;
use sonorant_core::settings::{FrameCap, Number, RenderQuality, Settings};
use sonorant_core::store::{self, Store};
use sonorant_render::{
    ArtworkPass, BackdropPass, BandLayout, BeatPhase, Camera, CurveData, CurveLook, CurvePass,
    CurveView, DeckState, Deposit, FieldView, GpuTimer, HistoryStore, Landscape, Layer, Overlay,
    PaneView, Phosphor, PhosphorLook, QuickBar, Readback, Reading, Readout, Rect, Rgba, RowIn,
    ScopeLayout, SpectrogramPass, Sweep, Visuals, WaterfallPass, WaveRing, axes, backdrop, bloom,
    curves, deck, hover, phosphor, quickbar, waterfall,
};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoopProxy};
use winit::keyboard::Key;
use winit::window::{Fullscreen, Window, WindowId};

use crate::audio::{Audio, Input};
use crate::chrome::Chrome;
use crate::gpu::Gpu;
use crate::idle::{IDLE_INTERVAL, Idle};
use crate::latency::Latency;
use crate::nowplaying::NowPlaying;
use crate::options::Options;
use crate::pacing::FramePacing;
use crate::present::PresentMonitor;
use crate::timeview::{Strip, TimeView};
use crate::ui::{Around, Ask, Shell, Status};
use sonorant_core::source::SourceStatus;

#[cfg(target_os = "linux")]
use sonorant_platform::linux::{Appearance, ScreenAwake, appearance};
#[cfg(windows)]
use sonorant_platform::windows::{Appearance, ScreenAwake, appearance};

/// What the app calls itself to the desktop: the Wayland app id, the X11 window class,
/// the `.desktop` file's name and the Flatpak id, which all have to be the same string
/// for a compositor to pair the window with its icon and its entry.
///
/// Linux alone, because nothing on Windows reads it: there a window is known by its
/// own icon and title, and a constant nothing uses is dead code that CI fails on.
#[cfg(target_os = "linux")]
const APP_ID: &str = "io.github.nifraz.Sonorant";
/// The pixels a side of the icon handed to the window. X11 and Windows scale this to
/// whatever the title bar and the switcher want; Wayland ignores it and takes the icon
/// from the `.desktop` file [`APP_ID`] names.
const WINDOW_ICON: u32 = 64;

/// How often the status line's numbers change, so they can be read.
const STATUS_EVERY: Duration = Duration::from_millis(250);
/// The colour bar's column down the right edge, in pixels.
const COLOUR_BAR_WIDTH: i32 = 40;
/// A frame interval this long is logged with where the time went.
const STALL: Duration = Duration::from_millis(100);
/// How much more history the landscape shows than a flat pane does at the same zoom.
/// A pane is as deep as it is wide here, so the surface recedes into a useful stretch of
/// the past rather than showing the same few seconds lying down.
const WATERFALL_DEPTH: f32 = 2.5;
/// Bytes one row of history costs on the GPU: the grid, as Float16 pairs.
const ROW_BYTES: u64 = GRID_BINS as u64 * 4;
/// The most the history store may take. The plan's memory target is under 300 MB with
/// five minutes of history, which at the default speed is about 150 MB, so this leaves
/// a slower speed or a longer setting somewhere to grow into without the two of them
/// together asking for a gigabyte.
const HISTORY_BUDGET: u64 = 256 << 20;

/// How far the wheel may zoom time, either side of the pixels-a-row setting. Past
/// twenty a row is wider than most windows, and under a twentieth a pixel is twenty
/// rows, which is where the history runs out long before the zoom does.
const ZOOM_RANGE: (f64, f64) = (0.05, 20.0);
/// The longest gap the phosphor treats as one frame. A longer one means the window was
/// hidden or the app was stalled, where fading a quarter of a second at a time is both
/// right to look at and a bound on how much trace one frame can draw.
const PHOSPHOR_GAP: f64 = 0.25;
/// How long a capture target that could not be opened is left alone before it is
/// tried again.
const CAPTURE_RETRY: Duration = Duration::from_secs(5);
/// How often a covered window looks again, in case the platform doesn't say when it is
/// uncovered.
const IDLE_LOOK: Duration = Duration::from_millis(250);

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
    /// The visuals' floating-point target and its glow.
    visuals: Visuals,
    /// What each pass costs on the GPU.
    timer: GpuTimer,
    curves: CurvePass,
    /// The history as a landscape, and where the eye is looking at it from.
    waterfall: WaterfallPass,
    camera: Camera,
    /// The field of light behind the analysis, and where the beat is in it.
    backdrop: BackdropPass,
    beat: BeatPhase,
    /// The goniometer's phosphor screen, when the setting has it drawing the figure.
    phosphor: Phosphor,
    /// A phosphor screen per curve strip, when the setting has them drawing the line.
    /// One each, because a screen is one accumulator over one rectangle.
    curve_phosphor: [Phosphor; 2],
    /// This frame's trace, in whichever screen's own pixels. Kept to be refilled rather
    /// than reallocated each frame.
    trace: Vec<[f32; 2]>,
    overlay: Overlay,
    lut: Lut,
    /// The newest curves and the range they were measured against.
    latest: Vec<PaneCurves>,
    range: (f64, f64),
    analysis: f64,
    /// How much of the last onset is left, 1 at the beat and decaying.
    pulse: f64,
    bpm: f64,
    /// The spectral centroid, 0 to 1 along the display axis.
    brightness: f64,
    /// Degrees the palette is currently rotated by.
    hue_shift: f64,
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
    /// The strip of buttons over the image, or empty.
    quick_bar: QuickBar,
    waves: WaveRing,
    /// The cover and the backdrop, and the quads they are drawn as.
    artwork: ArtworkPass,
    now_playing: NowPlaying,
    /// Whether capture may be re-pointed at the player. A `--wav` or `--app` run is
    /// the user saying exactly what to capture, so it is left alone.
    capture_follows: bool,
    /// A target that could not be opened, and when, so it isn't retried every frame
    /// and isn't given up on for good either.
    capture_failed: Option<(Input, Instant)>,
    columns: usize,
    held: Option<u64>,
    /// Where along the history the image is looking: the zoom, and how far back.
    view: TimeView,
    /// The row `Space` froze the image on, which is not the same thing as the view
    /// having been moved. See `settle_view`.
    frozen_at: Option<u64>,
    /// The row the wheel or a drag parked the image's newest edge on. Absolute, not a
    /// distance back from now: rows keep arriving behind a parked view, and a distance
    /// would carry the image forward with them.
    parked_at: Option<u64>,
    /// Rows the store was last asked for, so the length setting and the scroll speed
    /// are only acted on when one of them moves.
    history_rows: u32,
    pacing: FramePacing,
    presented: PresentMonitor,
    /// How long the last frame's CPU work took, from acquire to present.
    last_work: Duration,
    /// The menu, the help window and the state they share with the keys.
    shell: Shell,
    /// The settings and the session as the frame loop last acted on them, so a change
    /// is noticed once rather than reapplied every frame.
    applied: Settings,
    applied_session: menu::Session,
    /// What analysis was last told to do, so it is only told again when it changes.
    config: AnalysisConfig,
    /// The user's saved presets, reread whenever one is saved or deleted.
    presets: Vec<String>,
    /// The desktop's accent colour and whether it is dark, which stand in for the skin
    /// colours Nostalgia+ took from MusicBee.
    appearance: Appearance,
    /// Holds the screen on while there is something to watch.
    awake: ScreenAwake,
    /// How long there has been nothing to draw, and whether that is long enough to
    /// draw it slowly.
    idle: Idle,
    /// What `idle` said on the last frame, which is what this one is paced by.
    idling: bool,
    /// How old the audio being drawn is, frame by frame.
    latency: Latency,
    /// What the status line last said, kept for the screen reader's description of the
    /// picture.
    reading: String,
    /// How long egui says it can wait before it wants drawing again: zero while a menu
    /// is animating, and as good as forever when nothing is.
    egui_wait: Duration,
    /// Where the pointer is over the visuals, in points, or `None`.
    pointer: Option<egui::Pos2>,
    /// Whether the right-click menu is open, so the chrome doesn't fade under it.
    menu_open: bool,
    /// Whether the pointer was double-clicked over the visuals this frame.
    double_clicked: bool,
    /// Whether it was clicked, for the quick bar's buttons.
    clicked: bool,
    /// How far the furniture has faded, and whether the pointer is shown with it.
    chrome: Chrome,
    /// Whether the pointer is currently hidden, so it is only asked to change when it
    /// has to be.
    cursor_hidden: bool,
    /// When the next frame is due, under a frame-rate cap.
    next_frame: Option<Instant>,
    /// Where the pointer was last frame, so the fade answers movement rather than
    /// presence.
    pointer_was: Option<egui::Pos2>,
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
        let input_kind = input.clone();
        let audio_config = AnalysisConfig::from_settings(&settings, columns);
        let opening = std::thread::Builder::new()
            .name("sonorant-open-capture".into())
            .spawn(move || Audio::start(&input, audio_config))
            .map_err(|e| format!("cannot start a thread: {e}"))?;

        let mut attributes = Window::default_attributes()
            .with_title("Sonorant")
            .with_window_icon(
                winit::window::Icon::from_rgba(
                    sonorant_render::icon::rgba(WINDOW_ICON),
                    WINDOW_ICON,
                    WINDOW_ICON,
                )
                .ok(),
            )
            .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
            .with_min_inner_size(winit::dpi::LogicalSize::new(320.0, 200.0))
            // AccessKit has to be attached before the window is first shown.
            .with_visible(false);
        // Both take the same pair, so the call has to say which trait it means.
        #[cfg(target_os = "linux")]
        {
            use winit::platform::wayland::WindowAttributesExtWayland;
            use winit::platform::x11::WindowAttributesExtX11;
            attributes = WindowAttributesExtWayland::with_name(attributes, APP_ID, "");
            attributes = WindowAttributesExtX11::with_name(attributes, APP_ID, "sonorant");
        }
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

        let rows = history_rows(&settings);
        let history = HistoryStore::new(&gpu.device, GRID_BINS as u32, rows);
        log::info!(
            "history: {} rows, {:.1} minutes at {:.0} rows a second, {} MB",
            history.capacity(),
            history.capacity() as f64 / settings.effective_rows_per_second().max(1.0) / 60.0,
            settings.effective_rows_per_second(),
            history.capacity() * ROW_BYTES / (1 << 20),
        );
        let mut spectrogram = SpectrogramPass::new(&gpu.device, bloom::TARGET_FORMAT);
        spectrogram.bind(&gpu.device, &history);
        let lut = palette::build_lut(settings.palette);
        spectrogram.set_palette(&gpu.queue, &lut);
        let curves = CurvePass::new(&gpu.device, gpu.config.format);
        let quality = quality_index(settings.render_quality);
        let mut waterfall = WaterfallPass::new(&gpu.device, bloom::TARGET_FORMAT);
        waterfall.set_mesh(&gpu.device, waterfall::MESH[quality]);
        waterfall.bind(&gpu.device, &history);
        waterfall.set_palette(&gpu.queue, &lut);
        let backdrop = BackdropPass::new(&gpu.device, bloom::TARGET_FORMAT);
        let phosphor = Phosphor::new(&gpu.device, gpu.config.format);
        let curve_phosphor = [
            Phosphor::new(&gpu.device, gpu.config.format),
            Phosphor::new(&gpu.device, gpu.config.format),
        ];
        curves.set_palette(&gpu.queue, &lut);
        let overlay = Overlay::new(&gpu.device, &gpu.queue, gpu.config.format);
        let artwork = ArtworkPass::new(&gpu.device, gpu.config.format, bloom::TARGET_FORMAT);
        let mut visuals = Visuals::new(
            &gpu.device,
            gpu.view_format,
            gpu.config.width,
            gpu.config.height,
        );
        visuals.set_divisor(&gpu.device, bloom::GLOW_DIVISORS[quality]);
        let timer = GpuTimer::new(
            &gpu.device,
            &gpu.queue,
            &[
                "visuals",
                "landscape",
                "glow",
                "composite",
                "furniture",
                "ui",
            ],
        );
        if !timer.is_available() {
            log::info!("this GPU has no timestamp queries; pass times won't be shown");
        }
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

        let appearance = appearance();
        log::info!(
            "desktop: {}, accent {}",
            if appearance.dark { "dark" } else { "light" },
            appearance
                .accent
                .map_or("none".to_owned(), |c| c.to_string())
        );
        egui_ctx.set_theme(if appearance.dark {
            egui::Theme::Dark
        } else {
            egui::Theme::Light
        });
        let mut shell = Shell::new(self.options.fullscreen);
        shell.session.presentation = presentation_of(gpu.config.present_mode);
        let presets = store.as_ref().map(Store::list_presets).unwrap_or_default();
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
            visuals,
            timer,
            curves,
            overlay,
            lut,
            latest: Vec::new(),
            range: (-95.0, -5.0),
            analysis: 0.0,
            pulse: 0.0,
            bpm: 0.0,
            brightness: 0.0,
            hue_shift: 0.0,
            scope: (Vec::new(), Vec::new()),
            waterfall,
            camera: Camera::default(),
            backdrop,
            beat: BeatPhase::default(),
            phosphor,
            curve_phosphor,
            trace: Vec::new(),
            loudness: sonorant_core::dsp::LoudnessReadings::default(),
            reference: None,
            audio,
            store,
            layout: ScopeLayout::default(),
            band: BandLayout::default(),
            last_band: BandLayout::default(),
            colour_bar: Rect::EMPTY,
            quick_bar: QuickBar::default(),
            waves: WaveRing::new(4096),
            artwork,
            now_playing: NowPlaying::start(),
            capture_follows: matches!(input_kind, Input::System),
            capture_failed: None,
            columns,
            held: None,
            view: TimeView::default(),
            frozen_at: None,
            parked_at: None,
            history_rows: rows,
            pacing: FramePacing::new(refresh_hz),
            presented: PresentMonitor::default(),
            last_work: Duration::ZERO,
            applied: settings.clone(),
            applied_session: shell.session.clone(),
            config: AnalysisConfig::from_settings(&settings, columns),
            appearance,
            awake: ScreenAwake::new(),
            idle: Idle::default(),
            idling: false,
            latency: Latency::default(),
            reading: String::new(),
            egui_wait: Duration::ZERO,
            pointer: None,
            menu_open: false,
            double_clicked: false,
            clicked: false,
            chrome: Chrome::new(now),
            cursor_hidden: false,
            next_frame: None,
            pointer_was: None,
            settings,
            presets,
            shell,
            status_at: now,
            occluded: false,
        })
    }

    fn finish(&mut self) {
        let Some(r) = &self.running else { return };
        let overall = r.pacing.overall();
        log::info!("frame pacing: {overall}");
        let latency = r.latency.overall();
        if latency.readings > 0 {
            // What the app is answerable for, and what it is not: the sound was
            // already a graph cycle old when the system handed it over, and this
            // frame reaches the photon a refresh or more after it is drawn.
            log::info!(
                "audio to drawn: {latency}, not counting the capture buffer before it \
                 or the refresh after it"
            );
        }
        if r.timer.is_available() {
            log::info!(
                "GPU time: {:.2} ms a frame at {}x{} ({})",
                r.timer.total_ms(),
                r.gpu.config.width,
                r.gpu.config.height,
                r.timer.report()
            );
        }
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
        // egui answers every event with "draw again", including the redraw it has just
        // been handed. Asking for the next one there would ask for it at once, every
        // time, and no cap or idle rate below would ever be reached: the frame the app
        // draws for itself is scheduled where the frame is finished and nowhere else.
        // What egui wants for its own animations is carried by `egui_wait` instead.
        let asked_again = response.repaint && !matches!(event, WindowEvent::RedrawRequested);

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                r.gpu.resize(size.width, size.height);
                r.visuals
                    .resize(&r.gpu.device, r.gpu.config.width, r.gpu.config.height);
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
            } if !response.consumed => {
                // The key is looked up in the menu itself, so a shortcut can't drift
                // away from the item it belongs to.
                r.chrome.stir(Instant::now());
                r.press(&logical_key);
            }
            WindowEvent::RedrawRequested => {
                if r.occluded {
                    return;
                }
                r.frame();
                if r.shell.session.quit {
                    event_loop.exit();
                    return;
                }
                if let Some(limit) = self.options.pacing_seconds
                    && self.started.elapsed().as_secs_f64() >= limit
                {
                    event_loop.exit();
                    return;
                }
                // When the next frame is due: the cap, or no wait at all when there
                // is none, and never later than egui asked for. Uncapped with nothing
                // animating that is at once, and the swapchain does the pacing;
                // otherwise `about_to_wait` sleeps until then rather than spinning.
                let wait = r
                    .frame_interval()
                    .unwrap_or(Duration::ZERO)
                    .min(r.egui_wait);
                if wait.is_zero() {
                    r.next_frame = None;
                    r.window.request_redraw();
                } else {
                    r.next_frame = Some(Instant::now() + wait);
                }
            }
            _ => {}
        }
        if asked_again {
            r.window.request_redraw();
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A timed run ends on time whatever the window is doing. The deadline is
        // otherwise noticed where frames are drawn, and a window nobody can see is
        // asked for none: a Wayland compositor stops delivering frame callbacks to a
        // window it isn't showing, and doesn't always say it is occluded either, so
        // the loop would sit on a redraw request that never comes. The waits below are
        // the other half of that: `Wait` has nothing to wake it, so a run with a
        // deadline has to sleep until the deadline rather than until something happens.
        let deadline = self
            .options
            .pacing_seconds
            .map(|limit| self.started + Duration::from_secs_f64(limit));
        if deadline.is_some_and(|at| Instant::now() >= at) {
            event_loop.exit();
            return;
        }
        // Frames are driven by redraw requests; while the window is covered, look again
        // every so often in case the platform doesn't say when it's uncovered.
        let Some(r) = &mut self.running else {
            event_loop.set_control_flow(wait_for(deadline));
            return;
        };
        if r.occluded {
            event_loop.set_control_flow(wake_at(Instant::now() + IDLE_LOOK, deadline));
            return;
        }
        event_loop.set_control_flow(match r.next_frame {
            None => wait_for(deadline),
            Some(due) if Instant::now() >= due => {
                r.next_frame = None;
                r.window.request_redraw();
                wait_for(deadline)
            }
            Some(due) => wake_at(due, deadline),
        });
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.finish();
    }
}

/// Sleep until something happens, or until the deadline where there is one.
fn wait_for(deadline: Option<Instant>) -> ControlFlow {
    match deadline {
        Some(at) => ControlFlow::WaitUntil(at),
        None => ControlFlow::Wait,
    }
}

/// Sleep until `due`, or until the deadline where that comes first.
fn wake_at(due: Instant, deadline: Option<Instant>) -> ControlFlow {
    ControlFlow::WaitUntil(deadline.map_or(due, |at| due.min(at)))
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
        // A frame drawn on the idle clock is not a frame the display asked for, so it
        // is no evidence about pacing either way: counting a quiet minute's 100 ms
        // intervals would make it the worst stutter of the run.
        let since_last = if self.idling {
            self.pacing.break_sequence();
            None
        } else {
            self.pacing.record(now)
        };
        if let Some(interval) = since_last
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
        let mut capturing = false;
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
            let fallback_rate = audio.sample_rate;
            let s = audio.latest(now);
            if let Some(age) = crate::audio::pipeline_age(s, fallback_rate, now) {
                self.latency.record(age);
            }
            self.latest.clone_from(&s.panes);
            self.range = (s.floor_db, s.ceiling_db);
            self.analysis = s.analysis_seconds;
            self.pulse = s.pulse;
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
            capturing = matches!(
                audio.status,
                SourceStatus::Starting | SourceStatus::Running(_)
            );
            self.status.capture = audio.status.to_string();
        } else {
            self.status.capture = "no capture".into();
        }
        self.settle_delay();
        self.idling = self.idle.update(now, self.loudness.momentary, capturing);
        // Say whose sound this is when capture went looking for it: "MusicBee, 48 kHz"
        // alone doesn't tell you whether that was chosen or followed.
        if self.capture_follows
            && self.shell.session.capture == Capture::FollowPlayer
            && let Some(audio) = &self.audio
            && matches!(audio.input(), Input::App(_))
            && let Some(player) = self.now_playing.player()
        {
            self.status.capture = format!("following {}  ·  {}", player.name, self.status.capture);
        }

        // Now playing, and the programme measures starting again on a track change.
        if self
            .now_playing
            .poll(now, &self.gpu.device, &self.gpu.queue, &self.artwork)
            && let Some(audio) = &self.audio
        {
            audio.reset_track();
            self.idle.stir();
        }
        self.follow_capture();

        if now.duration_since(self.status_at) >= STATUS_EVERY {
            self.status_at = now;
            self.status.pacing = self.pacing.recent();
            self.status.presented = self.presented.counts();
            // A window doesn't always know which monitor it is on at start-up: on
            // Wayland the compositor says so afterwards, and until it has, the rate is
            // unknown and nothing can be counted as missed. Asking again while it is
            // unknown costs a pointer chase four times a second and turns the pacing
            // figures from a guess into a count.
            if self.status.pacing.refresh_hz.is_none()
                && let Some(hz) =
                    crate::present::compositor_refresh_hz().or_else(|| refresh_rate(&self.window))
            {
                log::info!("display: {hz:.2} Hz");
                self.pacing.set_refresh_hz(Some(hz));
            }
        }

        // The UI first, so the visuals know the space left to them.
        let raw_input = self.egui_state.take_egui_input(&self.window);
        let mut area = crate::ui::Area::default();
        let presentations: Vec<Presentation> = self
            .gpu
            .present_modes
            .iter()
            .map(|&m| presentation_of(m))
            .collect();
        let players = self.now_playing.players().to_vec();
        let around = Around {
            players: &players,
            presets: &self.presets,
            controls: self.now_playing.controls(),
            presentations: &presentations,
            reported_delay_ms: self.reported_delay_ms(),
            // Last frame's, because this frame's is written once the layout is known
            // and the UI is laid out first. A reading a sixtieth of a second old is
            // not a reading anybody can tell from a fresh one.
            reading: &self.reading,
        };
        let shell = &mut self.shell;
        let settings = &mut self.settings;
        let mut full_output = self
            .egui_ctx
            .run_ui(raw_input, |ui| area = shell.show(ui, settings, &around));
        self.pointer = area.hovered;
        self.menu_open = area.menu_open;
        self.double_clicked = area.double_clicked;
        self.clicked = area.clicked;
        self.move_through_history(&area, full_output.pixels_per_point);
        let alpha = self.fade(now, &area);
        for ask in self.shell.take_asks() {
            self.carry_out(&ask);
        }
        // What egui wants next. Without this the cap could hold a menu's animation
        // still, because nothing else in the frame knows egui is in the middle of one.
        self.egui_wait = full_output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map_or(Duration::MAX, |v| v.repaint_delay);
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
            (area.rect.min.x * ppp).round() as i32,
            (area.rect.min.y * ppp).round() as i32,
            (area.rect.width() * ppp).round() as i32,
            (area.rect.height() * ppp).round() as i32,
        );
        let drawn = self.drawn(ppp);
        let bar_w = if drawn.show_color_bar {
            (COLOUR_BAR_WIDTH as f32 * ppp).round() as i32
        } else {
            0
        };
        self.colour_bar = Rect::new(client.right() - bar_w, client.y, bar_w, client.h);
        let whole = Rect::new(client.x, client.y, (client.w - bar_w).max(16), client.h);
        // The quick bar is reserved rather than drawn over the image, so nothing it
        // covers is analysis, and a button is never also a row of the image.
        let label_px = axes::label_px(&self.settings, ppp);
        let (quick, view) = QuickBar::reserve(whole, &drawn, label_px);
        let band_h = BandLayout::height_for(&drawn, view.h, ppp);
        let bounds = Rect::new(view.x, view.y, view.w, (view.h - band_h).max(16));
        self.layout = ScopeLayout::new(bounds, &drawn, ppp);
        if self.layout.columns() != self.columns {
            // A resized pane is as much a change to what analysis measures as a menu
            // item is, and goes the same way, so the two can't disagree about what
            // analysis was last told.
            self.columns = self.layout.columns();
            self.retune_analysis();
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
                visible_rows: (f64::from(p.spectro.w) / self.px_per_row_now()).max(1.0) as f32,
                frac: frac as f32,
                scale: map.scale,
                fmin: map.fmin as f32,
                fmax: map.fmax as f32,
                global_range: None,
                smooth_time: self.settings.smooth_time,
            })
            .collect();
        self.spectrogram
            .prepare(&self.gpu.queue, &self.history, &panes, self.held);
        self.update_hue();
        self.prepare_curves(&drawn);
        self.overlay
            .begin(self.gpu.config.width, self.gpu.config.height);
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
            &drawn,
            &self.layout.panes,
            ppp,
            &mut measure,
        );
        self.waves
            .resize(self.band.wave_a.w.max(self.band.wave_b.w).max(8) as usize);
        if log::log_enabled!(log::Level::Debug) && self.band != self.last_band {
            self.last_band = self.band;
            log::debug!("band {:?}", self.band);
        }

        // The status line is chrome over the image: it starts below the scale lane, and
        // labels drawn over the image step below it in turn. Measured from where the
        // panes actually begin, which is under the quick bar when there is one.
        let chrome_top = self.layout.bounds.y as f32
            + self
                .layout
                .panes
                .first()
                .filter(|p| p.lane.h > 0 && p.lane.y <= p.bounds.y)
                .map_or(0.0, |p| p.lane.h as f32);
        let inset = if self.settings.show_status { 14.0 } else { 0.0 };
        let scales = axes::Scales {
            layout: &self.layout,
            settings: &drawn,
            map: &map,
            floor_db: self.range.0,
            ceiling_db: self.range.1,
            px_per_second: rps * self.px_per_row_now(),
            scale: ppp,
            alpha,
            top_inset: inset,
            label_floor_y: if self.settings.show_status {
                chrome_top + inset + 16.0
            } else {
                0.0
            },
        };
        // The frequency grid, the time marks and the level scale all belong to a flat
        // pane. Over a landscape they would be lines drawn across a picture they have
        // nothing to do with, so the landscape carries its own axis or none.
        if !self.settings.waterfall {
            axes::draw(&mut self.overlay, &scales);
        }
        if self.settings.immersive && self.settings.imm_beat_reactive {
            // The flare is part of the picture rather than furniture, so it stays when
            // the chrome has gone.
            deck::draw_beat_flare(&mut self.overlay, client, &self.lut, self.pulse);
        }
        let state = DeckState {
            loudness: &self.loudness,
            bpm: self.bpm,
            brightness_hz: map.x_to_freq(self.brightness * map.width as f64),
            scope_left: &self.scope.0,
            scope_right: &self.scope.1,
            phosphor: self.phosphor_look().is_some(),
            track: self.now_playing.track(),
            position: self.now_playing.position(now),
            playing: self.now_playing.playing(),
            has_art: self.now_playing.art().is_some(),
        };
        deck::draw_band(
            &mut self.overlay,
            &self.band,
            &self.layout.panes,
            &drawn,
            &self.lut,
            &self.waves,
            &state,
            alpha,
            label_px,
        );
        let overlay = &mut self.overlay;
        let mut measure = |text: &str| {
            overlay
                .measure(text, sonorant_render::Face::Sans, label_px)
                .w
        };
        self.quick_bar = QuickBar::new(
            quick,
            self.layout.gutter,
            &drawn,
            self.held.is_some(),
            ppp,
            &mut measure,
        );
        self.press_quick_bar(ppp);
        let under = self
            .pointer
            .map(|p| ((p.x * ppp).round() as i32, (p.y * ppp).round() as i32))
            .and_then(|(x, y)| self.quick_bar.hit(x, y))
            .map(|b| b.rect);
        quickbar::draw(
            &mut self.overlay,
            &self.quick_bar,
            &drawn,
            under.as_ref(),
            alpha,
            label_px,
        );
        self.press_deck(ppp, now);
        if !self.settings.waterfall {
            self.draw_hover(&map, rps, ppp, now, alpha, &drawn);
        }
        // A full-screen analyser with music playing has no keypresses and no pointer
        // movement, which is exactly what a screen blanks for.
        self.awake
            .set(self.shell.session.fullscreen && self.now_playing.playing());
        if self.settings.show_color_bar {
            deck::draw_colour_bar(
                &mut self.overlay,
                self.colour_bar,
                &drawn,
                &self.lut,
                self.range.0,
                self.range.1,
                alpha,
                label_px,
            );
        }
        // Built whether or not it is drawn: a screen reader is told the same reading
        // the status line would show, and switching the line off is about the picture
        // rather than about what the app is willing to say it is doing.
        self.reading = self.status_line();
        if self.settings.show_status {
            deck::draw_status(
                &mut self.overlay,
                &drawn,
                &self.reading,
                chrome_top,
                alpha,
                label_px,
            );
        }
        self.overlay.prepare(&self.gpu.device, &self.gpu.queue);
        self.artwork.prepare(
            &self.gpu.queue,
            (self.gpu.config.width, self.gpu.config.height),
            self.deck_art(),
            alpha as f32,
            self.backdrop_strength(),
        );

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
        // How much real time this frame stands for. The backdrop's beat and the
        // phosphor's fade both run on it rather than on frames, so neither changes with
        // the frame rate; a gap longer than this was the window being hidden, not a
        // frame, and is capped rather than believed.
        let dt = since_last.map_or(0.0, |d| d.as_secs_f64().min(PHOSPHOR_GAP));
        let beat_phase = self.beat.advance(dt, self.bpm, self.pulse);
        let quality = quality_index(self.settings.render_quality);
        // The visuals go through the floating-point target, so the glow has room to
        // work in before everything is tonemapped onto the screen.
        let glow = self.settings.immersive && self.settings.glow;
        {
            let bg = palette::background(&self.lut);
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("visuals"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: self.visuals.view(),
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
                    timestamp_writes: self.timer.writes(0),
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            // Under the analysis, not over it: wherever the spectrogram has data it
            // covers this, and wherever it hasn't the ground shows through.
            let strength = self.backdrop_strength();
            if strength > 0.0 {
                if let Some(art) = self.now_playing.art() {
                    self.artwork.draw_backdrop(&mut pass, art);
                }
                // Over the cover and under the analysis, adding light and never taking
                // any away, so it lifts whatever ground is there rather than replacing
                // it. With no cover it is the whole backdrop.
                self.backdrop.draw(
                    &self.gpu.queue,
                    &mut pass,
                    &FieldView {
                        size: (self.gpu.config.width, self.gpu.config.height),
                        time: self.started.elapsed().as_secs_f64(),
                        phase: beat_phase,
                        pulse: self.pulse,
                        brightness: self.brightness,
                        strength: f64::from(strength),
                        reactive: self.settings.imm_beat_reactive,
                        ridges: backdrop::RIDGES[quality],
                        deep: Rgba::rgb(palette::color_at(&self.lut, 0.30), 255),
                        hot: Rgba::rgb(palette::color_at(&self.lut, 0.92), 255),
                    },
                );
            }
            if !self.settings.waterfall {
                self.spectrogram.draw(&mut pass, &panes);
            }
        }
        // The landscape goes in a pass of its own, because it needs a depth buffer and
        // the flat views do not. The backdrop it loads over is its sky.
        if self.settings.waterfall {
            let bg = palette::background(&self.lut);
            let landscape = Landscape {
                camera: self.camera,
                rect: self.layout.bounds,
                framebuffer: self.visuals.size(),
                held: self.held,
                sheen: true,
                channel: 0,
                span_rows: panes
                    .first()
                    .map_or(600.0, |p| p.visible_rows * WATERFALL_DEPTH),
                frac: panes.first().map_or(0.0, |p| p.frac),
                scale: map.scale,
                fmin: map.fmin as f32,
                fmax: map.fmax as f32,
                global_range: None,
                background: Rgba::argb(255, bg.r, bg.g, bg.b),
            };
            self.waterfall.draw(
                &self.gpu.device,
                &self.gpu.queue,
                &mut encoder,
                waterfall::Target {
                    view: self.visuals.view(),
                    timestamps: self.timer.writes(1),
                },
                &self.history,
                &landscape,
            );
        }
        if glow {
            self.visuals
                .build_glow(&self.gpu.queue, &mut encoder, self.timer.writes(2));
        }
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("composite"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &linear,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: self.timer.writes(3),
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.visuals
                .composite(&self.gpu.queue, &mut pass, if glow { 1.0 } else { 0.0 });
        }
        // The phosphor fades and gathers before the furniture pass, because it writes
        // its own render targets and the furniture pass is already open by then.
        let scope = self.phosphor_look();
        match &scope {
            Some(look) => {
                let square = self.band.deck.goniometer;
                // The samples that really passed since the last frame, so the figure is
                // continuous and its brightness does not follow the frame rate.
                let want =
                    phosphor::samples_for(dt, rate, self.scope.0.len().min(self.scope.1.len()));
                deck::goniometer_trace(square, &self.scope.0, &self.scope.1, want, &mut self.trace);
                self.phosphor.accumulate(
                    &self.gpu.device,
                    &self.gpu.queue,
                    &mut encoder,
                    &Sweep {
                        rect: square,
                        dt,
                        look,
                        points: &self.trace,
                        deposit: Deposit::AlongThePath,
                    },
                );
            }
            // Switched off, or the deck is gone: drop what it was holding so turning it
            // back on doesn't bring a stale figure with it.
            None => self.phosphor.clear(),
        }
        // The curve strips' screens, one each. Unlike the scope's trace, a curve arrives
        // once a frame however long the frame was, so the light carries the length.
        let curve_scope = self.curve_phosphor_look(&drawn);
        for (i, screen) in self.curve_phosphor.iter_mut().enumerate() {
            let strip = self.layout.panes.get(i).map(|p| p.curve);
            let values = self.latest.get(i).map(|c| c.display.as_slice());
            match (&curve_scope, strip, values) {
                (Some(look), Some(rect), Some(display))
                    if rect.w > 0 && display.len() == rect.h as usize =>
                {
                    let view = CurveView {
                        rect,
                        curve_on_left: self.layout.panes[i].curve_on_left,
                        floor_db: self.range.0,
                        ceiling_db: self.range.1,
                    };
                    curves::phosphor_trace(&view, display, &mut self.trace);
                    screen.accumulate(
                        &self.gpu.device,
                        &self.gpu.queue,
                        &mut encoder,
                        &Sweep {
                            rect,
                            dt,
                            look,
                            points: &self.trace,
                            deposit: Deposit::OncePerFrame,
                        },
                    );
                }
                _ => screen.clear(),
            }
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
                    timestamp_writes: self.timer.writes(4),
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.overlay.draw(Layer::Under, &mut pass);
            if !self.settings.waterfall {
                self.curves.draw(&mut pass);
            }
            if curve_scope.is_some() {
                let size = (self.gpu.config.width, self.gpu.config.height);
                for screen in &self.curve_phosphor {
                    screen.draw(&self.gpu.queue, &mut pass, size, alpha);
                }
            }
            self.overlay.draw(Layer::Over, &mut pass);
            // Over the goniometer's frame and its guides, which `Layer::Over` just drew.
            if scope.is_some() {
                let size = (self.gpu.config.width, self.gpu.config.height);
                self.phosphor.draw(&self.gpu.queue, &mut pass, size, alpha);
            }
            // Between the two: over the deck's backing, under the frame the deck
            // draws on Top, which is the order GDI+ painted them in.
            if self.deck_art().w > 0
                && let Some(art) = self.now_playing.art()
            {
                self.artwork.draw_deck(&mut pass, art);
            }
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
                    timestamp_writes: self.timer.writes(5),
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            self.egui_renderer.render(&mut pass, &paint_jobs, &screen);
        }
        self.timer.resolve(&mut encoder);
        // A backend that won't let its swapchain be copied from can still draw; it
        // just can't be photographed. Saying so once and carrying on beats taking the
        // program down over a diagnostic.
        let shot = match &self.screenshot {
            Some((_, after)) if self.started.elapsed() >= *after => {
                if self.gpu.readable {
                    Some(Readback::record(
                        &self.gpu.device,
                        &mut encoder,
                        &frame.texture,
                    ))
                } else {
                    log::error!("this backend won't let a finished frame be read back");
                    self.screenshot = None;
                    self.shell.session.quit = true;
                    None
                }
            }
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
            self.shell.session.quit = true;
        }
        self.overlay.finish();
        self.timer.poll();
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

    /// Points capture at whatever the followed player is using.
    ///
    /// Switching costs the audio clock its bearings, so this changes as little as it
    /// can: it moves to a player when there is one with a process to follow, and back
    /// to the whole mix only when the player has gone or the source it was on has
    /// stopped working. A player pausing is not a reason to move.
    fn follow_capture(&mut self) {
        if !self.capture_follows {
            return;
        }
        let Some(audio) = &self.audio else { return };
        let on_app = matches!(audio.input(), Input::App(_));
        // The source can only say it has failed once it has tried, so a target that
        // has gone shows up here rather than in the player list.
        let source_gone = matches!(
            audio.status,
            SourceStatus::NoDevice | SourceStatus::Failed(_)
        );
        let want = match self.shell.session.capture {
            Capture::WholeSystem => Input::System,
            Capture::FollowPlayer => match self.now_playing.player().and_then(|p| p.pid) {
                Some(pid) => Input::App(pid.to_string()),
                None if on_app && !source_gone => return,
                None => Input::System,
            },
        };
        if *audio.input() == want {
            self.capture_failed = None;
            return;
        }
        // A process loopback can fail because the app hasn't opened its stream yet,
        // which is a moment's difference, so a failure is worth trying again - just
        // not sixty times a second.
        if let Some((failed, at)) = &self.capture_failed
            && *failed == want
            && at.elapsed() < CAPTURE_RETRY
        {
            return;
        }
        let Some(audio) = &mut self.audio else { return };
        match audio.set_input(&want) {
            Ok(()) => {
                log::info!("capture now follows {want:?}");
                self.capture_failed = None;
                self.waves.clear();
            }
            Err(e) => {
                log::warn!("cannot capture {want:?}: {e}");
                self.capture_failed = Some((want, Instant::now()));
            }
        }
    }

    /// Where the cover goes, or nothing when the deck has no room or it is turned off.
    fn deck_art(&self) -> Rect {
        if self.settings.show_center_deck {
            self.band.deck.art
        } else {
            Rect::EMPTY
        }
    }

    /// How the goniometer's phosphor screen should look, or `None` when the deck, the
    /// goniometer or the phosphor itself is switched off and the overlay draws the
    /// figure the old way.
    fn phosphor_look(&self) -> Option<PhosphorLook> {
        let s = &self.settings;
        if !s.show_center_deck || !s.deck_show_goniometer || !s.deck_phosphor {
            return None;
        }
        if self.band.deck.goniometer.w <= 0 {
            return None;
        }
        Some(PhosphorLook {
            // The palette's colour, as the overlay's trace used, so the scope still
            // belongs to the theme rather than being a fixed CRT green.
            colour: Rgba::rgb(palette::color_at(&self.lut, 0.80), 255),
            ..self.phosphor_base()
        })
    }

    /// How the curve strips' phosphor screens should look, or `None` when the setting
    /// has the curve pass drawing the line itself.
    ///
    /// The persistence and the intensity are the scope's: one screen with one feel,
    /// rather than two sets of numbers meaning the same thing.
    fn curve_phosphor_look(&self, drawn: &Settings) -> Option<PhosphorLook> {
        let look = CurveLook::new(drawn, &self.lut, 1.0);
        look.phosphor.then(|| PhosphorLook {
            colour: look.hi,
            // Tighter than the scope's: a strip is tall and narrow, and a halo as wide
            // as the scope's would bleed the curve into the image beside it.
            glow: 0.45,
            ..self.phosphor_base()
        })
    }

    /// The persistence and intensity both phosphor screens share.
    fn phosphor_base(&self) -> PhosphorLook {
        PhosphorLook {
            persistence: f64::from(self.settings.phosphor_ms) / 1000.0,
            intensity: f64::from(self.settings.phosphor_intensity) / 100.0,
            ..PhosphorLook::default()
        }
    }

    /// How far the backdrop comes forward, from 0 to 1. Nostalgia+ capped the setting
    /// at 60 per cent, past which the analysis stops being readable.
    fn backdrop_strength(&self) -> f32 {
        if !self.settings.immersive || !self.settings.imm_backdrop {
            return 0.0;
        }
        self.settings.backdrop_pct.clamp(0, 60) as f32 / 100.0
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
        if self.timer.is_available() {
            line += &format!("  |  GPU {:.1} ms", self.timer.total_ms());
        }
        if self.shell.session.frozen {
            // Where it is looking, not merely that it has stopped: parked in the
            // history, the one thing worth knowing is how far back.
            let back = self.parked_rows() / self.settings.effective_rows_per_second().max(1.0);
            if back >= 0.5 {
                line += &format!("  |  FROZEN {back:.0} s back");
            } else {
                line += "  |  FROZEN";
            }
        }
        if (self.view.zoom - 1.0).abs() > 0.01 {
            line += &format!("  |  zoom {:.2}x", self.view.zoom);
        }
        // The pipeline's own delay, and the offset that is held on purpose. Two
        // different things that both answer "how late is this", so they are named
        // rather than both being a number of milliseconds behind something.
        let latency = self.latency.recent();
        if latency.readings > 0 {
            line += &format!("  |  lag {:.0} ms", latency.mean_ms);
        }
        if self.idling {
            line += &format!(
                "  |  idle {:.0} fps",
                1.0 / IDLE_INTERVAL.as_secs_f64().max(f64::MIN_POSITIVE)
            );
        }
        // Only when there is one: an offset of nothing is the ordinary case, and
        // saying "delay 0 ms" every frame would be a figure that never means anything.
        if self.settings.visual_delay_ms > 0 {
            line += &format!(
                "  |  delay {} ms{}",
                self.settings.visual_delay_ms,
                if self.settings.auto_visual_delay {
                    " auto"
                } else {
                    ""
                }
            );
        }
        line
    }

    /// Uploads the curve strips for this frame's layout.
    fn prepare_curves(&mut self, drawn: &Settings) {
        // A reference taken at another size no longer lines up with the rows.
        if let Some(r) = &self.reference
            && r.iter()
                .zip(&self.latest)
                .any(|(r, p)| r.len() != p.display.len())
        {
            self.reference = None;
        }
        let look = CurveLook::new(drawn, &self.lut, 1.0);
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

    /// Rebuilds the palette when the music's brightness has moved far enough to see.
    ///
    /// Nostalgia+ left the columns already drawn in the hue they were pushed with, so
    /// the image carried its own recent history in colour. Here the shader colours the
    /// whole history from one palette, so a drift recolours all of it at once, as a
    /// palette change does.
    fn update_hue(&mut self) {
        let want = if self.settings.immersive && self.settings.imm_colour_follows {
            (self.brightness - 0.5) * 2.0 * self.settings.colour_follow_degrees as f64
        } else {
            0.0
        };
        // A degree either way is invisible, and rebuilding the table isn't free.
        if (want - self.hue_shift).abs() < 1.0 {
            return;
        }
        self.hue_shift = want;
        self.lut = palette::build_lut_shifted(self.settings.palette, want);
        self.spectrogram.set_palette(&self.gpu.queue, &self.lut);
        self.waterfall.set_palette(&self.gpu.queue, &self.lut);
        self.curves.set_palette(&self.gpu.queue, &self.lut);
    }

    /// Applies whatever the menu, the keys or a preset changed since the last frame.
    ///
    /// The menu writes into the settings directly, so this compares them with what the
    /// frame loop last acted on rather than being told what moved. Most settings are
    /// read afresh every frame and need nothing here; these are the ones that own
    /// something outside them, like the palette table, the analysis thread or the
    /// window itself.
    fn apply_settings(&mut self) {
        let session = self.shell.session.clone();
        if session.frozen != self.applied_session.frozen {
            if session.frozen {
                self.frozen_at = Some(self.history.written());
            } else {
                // Unfreezing is a way back to now, whether the image was frozen there or
                // taken back through the history by the wheel.
                self.frozen_at = None;
                self.parked_at = None;
                self.view.offset = 0.0;
            }
            self.settle_view();
        }
        if self.settings.render_quality != self.applied.render_quality {
            let q = quality_index(self.settings.render_quality);
            self.waterfall
                .set_mesh(&self.gpu.device, waterfall::MESH[q]);
            self.visuals
                .set_divisor(&self.gpu.device, bloom::GLOW_DIVISORS[q]);
            log::info!(
                "visual quality {}: mesh {:?}, glow at a {}th, {} ridges",
                self.settings.render_quality.name(),
                waterfall::MESH[q],
                bloom::GLOW_DIVISORS[q],
                backdrop::RIDGES[q],
            );
        }
        let rows = history_rows(&self.settings);
        if rows != self.history_rows {
            // A different size is a different ring, so what is in it goes. Better than
            // the alternative, which was the reach depending on the speed the app
            // happened to start at.
            self.history_rows = rows;
            self.history = HistoryStore::new(&self.gpu.device, GRID_BINS as u32, rows);
            self.spectrogram.bind(&self.gpu.device, &self.history);
            self.waterfall.bind(&self.gpu.device, &self.history);
            self.held = None;
            self.frozen_at = None;
            self.parked_at = None;
            self.view.offset = 0.0;
            self.shell.session.parked = false;
            log::info!(
                "history resized to {} rows, {:.1} minutes at {:.0} rows a second",
                self.history.capacity(),
                self.history.capacity() as f64
                    / self.settings.effective_rows_per_second().max(1.0)
                    / 60.0,
                self.settings.effective_rows_per_second(),
            );
        }
        if session.reference != self.applied_session.reference {
            self.take_reference(session.reference);
        }
        if session.follow != self.applied_session.follow {
            self.now_playing.set_follow(session.follow.clone());
        }
        if session.presentation != self.applied_session.presentation {
            self.gpu
                .set_present_mode(present_mode_of(session.presentation));
            self.pacing.break_sequence();
            self.presented.reset();
        }
        if session.fullscreen != self.applied_session.fullscreen {
            self.window
                .set_fullscreen(session.fullscreen.then_some(Fullscreen::Borderless(None)));
            self.pacing.set_refresh_hz(
                crate::present::compositor_refresh_hz().or_else(|| refresh_rate(&self.window)),
            );
            self.presented.reset();
        }
        // A transport command is a one-off rather than a state, so it is taken rather
        // than compared. A player that says it cannot do it refuses inside `send`.
        if let Some(command) = self.shell.session.command.take() {
            self.now_playing.send(command);
        }
        self.applied_session = self.shell.session.clone();

        if self.settings == self.applied {
            return;
        }
        if self.settings.palette != self.applied.palette {
            self.hue_shift = 0.0;
            self.lut = palette::build_lut(self.settings.palette);
            self.spectrogram.set_palette(&self.gpu.queue, &self.lut);
            self.waterfall.set_palette(&self.gpu.queue, &self.lut);
            self.curves.set_palette(&self.gpu.queue, &self.lut);
        }
        self.retune_analysis();
        self.applied = self.settings.clone();
    }

    /// What the system says the output path costs, in milliseconds, for the menu.
    fn reported_delay_ms(&self) -> Option<f64> {
        let d = self.audio.as_ref()?.reported_delay()?;
        Some(d.as_secs_f64() * 1000.0)
    }

    /// Keeps the hold on the analysis in step with the offset, and the offset in step
    /// with what the system reports while it is left automatic.
    ///
    /// The reported figure is written into the setting rather than kept beside it, so
    /// the menu shows the number really in use, and switching automatic off leaves
    /// that number there to be adjusted rather than dropping back to whatever was
    /// last typed. Switching it back on takes the reported figure again at once,
    /// which is why the setting follows the report every frame rather than only when
    /// a new one arrives.
    ///
    /// It goes in through the settings directly rather than through the menu's
    /// `apply`, because a sink changing is not the user changing a setting: it must
    /// not switch the automatic off, and it must not turn a preset into `Custom`.
    fn settle_delay(&mut self) {
        let Some(audio) = &mut self.audio else { return };
        if let Some(reported) = audio.reported_delay()
            && self.settings.auto_visual_delay
        {
            let step = Number::VisualDelayMs.range().step;
            let ms = (reported.as_secs_f64() * 1000.0 / step).round() * step;
            if self.settings.number(Number::VisualDelayMs) != ms {
                self.settings.set_number(Number::VisualDelayMs, ms);
                log::info!("the visual delay follows the output: {ms:.0} ms");
            }
        }
        audio.set_delay(self.settings.visual_delay_seconds());
    }

    /// Tells the analysis thread what to measure, when that has changed.
    ///
    /// Everything analysis cares about is in one config, so comparing it catches a
    /// change from the menu, a key or a preset without listing the fields twice.
    fn retune_analysis(&mut self) {
        let config = AnalysisConfig::from_settings(&self.settings, self.columns);
        if config == self.config {
            return;
        }
        self.config = config.clone();
        if let Some(audio) = &self.audio {
            audio.set_config(config);
        }
    }

    /// Holds each pane's average spectrum as an amber reference, or drops the one held.
    fn take_reference(&mut self, hold: bool) {
        self.reference = match hold {
            true if !self.latest.is_empty() => {
                Some(self.latest.iter().map(|p| p.average.clone()).collect())
            }
            // Nothing measured yet: there is no spectrum to hold, so the switch goes
            // back where it was rather than claiming a reference that isn't there.
            true => {
                self.shell.session.reference = false;
                None
            }
            false => None,
        };
    }

    /// Looks a key up in the menu and performs whatever it finds there.
    fn press(&mut self, key: &Key) {
        let Some(name) = crate::ui::key_name(key) else {
            return;
        };
        // Esc takes one thing off at a time, nearest first: the menu, then the help
        // window, then fullscreen. Both of those are closed by the UI itself when it
        // sees the key, and egui does not mark Esc as consumed, so without this one
        // press would put the menu away and leave fullscreen in the same breath.
        if name == "Esc" && (self.menu_open || self.shell.session.help) {
            // The menu closes itself, because egui takes Escape for its own popup. The
            // help window doesn't, so it is closed here.
            if !self.menu_open {
                self.shell.session.help = false;
            }
            self.window.request_redraw();
            return;
        }
        let players = self.now_playing.players().to_vec();
        let presentations: Vec<Presentation> = self
            .gpu
            .present_modes
            .iter()
            .map(|&m| presentation_of(m))
            .collect();
        let action = menu::action_for_key(
            &menu::Context {
                settings: &self.settings,
                session: &self.shell.session,
                players: &players,
                presets: &self.presets,
                controls: self.now_playing.controls(),
                presentations: &presentations,
                reported_delay_ms: self.reported_delay_ms(),
            },
            &name,
        );
        if let Some(action) = action {
            self.shell.act(&action, &mut self.settings);
            for ask in self.shell.take_asks() {
                self.carry_out(&ask);
            }
        }
    }

    /// Does what the menu asked of the settings folder.
    fn carry_out(&mut self, ask: &Ask) {
        let Some(store) = &self.store else {
            log::warn!("no settings folder, so presets cannot be saved or loaded");
            return;
        };
        match ask {
            Ask::Load(name) => match store.load_preset(name) {
                Some(settings) => {
                    log::info!("loaded the preset {name}");
                    self.settings = settings;
                }
                None => log::error!("cannot read the preset {name}"),
            },
            Ask::Save(name) => match store.save_preset(name, &self.settings) {
                Ok(replaced) => {
                    log::info!(
                        "{} the preset {name}",
                        if replaced { "replaced" } else { "saved" }
                    );
                    self.presets = store.list_presets();
                }
                Err(e) => log::error!("cannot save the preset {name}: {e}"),
            },
            Ask::Delete(name) => {
                if store.delete_preset(name) {
                    log::info!("deleted the preset {name}");
                    self.presets = store.list_presets();
                } else {
                    log::error!("cannot delete the preset {name}");
                }
            }
        }
    }

    /// How far the furniture has faded this frame, and the pointer shown or hidden
    /// to match.
    ///
    /// The chrome answers the pointer moving, not merely being somewhere: a still
    /// pointer over the image is someone watching, which is what immersive mode is for.
    fn fade(&mut self, now: Instant, area: &crate::ui::Area) -> f64 {
        if area.hovered != self.pointer_was || area.clicked || area.double_clicked {
            self.chrome.stir(now);
        }
        self.pointer_was = area.hovered;
        // A menu or a dialog holds everything up: they are drawn over the furniture and
        // are useless without it.
        let holding = area.menu_open || self.shell.session.help;
        let alpha = self.chrome.update(now, &self.settings, holding);
        let hide = !self.chrome.cursor_visible();
        if hide != self.cursor_hidden {
            self.cursor_hidden = hide;
            self.window.set_cursor_visible(!hide);
        }
        alpha
    }

    /// Performs the quick bar's button under the pointer, if one was clicked.
    fn press_quick_bar(&mut self, ppp: f32) {
        if !self.clicked {
            return;
        }
        let Some(p) = self.pointer else { return };
        let at = ((p.x * ppp).round() as i32, (p.y * ppp).round() as i32);
        let Some(action) = self.quick_bar.hit(at.0, at.1).map(|b| b.action.clone()) else {
            return;
        };
        self.shell.act(&action, &mut self.settings);
        for ask in self.shell.take_asks() {
            self.carry_out(&ask);
        }
    }

    /// Presses whichever of the deck's transport buttons the pointer is on.
    ///
    /// The seek bar is a button too: clicking along it asks for that point in the
    /// track. A player that says it cannot be asked refuses inside `send`, so a deck
    /// that draws a button a player won't honour still does nothing rather than
    /// something wrong.
    fn press_deck(&mut self, ppp: f32, now: Instant) {
        if !self.clicked || !self.settings.show_center_deck || !self.settings.deck_show_transport {
            return;
        }
        let Some(p) = self.pointer else { return };
        let (x, y) = ((p.x * ppp).round() as i32, (p.y * ppp).round() as i32);
        let d = self.band.deck;
        let command = if d.play.contains(x, y) {
            Some(Transport::PlayPause)
        } else if d.next.contains(x, y) {
            Some(Transport::Next)
        } else if d.prev.contains(x, y) {
            Some(Transport::Previous)
        } else if d.seek.contains(x, y) {
            // Where along the bar it was clicked, as a fraction of the track.
            self.now_playing
                .position(now)
                .filter(|&(_, length)| length > 0.0)
                .map(|(_, length)| {
                    let along = f64::from(x - d.seek.x) / f64::from(d.seek.w.max(1));
                    Transport::SeekTo(Duration::from_secs_f64(along.clamp(0.0, 1.0) * length))
                })
        } else {
            None
        };
        if let Some(command) = command {
            self.now_playing.send(command);
        }
    }

    /// Reads out whatever the pointer is over, and sends the player to that moment on
    /// a double-click.
    fn draw_hover(
        &mut self,
        map: &sonorant_core::dsp::FrequencyMap,
        rows_per_second: f64,
        ppp: f32,
        now: Instant,
        alpha: f64,
        drawn: &Settings,
    ) {
        // The panes are laid out in physical pixels; the pointer arrives in points.
        let at = match self.pointer {
            Some(p) => ((p.x * ppp).round() as i32, (p.y * ppp).round() as i32),
            None => return,
        };
        let Some(hovered) = hover::locate(
            &self.layout,
            map,
            at,
            self.px_per_row_now(),
            rows_per_second,
            self.parked_rows() / rows_per_second.max(1e-9),
        ) else {
            return;
        };

        // A double-click over the image asks the player for the moment under the
        // pointer. What is drawn there is that much older than the position the clock
        // is carrying now.
        if self.double_clicked
            && self.settings.seek_on_image_click
            && hovered.on_image
            && let Some((position, _)) = self.now_playing.position(now)
        {
            let target = (position - hovered.age).max(0.0);
            if self
                .now_playing
                .send(Transport::SeekTo(Duration::from_secs_f64(target)))
            {
                log::debug!("seeking to {target:.2} s, {:.2} s back", hovered.age);
            }
        }

        // Every pane's level at that frequency when the hover is synced, otherwise the
        // one the pointer is in. A pane whose curve is a different length belongs to a
        // layout that has since changed, so it has nothing to say about this row.
        let readings: Vec<Reading<'_>> = self
            .layout
            .panes
            .iter()
            .zip(&self.latest)
            .enumerate()
            .filter(|(i, _)| drawn.sync_hover || *i == hovered.pane)
            .map(|(_, (pane, curves))| Reading {
                label: pane.label,
                db: curves
                    .display
                    .get(hovered.bin)
                    .map_or(f64::NAN, |&db| f64::from(db)),
            })
            .collect();
        hover::draw(
            &mut self.overlay,
            &Readout {
                layout: &self.layout,
                settings: drawn,
                map,
                hover: hovered,
                levels: &readings,
                alpha,
                label_px: axes::label_px(&self.settings, ppp),
            },
        );
    }

    /// The settings as they are drawn, rather than as they are saved.
    ///
    /// Two things happen here. The desktop's accent colour fills in the hover slot when
    /// no colour was chosen for it, standing in for the skin colours Nostalgia+ took
    /// from MusicBee, without being written into the saved theme, so it follows the
    /// desktop rather than freezing at whatever it was on the day. And every setting
    /// that means a size on screen is scaled to the display, so the deck, the gutter
    /// and the bars are the size they look at 100% however the screen is scaled. The
    /// image, the scope and the lanes are left in physical pixels, where they keep
    /// their detail.
    fn drawn(&self, ppp: f32) -> Settings {
        let mut s = self.settings.clone();
        if s.theme.hover.is_none()
            && let Some(accent) = self.appearance.accent
        {
            // Lifted first: an accent is chosen against the desktop's background, and a
            // dark one drawn over the visuals would make "on" read as off.
            s.theme.hover = Some(sonorant_render::colour::as_ink(accent));
        }
        if (ppp - 1.0).abs() > 0.01 {
            let px = |n: i32| (n as f32 * ppp).round() as i32;
            s.deck_height_px = px(s.deck_height_px);
            s.gutter_width = px(s.gutter_width);
            s.bar_size = px(s.bar_size);
            s.led_segment = px(s.led_segment);
        }
        s
    }

    /// How long to leave between frames, or `None` to draw at the display's own rate.
    ///
    /// Nostalgia+'s `TargetFps` set how fast it redrew and, with it, how fast the image
    /// scrolled. Here scrolling follows audio time whatever this is, so the cap only
    /// decides how often the screen is redrawn: it is a way to give a weak machine or a
    /// battery some room, not a way to change the picture.
    fn frame_interval(&self) -> Option<Duration> {
        let capped = match self.settings.frame_cap {
            FrameCap::Display => None,
            FrameCap::Fps60 => Some(Duration::from_nanos(1_000_000_000 / 60)),
            FrameCap::Fps30 => Some(Duration::from_nanos(1_000_000_000 / 30)),
        };
        // Nothing playing: the picture is a black scroll under resting meters, so it is
        // redrawn slowly. Never faster than the cap, which is a ceiling and not a rate.
        if self.idling {
            return Some(IDLE_INTERVAL.max(capped.unwrap_or_default()));
        }
        capped
    }

    /// How many screen pixels one history row is drawn across.
    fn px_per_row(&self) -> u32 {
        self.settings.px_per_row.clamp(1, 8) as u32
    }

    /// Pixels one row is drawn across, the setting stretched by however far the wheel
    /// has zoomed. Everything that turns pixels into time reads this, not the setting:
    /// the panes, the time marks and the hover readout, which is what keeps them
    /// agreeing about where a column is.
    fn px_per_row_now(&self) -> f64 {
        f64::from(self.px_per_row()) * self.view.zoom
    }

    /// Rows the newest edge of the image is behind the newest row there is. Zero while
    /// the view is live.
    fn parked_rows(&self) -> f64 {
        match self.held {
            Some(anchor) => self.history.written().saturating_sub(anchor) as f64,
            None => 0.0,
        }
    }

    /// The wheel zooms time about the pointer and a drag pans through the history.
    ///
    /// Both work on the anchor the freeze already had: the row the image's newest edge
    /// sits on. Parking the view is freezing it, so the Freeze item, the status line and
    /// `Space` all keep saying the truth, and the way back is `End`, the Live button or
    /// unfreezing.
    fn move_through_history(&mut self, area: &crate::ui::Area, ppp: f32) {
        if std::mem::take(&mut self.shell.session.go_live) {
            self.view = TimeView::default();
            self.frozen_at = None;
            self.parked_at = None;
            self.settle_view();
            return;
        }
        // A notch of the wheel is about 50 points.
        let notches = f64::from(area.scrolled) / 50.0;
        let turned = area.scrolled.abs() > 0.01;
        let dragged = area.dragged.unwrap_or(egui::Vec2::ZERO);
        if self.settings.waterfall {
            self.move_the_camera(turned.then_some(notches), dragged, ppp);
            return;
        }
        if !turned && dragged.x == 0.0 {
            self.settle_view();
            return;
        }
        // The pane under the pointer decides which way time runs; without one, the
        // first, which is the one a mirrored layout agrees with.
        let pointer = self
            .pointer
            .map(|p| ((p.x * ppp).round() as i32, (p.y * ppp).round() as i32));
        let Some(pane) = pointer
            .and_then(|(x, y)| self.layout.panes.iter().find(|p| p.spectro.contains(x, y)))
            .or_else(|| self.layout.panes.first())
        else {
            self.settle_view();
            return;
        };
        let (rect, newest_left) = (pane.spectro, pane.curve_on_left);
        if rect.w <= 0 {
            self.settle_view();
            return;
        }
        let strip = Strip {
            width: f64::from(rect.w),
            px_per_row: f64::from(self.px_per_row()),
        };
        self.view.offset = self.parked_rows();

        if turned {
            let along = match pointer {
                Some((x, _)) => {
                    let t = (f64::from(x - rect.x) / f64::from(rect.w)).clamp(0.0, 1.0);
                    if newest_left { t } else { 1.0 - t }
                }
                None => 0.0,
            };
            self.view.zoom_about(notches, along, strip, ZOOM_RANGE);
        }
        if dragged.x != 0.0 {
            self.view
                .pan(f64::from(dragged.x) * f64::from(ppp), newest_left, strip);
        }

        let written = self.history.written();
        self.view
            .clamp_to(written as f64, self.history.capacity() as f64, strip);
        // From a distance back to the row itself, now, while `written` is the one the
        // distance was measured from.
        self.parked_at = self
            .view
            .parked()
            .then(|| written.saturating_sub(self.view.offset.round() as u64));
        self.settle_view();
    }

    /// The wheel moves the waterfall's eye in and out and a drag orbits it, which is
    /// what the same two gestures mean once the view is a landscape rather than a wall.
    ///
    /// Where the image is looking along the history is left exactly as it was, so
    /// turning the waterfall off puts the flat view back where it stood.
    fn move_the_camera(&mut self, notches: Option<f64>, dragged: egui::Vec2, ppp: f32) {
        if let Some(view) = self.shell.session.camera.take() {
            self.camera = Camera::of(view);
        }
        if let Some(notches) = notches {
            self.camera.zoom(notches);
        }
        if dragged != egui::Vec2::ZERO {
            let across = f64::from(self.layout.bounds.w.max(1));
            self.camera.orbit(
                f64::from(dragged.x) * f64::from(ppp),
                f64::from(dragged.y) * f64::from(ppp),
                across,
            );
        }
    }

    /// Works out the row the image's newest edge sits on, and tells the session what it
    /// came to.
    ///
    /// Two things can hold the image still, and they are kept apart on purpose. `Space`
    /// freezes it where it is, which is `frozen_at`. The wheel and a drag park it
    /// somewhere in the history, which is `parked_at`. A parked view wins, because it is
    /// the more particular of the two; with the view back at now, the freeze is what is
    /// left. Rolling them into one flag looked tidier and was wrong: zooming back to now
    /// could not get out of the freeze that zooming away from it had switched on.
    fn settle_view(&mut self) {
        self.held = self.parked_at.or(self.frozen_at);
        // The menu item, the status line and the quick bar all read one answer.
        let held = self.held.is_some();
        self.shell.session.frozen = held;
        self.applied_session.frozen = held;
        self.shell.session.parked = held;
    }
}

/// Rows the store should hold: the length setting at the scroll speed, held under the
/// memory budget.
///
/// Nostalgia+ had no history to speak of, and until now this was five minutes at
/// whatever speed the app started with, so changing the speed quietly changed how far
/// back the image reached. It is the setting's job now, and the store is rebuilt when
/// either moves.
fn history_rows(s: &Settings) -> u32 {
    let wanted =
        s.effective_rows_per_second().max(1.0) * 60.0 * f64::from(s.history_minutes.clamp(1, 15));
    let affordable = (HISTORY_BUDGET / ROW_BYTES) as f64;
    wanted.min(affordable).max(64.0) as u32
}

/// Where a visual quality sits in the tables the renderer keeps: the waterfall's mesh,
/// the glow's chain and the backdrop's ridges are all indexed the same way.
fn quality_index(q: RenderQuality) -> usize {
    match q {
        RenderQuality::Low => 0,
        RenderQuality::Medium => 1,
        RenderQuality::High => 2,
    }
}

/// The model's name for a backend present mode.
fn presentation_of(mode: wgpu::PresentMode) -> Presentation {
    match mode {
        wgpu::PresentMode::Mailbox => Presentation::Newest,
        wgpu::PresentMode::Immediate => Presentation::Immediate,
        _ => Presentation::EveryRefresh,
    }
}

/// The backend present mode the model means.
fn present_mode_of(p: Presentation) -> wgpu::PresentMode {
    match p {
        Presentation::EveryRefresh => wgpu::PresentMode::Fifo,
        Presentation::Newest => wgpu::PresentMode::Mailbox,
        Presentation::Immediate => wgpu::PresentMode::Immediate,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_history_reaches_as_far_back_as_the_setting_asks() {
        let at = |minutes, rows_per_second| {
            history_rows(&Settings {
                history_minutes: minutes,
                rows_per_second,
                ..Settings::default()
            })
        };
        // The default: five minutes at sixty rows a second.
        assert_eq!(at(5, 60.0), 18_000);
        assert_eq!(at(1, 60.0), 3_600);
        // The reach is the setting's, not the speed's. This is what it is here for:
        // before, the store was five minutes at whatever speed the app started at, so
        // halving the speed quietly doubled how far back the image went.
        assert_eq!(at(5, 30.0), 9_000);
        assert_eq!(at(3, 120.0), 21_600);
        // Past the budget the setting is a request rather than a promise: a row costs
        // the same whatever the speed, and ten minutes at the default is already 288 MB.
        // The app logs the reach it settled on.
        let most = (HISTORY_BUDGET / ROW_BYTES) as u32;
        assert_eq!(at(10, 60.0), most);
        assert_eq!(at(5, 120.0), most);
    }

    #[test]
    fn the_history_stops_at_the_memory_budget() {
        let most = (HISTORY_BUDGET / ROW_BYTES) as u32;
        let greedy = history_rows(&Settings {
            history_minutes: 15,
            rows_per_second: 240.0,
            ..Settings::default()
        });
        assert_eq!(
            greedy, most,
            "15 minutes at 240 rows a second is not affordable"
        );
        assert!(
            u64::from(greedy) * ROW_BYTES <= HISTORY_BUDGET,
            "{greedy} rows is over the budget"
        );
    }

    /// Cinematic quarters the scroll speed, and the reach follows the speed the rows
    /// are really cut at rather than the number in the settings.
    #[test]
    fn the_reach_follows_the_speed_the_rows_are_really_cut_at() {
        let plain = history_rows(&Settings {
            history_minutes: 5,
            rows_per_second: 60.0,
            ..Settings::default()
        });
        let cinematic = history_rows(&Settings {
            history_minutes: 5,
            rows_per_second: 60.0,
            imm_cinematic: true,
            ..Settings::default()
        });
        assert_eq!(cinematic, plain / 4);
    }
}
