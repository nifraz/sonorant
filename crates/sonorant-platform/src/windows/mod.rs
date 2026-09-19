//! Windows: WASAPI capture, of the whole system or one app.

pub mod sessions;
pub mod wasapi;

pub use sessions::{AudioApp, audio_apps};
pub use wasapi::{Target, WasapiSource};
