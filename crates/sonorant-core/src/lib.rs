//! Sonorant's model: settings and presets, palettes, the menu model and the analysis
//! engine that moves audio from capture to the renderer.

pub mod engine;
pub mod legacy;
pub mod palette;
pub mod runtime;
pub mod settings;
pub mod source;
pub mod store;

pub use sonorant_dsp as dsp;
