//! Windows: now playing from the system media transport controls.
//!
//! SMTC is what the volume flyout's media tile reads, so anything that appears there -
//! MusicBee, Spotify, a browser tab - appears here too, without asking the player for
//! anything of its own.
//!
//! The work happens on a thread of its own. WinRT calls it into cross-process RPC and
//! the event handlers arrive on thread pool threads, neither of which belongs anywhere
//! near a frame. The thread keeps a [`Snapshot`] behind a mutex and the app copies it
//! out once a frame.
//!
//! Two fields the deck lays out are missing here and stay empty on Windows: SMTC
//! carries no composer and no year. MusicBee knows both, but only over its own plugin
//! interface, which this app deliberately does not use.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use sonorant_core::media::{
    Controls, Follow, MediaSession, NowPlaying, PlayState, Player, RESYNC_AFTER, Snapshot,
    Transport,
};
use windows::Foundation::TypedEventHandler;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session,
    GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx, CoUninitialize};

/// A thumbnail bigger than this is not artwork, it is a mistake; reading it would stall
/// the thread for something the deck draws at 90 pixels.
const MAX_THUMBNAIL: u64 = 16 * 1024 * 1024;

/// What the worker thread is asked to do.
enum Msg {
    /// Something changed: read everything again.
    Refresh,
    Follow(Follow),
    Transport(Transport),
    Stop,
}

#[derive(Debug, Default)]
struct Shared {
    snapshot: Mutex<Snapshot>,
}

/// Now playing from SMTC.
#[derive(Debug)]
pub struct SmtcSession {
    shared: Arc<Shared>,
    tx: SyncSender<Msg>,
    thread: Option<JoinHandle<()>>,
    /// Set once the worker has a manager, so `available` doesn't lie during start-up.
    ready: Arc<AtomicBool>,
}

impl SmtcSession {
    /// Starts watching. This returns at once; the first snapshot arrives when Windows
    /// hands over the session manager, which takes a moment on a cold start.
    pub fn start() -> SmtcSession {
        let shared = Arc::new(Shared::default());
        let ready = Arc::new(AtomicBool::new(false));
        // Bounded, because the handlers must never block a WinRT callback; a full
        // queue already means a refresh is coming.
        let (tx, rx) = sync_channel(64);
        let thread = {
            let shared = Arc::clone(&shared);
            let ready = Arc::clone(&ready);
            let tx = tx.clone();
            std::thread::Builder::new()
                .name("sonorant-smtc".into())
                .spawn(move || run(&shared, &ready, &tx, &rx))
                .ok()
        };
        if thread.is_none() {
            log::error!("cannot start the now-playing thread");
        }
        SmtcSession {
            shared,
            tx,
            thread,
            ready,
        }
    }

    /// Whether Windows has handed over a session manager.
    pub fn is_ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }
}

impl MediaSession for SmtcSession {
    fn snapshot(&self) -> Snapshot {
        self.shared
            .snapshot
            .lock()
            .map(|s| s.clone())
            .unwrap_or_default()
    }

    fn follow(&self, choice: Follow) {
        let _ = self.tx.try_send(Msg::Follow(choice));
    }

    fn send(&self, command: Transport) -> bool {
        self.tx.try_send(Msg::Transport(command)).is_ok()
    }
}

