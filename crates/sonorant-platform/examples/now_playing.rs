//! What the media session sees, printed as it changes. For checking a real player
//! without running the whole app.
//!
//! `cargo run -p sonorant-platform --example now_playing`

#![allow(clippy::print_stdout)] // a command-line report

use std::time::{Duration, Instant};

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("debug")).init();
    #[cfg(windows)]
    watch(sonorant_platform::windows::SmtcSession::start());
    #[cfg(target_os = "linux")]
    match sonorant_platform::linux::MprisSession::start() {
        Ok(session) => watch(session),
        Err(e) => println!("no MPRIS: {e}"),
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    println!("no media session on this platform");
}

#[cfg(any(windows, target_os = "linux"))]
fn watch(session: impl sonorant_core::media::MediaSession) {
    use sonorant_core::media::PositionClock;

    let mut clock = PositionClock::new();
    let mut seen = u64::MAX;
    let until = Instant::now() + Duration::from_secs(20);
    println!("watching for 20 seconds...");
    while Instant::now() < until {
        let s = session.snapshot();
        let now = Instant::now();
        if let Some((position, at)) = s.position {
            clock.sync(position, at);
        }
        clock.set_length(s.track.length);
        clock.set_playing(s.state.is_playing(), now);
        if s.generation != seen {
            seen = s.generation;
            match &s.player {
                Some(p) => println!(
                    "\n-- {} (id {}, pid {:?}), {:?}",
                    p.name, p.id, p.pid, s.state
                ),
                None => println!("\n-- no player"),
            }
            println!("   title    {}", s.track.title);
            println!("   artists  {}", s.track.artists_line());
            println!("   album    {}", s.track.album);
            println!("   composer {}", s.track.composer);
            println!("   year     {}", s.track.year);
            println!("   length   {:?}", s.track.length);
            println!(
                "   art      {}",
                match &s.track.art {
                    Some(a) => a.key(),
                    None => "none".into(),
                }
            );
            println!("   controls {:?}", s.controls);
            println!(
                "   players  {:?}",
                s.players.iter().map(|p| &p.name).collect::<Vec<_>>()
            );
        }
        if let Some((at, length)) = clock.deck_position(now) {
            print!("\r   {at:7.2} / {length:7.2} s   ");
            use std::io::Write;
            let _ = std::io::stdout().flush();
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    println!();
}
