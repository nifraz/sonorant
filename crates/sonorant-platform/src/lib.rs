//! Sonorant's platform layer: audio capture and now-playing sources.
//!
//! Ubuntu uses PipeWire for capture and MPRIS for now-playing; Windows uses WASAPI
//! loopback and the system media transport controls.

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(windows)]
pub mod windows;
