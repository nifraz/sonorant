//! Windows: WASAPI capture, of the whole system or one app, and now playing from the
//! system media transport controls.

pub mod sessions;
pub mod smtc;
pub mod wasapi;

pub use sessions::{AudioApp, audio_apps, match_app_id};
pub use smtc::SmtcSession;
pub use wasapi::{Target, WasapiSource};
