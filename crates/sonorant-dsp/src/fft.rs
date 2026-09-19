//! Forward FFTs and the magnitude spectra built on them.

use std::fmt;
use std::sync::Arc;

use rustfft::FftPlanner;
pub use rustfft::num_complex::Complex;

/// A planned forward transform of one power-of-two size.
///
/// Planning allocates; running does not. One instance is built per transform size when
/// the analysis is configured and reused for every hop after that.
pub struct Fft {
    n: usize,
    plan: Arc<dyn rustfft::Fft<f64>>,
    buf: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
}

impl fmt::Debug for Fft {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Fft")
            .field("n", &self.n)
            .finish_non_exhaustive()
    }
}

impl Fft {
    /// Plans an `n`-point transform.
    ///
    /// # Panics
    /// If `n` is not a power of two of at least 2.
    pub fn new(n: usize) -> Fft {
        assert!(
            n >= 2 && n.is_power_of_two(),
            "FFT size must be a power of two"
        );
        let plan = FftPlanner::new().plan_fft_forward(n);
        let scratch = vec![Complex::default(); plan.get_inplace_scratch_len()];
        Fft {
            n,
            plan,
            buf: vec![Complex::default(); n],
            scratch,
        }
    }

    pub fn size(&self) -> usize {
        self.n
    }

    /// In-place forward transform, unnormalised, with the e^(-2πi kn/N) kernel.
    pub fn forward(&mut self, data: &mut [Complex<f64>]) {
        assert_eq!(data.len(), self.n);
        self.plan.process_with_scratch(data, &mut self.scratch);
    }

    /// Magnitude spectrum of a real windowed signal in linear amplitude, scaled so a
    /// full-scale sine reads 1.0 whatever the window or size. Writes bins 0..=n/2.
    pub fn magnitude_real(
        &mut self,
        input: &[f64],
        window: &[f64],
        window_gain: f64,
        out: &mut [f64],
    ) {
        let n = self.n;
        for ((b, &x), &w) in self.buf.iter_mut().zip(&input[..n]).zip(&window[..n]) {
            *b = Complex::new(x * w, 0.0);
        }
        self.plan
            .process_with_scratch(&mut self.buf, &mut self.scratch);

        // 2/N for the one-sided spectrum; the window gain undoes the window's attenuation.
        let scale = 2.0 / (n as f64 * window_gain);
        let half = n / 2;
        for (o, c) in out[..=half].iter_mut().zip(&self.buf) {
            *o = (c.re * c.re + c.im * c.im).sqrt() * scale;
        }
        out[0] *= 0.5;
        out[half] *= 0.5;
    }

    /// Magnitude spectra of two real signals from one complex transform.
    ///
    /// The left signal goes in the real part and the right in the imaginary part. A real
    /// signal's spectrum is Hermitian, so the two separate afterwards:
    /// `X[k] = (Z[k] + conj(Z[N-k])) / 2` and `Y[k] = (Z[k] - conj(Z[N-k])) / 2j`.
    /// Stereo therefore costs one transform, not two. Scaled as [`Fft::magnitude_real`].
    pub fn magnitude_real_pair(
        &mut self,
        left: &[f64],
        right: &[f64],
        window: &[f64],
        window_gain: f64,
        out_left: &mut [f64],
        out_right: &mut [f64],
    ) {
        let n = self.n;
        for (i, b) in self.buf.iter_mut().enumerate() {
            let w = window[i];
            *b = Complex::new(left[i] * w, right[i] * w);
        }
        self.plan
            .process_with_scratch(&mut self.buf, &mut self.scratch);

        // The 0.5 from the separation cancels one factor of the one-sided 2/N.
        let scale = 1.0 / (n as f64 * window_gain);
        let half = n / 2;
        for k in 0..=half {
            let m = (n - k) & (n - 1); // N-k, wrapping k = 0 to 0
            let (ar, ai) = (self.buf[k].re, self.buf[k].im);
            let (br, bi) = (self.buf[m].re, self.buf[m].im);
            let (lr, li) = (ar + br, ai - bi);
            let (rr, ri) = (ai + bi, ar - br);
            out_left[k] = (lr * lr + li * li).sqrt() * scale;
            out_right[k] = (rr * rr + ri * ri).sqrt() * scale;
        }
        out_left[0] *= 0.5;
        out_left[half] *= 0.5;
        out_right[0] *= 0.5;
        out_right[half] *= 0.5;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::window::{WindowType, build};
    use std::f64::consts::PI;

    #[test]
    fn full_scale_sine_reads_one() {
        let n = 4096;
        let mut fft = Fft::new(n);
        let bin = 100.0;
        let x: Vec<f64> = (0..n)
            .map(|i| (2.0 * PI * bin * i as f64 / n as f64).sin())
            .collect();
        for kind in WindowType::ALL {
            let (w, g) = build(kind, n);
            let mut mag = vec![0.0; n / 2 + 1];
            fft.magnitude_real(&x, &w, g, &mut mag);
            assert!((mag[100] - 1.0).abs() < 0.01, "{kind}: {}", mag[100]);
        }
    }

    #[test]
    fn pair_matches_two_real_transforms() {
        let n = 1024;
        let mut fft = Fft::new(n);
        let a: Vec<f64> = (0..n).map(|i| (i as f64 * 0.37).sin() * 0.5).collect();
        let b: Vec<f64> = (0..n)
            .map(|i| (i as f64 * 1.91).cos() * 0.25 + 0.01)
            .collect();
        let (w, g) = build(WindowType::Hann, n);
        let (mut ma, mut mb) = (vec![0.0; n / 2 + 1], vec![0.0; n / 2 + 1]);
        let (mut pa, mut pb) = (vec![0.0; n / 2 + 1], vec![0.0; n / 2 + 1]);
        fft.magnitude_real(&a, &w, g, &mut ma);
        fft.magnitude_real(&b, &w, g, &mut mb);
        fft.magnitude_real_pair(&a, &b, &w, g, &mut pa, &mut pb);
        for k in 0..=n / 2 {
            assert!((ma[k] - pa[k]).abs() < 1e-12);
            assert!((mb[k] - pb[k]).abs() < 1e-12);
        }
    }

    #[test]
    #[should_panic(expected = "power of two")]
    fn rejects_other_sizes() {
        let _ = Fft::new(1000);
    }
}
