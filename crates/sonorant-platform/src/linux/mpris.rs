//! Ubuntu: now playing from MPRIS over D-Bus.
//!
//! Every desktop player worth following speaks MPRIS: Rhythmbox, Strawberry, VLC,
//! Spotify and the browsers all own a `org.mpris.MediaPlayer2.*` name on the session
//! bus and put the track behind it.
//!
//! The work happens on a thread of its own, because a D-Bus round trip to a player
//! that is busy can take longer than a frame. The thread keeps a [`Snapshot`] behind a
//! mutex and the app copies it out once a frame.
//!
//! The players are polled rather than subscribed to. MPRIS does signal metadata
//! changes, through `PropertiesChanged`, and seeks through `Seeked`, and following
//! those would cut both the delay and the traffic; a poll every [`POLL`] is the simpler
//! thing to get right first, and the position is extrapolated between polls either way.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use sonorant_core::media::{
    ArtSource, Controls, Follow, MediaSession, NowPlaying, PlayState, Player, Snapshot, Transport,
    art_source_from_url, year_from_date,
};
use zbus::blocking::{
    Connection, Proxy,
    fdo::{DBusProxy, PropertiesProxy},
};
use zbus::names::{BusName, InterfaceName};
use zbus::zvariant::{ObjectPath, OwnedValue};

/// How often the players are asked what they are doing.
///
/// Short enough that a track change reaches the programme measures before anyone
/// notices, long enough that five players on the bus cost nothing worth measuring.
const POLL: Duration = Duration::from_millis(400);

/// The prefix every MPRIS player's bus name starts with.
const MPRIS_PREFIX: &str = "org.mpris.MediaPlayer2.";
const PLAYER_PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER_IFACE: &str = "org.mpris.MediaPlayer2.Player";
const ROOT_IFACE: &str = "org.mpris.MediaPlayer2";

enum Msg {
    Follow(Follow),
    Transport(Transport),
    Stop,
}

#[derive(Debug, Default)]
struct Shared {
    snapshot: Mutex<Snapshot>,
}

/// Now playing from MPRIS.
#[derive(Debug)]
pub struct MprisSession {
    shared: Arc<Shared>,
    tx: SyncSender<Msg>,
    thread: Option<JoinHandle<()>>,
}

impl MprisSession {
    /// Connects to the session bus and starts watching. Fails only when there is no
    /// session bus at all, which is a headless machine or a broken login.
    pub fn start() -> Result<MprisSession, String> {
        // Opened here rather than on the thread so a missing bus is reported to the
        // caller instead of disappearing into a log line.
        let connection = Connection::session().map_err(|e| e.to_string())?;
        let shared = Arc::new(Shared::default());
        let (tx, rx) = sync_channel(64);
        let thread = {
            let shared = Arc::clone(&shared);
            std::thread::Builder::new()
                .name("sonorant-mpris".into())
                .spawn(move || run(&connection, &shared, &rx))
                .ok()
        };
        if thread.is_none() {
            return Err("cannot start the now-playing thread".into());
        }
        Ok(MprisSession { shared, tx, thread })
    }
}

impl MediaSession for MprisSession {
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
        match self.tx.try_send(Msg::Transport(command)) {
            Ok(()) => true,
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => false,
        }
    }
}

impl Drop for MprisSession {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// What a player says about itself, which doesn't change while it is running.
///
/// Asking costs two round trips, so it is asked once per player and kept until that
/// player's name leaves the bus.
struct Known {
    identity: String,
    pid: Option<u32>,
}

fn run(connection: &Connection, shared: &Shared, rx: &Receiver<Msg>) {
    let dbus = match DBusProxy::new(connection) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("no D-Bus proxy: {e}; now playing will be empty");
            return;
        }
    };
    let mut follow = Follow::default();
    let mut known: HashMap<String, Known> = HashMap::new();
    loop {
        let names = mpris_names(&dbus);
        known.retain(|name, _| names.contains(name));

        // One `GetAll` a player: the state, metadata, position and capability flags
        // come back together, so a poll costs a round trip each rather than a dozen.
        let mut players = Vec::with_capacity(names.len());
        let mut properties: HashMap<String, HashMap<String, OwnedValue>> = HashMap::new();
        for name in &names {
            let entry = known
                .entry(name.clone())
                .or_insert_with(|| describe(connection, &dbus, name));
            players.push(Player {
                id: name.clone(),
                name: entry.identity.clone(),
                pid: entry.pid,
            });
            if let Some(all) = all_properties(connection, name) {
                properties.insert(name.clone(), all);
            }
        }
        players.sort_by_key(|p| p.name.to_lowercase());

        let chosen = pick(&players, &properties, &follow);
        let mut next = Snapshot {
            players,
            ..Snapshot::default()
        };
        if let Some(player) = chosen {
            if let Some(all) = properties.get(&player.id) {
                read_into(connection, &player.id, all, &mut next);
            }
            next.player = Some(player);
        }
        publish(shared, next);

        match rx.recv_timeout(POLL) {
            Ok(Msg::Stop) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Ok(Msg::Follow(f)) => follow = f,
            Ok(Msg::Transport(command)) => {
                let target = shared
                    .snapshot
                    .lock()
                    .ok()
                    .and_then(|s| s.player.clone().map(|p| (p, s.track.track_id.clone())));
                if let Some((player, track)) = target {
                    perform(connection, &player, &track, command);
                }
            }
        }
    }
}

