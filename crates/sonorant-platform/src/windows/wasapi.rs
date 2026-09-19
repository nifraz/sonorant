//! WASAPI loopback capture: the whole system mix, or one process tree.
//!
//! The capture thread is event-driven and registered with MMCSS as "Pro Audio". It
//! copies each packet into the analysis ring and nothing else: no allocation once
//! running, no locks, no logging on the audio path. When the default output device
//! changes it reopens on the new one; while another app holds the device exclusively it
//! reports that and tries again every couple of seconds.
//!
//! Loopback delivers nothing at all while nothing plays, so the thread feeds silence
//! for those stretches: the audio clock keeps moving and the picture keeps scrolling.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, SyncSender, sync_channel};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use sonorant_core::runtime::AudioInput;
use sonorant_core::source::{AudioSource, SourceEvent, SourceStatus, StreamFormat};
use windows::Win32::Foundation::{CloseHandle, HANDLE, PROPERTYKEY, WAIT_OBJECT_0};
use windows::Win32::Media::Audio::*;
use windows::Win32::Media::KernelStreaming::{KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE};
use windows::Win32::Media::Multimedia::{KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT};
use windows::Win32::System::Com::StructuredStorage::{PROPVARIANT, PropVariantToStringAlloc};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
    CoUninitialize, STGM_READ,
};
use windows::Win32::System::Threading::{
    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
    WaitForSingleObject,
};
use windows::core::{Interface, PCWSTR, implement, w};

/// What to capture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// Everything the default output device plays.
    WholeSystem,
    /// One process and the processes it started, such as a browser's audio process.
    /// Needs Windows 10 2004 (build 19041) or later.
    Process { pid: u32, name: String },
}

/// `AUDCLNT_E_DEVICE_INVALIDATED`: the device went away.
const DEVICE_INVALIDATED: i32 = 0x8889_0004_u32 as i32;
/// `AUDCLNT_E_DEVICE_IN_USE`: held in exclusive mode by another app.
const DEVICE_IN_USE: i32 = 0x8889_000A_u32 as i32;
/// `AUDCLNT_E_SERVICE_NOT_RUNNING`: the Windows audio service is stopped.
const SERVICE_NOT_RUNNING: i32 = 0x8889_0010_u32 as i32;
/// `AUDCLNT_E_RESOURCES_INVALIDATED`: the stream was suspended, as for a sleep.
const RESOURCES_INVALIDATED: i32 = 0x8889_0026_u32 as i32;

