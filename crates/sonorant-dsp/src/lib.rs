//! Sonorant's signal processing.
//!
//! Everything here is plain `f64` arithmetic with no platform dependencies: the
//! multi-resolution FFT bank, BS.1770 loudness and true peak, the rolling-percentile
//! dynamic range, curve shaping and ballistics, the frequency map and the music
//! features. Each part is checked against reference vectors exported from Nostalgia+,
//! whose DSP this replaces (see `tests/reference/` at the workspace root).
//!
//! Nothing here allocates once configured, so all of it can run on the analysis thread
//! every hop.

pub mod analyzer;
pub mod curve;
pub mod dynamic_range;
pub mod features;
pub mod fft;
pub mod frequency_map;
pub mod loudness;
pub mod math;
pub mod window;

pub use analyzer::{
    AnalysisQuality, BandAggregate, ChannelMode, ChannelPairMode, FLOOR_DB, SpectrumAnalyzer,
};
pub use curve::{ChannelCurves, CurveInterpolation, ExtremumTracker, FilteringAmount, smooth};
pub use dynamic_range::DynamicRange;
pub use features::MusicFeatures;
pub use frequency_map::{FreqScale, FrequencyMap, Note, describe_note, freq_to_midi, midi_to_freq};
pub use loudness::{LoudnessMeter, LoudnessReadings};
pub use window::WindowType;
