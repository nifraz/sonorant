//! PipeWire capture: the default sink's monitor, or one app's output stream.
//!
//! The stream asks for a 256-frame quantum to keep latency low and has no fixed target
//! when capturing the whole mix, so the session manager keeps it on the monitor of
//! whatever the default sink is, moving it when Bluetooth headphones connect.
//!
//! The process callback and a silence timer both run on this source's own loop thread,
//! so they can share the one producer end of the analysis ring. Monitors of a suspended
//! sink deliver nothing; the timer feeds silence for those stretches so the audio clock
//! keeps moving.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use sonorant_core::runtime::AudioInput;
use sonorant_core::source::{AudioSource, SourceEvent, SourceStatus, StreamFormat, to_stereo};
use pipewire as pw;
use pw::properties::properties;
use pw::spa;
use spa::param::format::{MediaSubtype, MediaType};
use spa::param::format_utils;
use spa::pod::Pod;

/// What to capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Everything the default sink plays.
    WholeSystem,
    /// One app's output stream node, by its `object.serial`.
    Node { serial: u64, name: String },
}

/// One app with an output stream, for the "Capture from" menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioApp {
    pub serial: u64,
    pub name: String,
    /// `application.process.id`, when the app reports it. Phase 4 matches it to the
    /// MPRIS player's process.
    pub pid: Option<u32>,
}

/// How long the monitor may go quiet before the gap is filled with silence.
const QUIET: Duration = Duration::from_millis(40);
const TICK: Duration = Duration::from_millis(10);

#[derive(Debug)]
pub struct PipeWireSource {
    target: Target,
    quit: Option<pw::channel::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl PipeWireSource {
    pub fn new(target: Target) -> PipeWireSource {
        PipeWireSource {
            target,
            quit: None,
            thread: None,
        }
    }
}

impl AudioSource for PipeWireSource {
    fn start(&mut self, input: AudioInput, events: Sender<SourceEvent>) {
        self.stop();
        let (tx, rx) = pw::channel::channel::<()>();
        let target = self.target.clone();
        self.quit = Some(tx);
        self.thread = Some(
            thread::Builder::new()
                .name("sonorant-capture".into())
                .spawn(move || {
                    let _ = events.send(SourceEvent::Status(SourceStatus::Starting));
                    if let Err(status) = run(&target, input, events.clone(), rx) {
                        let _ = events.send(SourceEvent::Status(status));
                    }
                    let _ = events.send(SourceEvent::Status(SourceStatus::Stopped));
                })
                .expect("the capture thread starts"),
        );
    }

