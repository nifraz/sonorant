//! Now playing: what the deck shows, where it comes from, and the clock that keeps the
//! position moving between the updates a player sends.
//!
//! A [`MediaSession`] is a background thread watching the desktop's players: SMTC on
//! Windows, MPRIS on Ubuntu. The app reads a [`Snapshot`] once a frame, which is a
//! cheap copy out of a mutex, and never waits on D-Bus or WinRT in the frame loop.
//!
//! Neither platform tells us the position continuously, so nothing here treats the
//! reported position as current: it is a reading with a time attached, and
//! [`PositionClock`] carries it forward.

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Whether the player is playing, paused or stopped.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PlayState {
    #[default]
    Stopped,
    Paused,
    Playing,
}

impl PlayState {
    pub fn is_playing(self) -> bool {
        self == PlayState::Playing
    }
}

/// What the player will let us do. A control it can't do stays unwired and the deck
/// hides it, rather than offering a button that does nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Controls {
    pub play_pause: bool,
    pub next: bool,
    pub previous: bool,
    pub seek: bool,
}

/// Something to ask the player to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    PlayPause,
    Next,
    Previous,
    /// An absolute position in the current track.
    SeekTo(Duration),
}

/// Where a track's artwork lives.
///
/// The session hands over the cheapest form it has: SMTC opens a stream and gives us
/// bytes, MPRIS gives a URL. Reading a file or fetching a URL happens off the frame
/// loop, in [`crate::media::ArtLoader`]'s worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ArtSource {
    /// Encoded bytes the session already held, such as an SMTC thumbnail.
    Bytes(Arc<[u8]>),
    /// A local file to read, from a `file://` URL.
    File(PathBuf),
    /// A picture to fetch over the network. Spotify and browsers report these.
    Url(String),
}

impl ArtSource {
    /// A short, stable name for this artwork, so the same picture is decoded once.
    ///
    /// Bytes are keyed by length and a cheap digest rather than the whole buffer: the
    /// key is compared every frame and copying half a megabyte to do it would be
    /// sillier than the collision it avoids.
    pub fn key(&self) -> String {
        match self {
            ArtSource::Bytes(b) => {
                let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
                for chunk in b.chunks(64) {
                    for &byte in chunk.iter().take(8) {
                        hash ^= byte as u64;
                        hash = hash.wrapping_mul(0x100_0000_01b3);
                    }
                }
                format!("bytes:{}:{hash:016x}", b.len())
            }
            ArtSource::File(p) => format!("file:{}", p.display()),
            ArtSource::Url(u) => format!("url:{u}"),
        }
    }
}

/// One track, as the deck shows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NowPlaying {
    /// The player's own identity for this track. A new one starts the programme
    /// measures again; a changed title with the same id does not.
    pub track_id: String,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub composer: String,
    /// The release year as four digits, or empty.
    pub year: String,
    pub length: Option<Duration>,
    pub art: Option<ArtSource>,
}

impl NowPlaying {
    /// The artists on one line, as the deck shows them.
    pub fn artists_line(&self) -> String {
        self.artists.join(", ")
    }

    /// Whether this is a different track from `other`, and so the programme measures
    /// should start again.
    ///
    /// The id is the player's word for it and is trusted when there is one. Players
    /// that report no id (some browsers) fall back to the title and album, which is
    /// enough to catch a track change and not so eager that a metadata refresh on the
    /// same track throws away a minute of integrated loudness.
    pub fn is_new_track(&self, other: &NowPlaying) -> bool {
        if !self.track_id.is_empty() || !other.track_id.is_empty() {
            return self.track_id != other.track_id;
        }
        self.title != other.title || self.album != other.album
    }
}

/// A player the session can see.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Player {
    /// How the session names it: an MPRIS bus name, or an SMTC app id.
    pub id: String,
    /// What to show in a menu, such as "MusicBee" or "Spotify".
    pub name: String,
    /// The process behind it, when the platform will say. Capture uses it to follow
    /// the player.
    pub pid: Option<u32>,
}

