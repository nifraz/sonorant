//! What the music is doing, as numbers the picture can react to: when a hit lands, how
//! fast hits are landing, and whether the sound is bright or dark.
//!
//! All of it comes from the spectrum the analyser already made, so it costs one pass
//! over the columns per hop and no extra transforms. It drives how the picture looks
//! and is tuned for that, not for accuracy.
//!
//! Nostalgia+ counted its history windows in frames at an assumed 60 fps. Here they are
//! seconds, turned into hops at the analysis rate, so the same music gives the same
//! answers at any hop rate; at 60 hops per second the numbers match Nostalgia+ exactly.

/// Flux history for the adaptive onset threshold, in seconds (90 frames at 60 fps).
const THRESHOLD_SECONDS: f64 = 1.5;
/// Flux history for the tempo autocorrelation, in seconds (512 frames at 60 fps).
const TEMPO_SECONDS: f64 = 512.0 / 60.0;
/// Rise needed before an onset is possible, in dB per column.
const MIN_FLUX: f64 = 0.05;
/// Standard deviations above the running mean an onset has to clear.
const THRESHOLD_SIGMA: f64 = 1.6;
/// Hits closer than this are one hit: sixteenths at 240 BPM.
const MIN_ONSET_GAP: f64 = 0.06;
/// Tempo search range.
const MIN_BPM: f64 = 60.0;
const MAX_BPM: f64 = 200.0;
/// How well half the period has to correlate, relative to the best lag, to be taken
/// instead of it.
const OCTAVE_RATIO: f64 = 0.8;

use crate::analyzer::FLOOR_DB;

/// Onsets, tempo and spectral centroid from a stream of spectra.
#[derive(Clone, Debug)]
pub struct MusicFeatures {
    hop_rate: f64,
    prev: Vec<f64>,
    recent: Vec<f64>,
    recent_count: usize,
    tempo: Vec<f64>,
    tempo_head: usize,
    tempo_count: usize,
    scratch: Vec<f64>,
    since_onset: f64,
    since_tempo: f64,
    centroid: f64,

    flux: f64,
    onset: bool,
    pulse: f64,
    bpm: f64,
    corr: Vec<f64>,
    /// How long a pulse takes to fall away. Short enough to feel like a hit.
    pub pulse_decay_seconds: f64,
    /// Prefer half the period when it correlates nearly as well as the best one.
    ///
    /// A steady beat correlates as strongly at two beats as at one, and dividing by the
    /// shrinking overlap tips the balance to the longer lag, so Nostalgia+ often read a
    /// 120 BPM track as 60. On by default; the parity tests turn it off to reproduce
    /// Nostalgia+ exactly.
    pub octave_check: bool,
}

impl MusicFeatures {
    /// A detector fed `hop_rate` spectra per second.
    pub fn new(hop_rate: f64) -> MusicFeatures {
        let hop_rate = if hop_rate > 0.0 { hop_rate } else { 60.0 };
        let threshold = ((THRESHOLD_SECONDS * hop_rate).round() as usize).max(3);
        let tempo = ((TEMPO_SECONDS * hop_rate).round() as usize).max(4);
        MusicFeatures {
            hop_rate,
            prev: Vec::new(),
            recent: vec![0.0; threshold],
            recent_count: 0,
            tempo: vec![0.0; tempo],
            tempo_head: 0,
            tempo_count: 0,
            scratch: vec![0.0; tempo],
            since_onset: 1.0,
            since_tempo: 0.0,
            centroid: 0.5,
            flux: 0.0,
            onset: false,
            pulse: 0.0,
            bpm: 0.0,
            corr: vec![0.0; tempo + 1],
            pulse_decay_seconds: 0.20,
            octave_check: true,
        }
    }

    pub fn hop_rate(&self) -> f64 {
        self.hop_rate
    }

    /// Rising spectral energy on the last hop, in dB per column.
    pub fn flux(&self) -> f64 {
        self.flux
    }

    /// True on the hop an onset was accepted.
    pub fn onset(&self) -> bool {
        self.onset
    }