/// How long the capture waits for a packet before treating the gap as silence.
const WAIT_MS: u32 = 10;
/// How often the default device is checked.
const DEVICE_CHECK: Duration = Duration::from_secs(1);
/// How long to wait before trying again after a failure.
const RETRY: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub struct WasapiSource {
    target: Target,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WasapiSource {
    pub fn new(target: Target) -> WasapiSource {
        WasapiSource {
            target,
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl AudioSource for WasapiSource {
    fn start(&mut self, input: AudioInput, events: Sender<SourceEvent>) {
        self.stop();
        self.stop.store(false, Ordering::Relaxed);
        let (target, stop) = (self.target.clone(), self.stop.clone());
        self.thread = Some(
            thread::Builder::new()
                .name("sonorant-capture".into())
                .spawn(move || run(target, input, events, stop))
                .expect("the capture thread starts"),
        );
    }

    fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for WasapiSource {
    fn drop(&mut self) {
        self.stop();
    }
}

enum Ended {
    Stopped,
    /// The device changed or vanished: open the new default at once.
    Reopen,
}

fn run(target: Target, mut input: AudioInput, events: Sender<SourceEvent>, stop: Arc<AtomicBool>) {
    // SAFETY: plain COM and MMCSS setup for this thread, undone below.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let mut task = 0u32;
    // SAFETY: registers this thread with MMCSS; the handle is reverted before exit.
    let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) }.ok();
    let _ = events.send(SourceEvent::Status(SourceStatus::Starting));

    let mut scratch = Scratch::default();
    let mut last_status = None;
    while !stop.load(Ordering::Relaxed) {
        match session(&target, &mut input, &events, &stop, &mut scratch) {
            Ok(Ended::Stopped) => break,
            Ok(Ended::Reopen) => continue,
            Err(status) => {
                if last_status.as_ref() != Some(&status) {
                    let _ = events.send(SourceEvent::Status(status.clone()));
                    last_status = Some(status);
                }
                let until = Instant::now() + RETRY;
                while Instant::now() < until && !stop.load(Ordering::Relaxed) {
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    let _ = events.send(SourceEvent::Status(SourceStatus::Stopped));
    if let Some(h) = mmcss {
        // SAFETY: `h` came from AvSetMmThreadCharacteristicsW on this thread.
        unsafe {
            let _ = AvRevertMmThreadCharacteristics(h);
        }
    }
    if com {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
}

/// Buffers reused from packet to packet, so the audio path doesn't allocate once warm.
#[derive(Default)]
struct Scratch {
    stereo: Vec<f32>,
}

/// The layout of the samples WASAPI delivers.
#[derive(Clone, Copy, Debug)]
struct Format {
    rate: u32,
    channels: usize,
    kind: SampleKind,
    block_align: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SampleKind {
    F32,
    I16,
    /// 24 valid bits packed in 3 bytes.
    I24,
    /// Integer samples in 4 bytes, whatever the valid bits.
    I32,
}

/// Reads a `WAVEFORMATEX` or `WAVEFORMATEXTENSIBLE`.
///
/// # Safety
/// `p` must point at a valid format structure.
unsafe fn read_format(p: *const WAVEFORMATEX) -> Option<Format> {
    // SAFETY: the caller promises a valid structure; packed fields are read unaligned.
    let wf = unsafe { std::ptr::read_unaligned(p) };
    let tag = wf.wFormatTag as u32;
    let bits = wf.wBitsPerSample;
    let float = if tag == WAVE_FORMAT_EXTENSIBLE {
        // SAFETY: an extensible tag means the larger structure is there.
        let ext = unsafe { std::ptr::read_unaligned(p as *const WAVEFORMATEXTENSIBLE) };
        let sub = ext.SubFormat;
        if sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT {
            true
        } else if sub == KSDATAFORMAT_SUBTYPE_PCM {
            false
        } else {
            return None;
        }
    } else {
        tag == WAVE_FORMAT_IEEE_FLOAT
    };
    let kind = match (float, bits) {
        (true, 32) => SampleKind::F32,
        (false, 16) => SampleKind::I16,
        (false, 24) => SampleKind::I24,
        (false, 32) => SampleKind::I32,
        _ => return None,
    };
    Some(Format {
        rate: wf.nSamplesPerSec,
        channels: wf.nChannels.max(1) as usize,
        kind,
        block_align: wf.nBlockAlign as usize,
    })
}

/// A 32-bit float stereo format at `rate`, for process loopback, which has no mix
/// format of its own and converts to whatever it's asked for.
fn float_stereo(rate: u32) -> WAVEFORMATEX {
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_IEEE_FLOAT as u16,
        nChannels: 2,
        nSamplesPerSec: rate,
        nAvgBytesPerSec: rate * 8,
        nBlockAlign: 8,
        wBitsPerSample: 32,
        cbSize: 0,
    }
}

/// Converts one packet to interleaved stereo `f32`.
///
/// # Safety
/// `data` must hold `frames` frames in `format`.
unsafe fn convert(data: *const u8, frames: usize, format: &Format, out: &mut Vec<f32>) {
    out.clear();
    let ch = format.channels;
    // SAFETY: the caller promises `frames * block_align` readable bytes.
    let bytes = unsafe { std::slice::from_raw_parts(data, frames * format.block_align) };
    let sample = |frame: &[u8], c: usize| -> f32 {
        match format.kind {
            SampleKind::F32 => {
                f32::from_le_bytes(frame[c * 4..c * 4 + 4].try_into().expect("4 bytes"))
            }
            SampleKind::I16 => {
                i16::from_le_bytes([frame[c * 2], frame[c * 2 + 1]]) as f32 / 32768.0
            }
            SampleKind::I24 => {
                let b = &frame[c * 3..c * 3 + 3];
                (i32::from_le_bytes([0, b[0], b[1], b[2]]) >> 8) as f32 / 8_388_608.0
            }
            SampleKind::I32 => {
                (i32::from_le_bytes(frame[c * 4..c * 4 + 4].try_into().expect("4 bytes")) as f64
                    / 2_147_483_648.0) as f32
            }
        }
    };
    for frame in bytes.chunks_exact(format.block_align) {
        let l = sample(frame, 0);
        let r = if ch > 1 { sample(frame, 1) } else { l };
        out.push(l);
        out.push(r);
    }
}

fn hr_status(e: &windows::core::Error) -> SourceStatus {
    match e.code().0 {
        DEVICE_IN_USE => SourceStatus::ExclusiveMode,
        SERVICE_NOT_RUNNING => SourceStatus::NoAudioServer,
        RESOURCES_INVALIDATED => SourceStatus::Suspended,
        _ => SourceStatus::Failed(format!("{:#010X} {}", e.code().0 as u32, e.message())),
    }
}

/// An event handle closed when dropped.
struct Event(HANDLE);

impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn device_id(device: &IMMDevice) -> Option<String> {
    // SAFETY: GetId returns a CoTaskMem string that is freed here.
    unsafe {
        let p = device.GetId().ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

/// `PKEY_Device_FriendlyName`, spelled out to avoid another feature of the windows crate.
const FRIENDLY_NAME: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0),
    pid: 14,
};

fn device_name(device: &IMMDevice) -> Option<String> {
    // SAFETY: standard property-store read; the PROPVARIANT and string are released.
    unsafe {
        let store = device.OpenPropertyStore(STGM_READ).ok()?;
        let value: PROPVARIANT = store.GetValue(&FRIENDLY_NAME).ok()?;
        let p = PropVariantToStringAlloc(&value).ok()?;
        let s = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as _));
        s
    }
}

fn session(
    target: &Target,
    input: &mut AudioInput,
    events: &Sender<SourceEvent>,
    stop: &AtomicBool,
    scratch: &mut Scratch,
) -> Result<Ended, SourceStatus> {
    // SAFETY: COM calls on a COM-initialised thread; every returned pointer is owned by
    // a windows-rs wrapper or freed explicitly.
    unsafe {
        let enumerator: IMMDeviceEnumerator =
            CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                .map_err(|_| SourceStatus::NoAudioServer)?;
        let device = enumerator
            .GetDefaultAudioEndpoint(eRender, eConsole)
            .map_err(|_| SourceStatus::NoDevice)?;
        let id = device_id(&device);
        let name = device_name(&device).unwrap_or_else(|| "default output".to_owned());

        // The device's own client tells us the mix format and rate.
        let mix_client: IAudioClient = device
            .Activate(CLSCTX_ALL, None)
            .map_err(|e| hr_status(&e))?;
        let mix_ptr = mix_client.GetMixFormat().map_err(|e| hr_status(&e))?;
        let mix = read_format(mix_ptr);
        let mix_rate = std::ptr::read_unaligned(mix_ptr).nSamplesPerSec;

        let (client, format, flags, fmt_ptr, what): (
            IAudioClient,
            Format,
            u32,
            *const WAVEFORMATEX,
            String,
        );
        let process_format;
        match target {
            Target::WholeSystem => {
                let Some(f) = mix else {
                    CoTaskMemFree(Some(mix_ptr as _));
                    return Err(SourceStatus::Failed(
                        "the mix format isn't one Sonorant reads".into(),
                    ));
                };
                client = mix_client;
                format = f;
                flags = AUDCLNT_STREAMFLAGS_LOOPBACK | AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
                fmt_ptr = mix_ptr;
                what = format!("Whole system · {name} · {} kHz", f.rate as f64 / 1000.0);
            }
            Target::Process { pid, name: app } => {
                CoTaskMemFree(Some(mix_ptr as _));
                client = activate_process_loopback(*pid)?;
                process_format = float_stereo(mix_rate);
                format = read_format(&process_format).expect("a float stereo format reads");
                flags = AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
                fmt_ptr = &process_format;
                what = format!("{app} only · {} kHz", mix_rate as f64 / 1000.0);
            }
        }

        // A zero duration gives the engine's default period, about 10 ms.
        let init = client.Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 0, 0, fmt_ptr, None);
        if matches!(target, Target::WholeSystem) {
            CoTaskMemFree(Some(fmt_ptr as _));
        }
        init.map_err(|e| hr_status(&e))?;

        let event =
            Event(CreateEventW(None, false, false, PCWSTR::null()).map_err(|e| hr_status(&e))?);
        client.SetEventHandle(event.0).map_err(|e| hr_status(&e))?;
        let capture: IAudioCaptureClient = client.GetService().map_err(|e| hr_status(&e))?;
        client.Start().map_err(|e| hr_status(&e))?;

        let _ = events.send(SourceEvent::Format(StreamFormat {
            sample_rate: format.rate,
            source_channels: format.channels as u16,
        }));
        let _ = events.send(SourceEvent::Status(SourceStatus::Running(what)));

        let result = pump(
            &client,
            &capture,
            &event,
            &format,
            &enumerator,
            id.as_deref(),
            target,
            input,
            stop,
            scratch,
        );
        let _ = client.Stop();
        result
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn pump(
    client: &IAudioClient,
    capture: &IAudioCaptureClient,
    event: &Event,
    format: &Format,
    enumerator: &IMMDeviceEnumerator,
    device_id_now: Option<&str>,
    target: &Target,
    input: &mut AudioInput,
    stop: &AtomicBool,
    scratch: &mut Scratch,
) -> Result<Ended, SourceStatus> {
    let _ = client;
    let mut checked = Instant::now();
    // Silence is fed against the clock: frames owed since the last delivery.
    let mut quiet_since: Option<Instant> = None;
    let mut quiet_fed = 0u64;
    let silence = [0.0f32; 2 * 480];

    loop {
        if stop.load(Ordering::Relaxed) {
            return Ok(Ended::Stopped);
        }
        // SAFETY: waits on our own event handle.
        let woke = unsafe { WaitForSingleObject(event.0, WAIT_MS) } == WAIT_OBJECT_0;

        let mut got = false;
        loop {
            // SAFETY: the capture client is started; buffers are released in order.
            let packet = match unsafe { capture.GetNextPacketSize() } {
                Ok(n) => n,
                Err(e) if e.code().0 == DEVICE_INVALIDATED => return Ok(Ended::Reopen),
                Err(e) => return Err(hr_status(&e)),
            };
            if packet == 0 {
                break;
            }
            let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
            // SAFETY: out-pointers are valid locals.
            if let Err(e) =
                unsafe { capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None) }
            {
                if e.code().0 == DEVICE_INVALIDATED {
                    return Ok(Ended::Reopen);
                }
                return Err(hr_status(&e));
            }
            let n = frames as usize;
            if flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || data.is_null() {
                scratch.stereo.clear();
                scratch.stereo.resize(n * 2, 0.0);
            } else {
                // SAFETY: WASAPI hands us `frames` frames in `format`.
                unsafe { convert(data, n, format, &mut scratch.stereo) };
            }
            input.push_interleaved(&scratch.stereo);
            // SAFETY: releases exactly what GetBuffer handed out.
            let _ = unsafe { capture.ReleaseBuffer(frames) };
            got = true;
        }

        let now = Instant::now();
        if got || woke {
            quiet_since = None;
        } else {
            let since = *quiet_since.get_or_insert_with(|| {
                quiet_fed = 0;
                now
            });
            // Owe the analysis the frames this quiet stretch has lasted, beyond one wait.
            let owed = (now.duration_since(since).as_secs_f64() * format.rate as f64) as u64;
            while quiet_fed + (silence.len() as u64 / 2) <= owed {
                input.push_interleaved(&silence);
                quiet_fed += silence.len() as u64 / 2;
            }
        }

        if matches!(target, Target::WholeSystem) && now.duration_since(checked) >= DEVICE_CHECK {
            checked = now;
            // SAFETY: plain COM query.
            let current = unsafe { enumerator.GetDefaultAudioEndpoint(eRender, eConsole) }.ok();
            let current_id = current.as_ref().and_then(device_id);
            if current_id.as_deref() != device_id_now {
                return Ok(Ended::Reopen);
            }
        }
    }
}

// ---------------------------------------------------------------- process loopback

// windows-rs makes every #[implement] object agile, as ActivateAudioInterfaceAsync needs.
#[implement(IActivateAudioInterfaceCompletionHandler)]
struct Completion {
    done: SyncSender<()>,
}

impl IActivateAudioInterfaceCompletionHandler_Impl for Completion_Impl {
    fn ActivateCompleted(
        &self,
        _op: windows::core::Ref<'_, IActivateAudioInterfaceAsyncOperation>,
    ) -> windows::core::Result<()> {
        let _ = self.done.try_send(());
        Ok(())
    }
}

/// Activates a loopback client for one process tree.
unsafe fn activate_process_loopback(pid: u32) -> Result<IAudioClient, SourceStatus> {
    let params = AUDIOCLIENT_ACTIVATION_PARAMS {
        ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
        Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
            ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                TargetProcessId: pid,
                ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
            },
        },
    };
    let blob = windows::Win32::System::Com::BLOB {
        cbSize: size_of::<AUDIOCLIENT_ACTIVATION_PARAMS>() as u32,
        pBlobData: &params as *const _ as *mut u8,
    };
    let mut prop = PROPVARIANT::default();
    // SAFETY: fills a VT_BLOB PROPVARIANT that borrows `params`; it is not cleared with
    // PropVariantClear, so nothing tries to free the borrowed bytes.
    unsafe {
        let inner = &mut prop.Anonymous.Anonymous;
        inner.vt = windows::Win32::System::Variant::VT_BLOB;
        inner.Anonymous.blob = blob;
    }

    let (tx, rx) = sync_channel(1);
    let handler: IActivateAudioInterfaceCompletionHandler = Completion { done: tx }.into();
    // SAFETY: the parameters outlive the call and the wait below.
    let op = unsafe {
        ActivateAudioInterfaceAsync(
            VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
            &IAudioClient::IID,
            Some(&prop),
            &handler,
        )
    }
    .map_err(|e| hr_status(&e))?;
    if rx.recv_timeout(Duration::from_secs(5)).is_err() {
        return Err(SourceStatus::Failed(
            "process loopback didn't answer".into(),
        ));
    }
    let mut hr = windows::core::HRESULT(0);
    let mut unknown = None;
    // SAFETY: the operation completed; out-pointers are valid locals.
    unsafe { op.GetActivateResult(&mut hr, &mut unknown) }.map_err(|e| hr_status(&e))?;
    hr.ok().map_err(|e| hr_status(&e))?;
    unknown
        .ok_or(SourceStatus::NoDevice)?
        .cast::<IAudioClient>()
        .map_err(|e| hr_status(&e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_packets_to_stereo() {
        let f = Format {
            rate: 48000,
            channels: 1,
            kind: SampleKind::I16,
            block_align: 2,
        };
        let data: Vec<u8> = [16384i16, -32768]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let mut out = Vec::new();
        // SAFETY: two mono 16-bit frames.
        unsafe { convert(data.as_ptr(), 2, &f, &mut out) };
        assert_eq!(out, [0.5, 0.5, -1.0, -1.0]);

        let f = Format {
            rate: 48000,
            channels: 4,
            kind: SampleKind::F32,
            block_align: 16,
        };
        let data: Vec<u8> = [0.1f32, 0.2, 0.3, 0.4]
            .iter()
            .flat_map(|s| s.to_le_bytes())
            .collect();
        unsafe { convert(data.as_ptr(), 1, &f, &mut out) };
        assert_eq!(out, [0.1, 0.2]);
    }

    #[test]
    fn reads_the_float_stereo_format() {
        let wf = float_stereo(44100);
        // SAFETY: a valid WAVEFORMATEX.
        let f = unsafe { read_format(&wf) }.unwrap();
        assert_eq!(
            (f.rate, f.channels, f.kind, f.block_align),
            (44100, 2, SampleKind::F32, 8)
        );
    }
}
