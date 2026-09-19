//! Forward FFTs and the power and magnitude spectra built on them.

use std::fmt;
use std::sync::Arc;

use rustfft::FftPlanner;
pub use rustfft::num_complex::Complex;

/// A planned forward transform of one power-of-two size.
///
/// Planning allocates; running does not. One instance is built per transform size when
/// the analysis is configured and reused for every hop after that.
///
/// The spectrum functions transform whatever is in [`Fft::input_mut`], so a caller can
/// window its samples straight into it instead of staging them in another buffer: at
/// 16K points each staging copy is a quarter of a megabyte of memory traffic.
pub struct Fft {
    n: usize,
    plan: Arc<dyn rustfft::Fft<f64>>,
    input: Vec<Complex<f64>>,
    output: Vec<Complex<f64>>,
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
        let scratch_len = plan
            .get_inplace_scratch_len()
            .max(plan.get_outofplace_scratch_len());
        Fft {
            n,
            plan,
            input: vec![Complex::default(); n],
            output: vec![Complex::default(); n],
            scratch: vec![Complex::default(); scratch_len],
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

    /// The next transform's input, `n` points. Fill it with windowed samples: one real
    /// signal in the real parts for [`Fft::transform_power`], or two with the left in
    /// the real parts and the right in the imaginary parts for
    /// [`Fft::transform_power_pair`]. The transforms leave it scrambled.
    pub fn input_mut(&mut self) -> &mut [Complex<f64>] {
        &mut self.input
    }

    /// Transforms [`Fft::input_mut`], a real windowed signal, into its power spectrum,
    /// unscaled, and returns the scale: bin `k`'s magnitude in linear amplitude is
    /// `out[k].sqrt() * scale`, which reads 1.0 for a full-scale sine whatever the
    /// window or size. Writes bins 0..=n/2.
    ///
    /// The square root is left to whoever combines bins, because a peak or an energy
    /// over many bins needs only one: a root per bin was most of what a hop cost after
    /// the transform itself.
    pub fn transform_power(&mut self, window_gain: f64, out: &mut [f64]) -> f64 {
        self.plan.process_outofplace_with_scratch(
            &mut self.input,
            &mut self.output,
            &mut self.scratch,
        );
        let n = self.n;
        let half = n / 2;
        for (o, c) in out[..=half].iter_mut().zip(&self.output) {
            *o = c.re * c.re + c.im * c.im;
        }
        // DC and Nyquist have no mirror image: half the magnitude, a quarter the power.
        out[0] *= 0.25;
        out[half] *= 0.25;
        // 2/N for the one-sided spectrum; the window gain undoes the window's attenuation.
        2.0 / (n as f64 * window_gain)
    }

    /// Transforms [`Fft::input_mut`], two real windowed signals, into their power
    /// spectra and returns their scale, as [`Fft::transform_power`].
    ///
    /// A real signal's spectrum is Hermitian, so the two separate afterwards:
    /// `X[k] = (Z[k] + conj(Z[N-k])) / 2` and `Y[k] = (Z[k] - conj(Z[N-k])) / 2j`.
    /// Stereo therefore costs one transform, not two.
    pub fn transform_power_pair(
        &mut self,
        window_gain: f64,
        out_left: &mut [f64],
        out_right: &mut [f64],
    ) -> f64 {
        self.plan.process_outofplace_with_scratch(
            &mut self.input,
            &mut self.output,
            &mut self.scratch,
        );
        let n = self.n;
        let half = n / 2;
        let z = &self.output;
        for k in 0..=half {
            let m = (n - k) & (n - 1); // N-k, wrapping k = 0 to 0
            let (ar, ai) = (z[k].re, z[k].im);
            let (br, bi) = (z[m].re, z[m].im);
            let (lr, li) = (ar + br, ai - bi);
            let (rr, ri) = (ai + bi, ar - br);
            out_left[k] = lr * lr + li * li;
            out_right[k] = rr * rr + ri * ri;
        }
        out_left[0] *= 0.25;
        out_left[half] *= 0.25;
        out_right[0] *= 0.25;
        out_right[half] *= 0.25;
        // The 0.5 from the separation cancels one factor of the one-sided 2/N.
        1.0 / (n as f64 * window_gain)
    }

    /// Windows `input` and returns its power spectrum and scale; see
    /// [`Fft::transform_power`].
    pub fn power_real(
        &mut self,
        input: &[f64],
        window: &[f64],
        window_gain: f64,
        out: &mut [f64],
    ) -> f64 {
        let n = self.n;
        for ((b, &x), &w) in self.input.iter_mut().zip(&input[..n]).zip(&window[..n]) {
            *b = Complex::new(x * w, 0.0);
        }
        self.transform_power(window_gain, out)
    }

    /// Windows `left` and `right` and returns their power spectra and scale; see
    /// [`Fft::transform_power_pair`].
    pub fn power_real_pair(
        &mut self,
        left: &[f64],
        right: &[f64],
        window: &[f64],
        window_gain: f64,
        out_left: &mut [f64],
        out_right: &mut [f64],
    ) -> f64 {
        let n = self.n;
        for (((b, &l), &r), &w) in self
            .input
            .iter_mut()
            .zip(&left[..n])
            .zip(&right[..n])
            .zip(&window[..n])
        {
            *b = Complex::new(l * w, r * w);
        }
        self.transform_power_pair(window_gain, out_left, out_right)
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
        let scale = self.power_real(input, window, window_gain, out);
        for o in &mut out[..=self.n / 2] {
            *o = o.sqrt() * scale;
        }
    }

    /// Magnitude spectra of two real signals from one complex transform, scaled as
    /// [`Fft::magnitude_real`]. See [`Fft::transform_power_pair`].
    pub fn magnitude_real_pair(
        &mut self,
        left: &[f64],
        right: &[f64],
        window: &[f64],
        window_gain: f64,
        out_left: &mut [f64],
        out_right: &mut [f64],
    ) {
        let scale = self.power_real_pair(left, right, window, window_gain, out_left, out_right);
        let half = self.n / 2;
        for o in out_left[..=half].iter_mut().chain(&mut out_right[..=half]) {
            *o = o.sqrt() * scale;
        }
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