/// Which player to follow.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Follow {
    /// Whichever player is playing, preferring the one that started most recently.
    #[default]
    Whichever,
    /// One the user pinned, by [`Player::id`].
    Pinned(String),
}

/// Everything the app reads from a session in one frame.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    /// The player this is about, or `None` when no player is running.
    pub player: Option<Player>,
    pub track: NowPlaying,
    pub state: PlayState,
    pub controls: Controls,
    /// The position the player last reported and the instant we heard it, which is not
    /// the position now. Feed it to a [`PositionClock`].
    pub position: Option<(Duration, Instant)>,
    /// Every player the session can see, for the "Follow player" menu.
    pub players: Vec<Player>,
    /// Bumped whenever anything above changed, so the app can skip the work of
    /// reacting to a snapshot it has already seen.
    pub generation: u64,
}

/// A source of now-playing information.
///
/// Implementations own a thread and keep a snapshot behind a mutex; every method here
/// is expected to return without blocking on the platform, because the frame loop
/// calls [`MediaSession::snapshot`] once a frame.
pub trait MediaSession: Send + fmt::Debug {
    /// The current state of the followed player.
    fn snapshot(&self) -> Snapshot;
    /// Chooses which player to follow. Takes effect on the next update.
    fn follow(&self, choice: Follow);
    /// Asks the player to do something. Returns false if it was not sent, which is
    /// what happens when the player says it can't.
    fn send(&self, command: Transport) -> bool;
}

/// How long a reported position is trusted before the session is asked again.
///
/// A second is often enough to keep a seek bar honest and rare enough that a player
/// polled over D-Bus doesn't notice.
pub const RESYNC_AFTER: Duration = Duration::from_millis(1000);

/// The playing position, carried forward between the readings a player sends.
///
/// MPRIS has no signal for the position changing: it moves because time passes, and
/// jumps only on `Seeked`. SMTC is the same in practice, updating its timeline
/// properties when it feels like it. So the position is held as a reading with a time
/// attached and advanced by the clock, which keeps the seek bar smooth instead of
/// stepping once a second, and resyncs often enough that drift never shows.
#[derive(Clone, Debug, Default)]
pub struct PositionClock {
    /// What the player said, and when we heard it.
    reading: Option<(Duration, Instant)>,
    /// When the reading was last replaced, so we know when to ask again.
    synced: Option<Instant>,
    playing: bool,
    length: Option<Duration>,
}

impl PositionClock {
    pub fn new() -> PositionClock {
        PositionClock::default()
    }

    /// Takes a fresh reading. `at` is when the player reported it.
    pub fn sync(&mut self, position: Duration, at: Instant) {
        self.reading = Some((position, at));
        self.synced = Some(at);
    }

    /// The player started or stopped moving through the track.
    ///
    /// The position is first brought up to date at the old state, so a pause keeps
    /// what had been played rather than freezing at the last reading.
    pub fn set_playing(&mut self, playing: bool, now: Instant) {
        if playing == self.playing {
            return;
        }
        if let Some(at) = self.at(now) {
            self.reading = Some((at, now));
        }
        self.playing = playing;
    }

    pub fn set_length(&mut self, length: Option<Duration>) {
        self.length = length;
    }

    pub fn length(&self) -> Option<Duration> {
        self.length
    }

    /// Forgets everything, for a track change.
    pub fn clear(&mut self) {
        *self = PositionClock {
            playing: self.playing,
            ..PositionClock::default()
        };
    }

    /// The position now, or `None` when no player has reported one.
    pub fn at(&self, now: Instant) -> Option<Duration> {
        let (position, heard) = self.reading?;
        let mut out = position;
        if self.playing {
            out += now.saturating_duration_since(heard);
        }
        if let Some(length) = self.length
            && out > length
        {
            out = length;
        }
        Some(out)
    }

