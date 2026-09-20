//! Sonorant's GPU renderer, built on wgpu and WGSL shaders.

pub mod artwork;
pub mod axes;
pub mod band;
pub mod bloom;
pub mod colour;
pub mod curves;
pub mod deck;
pub mod history;
pub mod layout;
pub mod overlay;
pub mod readback;
pub mod spectrogram;
pub mod timing;

pub use artwork::{Artwork, ArtworkPass, Picture};
pub use band::{BandLayout, DeckLayout};
pub use bloom::Visuals;
pub use colour::Rgba;
pub use curves::{CurveData, CurveLook, CurvePass, CurveView};
pub use deck::{DeckState, TrackInfo, WaveRing};
pub use history::{HistoryStore, RowIn, RowMeta};
pub use layout::{PaneLayout, Rect, ScopeLayout};
pub use overlay::{Face, Layer, Overlay};
pub use readback::Readback;
pub use spectrogram::{PaneView, SpectrogramPass};
pub use timing::GpuTimer;
