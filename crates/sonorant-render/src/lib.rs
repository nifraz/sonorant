//! Sonorant's GPU renderer, built on wgpu and WGSL shaders.

pub mod axes;
pub mod colour;
pub mod curves;
pub mod history;
pub mod layout;
pub mod overlay;
pub mod readback;
pub mod spectrogram;

pub use colour::Rgba;
pub use curves::{CurveData, CurveLook, CurvePass, CurveView};
pub use history::{HistoryStore, RowIn, RowMeta};
pub use layout::{PaneLayout, Rect, ScopeLayout};
pub use overlay::{Face, Layer, Overlay};
pub use readback::Readback;
pub use spectrogram::{PaneView, SpectrogramPass};