    fn stop(&mut self) {
        if let Some(q) = self.quit.take() {
            let _ = q.send(());
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for PipeWireSource {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Shared {
    input: AudioInput,
    events: Sender<SourceEvent>,
    target: Target,
    rate: u32,
    channels: usize,
    streaming: bool,
    last_data: Instant,
    quiet_fed: u64,
    stereo: Vec<f32>,
    samples: Vec<f32>,
}

fn run(
    target: &Target,
    input: AudioInput,
    events: Sender<SourceEvent>,
    quit: pw::channel::Receiver<()>,
) -> Result<(), SourceStatus> {
    pw::init();
    let failed = |e: pw::Error| SourceStatus::Failed(e.to_string());
    let mainloop = pw::main_loop::MainLoopRc::new(None).map_err(failed)?;
    let context = pw::context::ContextRc::new(&mainloop, None).map_err(failed)?;
    let core = context
        .connect_rc(None)
        .map_err(|_| SourceStatus::NoAudioServer)?;

    let _quit = quit.attach(mainloop.loop_(), {
        let ml = mainloop.clone();
        move |()| ml.quit()
    });

    let mut props = properties! {
        *pw::keys::MEDIA_TYPE => "Audio",
        *pw::keys::MEDIA_CATEGORY => "Capture",
        *pw::keys::MEDIA_ROLE => "Music",
        *pw::keys::APP_NAME => "Sonorant",
        *pw::keys::NODE_NAME => "sonorant-capture",
        *pw::keys::NODE_LATENCY => "256/48000",
    };
    match target {
        Target::WholeSystem => props.insert(*pw::keys::STREAM_CAPTURE_SINK, "true"),
        Target::Node { serial, .. } => props.insert(*pw::keys::TARGET_OBJECT, serial.to_string()),
    }
    let stream = pw::stream::StreamBox::new(&core, "Sonorant capture", props).map_err(failed)?;

    let shared = Rc::new(RefCell::new(Shared {
        input,
        events,
        target: target.clone(),
        rate: 48000,
        channels: 2,
        streaming: false,
        last_data: Instant::now(),
        quiet_fed: 0,
        stereo: Vec::with_capacity(8192),
        samples: Vec::with_capacity(8192),
    }));

    let _listener = stream
        .add_local_listener_with_user_data(())
        .state_changed({
            let shared = shared.clone();
            move |_, _, _old, new| {
                let mut s = shared.borrow_mut();
                let status = match new {
                    pw::stream::StreamState::Streaming => {
                        s.streaming = true;
                        s.last_data = Instant::now();
                        let what = match &s.target {
                            Target::WholeSystem => "Whole system".to_owned(),
                            Target::Node { name, .. } => format!("{name} only"),
                        };
                        SourceStatus::Running(format!("{what} · {} kHz", s.rate as f64 / 1000.0))
                    }
                    pw::stream::StreamState::Error(e) => {
                        s.streaming = false;
                        SourceStatus::Failed(e)
                    }
                    pw::stream::StreamState::Unconnected => {
                        s.streaming = false;
                        SourceStatus::NoDevice
                    }
                    pw::stream::StreamState::Paused => {
                        s.streaming = false;
                        SourceStatus::Suspended
                    }
                    pw::stream::StreamState::Connecting => SourceStatus::Starting,
                };
                let _ = s.events.send(SourceEvent::Status(status));
            }
        })
        .param_changed({
            let shared = shared.clone();
            move |_, _, id, param| {
                let Some(param) = param else { return };
                if id != spa::param::ParamType::Format.as_raw() {
                    return;
                }
                let Ok((media_type, media_subtype)) = format_utils::parse_format(param) else {
                    return;
                };
                if media_type != MediaType::Audio || media_subtype != MediaSubtype::Raw {
                    return;
                }
                let mut info = spa::param::audio::AudioInfoRaw::new();
                if info.parse(param).is_err() {
                    return;
                }
                let mut s = shared.borrow_mut();
                s.rate = info.rate();
                s.channels = info.channels().max(1) as usize;
                let format = StreamFormat {
                    sample_rate: s.rate,
                    source_channels: s.channels as u16,
                };
                let _ = s.events.send(SourceEvent::Format(format));
            }
        })
        .process({
            let shared = shared.clone();
            move |stream, _| {
                let Some(mut buffer) = stream.dequeue_buffer() else {
                    return;
                };
                let datas = buffer.datas_mut();
                let Some(data) = datas.first_mut() else {
                    return;
                };
                let (offset, size) = (data.chunk().offset() as usize, data.chunk().size() as usize);
                let mut guard = shared.borrow_mut();
                let s = &mut *guard;
                if let Some(bytes) = data.data() {
                    let end = (offset + size).min(bytes.len());
                    s.samples.clear();
                    s.samples.extend(
                        bytes[offset.min(end)..end]
                            .chunks_exact(4)
                            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])),
                    );
                    to_stereo(&s.samples, s.channels, &mut s.stereo);
                    s.input.push_interleaved(&s.stereo);
                    s.last_data = Instant::now();
                    s.quiet_fed = 0;
                }
            }
        })
        .register()
        .map_err(failed)?;

    // Raw 32-bit float in the graph's own rate and channels.
    let mut audio_info = spa::param::audio::AudioInfoRaw::new();
    audio_info.set_format(spa::param::audio::AudioFormat::F32LE);
    let obj = spa::pod::Object {
        type_: spa::utils::SpaTypes::ObjectParamFormat.as_raw(),
        id: spa::param::ParamType::EnumFormat.as_raw(),
        properties: audio_info.into(),
    };
    let values: Vec<u8> = spa::pod::serialize::PodSerializer::serialize(
        std::io::Cursor::new(Vec::new()),
        &spa::pod::Value::Object(obj),
    )
    .map_err(|e| SourceStatus::Failed(format!("{e:?}")))?
    .0
    .into_inner();
    let mut params =
        [Pod::from_bytes(&values).ok_or(SourceStatus::Failed("bad format pod".into()))?];
    stream
        .connect(
            spa::utils::Direction::Input,
            None,
            pw::stream::StreamFlags::AUTOCONNECT | pw::stream::StreamFlags::MAP_BUFFERS,
            &mut params,
        )
        .map_err(failed)?;

    // Silence for quiet stretches, against the clock.
    let timer = mainloop.loop_().add_timer({
        let shared = shared.clone();
        move |_| {
            let mut guard = shared.borrow_mut();
            let s = &mut *guard;
            if !s.streaming {
                return;
            }
            let quiet = s.last_data.elapsed();
            if quiet < QUIET {
                return;
            }
            let owed = (quiet.as_secs_f64() * s.rate as f64) as u64;
            if owed > s.quiet_fed {
                let n = (owed - s.quiet_fed).min(48000) as usize;
                s.stereo.clear();
                s.stereo.resize(n * 2, 0.0);
                s.input.push_interleaved(&s.stereo);
                s.quiet_fed += n as u64;
            }
        }
    });
    let _ = timer.update_timer(Some(TICK), Some(TICK));

    mainloop.run();
    Ok(())
}

/// Apps with an audio output stream right now, by name.
pub fn audio_apps() -> Vec<AudioApp> {
    pw::init();
    let found = Rc::new(RefCell::new(Vec::new()));
    let result = (|| -> Result<(), pw::Error> {
        let mainloop = pw::main_loop::MainLoopRc::new(None)?;
        let context = pw::context::ContextRc::new(&mainloop, None)?;
        let core = context.connect_rc(None)?;
        let registry = core.get_registry_rc()?;

        let _reg = registry
            .add_listener_local()
            .global({
                let found = found.clone();
                move |global| {
                    let Some(props) = global.props else { return };
                    if props.get("media.class") != Some("Stream/Output/Audio") {
                        return;
                    }
                    let Some(serial) = props.get("object.serial").and_then(|s| s.parse().ok())
                    else {
                        return;
                    };
                    let name = props
                        .get("application.name")
                        .or_else(|| props.get("node.name"))
                        .unwrap_or("unknown")
                        .to_owned();
                    let pid = props
                        .get("application.process.id")
                        .and_then(|p| p.parse().ok());
                    found.borrow_mut().push(AudioApp { serial, name, pid });
                }
            })
            .register();

        // Everything the registry knows arrives before the answer to this sync.
        let pending = core.sync(0)?;
        let _core_listener = core
            .add_listener_local()
            .done({
                let ml = mainloop.clone();
                move |id, seq| {
                    if id == pw::core::PW_ID_CORE && seq == pending {
                        ml.quit();
                    }
                }
            })
            .register();
        mainloop.run();
        Ok(())
    })();
    if let Err(e) = result {
        log::warn!("cannot list PipeWire streams: {e}");
    }
    let mut apps = found.borrow().clone();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps
}
