//! Linux: PipeWire capture, of the whole mix or one app, and now playing from MPRIS.

pub mod appearance;
pub mod awake;
pub mod mpris;
#[cfg(feature = "capture")]
pub mod pipewire;
#[cfg(feature = "capture")]
pub mod sink_delay;

#[cfg(feature = "capture")]
pub use self::pipewire::{AudioApp, PipeWireSource, Target, audio_apps};
pub use appearance::{Appearance, appearance};
pub use awake::ScreenAwake;
pub use mpris::MprisSession;
