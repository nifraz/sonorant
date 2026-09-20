//! Linux: PipeWire capture, of the whole mix or one app, and now playing from MPRIS.

pub mod mpris;
#[cfg(feature = "capture")]
pub mod pipewire;

#[cfg(feature = "capture")]
pub use self::pipewire::{AudioApp, PipeWireSource, Target, audio_apps};
pub use mpris::MprisSession;