/// Every MPRIS player's bus name.
fn mpris_names(dbus: &DBusProxy<'_>) -> Vec<String> {
    let Ok(names) = dbus.list_names() else {
        return Vec::new();
    };
    names
        .into_iter()
        .map(|n| n.as_str().to_owned())
        .filter(|n| n.starts_with(MPRIS_PREFIX))
        .collect()
}

/// What a player calls itself, and the process behind it.
///
/// `Identity` is the player's own name, such as "Rhythmbox"; the tail of the bus name
/// is the fallback for one that won't say. The process id is what lets capture follow
/// the player, and only the bus knows it.
fn describe(connection: &Connection, dbus: &DBusProxy<'_>, name: &str) -> Known {
    let identity = root_proxy(connection, name)
        .and_then(|p| p.get_property::<String>("Identity").ok())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| name[MPRIS_PREFIX.len()..].to_owned());
    let pid = BusName::try_from(name)
        .ok()
        .and_then(|bus| dbus.get_connection_unix_process_id(bus).ok());
    Known { identity, pid }
}

/// Every property of a player's `Player` interface, in one call.
fn all_properties(connection: &Connection, name: &str) -> Option<HashMap<String, OwnedValue>> {
    let interface = InterfaceName::try_from(PLAYER_IFACE).ok()?;
    let proxy = PropertiesProxy::builder(connection)
        .destination(name)
        .ok()?
        .path(PLAYER_PATH)
        .ok()?
        .build()
        .ok()?;
    proxy.get_all(interface).ok()
}

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

/// The player to follow: the one the user pinned, or whichever is playing.
///
/// A paused player is taken only when nothing is playing, so one left paused in
/// another window doesn't take the deck away from what you are listening to.
fn pick(
    players: &[Player],
    properties: &HashMap<String, HashMap<String, OwnedValue>>,
    follow: &Follow,
) -> Option<Player> {
    if let Follow::Pinned(id) = follow {
        return players.iter().find(|p| p.id == *id).cloned();
    }
    let mut paused = None;
    for player in players {
        let state = properties
            .get(&player.id)
            .map(play_state)
            .unwrap_or_default();
        match state {
            PlayState::Playing => return Some(player.clone()),
            PlayState::Paused if paused.is_none() => paused = Some(player.clone()),
            _ => {}
        }
    }
    paused
}

fn root_proxy<'a>(connection: &Connection, id: &'a str) -> Option<Proxy<'a>> {
    Proxy::new(connection, id, PLAYER_PATH, ROOT_IFACE).ok()
}

fn player_proxy<'a>(connection: &Connection, id: &'a str) -> Option<Proxy<'a>> {
    Proxy::new(connection, id, PLAYER_PATH, PLAYER_IFACE).ok()
}

fn play_state(all: &HashMap<String, OwnedValue>) -> PlayState {
    match text(all, "PlaybackStatus").as_str() {
        "Playing" => PlayState::Playing,
        "Paused" => PlayState::Paused,
        _ => PlayState::Stopped,
    }
}

/// Reads a player's metadata, state, controls and position into `into`.
fn read_into(
    connection: &Connection,
    id: &str,
    all: &HashMap<String, OwnedValue>,
    into: &mut Snapshot,
) {
    into.state = play_state(all);

    let flag = |name: &str| {
        all.get(name)
            .and_then(|v| v.downcast_ref::<bool>().ok())
            .unwrap_or(false)
    };
    // `CanControl` is the player's blanket answer: when it is false the rest mean
    // nothing, whatever they say. A player that doesn't send it at all is taken at
    // its word on the individual flags.
    let controllable = all
        .get("CanControl")
        .and_then(|v| v.downcast_ref::<bool>().ok())
        .unwrap_or(true);
    into.controls = Controls {
        play_pause: controllable && (flag("CanPlay") || flag("CanPause")),
        next: controllable && flag("CanGoNext"),
        previous: controllable && flag("CanGoPrevious"),
        seek: controllable && flag("CanSeek"),
    };

    if let Some(metadata) = all
        .get("Metadata")
        .and_then(|v| HashMap::<String, OwnedValue>::try_from(v.clone()).ok())
    {
        into.track = track_from(&metadata);
    }

    // `Position` is in microseconds, and it is a reading rather than a running clock:
    // what it is worth depends on when it was taken, so the instant goes with it.
    // Some players leave it out of `GetAll`, so it is asked for on its own then.
    let micros = number(all, "Position").or_else(|| {
        player_proxy(connection, id).and_then(|p| p.get_property::<i64>("Position").ok())
    });
    if let Some(micros) = micros {
        into.position = Some((Duration::from_micros(micros.max(0) as u64), Instant::now()));
    }
}

