//! Sonorant's GPU renderer, built on wgpu and WGSL shaders.

pub mod history;
pub mod layout;
pub mod spectrogram;

pub use history::{HistoryStore, RowIn, RowMeta};
pub use layout::{PaneLayout, Rect, ScopeLayout};
pub use spectrogram::{PaneView, SpectrogramPass};