    /// 1 at an onset, decaying towards 0: what the picture animates on.
    pub fn pulse(&self) -> f64 {
        self.pulse
    }

    /// The tempo, or 0 when nothing convincing was found.
    pub fn bpm(&self) -> f64 {
        self.bpm
    }

    /// Where the energy sits along the axis, 0 at the bottom and 1 at the top. Taken over
    /// columns rather than hertz, so on a note axis it is a musical centre of gravity
    /// rather than one dominated by the top octave.
    pub fn centroid(&self) -> f64 {
        self.centroid
    }

    /// Starts again, as for a new track.
    pub fn reset(&mut self) {
        self.prev.clear();
        self.recent_count = 0;
        self.tempo_head = 0;
        self.tempo_count = 0;
        self.since_onset = 1.0;
        self.since_tempo = 0.0;
        self.centroid = 0.5;
        self.flux = 0.0;
        self.onset = false;
        self.pulse = 0.0;
        self.bpm = 0.0;
    }

    /// Takes one spectrum, `dt` seconds after the previous one.
    pub fn update(&mut self, db: &[f64], dt: f64) {
        self.onset = false;
        let count = db.len();
        if count < 8 {
            return;
        }
        let dt = if dt <= 0.0 { 1.0 / 60.0 } else { dt.min(0.5) };

        if self.prev.len() != count {
            self.prev.clear();
            self.prev.extend_from_slice(db);
            return; // no flux from the first spectrum
        }

        let (mut rise, mut weight, mut energy) = (0.0, 0.0, 0.0);
        for (i, (&v, p)) in db.iter().zip(self.prev.iter_mut()).enumerate() {
            let d = v - *p;
            if d > 0.0 {
                rise += d;
            }
            *p = v;
            // Linear amplitude for the centroid: in dB the noise floor, which fills most
            // of the axis, would drag the answer to the middle.
            let amp = if v <= FLOOR_DB {
                0.0
            } else {
                10.0f64.powf(v / 20.0)
            };
            energy += amp;
            weight += amp * i as f64;
        }
        self.flux = rise / count as f64;

        if energy > 1e-12 {
            let c = weight / energy / (count - 1).max(1) as f64;
            // Smoothed hard: it drives colour, and a hue that jitters every hop reads as
            // a fault rather than a response.
            let k = 1.0 - (-dt / 0.45).exp();
            self.centroid += (c - self.centroid) * k;
        }

        self.push_tempo(self.flux);
        self.detect_onset(dt);

        self.pulse *= (-dt / self.pulse_decay_seconds.max(0.02)).exp();
        if self.onset {
            self.pulse = 1.0;
        }

        self.since_tempo += dt;
        if self.since_tempo >= 0.5 {
            self.since_tempo = 0.0;
            self.bpm = self.estimate_bpm();
        }
    }

    fn detect_onset(&mut self, dt: f64) {
        self.since_onset += dt;

        // An adaptive threshold, so quiet passages still register their own hits and a
        // loud one doesn't fire on every hop.
        let recent = &self.recent[..self.recent_count];
        let mut mean = recent.iter().sum::<f64>();
        if !recent.is_empty() {
            mean /= recent.len() as f64;
        }
        let var: f64 = recent.iter().map(|&x| (x - mean) * (x - mean)).sum();
        let sd = if recent.len() > 1 {
            (var / (recent.len() - 1) as f64).sqrt()
        } else {
            0.0
        };
        let threshold = MIN_FLUX.max(mean + THRESHOLD_SIGMA * sd);

        if self.recent_count >= self.recent.len() / 3
            && self.flux > threshold
            && self.since_onset >= MIN_ONSET_GAP
        {
            self.onset = true;
            self.since_onset = 0.0;
        }

        // The history includes the onsets: leaving them out would let a steady beat
        // lower its own bar until every hop qualified.
        if self.recent_count < self.recent.len() {
            self.recent[self.recent_count] = self.flux;
            self.recent_count += 1;
        } else {
            self.recent.copy_within(1.., 0);
            let last = self.recent.len() - 1;
            self.recent[last] = self.flux;
        }
    }

