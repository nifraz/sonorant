//! Display columns to frequency bands, and musical note naming.

use std::fmt;

/// How frequency is laid out along the axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum FreqScale {
    Linear,
    Log,
    /// Logarithmic, with gridlines on notes rather than decades.
    #[default]
    Note,
}

impl FreqScale {
    pub const ALL: [FreqScale; 3] = [FreqScale::Linear, FreqScale::Log, FreqScale::Note];

    pub fn name(self) -> &'static str {
        match self {
            FreqScale::Linear => "Linear",
            FreqScale::Log => "Log",
            FreqScale::Note => "Note",
        }
    }

    pub fn from_name(name: &str) -> Option<FreqScale> {
        Self::ALL
            .into_iter()
            .find(|s| s.name().eq_ignore_ascii_case(name))
    }
}

/// Band edges for each display column.
///
/// Each column owns a frequency interval, not a point, so the analyser can combine
/// every FFT bin inside it. That is what stops the high end aliasing into noise on a
/// log axis. Built when the axis or its size changes, then reused every hop.
#[derive(Clone, Debug, PartialEq)]
pub struct FrequencyMap {
    pub scale: FreqScale,
    pub width: usize,
    pub fmin: f64,
    pub fmax: f64,
    /// `width + 1` edges: column `i` spans `edges[i]..edges[i + 1]`.
    pub edges: Vec<f64>,
    /// The geometric centre of each column.
    pub centres: Vec<f64>,
}

impl FrequencyMap {
    pub fn new(scale: FreqScale, width: usize, fmin: f64, fmax: f64) -> FrequencyMap {
        let width = width.max(1);
        let edges: Vec<f64> = (0..=width)
            .map(|i| position_to_freq(i as f64 / width as f64, scale, fmin, fmax))
            .collect();
        let centres = edges.windows(2).map(|e| (e[0] * e[1]).sqrt()).collect();
        FrequencyMap {
            scale,
            width,
            fmin,
            fmax,
            edges,
            centres,
        }
    }

    /// Where a frequency sits along the axis, 0 at `fmin` and 1 at `fmax`.
    pub fn freq_to_position(&self, f: f64) -> f64 {
        if self.scale == FreqScale::Linear {
            return (f - self.fmin) / (self.fmax - self.fmin);
        }
        if f <= 0.0 {
            return 0.0;
        }
        (f / self.fmin).ln() / (self.fmax / self.fmin).ln()
    }

    pub fn freq_to_x(&self, f: f64) -> f64 {
        self.freq_to_position(f) * self.width as f64
    }

    pub fn x_to_freq(&self, x: f64) -> f64 {
        position_to_freq(x / self.width as f64, self.scale, self.fmin, self.fmax)
    }
}

fn position_to_freq(t: f64, scale: FreqScale, fmin: f64, fmax: f64) -> f64 {
    match scale {
        FreqScale::Linear => fmin + (fmax - fmin) * t,
        // Log and Note share a mapping; they differ only in where the gridlines go.
        FreqScale::Log | FreqScale::Note => fmin * (fmax / fmin).powf(t),
    }
}

/// `log2` computed as Nostalgia+ did, `ln(x) / ln(2)`, so values that sit exactly on a
/// rounding boundary land on the same side.
pub(crate) fn log2_ratio(x: f64) -> f64 {
    x.ln() / 2.0f64.ln()
}

pub fn midi_to_freq(midi: f64) -> f64 {
    440.0 * 2.0f64.powf((midi - 69.0) / 12.0)
}

pub fn freq_to_midi(f: f64) -> f64 {
    69.0 + 12.0 * log2_ratio(f / 440.0)
}

/// A note in scientific pitch notation: A4 is 440 Hz, C4 is middle C.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Note {
    /// 0 for C up to 11 for B.
    pub pitch_class: u8,
    pub octave: i32,
}

const NOTE_NAMES: [&str; 12] = [
    "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
];

impl Note {
    pub fn name(self) -> &'static str {
        NOTE_NAMES[self.pitch_class as usize]
    }
}

impl fmt::Display for Note {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}{}", self.name(), self.octave)
    }
}

/// The nearest note and the signed deviation from it in cents, or `None` for a
/// frequency of zero or less.
///
/// A frequency exactly between two notes rounds to the even MIDI number, as .NET's
/// `Math.Round` did, so readouts match Nostalgia+ at the boundary.
pub fn describe_note(freq: f64) -> Option<(Note, f64)> {
    if freq <= 0.0 {
        return None;
    }
    let midi = freq_to_midi(freq);
    let nearest = midi.round_ties_even();
    let cents = (midi - nearest) * 100.0;
    let nearest = nearest as i32;
    let pitch_class = nearest.rem_euclid(12) as u8;
    // Truncating division, as the original: only differs below MIDI 0, far under 20 Hz.
    let octave = nearest / 12 - 1;
    Some((
        Note {
            pitch_class,
            octave,
        },
        cents,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn harness_notes() {
        let cases = [(440.0, "A4"), (261.626, "C4"), (41.203, "E1")];
        for (f, name) in cases {
            let (n, c) = describe_note(f).unwrap();
            assert_eq!(n.to_string(), name);
            assert!(c.abs() < 0.5);
        }
        let (n, c) = describe_note(444.0).unwrap();
        assert_eq!(n.to_string(), "A4");
        assert!(c > 10.0 && c < 20.0);
        assert!(describe_note(0.0).is_none());
    }

    #[test]
    fn columns_tile_the_range() {
        for scale in FreqScale::ALL {
            let m = FrequencyMap::new(scale, 300, 20.0, 20000.0);
            assert_eq!(m.edges.len(), 301);
            assert!((m.edges[0] - 20.0).abs() < 1e-9 && (m.edges[300] - 20000.0).abs() < 1e-6);
            assert!(m.edges.windows(2).all(|e| e[1] > e[0]));
            assert!((m.freq_to_position(m.x_to_freq(123.0)) * 300.0 - 123.0).abs() < 1e-9);
        }
    }
}
