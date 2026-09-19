//! Linux: PipeWire capture, of the whole mix or one app.

pub mod pipewire;

pub use self::pipewire::{AudioApp, PipeWireSource, Target, audio_apps};