    fn push_tempo(&mut self, flux: f64) {
        let len = self.tempo.len();
        self.tempo[self.tempo_head] = flux;
        self.tempo_head = (self.tempo_head + 1) % len;
        if self.tempo_count < len {
            self.tempo_count += 1;
        }
    }

    /// Autocorrelation of the flux, which finds a period whether or not individual
    /// onsets were accepted: more forgiving than timing gaps between hits, where one
    /// missed beat doubles the answer. Run twice a second, not every hop.
    fn estimate_bpm(&mut self) -> f64 {
        let len = self.tempo.len();
        if self.tempo_count < len / 2 {
            return 0.0;
        }
        let n = self.tempo_count;
        let start = (self.tempo_head + len - n) % len;
        let x = &mut self.scratch[..n];
        let mut mean = 0.0;
        for (i, v) in x.iter_mut().enumerate() {
            *v = self.tempo[(start + i) % len];
            mean += *v;
        }
        mean /= n as f64;
        for v in x.iter_mut() {
            *v -= mean;
        }

        let min_lag = (self.hop_rate * 60.0 / MAX_BPM) as usize;
        let max_lag = ((self.hop_rate * 60.0 / MIN_BPM) as usize).min(n - 1);
        if min_lag < 2 || min_lag >= max_lag {
            return 0.0;
        }

        let (mut best, mut best_lag, mut total) = (0.0, 0usize, 0.0);
        for lag in min_lag..=max_lag {
            let mut sum = 0.0;
            for i in 0..n - lag {
                sum += x[i] * x[i + lag];
            }
            sum /= (n - lag) as f64;
            self.corr[lag] = sum;
            total += f64::abs(sum);
            if sum > best {
                best = sum;
                best_lag = lag;
            }
        }
        if best_lag == 0 || best <= 0.0 {
            return 0.0;
        }
        // A peak not clearly above the average correlation is noise with a maximum.
        let average = total / (max_lag - min_lag + 1) as f64;
        if best < average * 2.0 {
            return 0.0;
        }
        if self.octave_check {
            // Half the lag, give or take a hop for the rounding.
            let half = best_lag.div_ceil(2);
            let candidates = half.saturating_sub(1).max(min_lag)..=(half + 1).min(max_lag);
            if let Some(lag) = candidates.max_by(|&a, &b| self.corr[a].total_cmp(&self.corr[b]))
                && lag < best_lag
                && self.corr[lag] >= OCTAVE_RATIO * best
            {
                best_lag = lag;
            }
        }
        60.0 * self.hop_rate / best_lag as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BINS: usize = 256;

    fn click(hop_rate: f64) -> (MusicFeatures, usize, usize) {
        let mut f = MusicFeatures::new(hop_rate);
        let every = (hop_rate * 0.5) as usize;
        let (mut onsets, mut on_beat) = (0, 0);
        let dt = 1.0 / hop_rate;
        for frame in 0..(15.0 * hop_rate) as usize {
            let hit = frame % every == 0;
            let db = vec![if hit { -30.0 } else { -75.0 }; BINS];
            f.update(&db, dt);
            if f.onset() {
                onsets += 1;
                if hit {
                    on_beat += 1;
                }
            }
        }
        (f, onsets, on_beat)
    }

    #[test]
    fn finds_120_bpm_at_60_and_120_hops() {
        for rate in [60.0, 120.0] {
            let (f, onsets, on_beat) = click(rate);
            assert!((25..=32).contains(&onsets), "{rate}: {onsets} onsets");
            assert_eq!(on_beat, onsets);
            assert!((f.bpm() - 120.0).abs() < 4.0, "{rate}: {} BPM", f.bpm());
        }
    }

    #[test]
    fn steady_level_claims_nothing() {
        let mut g = MusicFeatures::new(60.0);
        let db = vec![-40.0; BINS];
        let mut spurious = 0;
        for frame in 0..300 {
            g.update(&db, 1.0 / 60.0);
            if g.onset() && frame > 10 {
                spurious += 1;
            }
        }
        assert_eq!(spurious, 0);
        assert_eq!(g.bpm(), 0.0);
    }
}
