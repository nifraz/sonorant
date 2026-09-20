//! The apps playing through the default output device, for the "Capture from" menu.

use windows::Win32::Foundation::{CloseHandle, MAX_PATH};
use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, IMMDeviceEnumerator,
    MMDeviceEnumerator, eConsole, eRender,
};
use windows::Win32::System::Com::{
    CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoUninitialize,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{Interface, PWSTR};

/// One app with an audio session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioApp {
    pub pid: u32,
    /// The executable's name without its extension, such as "MusicBee".
    pub name: String,
}

/// The app behind a media session's app user model id, if it is playing something.
///
/// SMTC names a player by app id and never says which process it is, and process
/// loopback capture needs a process. The two are matched by name, which is what those
/// ids reduce to: `MusicBee.exe` and `SpotifyAB.SpotifyMusic_...!Spotify` both come
/// down to the executable running behind them.
///
/// Only apps with a live audio session can be matched, which is the right restriction:
/// a player making no sound is not one worth following.
///
/// `apps` is passed in rather than fetched here because enumerating sessions costs a
/// round of COM, and a refresh matches every player against the same list.
pub fn match_app_id<'a>(apps: &'a [AudioApp], app_id: &str) -> Option<&'a AudioApp> {
    let want = super::smtc::display_name(app_id);
    if want.is_empty() {
        return None;
    }
    apps.iter().find(|a| a.name.eq_ignore_ascii_case(&want))
}

fn process_name(pid: u32) -> Option<String> {
    // SAFETY: a limited-information handle, closed before returning.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; MAX_PATH as usize];
        let mut len = buf.len() as u32;
        let ok =
            QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len)
                .is_ok();
        let _ = CloseHandle(h);
        if !ok {
            return None;
        }
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        let file = path.rsplit(['\\', '/']).next()?.to_owned();
        Some(
            file.strip_suffix(".exe")
                .or_else(|| file.strip_suffix(".EXE"))
                .unwrap_or(&file)
                .to_owned(),
        )
    }
}

/// Apps with a live audio session on the default output device, one entry per process,
/// sorted by name. The system sounds session and expired sessions are left out.
pub fn audio_apps() -> Vec<AudioApp> {
    // SAFETY: COM initialisation balanced below; everything else is owned by wrappers.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let apps = (|| -> windows::core::Result<Vec<AudioApp>> {
        // SAFETY: plain COM queries.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let manager: IAudioSessionManager2 = device.Activate(CLSCTX_ALL, None)?;
            let sessions = manager.GetSessionEnumerator()?;
            let mut apps: Vec<AudioApp> = Vec::new();
            for i in 0..sessions.GetCount()? {
                let control = sessions.GetSession(i)?;
                if control.GetState()? == AudioSessionStateExpired {
                    continue;
                }
                let c2: IAudioSessionControl2 = control.cast()?;
                if c2.IsSystemSoundsSession().is_ok()
                    && c2.IsSystemSoundsSession() == windows::Win32::Foundation::S_OK
                {
                    continue;
                }
                let pid = c2.GetProcessId()?;
                if pid == 0 || apps.iter().any(|a| a.pid == pid) {
                    continue;
                }
                if let Some(name) = process_name(pid) {
                    apps.push(AudioApp { pid, name });
                }
            }
            apps.sort_by_key(|a| a.name.to_lowercase());
            Ok(apps)
        }
    })()
    .unwrap_or_default();
    if com {
        // SAFETY: balances the successful CoInitializeEx.
        unsafe { CoUninitialize() };
    }
    apps
}
