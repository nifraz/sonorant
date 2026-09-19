//! Analysis windows.

use std::f64::consts::PI;
use std::fmt;

/// The taper applied to each FFT frame before the transform.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum WindowType {
    #[default]
    Hann,
    Hamming,
    BlackmanHarris,
    Nuttall,
    Gaussian,
    Rectangular,
}

impl WindowType {
    /// Every window, in the order Nostalgia+ declared them.
    pub const ALL: [WindowType; 6] = [
        WindowType::Hann,
        WindowType::Hamming,
        WindowType::BlackmanHarris,
        WindowType::Nuttall,
        WindowType::Gaussian,
        WindowType::Rectangular,
    ];

    /// The name used in settings files and reference vectors.
    pub fn name(self) -> &'static str {
        match self {
            WindowType::Hann => "Hann",
            WindowType::Hamming => "Hamming",
            WindowType::BlackmanHarris => "BlackmanHarris",
            WindowType::Nuttall => "Nuttall",
            WindowType::Gaussian => "Gaussian",
            WindowType::Rectangular => "Rectangular",
        }
    }

    /// Parses [`WindowType::name`], ignoring case.
    pub fn from_name(name: &str) -> Option<WindowType> {
        Self::ALL
            .into_iter()
            .find(|w| w.name().eq_ignore_ascii_case(name))
    }
}

impl fmt::Display for WindowType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Builds an `n`-point window and returns it with its coherent gain (its mean value),
/// which the magnitude spectra divide out so a full-scale sine reads 1.0 under any
/// window.
pub fn build(kind: WindowType, n: usize) -> (Vec<f64>, f64) {
    let mut w = Vec::with_capacity(n);
    let mut sum = 0.0;
    let span = (n as f64) - 1.0;
    for i in 0..n {
        let x = i as f64 / span;
        let v = match kind {
            WindowType::Rectangular => 1.0,
            WindowType::Hamming => 0.54 - 0.46 * (2.0 * PI * x).cos(),
            WindowType::BlackmanHarris => {
                0.35875 - 0.48829 * (2.0 * PI * x).cos() + 0.14128 * (4.0 * PI * x).cos()
                    - 0.01168 * (6.0 * PI * x).cos()
            }
            WindowType::Nuttall => {
                0.355768 - 0.487396 * (2.0 * PI * x).cos() + 0.144232 * (4.0 * PI * x).cos()
                    - 0.012604 * (6.0 * PI * x).cos()
            }
            WindowType::Gaussian => {
                let sigma = 0.4;
                let d = (i as f64 - span / 2.0) / (sigma * span / 2.0);
                (-0.5 * d * d).exp()
            }
            WindowType::Hann => 0.5 - 0.5 * (2.0 * PI * x).cos(),
        };
        w.push(v);
        sum += v;
    }
    (w, sum / n as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_round_trip() {
        for w in WindowType::ALL {
            assert_eq!(WindowType::from_name(w.name()), Some(w));
            assert_eq!(WindowType::from_name(&w.name().to_lowercase()), Some(w));
        }
        assert_eq!(WindowType::from_name("Kaiser"), None);
    }

    #[test]
    fn hann_is_symmetric_and_half_gain() {
        let (w, gain) = build(WindowType::Hann, 1024);
        assert!(w[0].abs() < 1e-15 && w[1023].abs() < 1e-15);
        for i in 0..512 {
            assert!((w[i] - w[1023 - i]).abs() < 1e-12);
        }
        assert!((gain - 0.5).abs() < 1e-3);
    }
}