impl Drop for SmtcSession {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// The worker: opens the manager, subscribes, and keeps the snapshot up to date.
fn run(
    shared: &Shared,
    ready: &AtomicBool,
    tx: &SyncSender<Msg>,
    rx: &std::sync::mpsc::Receiver<Msg>,
) {
    // SAFETY: balanced by the CoUninitialize below, on this thread only.
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let manager = match Manager::RequestAsync().and_then(|op| op.join()) {
        Ok(m) => m,
        Err(e) => {
            log::warn!("no media session manager: {e}; now playing will be empty");
            if com {
                // SAFETY: balances the successful CoInitializeEx above.
                unsafe { CoUninitialize() };
            }
            return;
        }
    };
    ready.store(true, Ordering::Relaxed);

    // The manager's own events: a player appearing, going, or taking over.
    let mut manager_tokens = Vec::new();
    if let Ok(t) = manager.SessionsChanged(&notifier(tx)) {
        manager_tokens.push((0u8, t));
    }
    if let Ok(t) = manager.CurrentSessionChanged(&notifier(tx)) {
        manager_tokens.push((1u8, t));
    }

    let mut follow = Follow::default();
    let mut watched: Option<Watched> = None;
    let mut track_key = String::new();
    let mut art: Option<Arc<[u8]>> = None;

    loop {
        // The followed session, resubscribing when it is a different one.
        let session = pick(&manager, &follow);
        let same = match (&watched, &session) {
            (Some(w), Some(s)) => w.is(s),
            (None, None) => true,
            _ => false,
        };
        if !same {
            if let Some(w) = watched.take() {
                w.unsubscribe();
            }
            watched = session.as_ref().map(|s| Watched::subscribe(s.clone(), tx));
            // A different player means different artwork, whatever it is called.
            track_key.clear();
            art = None;
        }

        // One round of COM for the whole refresh: every player is matched to a
        // process against the same list.
        let apps = super::sessions::audio_apps();
        let mut next = Snapshot {
            players: list_players(&manager, &apps),
            ..Snapshot::default()
        };
        if let Some(s) = &session {
            read_into(s, &apps, &mut next);
            // The thumbnail is a stream to open and read, so it is fetched once a
            // track, not once a second.
            let key = art_key(&next.track);
            if key != track_key {
                track_key = key;
                art = read_thumbnail(s);
            }
            next.track.art = art.clone().map(sonorant_core::media::ArtSource::Bytes);
        } else {
            track_key.clear();
            art = None;
        }
        publish(shared, next);

        // Wait for news, and fall back to a resync so a position nobody signalled
        // still lands on the seek bar.
        match rx.recv_timeout(RESYNC_AFTER) {
            Ok(Msg::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Msg::Refresh) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Msg::Follow(f)) => follow = f,
            Ok(Msg::Transport(command)) => {
                if let Some(s) = &session {
                    perform(s, command);
                }
            }
        }
        // Whatever else queued up behind that is the same refresh.
        let mut stopping = false;
        for msg in rx.try_iter() {
            match msg {
                Msg::Stop => stopping = true,
                Msg::Follow(f) => follow = f,
                Msg::Transport(command) => {
                    if let Some(s) = &session {
                        perform(s, command);
                    }
                }
                Msg::Refresh => {}
            }
        }
        if stopping {
            break;
        }
    }

    if let Some(w) = watched.take() {
        w.unsubscribe();
    }
    for (which, token) in manager_tokens {
        let _ = match which {
            0 => manager.RemoveSessionsChanged(token),
            _ => manager.RemoveCurrentSessionChanged(token),
        };
    }
    if com {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
}

/// Stores `next` under the lock, keeping the generation moving only when something the
/// app reacts to actually changed.
fn publish(shared: &Shared, mut next: Snapshot) {
    let Ok(mut held) = shared.snapshot.lock() else {
        return;
    };
    let changed = held.player != next.player
        || held.track != next.track
        || held.state != next.state
        || held.controls != next.controls
        || held.players != next.players;
    next.generation = held.generation + u64::from(changed);
    *held = next;
}

/// An event handler that only says "something changed"; the reading is done on the
/// worker thread, where it is allowed to block.
fn notifier<S, A>(tx: &SyncSender<Msg>) -> TypedEventHandler<S, A>
where
    S: windows_core::RuntimeType + 'static,
    A: windows_core::RuntimeType + 'static,
{
    let tx = tx.clone();
    TypedEventHandler::new(move |_, _| {
        match tx.try_send(Msg::Refresh) {
            // A full queue is a refresh already on its way, which is what this was for.
            Err(TrySendError::Full(_)) | Ok(()) => {}
            Err(TrySendError::Disconnected(_)) => {}
        }
        Ok(())
    })
}

/// A session we are subscribed to, and the tokens to undo it with.
struct Watched {
    session: Session,
    media: i64,
    playback: i64,
    timeline: i64,
}

impl Watched {
    fn subscribe(session: Session, tx: &SyncSender<Msg>) -> Watched {
        let media = session.MediaPropertiesChanged(&notifier(tx)).unwrap_or(0);
        let playback = session.PlaybackInfoChanged(&notifier(tx)).unwrap_or(0);
        let timeline = session
            .TimelinePropertiesChanged(&notifier(tx))
            .unwrap_or(0);
        Watched {
            session,
            media,
            playback,
            timeline,
        }
    }

