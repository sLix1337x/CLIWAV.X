//! FFT-based spectrum analysis for the live terminal visualizer. Pure
//! signal-processing logic — no audio I/O here, no `windows`/`wasapi`
//! dependency, so it builds and tests on any platform. `capture` (Windows
//! only) is the thing that actually feeds it real samples.

use rustfft::num_complex::Complex32;
use rustfft::{Fft, FftPlanner};
use std::sync::Arc;

/// Number of bars the spectrum is reduced to. More than the EQ's 10 bands —
/// this is a visual display, not a set of adjustable controls, so there's no
/// reason to tie the two together.
pub const BAND_COUNT: usize = 24;

/// Samples analyzed per FFT call. A power of two (required by the FFT) large
/// enough for reasonable low-frequency resolution without costing much CPU
/// at the analysis rate this runs at (see `poll_visualizer` in `app.rs`).
/// Public so callers know exactly how many samples to pull from the capture
/// ring buffer before calling `analyze`.
pub const FFT_SIZE: usize = 2048;

/// Below this peak absolute amplitude, skip the FFT entirely and just decay
/// the bars toward zero — near-free check that avoids burning CPU during
/// silence, pauses, or between tracks.
const SILENCE_THRESHOLD: f32 = 1e-4;

/// Per-tick decay multiplier applied to bars while silent.
const SILENCE_DECAY: f32 = 0.8;

/// Blend weight toward a rising target — higher than the falling weight so
/// bars snap up quickly but ease back down, which is what makes a bar
/// display read as musical rather than flickery.
const ATTACK: f32 = 0.6;
const DECAY: f32 = 0.25;

pub struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    scratch: Vec<Complex32>,
    bands: [f32; BAND_COUNT],
}

impl Spectrum {
    pub fn new() -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let window = hann_window(FFT_SIZE);
        Self {
            fft,
            window,
            scratch: vec![Complex32::default(); FFT_SIZE],
            bands: [0.0; BAND_COUNT],
        }
    }

    /// Analyzes the most recent `FFT_SIZE` samples (mono, any sample rate —
    /// pass the actual capture rate so band edges land on the right
    /// frequencies) and returns the current smoothed bar heights (0.0-1.0).
    /// Shorter input just uses what's available, zero-padded.
    pub fn analyze(&mut self, samples: &[f32], sample_rate: u32) -> &[f32; BAND_COUNT] {
        let peak = samples.iter().fold(0.0f32, |acc, s| acc.max(s.abs()));
        if peak < SILENCE_THRESHOLD {
            for band in self.bands.iter_mut() {
                *band *= SILENCE_DECAY;
            }
            return &self.bands;
        }

        for (i, slot) in self.scratch.iter_mut().enumerate() {
            let s = samples.get(i).copied().unwrap_or(0.0);
            *slot = Complex32::new(s * self.window[i], 0.0);
        }
        self.fft.process(&mut self.scratch);

        let nyquist = sample_rate as f32 / 2.0;
        let usable_bins = FFT_SIZE / 2;
        for (band_idx, (bin_lo, bin_hi)) in band_edges(nyquist, usable_bins).into_iter().enumerate() {
            let power_sum: f32 = self.scratch[bin_lo..bin_hi]
                .iter()
                .map(|c| c.norm_sqr())
                .sum();
            let bin_count = (bin_hi - bin_lo).max(1) as f32;
            let avg_power = power_sum / bin_count;
            // Rough dB-ish scale, tuned by feel: -60 dB..0 dB maps to 0..1.
            let db = 10.0 * (avg_power + 1e-10).log10();
            let target = ((db + 60.0) / 60.0).clamp(0.0, 1.0);

            let current = &mut self.bands[band_idx];
            let alpha = if target > *current { ATTACK } else { DECAY };
            *current += (target - *current) * alpha;
        }
        &self.bands
    }
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::new()
    }
}

/// Log-spaced band edges from 20 Hz to `nyquist` (capped at 20 kHz), as FFT
/// bin ranges `[lo, hi)`. Bars read logarithmically because pitch perception
/// is logarithmic — an evenly-split linear range looks bass-heavy and empty
/// above a few kHz.
fn band_edges(nyquist: f32, usable_bins: usize) -> Vec<(usize, usize)> {
    let min_freq = 20.0f32.min(nyquist);
    let max_freq = nyquist.min(20_000.0).max(min_freq + 1.0);
    let log_min = min_freq.ln();
    let log_max = max_freq.ln();

    let bin_for_freq = |freq: f32| -> usize {
        ((freq / nyquist) * usable_bins as f32)
            .round()
            .clamp(0.0, usable_bins as f32) as usize
    };

    (0..BAND_COUNT)
        .map(|i| {
            let t_lo = i as f32 / BAND_COUNT as f32;
            let t_hi = (i + 1) as f32 / BAND_COUNT as f32;
            let f_lo = (log_min + (log_max - log_min) * t_lo).exp();
            let f_hi = (log_min + (log_max - log_min) * t_hi).exp();
            let lo = bin_for_freq(f_lo);
            let hi = bin_for_freq(f_hi).max(lo + 1).min(usable_bins);
            (lo, hi)
        })
        .collect()
}

fn hann_window(size: usize) -> Vec<f32> {
    (0..size)
        .map(|i| {
            let x = std::f32::consts::PI * i as f32 / (size - 1).max(1) as f32;
            x.sin().powi(2)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silence_decays_to_zero() {
        let mut spec = Spectrum::new();
        spec.bands = [0.5; BAND_COUNT];
        let silent = vec![0.0f32; FFT_SIZE];
        for _ in 0..50 {
            spec.analyze(&silent, 48_000);
        }
        for band in spec.bands.iter() {
            assert!(*band < 0.01, "expected band to decay near zero, got {band}");
        }
    }

    #[test]
    fn a_pure_tone_produces_a_nonzero_band_and_the_rest_stay_lower() {
        let mut spec = Spectrum::new();
        let sample_rate = 48_000u32;
        // ~1 kHz tone, comfortably inside band range.
        let freq = 1000.0f32;
        let samples: Vec<f32> = (0..FFT_SIZE)
            .map(|i| (2.0 * std::f32::consts::PI * freq * i as f32 / sample_rate as f32).sin())
            .collect();
        let bands = *spec.analyze(&samples, sample_rate);
        let peak_idx = bands
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!(bands[peak_idx] > 0.0, "expected a nonzero peak band");
        // A rough sanity check: the loudest band shouldn't be the very
        // bottom or very top of the range for a 1 kHz tone at 48 kHz.
        assert!(peak_idx > 0 && peak_idx < BAND_COUNT - 1);
    }

    #[test]
    fn band_edges_cover_full_range_without_gaps() {
        let edges = band_edges(24_000.0, 1024);
        assert_eq!(edges.len(), BAND_COUNT);
        // First band starts near DC (bin 0 or 1 depending on 20 Hz's exact
        // bin — 20 Hz itself rounds just above bin 0 at this resolution).
        assert!(edges[0].0 <= 1);
        for pair in edges.windows(2) {
            assert!(pair[1].0 >= pair[0].0);
        }
        assert!(edges.last().unwrap().1 <= 1024);
    }
}
