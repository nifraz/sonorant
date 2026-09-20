//! The app's side of now playing: the session, the position, and the artwork on its
//! way to the GPU.
//!
//! Everything slow happens somewhere else. The session watches the desktop's players on
//! a thread of its own, the loader reads and fetches pictures on another, and this only
//! copies a snapshot, decodes a picture that has already arrived, and hands the result
//! to the deck.

use std::time::Instant;

use sonorant_core::media::{
    ArtLoader, Follow, MediaSession, Player, PositionClock, Snapshot, Transport,
};
use sonorant_render::TrackInfo;
use sonorant_render::artwork::{self, Artwork, ArtworkPass};

/// What's playing, as the app needs it.
#[derive(Debug)]
pub struct NowPlaying {
    session: Option<Box<dyn MediaSession>>,
    loader: ArtLoader,
    clock: PositionClock,
    snapshot: Snapshot,
    /// The generation last reacted to, so a snapshot is only unpacked when it changed.
    seen: Option<u64>,
    track: TrackInfo,
    /// The picture on the GPU, and what it was made from.
    art: Option<Artwork>,
    art_key: Option<String>,
    follow: Follow,
}

impl NowPlaying {
    /// Starts watching the desktop's players. A platform without a media session - or
    /// one where it cannot be opened - leaves everything empty, and the deck simply
    /// has nothing to show.
    pub fn start() -> NowPlaying {
        NowPlaying {
            session: open(),
            // No fetcher: `https://` artwork waits on a decision about shipping a TLS
            // stack, and the loader logs what it skipped.
            loader: ArtLoader::new(None),
            clock: PositionClock::new(),
            snapshot: Snapshot::default(),
            seen: None,
            track: TrackInfo::default(),
            art: None,
            art_key: None,
            follow: Follow::default(),
        }
    }

    /// Reads the session and takes in any artwork that has arrived. Returns true when
    /// a new track started, which is the caller's cue to start the programme measures
    /// again.
    pub fn poll(
        &mut self,
        now: Instant,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        pass: &ArtworkPass,
    ) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let next = session.snapshot();
        let mut new_track = false;
        if self.seen != Some(next.generation) {
            new_track = self.seen.is_some() && next.track.is_new_track(&self.snapshot.track);
            if new_track || self.seen.is_none() {
                // A new track's position starts again from whatever it reports next,
                // not from where the last one had got to.
                self.clock.clear();
            }
            self.seen = Some(next.generation);
            self.track = TrackInfo::from(&next.track);
            match &next.track.art {
                Some(source) => self.loader.want(source),
                None => {
                    self.loader.want_nothing();
                    self.art = None;
                    self.art_key = None;
                }
            }
            self.snapshot = next;
        } else {
            // The position moves without the rest of the snapshot changing.
            self.snapshot.position = next.position;
            self.snapshot.state = next.state;
        }

        self.clock.set_length(self.snapshot.track.length);
        self.clock
            .set_playing(self.snapshot.state.is_playing(), now);
        if let Some((position, at)) = self.snapshot.position {
            self.clock.sync(position, at);
        }

        if let Some((key, bytes)) = self.loader.take() {
            match bytes.as_deref().and_then(artwork::decode) {
                Some(picture) => {
                    self.art = Some(pass.upload(device, queue, &picture));
                    self.art_key = Some(key);
                }
                None => {
                    self.art = None;
                    self.art_key = None;
                }
            }
        }
        new_track
    }

    pub fn track(&self) -> &TrackInfo {
        &self.track
    }

    pub fn art(&self) -> Option<&Artwork> {
        self.art.as_ref()
    }

    /// Position and length in seconds, as the deck's seek bar wants them.
    pub fn position(&self, now: Instant) -> Option<(f64, f64)> {
        self.clock.deck_position(now)
    }

    pub fn playing(&self) -> bool {
        self.snapshot.state.is_playing()
    }

    /// The player being followed, if any.
    pub fn player(&self) -> Option<&Player> {
        self.snapshot.player.as_ref()
    }

    /// Every player the session can see, for the "Follow player" menu.
    pub fn players(&self) -> &[Player] {
        &self.snapshot.players
    }

    pub fn set_follow(&mut self, choice: Follow) {
        if self.follow == choice {
            return;
        }
        if let Some(session) = &self.session {
            session.follow(choice.clone());
        }
        self.follow = choice;
    }

    /// Asks the player to do something, if it says it can.
    ///
    /// The deck's transport buttons reach this in Phase 5, when there is input to
    /// press them with; what it guards is already here, so a control the player says
    /// it cannot do never turns into a command.
    #[expect(dead_code, reason = "the deck's buttons are wired up in Phase 5")]
    pub fn send(&self, command: Transport) -> bool {
        let controls = self.snapshot.controls;
        let allowed = match command {
            Transport::PlayPause => controls.play_pause,
            Transport::Next => controls.next,
            Transport::Previous => controls.previous,
            Transport::SeekTo(_) => controls.seek,
        };
        if !allowed {
            return false;
        }
        self.session.as_ref().is_some_and(|s| s.send(command))
    }
}

#[cfg(windows)]
fn open() -> Option<Box<dyn MediaSession>> {
    Some(Box::new(sonorant_platform::windows::SmtcSession::start()))
}

#[cfg(target_os = "linux")]
fn open() -> Option<Box<dyn MediaSession>> {
    match sonorant_platform::linux::MprisSession::start() {
        Ok(session) => Some(Box::new(session)),
        Err(e) => {
            log::warn!("no MPRIS: {e}; now playing will be empty");
            None
        }
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
fn open() -> Option<Box<dyn MediaSession>> {
    None
}