    /// The position and the track's length, as the deck wants them: seconds, and only
    /// when both are known and the length is real.
    pub fn deck_position(&self, now: Instant) -> Option<(f64, f64)> {
        let at = self.at(now)?;
        let length = self.length?;
        if length.is_zero() {
            return None;
        }
        Some((at.as_secs_f64(), length.as_secs_f64()))
    }

    /// Whether the reading is old enough to ask the player again.
    pub fn needs_resync(&self, now: Instant) -> bool {
        match self.synced {
            // Only a playing track drifts; a paused one stays where it was put.
            Some(at) => self.playing && now.saturating_duration_since(at) >= RESYNC_AFTER,
            None => true,
        }
    }
}

/// Fetches a picture from the network for [`ArtLoader`].
///
/// Nothing implements this yet. MPRIS players that keep their artwork on the web -
/// Spotify and the browsers - report an `https://` URL, and reaching it means an HTTP
/// client and a TLS stack, which is a download-size decision left until the size pass.
/// Until then the loader logs what it could not reach and the deck shows its empty
/// frame, which is what those players get today.
pub trait ArtFetcher: Send {
    /// The encoded picture at `url`, or `None` if it could not be had.
    fn fetch(&self, url: &str) -> Option<Arc<[u8]>>;
}

/// Turns an [`ArtSource`] into encoded bytes, off the frame loop.
///
/// Reading a file takes a millisecond and fetching a URL can take a second, and neither
/// belongs in a frame or on the session's thread, where it would hold up the next track
/// change. One request is in flight at a time: artwork that has been superseded is not
/// worth waiting for, so a new request replaces a queued one.
#[derive(Debug)]
pub struct ArtLoader {
    requests: std::sync::mpsc::Sender<(String, ArtSource)>,
    results: std::sync::mpsc::Receiver<(String, Option<Arc<[u8]>>)>,
    /// The key we last asked for, so the same picture isn't loaded twice.
    asked: Option<String>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl ArtLoader {
    /// Starts the worker. `fetcher` handles `https://` sources; without one they are
    /// logged and skipped.
    pub fn new(fetcher: Option<Box<dyn ArtFetcher>>) -> ArtLoader {
        let (requests, rx) = std::sync::mpsc::channel::<(String, ArtSource)>();
        let (tx, results) = std::sync::mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("sonorant-artwork".into())
            .spawn(move || {
                while let Ok((key, source)) = rx.recv() {
                    // Anything that queued up behind this one has already been
                    // replaced by something newer; skip to the last.
                    let (key, source) = rx.try_iter().last().unwrap_or((key, source));
                    let bytes = match &source {
                        ArtSource::Bytes(b) => Some(Arc::clone(b)),
                        ArtSource::File(path) => match std::fs::read(path) {
                            Ok(b) => Some(Arc::from(b.into_boxed_slice())),
                            Err(e) => {
                                log::debug!("artwork: cannot read {}: {e}", path.display());
                                None
                            }
                        },
                        ArtSource::Url(url) => match &fetcher {
                            Some(f) => f.fetch(url),
                            None => {
                                log::debug!("artwork: {url} needs a fetcher, which isn't built in");
                                None
                            }
                        },
                    };
                    if tx.send((key, bytes)).is_err() {
                        return;
                    }
                }
            })
            .ok();
        ArtLoader {
            requests,
            results,
            asked: None,
            thread,
        }
    }

    /// Asks for `source`, unless it is already the one being loaded.
    pub fn want(&mut self, source: &ArtSource) {
        let key = source.key();
        if self.asked.as_deref() == Some(key.as_str()) {
            return;
        }
        self.asked = Some(key.clone());
        let _ = self.requests.send((key, source.clone()));
    }

    /// Forgets what was asked for, so nothing is loaded until a track asks again.
    pub fn want_nothing(&mut self) {
        self.asked = None;
    }

    /// A loaded picture, if one is ready. Results for artwork that has since been
    /// replaced are thrown away here rather than reaching the screen.
    pub fn take(&mut self) -> Option<(String, Option<Arc<[u8]>>)> {
        let mut out = None;
        while let Ok((key, bytes)) = self.results.try_recv() {
            if self.asked.as_deref() == Some(key.as_str()) {
                out = Some((key, bytes));
            }
        }
        out
    }
}

impl Drop for ArtLoader {
    fn drop(&mut self) {
        // Dropping the sender ends the worker's loop; then wait for the read it is in.
        let (dead, _) = std::sync::mpsc::channel();
        let _ = std::mem::replace(&mut self.requests, dead);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Reads the four-digit year out of an ISO 8601 date, as MPRIS reports one.
///
/// `xesam:contentCreated` is a full timestamp such as `2004-03-15T00:00:00Z`, and
/// players are loose about it: some send only the year, some send nothing useful.
pub fn year_from_date(date: &str) -> String {
    let head: String = date.chars().take_while(char::is_ascii_digit).collect();
    if head.len() == 4 { head } else { String::new() }
}

/// Turns a `file://` URL into a path, leaving anything else alone.
///
/// Players percent-encode the path, and some of them encode more than they need to, so
/// the decoding is done on bytes and any stray `%` is kept as written rather than
/// dropping the character after it.
pub fn art_source_from_url(url: &str) -> Option<ArtSource> {
    if let Some(rest) = url.strip_prefix("file://") {
        // An empty or `localhost` authority, then the path.
        let path = match rest.find('/') {
            Some(0) => rest,
            Some(i) if rest[..i].eq_ignore_ascii_case("localhost") => &rest[i..],
            _ => return None,
        };
        let decoded = percent_decode(path);
        // A Windows path arrives as /C:/..., which needs its leading slash off.
        let trimmed = match decoded.as_bytes() {
            [b'/', c, b':', ..] if c.is_ascii_alphabetic() => &decoded[1..],
            _ => &decoded[..],
        };
        return Some(ArtSource::File(PathBuf::from(trimmed)));
    }
    if url.starts_with("http://") || url.starts_with("https://") {
        return Some(ArtSource::Url(url.to_owned()));
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (
                (bytes[i + 1] as char).to_digit(16),
                (bytes[i + 2] as char).to_digit(16),
            )
        {
            out.push((hi * 16 + lo) as u8);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_position_runs_on_while_playing_and_holds_while_paused() {
        let start = Instant::now();
        let mut clock = PositionClock::new();
        clock.set_length(Some(Duration::from_secs(200)));
        clock.set_playing(true, start);
        clock.sync(Duration::from_secs(10), start);

        let later = start + Duration::from_secs(5);
        assert_eq!(clock.at(later), Some(Duration::from_secs(15)));

        // Pausing keeps what had been played, and time no longer moves it.
        clock.set_playing(false, later);
        let later_still = later + Duration::from_secs(30);
        assert_eq!(clock.at(later_still), Some(Duration::from_secs(15)));

        // Playing again carries on from there.
        clock.set_playing(true, later_still);
        assert_eq!(
            clock.at(later_still + Duration::from_secs(2)),
            Some(Duration::from_secs(17))
        );
    }

    #[test]
    fn the_position_never_runs_past_the_end() {
        let start = Instant::now();
        let mut clock = PositionClock::new();
        clock.set_playing(true, start);
        clock.set_length(Some(Duration::from_secs(30)));
        clock.sync(Duration::from_secs(29), start);
        assert_eq!(
            clock.at(start + Duration::from_secs(10)),
            Some(Duration::from_secs(30))
        );
    }

    #[test]
    fn a_seek_replaces_the_reading_rather_than_adding_to_it() {
        let start = Instant::now();
        let mut clock = PositionClock::new();
        clock.set_playing(true, start);
        clock.sync(Duration::from_secs(100), start);
        let at = start + Duration::from_secs(4);
        clock.sync(Duration::from_secs(5), at);
        assert_eq!(
            clock.at(at + Duration::from_secs(1)),
            Some(Duration::from_secs(6))
        );
    }

    #[test]
    fn only_a_playing_track_is_resynced() {
        let start = Instant::now();
        let mut clock = PositionClock::new();
        assert!(clock.needs_resync(start), "with no reading, ask");
        clock.set_playing(true, start);
        clock.sync(Duration::ZERO, start);
        assert!(!clock.needs_resync(start + Duration::from_millis(500)));
        assert!(clock.needs_resync(start + RESYNC_AFTER));
        clock.set_playing(false, start + RESYNC_AFTER);
        assert!(!clock.needs_resync(start + Duration::from_secs(60)));
    }

    #[test]
    fn the_deck_only_gets_a_position_when_the_length_is_real() {
        let start = Instant::now();
        let mut clock = PositionClock::new();
        clock.sync(Duration::from_secs(3), start);
        assert_eq!(clock.deck_position(start), None, "no length yet");
        clock.set_length(Some(Duration::ZERO));
        assert_eq!(clock.deck_position(start), None, "a live stream has none");
        clock.set_length(Some(Duration::from_secs(60)));
        assert_eq!(clock.deck_position(start), Some((3.0, 60.0)));
    }

    #[test]
    fn a_track_change_is_the_players_id_when_it_has_one() {
        let a = NowPlaying {
            track_id: "/org/mpris/track/1".into(),
            title: "One".into(),
            ..NowPlaying::default()
        };
        let mut b = a.clone();
        b.title = "One (remastered)".into();
        assert!(!a.is_new_track(&b), "the same id is the same track");
        b.track_id = "/org/mpris/track/2".into();
        assert!(a.is_new_track(&b));
    }

    #[test]
    fn without_an_id_a_track_change_is_the_title_and_album() {
        let a = NowPlaying {
            title: "One".into(),
            album: "Record".into(),
            ..NowPlaying::default()
        };
        let mut b = a.clone();
        b.artists = vec!["Someone".into()];
        assert!(!a.is_new_track(&b), "artists arriving late is not a change");
        b.title = "Two".into();
        assert!(a.is_new_track(&b));
    }

    #[test]
    fn years_come_out_of_whatever_the_player_sent() {
        assert_eq!(year_from_date("2004-03-15T00:00:00Z"), "2004");
        assert_eq!(year_from_date("1978"), "1978");
        assert_eq!(year_from_date(""), "");
        assert_eq!(year_from_date("unknown"), "");
        assert_eq!(year_from_date("78"), "", "two digits is not a year");
    }

    #[test]
    fn art_urls_become_files_or_fetches() {
        assert_eq!(
            art_source_from_url("file:///home/me/Music/cover%20art.jpg"),
            Some(ArtSource::File("/home/me/Music/cover art.jpg".into()))
        );
        assert_eq!(
            art_source_from_url("file://localhost/home/me/a.png"),
            Some(ArtSource::File("/home/me/a.png".into()))
        );
        assert_eq!(
            art_source_from_url("file:///C:/Users/me/a.png"),
            Some(ArtSource::File("C:/Users/me/a.png".into())),
            "a Windows path loses the slash the URL added"
        );
        assert_eq!(
            art_source_from_url("https://i.scdn.co/image/abc"),
            Some(ArtSource::Url("https://i.scdn.co/image/abc".into()))
        );
        assert_eq!(art_source_from_url("data:image/png;base64,AAA"), None);
    }

    #[test]
    fn a_stray_percent_is_kept_rather_than_eating_the_next_character() {
        assert_eq!(percent_decode("/100%/a.png"), "/100%/a.png");
        assert_eq!(percent_decode("/a%2"), "/a%2");
    }

    #[test]
    fn the_same_bytes_key_the_same_and_different_ones_do_not() {
        let a = ArtSource::Bytes(vec![1u8, 2, 3, 4].into());
        let b = ArtSource::Bytes(vec![1u8, 2, 3, 4].into());
        let c = ArtSource::Bytes(vec![1u8, 2, 3, 5].into());
        assert_eq!(a.key(), b.key());
        assert_ne!(a.key(), c.key());
    }
}