    fn is(&self, other: &Session) -> bool {
        match (
            self.session.SourceAppUserModelId(),
            other.SourceAppUserModelId(),
        ) {
            (Ok(a), Ok(b)) => a == b,
            _ => false,
        }
    }

    fn unsubscribe(self) {
        let _ = self.session.RemoveMediaPropertiesChanged(self.media);
        let _ = self.session.RemovePlaybackInfoChanged(self.playback);
        let _ = self.session.RemoveTimelinePropertiesChanged(self.timeline);
    }
}

/// The session to follow: the one the user pinned, or whichever Windows says is
/// current, which is already "whichever is playing".
fn pick(manager: &Manager, follow: &Follow) -> Option<Session> {
    if let Follow::Pinned(id) = follow {
        let sessions = manager.GetSessions().ok()?;
        for i in 0..sessions.Size().ok()? {
            let Ok(s) = sessions.GetAt(i) else { continue };
            if s.SourceAppUserModelId().is_ok_and(|a| a == *id) {
                return Some(s);
            }
        }
        // The pinned player has gone; show nothing rather than silently following
        // something the user did not ask for.
        return None;
    }
    manager.GetCurrentSession().ok()
}

fn list_players(manager: &Manager, apps: &[super::AudioApp]) -> Vec<Player> {
    let Ok(sessions) = manager.GetSessions() else {
        return Vec::new();
    };
    let count = sessions.Size().unwrap_or(0);
    let mut out = Vec::with_capacity(count as usize);
    for i in 0..count {
        let Ok(s) = sessions.GetAt(i) else { continue };
        let Ok(id) = s.SourceAppUserModelId() else {
            continue;
        };
        out.push(player_of(&id.to_string(), apps));
    }
    out.sort_by_key(|a| a.name.to_lowercase());
    out
}

fn player_of(app_id: &str, apps: &[super::AudioApp]) -> Player {
    Player {
        id: app_id.to_owned(),
        name: display_name(app_id),
        pid: super::sessions::match_app_id(apps, app_id).map(|a| a.pid),
    }
}

/// Reads a session's metadata, play state, controls and position into `into`.
fn read_into(session: &Session, apps: &[super::AudioApp], into: &mut Snapshot) {
    if let Ok(id) = session.SourceAppUserModelId() {
        into.player = Some(player_of(&id.to_string(), apps));
    }
    if let Ok(props) = session
        .TryGetMediaPropertiesAsync()
        .and_then(|op| op.join())
    {
        let text = |r: windows_core::Result<windows_core::HSTRING>| {
            r.map(|s| s.to_string()).unwrap_or_default()
        };
        into.track.title = text(props.Title());
        into.track.album = text(props.AlbumTitle());
        // SMTC has one artist string, which players fill with whatever they like;
        // the album artist is the better answer when the track's own is empty.
        let artist = match text(props.Artist()) {
            a if a.is_empty() => text(props.AlbumArtist()),
            a => a,
        };
        into.track.artists = if artist.is_empty() {
            Vec::new()
        } else {
            vec![artist]
        };
        // No composer and no year: SMTC does not carry them.
    }
    if let Ok(info) = session.GetPlaybackInfo() {
        into.state = match info.PlaybackStatus() {
            Ok(Status::Playing) => PlayState::Playing,
            Ok(Status::Paused) | Ok(Status::Changing) => PlayState::Paused,
            _ => PlayState::Stopped,
        };
        if let Ok(c) = info.Controls() {
            let yes = |r: windows_core::Result<bool>| r.unwrap_or(false);
            into.controls = Controls {
                play_pause: yes(c.IsPlayPauseToggleEnabled())
                    || (yes(c.IsPlayEnabled()) && yes(c.IsPauseEnabled())),
                next: yes(c.IsNextEnabled()),
                previous: yes(c.IsPreviousEnabled()),
                seek: yes(c.IsPlaybackPositionEnabled()),
            };
        }
    }
    if let Ok(timeline) = session.GetTimelineProperties() {
        let length = timeline
            .EndTime()
            .ok()
            .map(|t| t.Duration)
            .filter(|&d| d > 0)
            .map(|d| Duration::from_nanos(d as u64 * 100));
        into.track.length = length;
        if let Ok(position) = timeline.Position() {
            let at = timeline
                .LastUpdatedTime()
                .ok()
                .and_then(|t| instant_of(t.UniversalTime))
                .unwrap_or_else(Instant::now);
            let seconds = Duration::from_nanos(position.Duration.max(0) as u64 * 100);
            into.position = Some((seconds, at));
        }
    }
}

/// Turns a WinRT `DateTime` into the instant it happened.
///
/// The position a player reports is often a second or two old, and the seek bar is
/// visibly wrong if that is treated as "now". `LastUpdatedTime` says when the reading
/// was taken, in 100 ns ticks since 1601; comparing it with the clock gives its age,
/// and the age gives the instant.
fn instant_of(universal_time: i64) -> Option<Instant> {
    /// Seconds between 1601-01-01 and the Unix epoch.
    const EPOCH_OFFSET: u64 = 11_644_473_600;
    if universal_time <= 0 {
        return None;
    }
    let now_unix = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
    let now_ticks = (now_unix.as_nanos() / 100) as u64 + EPOCH_OFFSET * 10_000_000;
    let age_ticks = now_ticks.checked_sub(universal_time as u64)?;
    let age = Duration::from_nanos(age_ticks.saturating_mul(100));
    // A reading from another era is a clock that has been changed under us, or a
    // player writing nonsense; either way it says nothing about when.
    if age > Duration::from_secs(3600) {
        return None;
    }
    Instant::now().checked_sub(age)
}

/// What the thumbnail belongs to, so it is read once a track rather than once a second.
fn art_key(track: &NowPlaying) -> String {
    format!("{}\u{1}{}\u{1}", track.title, track.album) + &track.artists_line()
}

/// Reads the session's thumbnail into memory.
fn read_thumbnail(session: &Session) -> Option<Arc<[u8]>> {
    let props = session
        .TryGetMediaPropertiesAsync()
        .and_then(|op| op.join())
        .ok()?;
    let reference = props.Thumbnail().ok()?;
    let stream = reference.OpenReadAsync().and_then(|op| op.join()).ok()?;
    let size = stream.Size().ok()?;
    if size == 0 || size > MAX_THUMBNAIL {
        if size > MAX_THUMBNAIL {
            log::debug!("ignoring a {size} byte thumbnail");
        }
        return None;
    }
    let reader = DataReader::CreateDataReader(&stream).ok()?;
    let loaded = reader
        .LoadAsync(size as u32)
        .and_then(|op| op.join())
        .ok()?;
    let mut bytes = vec![0u8; loaded as usize];
    reader.ReadBytes(&mut bytes).ok()?;
    Some(Arc::from(bytes.into_boxed_slice()))
}

fn perform(session: &Session, command: Transport) {
    let done = match command {
        Transport::PlayPause => session.TryTogglePlayPauseAsync().and_then(|op| op.join()),
        Transport::Next => session.TrySkipNextAsync().and_then(|op| op.join()),
        Transport::Previous => session.TrySkipPreviousAsync().and_then(|op| op.join()),
        Transport::SeekTo(to) => {
            let ticks = (to.as_nanos() / 100) as i64;
            session
                .TryChangePlaybackPositionAsync(ticks)
                .and_then(|op| op.join())
        }
    };
    match done {
        Ok(true) => {}
        Ok(false) => log::debug!("the player refused {command:?}"),
        Err(e) => log::debug!("{command:?} failed: {e}"),
    }
}

/// A readable name for an app user model id.
///
/// A desktop app reports its executable, such as `MusicBee.exe`. A packaged one reports
/// a family name and an application id joined by `!`, such as
/// `SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify`, where only the tail is worth showing.
pub fn display_name(app_id: &str) -> String {
    let tail = app_id.rsplit('!').next().unwrap_or(app_id);
    let name = tail
        .strip_suffix(".exe")
        .or_else(|| tail.strip_suffix(".EXE"))
        .unwrap_or(tail);
    // `Microsoft.ZuneMusic` and the like: the last part is the name.
    let name = name.rsplit('.').next().unwrap_or(name);
    if name.is_empty() {
        app_id.to_owned()
    } else {
        name.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::display_name;

    #[test]
    fn app_ids_become_names_worth_showing() {
        assert_eq!(display_name("MusicBee.exe"), "MusicBee");
        assert_eq!(display_name("Spotify.exe"), "Spotify");
        assert_eq!(
            display_name("SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"),
            "Spotify"
        );
        assert_eq!(
            display_name("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(display_name("firefox.exe"), "firefox");
        assert_eq!(display_name(""), "");
    }
}