/// One track out of an MPRIS metadata map.
fn track_from(metadata: &HashMap<String, OwnedValue>) -> NowPlaying {
    let artists = match strings(metadata, "xesam:artist") {
        list if list.is_empty() => strings(metadata, "xesam:albumArtist"),
        list => list,
    };
    NowPlaying {
        track_id: track_id(metadata),
        title: text(metadata, "xesam:title"),
        artists,
        album: text(metadata, "xesam:album"),
        composer: strings(metadata, "xesam:composer").join(", "),
        year: year_from_date(&text(metadata, "xesam:contentCreated")),
        // `mpris:length` is microseconds; a live stream reports none or zero.
        length: number(metadata, "mpris:length")
            .filter(|&n| n > 0)
            .map(|n| Duration::from_micros(n as u64)),
        art: art(metadata),
    }
}

/// The player's id for the track.
///
/// The spec says an object path, and most players send one, but enough of them send a
/// plain string that both are taken. Either way it is only ever compared, never
/// followed.
fn track_id(metadata: &HashMap<String, OwnedValue>) -> String {
    let Some(value) = metadata.get("mpris:trackid") else {
        return String::new();
    };
    if let Ok(path) = value.downcast_ref::<ObjectPath<'_>>() {
        return path.as_str().to_owned();
    }
    value
        .downcast_ref::<&str>()
        .map(str::to_owned)
        .unwrap_or_default()
}

fn art(metadata: &HashMap<String, OwnedValue>) -> Option<ArtSource> {
    let url = text(metadata, "mpris:artUrl");
    if url.is_empty() {
        return None;
    }
    art_source_from_url(&url)
}

fn text(map: &HashMap<String, OwnedValue>, key: &str) -> String {
    map.get(key)
        .and_then(|v| v.downcast_ref::<&str>().ok())
        .unwrap_or_default()
        .to_owned()
}

fn number(map: &HashMap<String, OwnedValue>, key: &str) -> Option<i64> {
    let value = map.get(key)?;
    if let Ok(n) = value.downcast_ref::<i64>() {
        return Some(n);
    }
    // Some players send the length as a u64 or an i32 instead.
    if let Ok(n) = value.downcast_ref::<u64>() {
        return Some(n as i64);
    }
    value.downcast_ref::<i32>().ok().map(i64::from)
}

/// A list of strings, allowing for the players that send a bare string instead.
fn strings(metadata: &HashMap<String, OwnedValue>, key: &str) -> Vec<String> {
    let Some(value) = metadata.get(key) else {
        return Vec::new();
    };
    if let Ok(list) = Vec::<String>::try_from(value.clone()) {
        return list.into_iter().filter(|s| !s.is_empty()).collect();
    }
    match value.downcast_ref::<&str>() {
        Ok(s) if !s.is_empty() => vec![s.to_owned()],
        _ => Vec::new(),
    }
}

fn perform(connection: &Connection, player: &Player, track_id: &str, command: Transport) {
    let Some(proxy) = player_proxy(connection, &player.id) else {
        return;
    };
    let result = match command {
        Transport::PlayPause => proxy.call::<_, _, ()>("PlayPause", &()),
        Transport::Next => proxy.call::<_, _, ()>("Next", &()),
        Transport::Previous => proxy.call::<_, _, ()>("Previous", &()),
        Transport::SeekTo(to) => {
            // SetPosition needs the track it applies to, so a seek that arrives after
            // the track changed is ignored by the player rather than applied to the
            // wrong one. Without an id there is nothing safe to send.
            let Ok(path) = ObjectPath::try_from(track_id) else {
                log::debug!("no track id to seek within");
                return;
            };
            let micros = (to.as_micros() as i64).max(0);
            proxy.call::<_, _, ()>("SetPosition", &(path, micros))
        }
    };
    if let Err(e) = result {
        log::debug!("{command:?} failed on {}: {e}", player.name);
    }
}
