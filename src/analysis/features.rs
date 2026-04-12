use realfft::RealFftPlanner;

use serde::{Deserialize, Serialize};

use crate::engine::kit::SampleCategory;

/// Audio features extracted from a sample for classification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AudioFeatures {
    /// Time from start to peak amplitude, in seconds.
    pub attack_time: f32,
    /// Time from peak to -20dB below peak, in seconds.
    pub decay_time: f32,
    /// Spectral centroid in Hz (brightness — low=kick, high=hat).
    pub spectral_centroid: f32,
    /// Spectral flatness 0..1 (1=noise/white, 0=tonal/pure tone).
    pub spectral_flatness: f32,
    /// Fraction of energy below ~120 Hz (high → kick, bass).
    pub sub_energy_ratio: f32,
    /// Fraction of energy above ~4000 Hz (high → hihat, clap, cymbal).
    pub high_freq_ratio: f32,
    /// Peak amplitude (0..1).
    pub peak: f32,
    /// Duration in seconds.
    pub duration: f32,
    /// Whether the sample has a sharp transient and decays.
    pub is_percussive: bool,
}

/// Internal: all spectral results computed in one FFT pass.
struct SpectralFeatures {
    centroid: f32,
    flatness: f32,
    sub_energy_ratio: f32,
    high_freq_ratio: f32,
}

/// Extract audio features from mono f32 sample data.
pub fn extract(samples: &[f32], sample_rate: f32) -> AudioFeatures {
    let duration = samples.len() as f32 / sample_rate;
    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    if samples.is_empty() || peak < 1e-6 {
        return AudioFeatures {
            attack_time: 0.0,
            decay_time: 0.0,
            spectral_centroid: 0.0,
            spectral_flatness: 0.0,
            sub_energy_ratio: 0.0,
            high_freq_ratio: 0.0,
            peak,
            duration,
            is_percussive: false,
        };
    }

    let attack_time = compute_attack_time(samples, sample_rate);
    let decay_time = compute_decay_time(samples, sample_rate, peak);
    let spec = compute_spectral_features(samples, sample_rate);
    let is_percussive = attack_time < 0.015 && decay_time < 2.0;

    AudioFeatures {
        attack_time,
        decay_time,
        spectral_centroid: spec.centroid,
        spectral_flatness: spec.flatness,
        sub_energy_ratio: spec.sub_energy_ratio,
        high_freq_ratio: spec.high_freq_ratio,
        peak,
        duration,
        is_percussive,
    }
}

/// Time from start to peak amplitude.
fn compute_attack_time(samples: &[f32], sample_rate: f32) -> f32 {
    let peak_idx = samples
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .map(|(i, _)| i)
        .unwrap_or(0);

    peak_idx as f32 / sample_rate
}

/// Time from peak to -20dB below peak.
fn compute_decay_time(samples: &[f32], sample_rate: f32, peak: f32) -> f32 {
    let threshold = peak * 0.1; // -20dB ≈ 0.1x amplitude
    let window_size = (sample_rate * 0.005) as usize; // 5ms RMS windows

    let peak_idx = samples
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().total_cmp(&b.abs()))
        .map(|(i, _)| i)
        .unwrap_or(0);

    // Walk forward from peak in RMS windows
    let mut pos = peak_idx;
    while pos + window_size < samples.len() {
        let window = &samples[pos..pos + window_size];
        let rms = (window.iter().map(|s| s * s).sum::<f32>() / window_size as f32).sqrt();
        if rms < threshold {
            return (pos - peak_idx) as f32 / sample_rate;
        }
        pos += window_size;
    }

    // Never dropped below threshold — return remaining duration
    (samples.len() - peak_idx) as f32 / sample_rate
}

/// Compute spectral centroid, flatness, sub energy ratio and high-freq ratio.
/// All four are derived in a single FFT pass to avoid redundant work.
fn compute_spectral_features(samples: &[f32], sample_rate: f32) -> SpectralFeatures {
    let fft_size = 2048;

    if samples.len() < fft_size {
        return compute_spectral_short(samples, sample_rate);
    }

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let mut scratch = fft.make_scratch_vec();

    let hop = fft_size / 2;
    let num_windows = ((samples.len() - fft_size) / hop).max(1);

    let mut total_centroid = 0.0f32;
    let mut total_flatness = 0.0f32;
    let mut total_sub = 0.0f32;
    let mut total_hf = 0.0f32;
    let mut valid_windows = 0u32;
    let mut buffer = vec![0.0f32; fft_size];

    let freq_resolution = sample_rate / fft_size as f32;
    // Bin boundaries for sub (<120 Hz) and high-freq (>4000 Hz)
    let sub_cutoff_bin = (120.0 / freq_resolution).round() as usize;
    let hf_cutoff_bin = (4000.0 / freq_resolution).round() as usize;
    let num_bins = fft_size / 2 + 1;
    let sub_cutoff_bin = sub_cutoff_bin.min(num_bins);
    let hf_cutoff_bin = hf_cutoff_bin.min(num_bins);

    for w in 0..num_windows {
        let start = w * hop;
        if start + fft_size > samples.len() {
            break;
        }

        buffer.copy_from_slice(&samples[start..start + fft_size]);

        // Apply Hann window
        for (i, s) in buffer.iter_mut().enumerate() {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / fft_size as f32).cos());
            *s *= hann;
        }

        let mut spectrum = fft.make_output_vec();
        if fft.process_with_scratch(&mut buffer, &mut spectrum, &mut scratch).is_err() {
            continue;
        }

        // magnitude² spectrum (energy per bin)
        let energies: Vec<f32> = spectrum.iter().map(|c| c.re * c.re + c.im * c.im).collect();
        let magnitudes: Vec<f32> = energies.iter().map(|e| e.sqrt()).collect();

        let total_energy: f32 = energies.iter().sum();
        let mag_sum: f32 = magnitudes.iter().sum();

        if mag_sum < 1e-10 || total_energy < 1e-20 {
            continue;
        }

        // Spectral centroid
        let centroid: f32 = magnitudes
            .iter()
            .enumerate()
            .map(|(i, &m)| i as f32 * freq_resolution * m)
            .sum::<f32>()
            / mag_sum;

        // Spectral flatness: geometric mean / arithmetic mean
        let n = magnitudes.len() as f32;
        let arith_mean = mag_sum / n;
        let log_sum: f32 = magnitudes.iter().map(|&m| (m + 1e-10).ln()).sum::<f32>();
        let geom_mean = (log_sum / n).exp();
        let flatness = if arith_mean > 1e-10 {
            (geom_mean / arith_mean).clamp(0.0, 1.0)
        } else {
            0.0
        };

        // Sub energy ratio: energy in bins 0..sub_cutoff / total
        let sub_energy: f32 = energies[..sub_cutoff_bin].iter().sum();
        let sub_ratio = (sub_energy / total_energy).clamp(0.0, 1.0);

        // High-freq ratio: energy in bins hf_cutoff..end / total
        let hf_energy: f32 = energies[hf_cutoff_bin..].iter().sum();
        let hf_ratio = (hf_energy / total_energy).clamp(0.0, 1.0);

        total_centroid += centroid;
        total_flatness += flatness;
        total_sub += sub_ratio;
        total_hf += hf_ratio;
        valid_windows += 1;
    }

    if valid_windows == 0 {
        return SpectralFeatures { centroid: 0.0, flatness: 0.0, sub_energy_ratio: 0.0, high_freq_ratio: 0.0 };
    }

    let n = valid_windows as f32;
    SpectralFeatures {
        centroid: total_centroid / n,
        flatness: total_flatness / n,
        sub_energy_ratio: total_sub / n,
        high_freq_ratio: total_hf / n,
    }
}

/// Fallback spectral analysis for very short samples (< 2048 samples).
fn compute_spectral_short(samples: &[f32], sample_rate: f32) -> SpectralFeatures {
    let fft_size = 1024;
    let mut buffer = vec![0.0f32; fft_size];
    let copy_len = samples.len().min(fft_size);
    buffer[..copy_len].copy_from_slice(&samples[..copy_len]);

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let mut scratch = fft.make_scratch_vec();
    let mut spectrum = fft.make_output_vec();

    if fft.process_with_scratch(&mut buffer, &mut spectrum, &mut scratch).is_err() {
        return SpectralFeatures { centroid: 0.0, flatness: 0.0, sub_energy_ratio: 0.0, high_freq_ratio: 0.0 };
    }

    let freq_resolution = sample_rate / fft_size as f32;
    let sub_cutoff_bin = ((120.0 / freq_resolution) as usize).min(fft_size / 2 + 1);
    let hf_cutoff_bin = ((4000.0 / freq_resolution) as usize).min(fft_size / 2 + 1);

    let energies: Vec<f32> = spectrum.iter().map(|c| c.re * c.re + c.im * c.im).collect();
    let magnitudes: Vec<f32> = energies.iter().map(|e| e.sqrt()).collect();
    let total_energy: f32 = energies.iter().sum();
    let mag_sum: f32 = magnitudes.iter().sum();

    if mag_sum < 1e-10 || total_energy < 1e-20 {
        return SpectralFeatures { centroid: 0.0, flatness: 0.0, sub_energy_ratio: 0.0, high_freq_ratio: 0.0 };
    }

    let centroid: f32 = magnitudes
        .iter()
        .enumerate()
        .map(|(i, &m)| i as f32 * freq_resolution * m)
        .sum::<f32>()
        / mag_sum;

    let n = magnitudes.len() as f32;
    let arith_mean = mag_sum / n;
    let log_sum: f32 = magnitudes.iter().map(|&m| (m + 1e-10).ln()).sum::<f32>();
    let geom_mean = (log_sum / n).exp();
    let flatness = if arith_mean > 1e-10 {
        (geom_mean / arith_mean).clamp(0.0, 1.0)
    } else {
        0.0
    };

    let sub_energy: f32 = energies[..sub_cutoff_bin].iter().sum();
    let sub_ratio = (sub_energy / total_energy).clamp(0.0, 1.0);
    let hf_energy: f32 = energies[hf_cutoff_bin..].iter().sum();
    let hf_ratio = (hf_energy / total_energy).clamp(0.0, 1.0);

    SpectralFeatures {
        centroid,
        flatness,
        sub_energy_ratio: sub_ratio,
        high_freq_ratio: hf_ratio,
    }
}

/// Classify a sample into a category using DSP features + optional folder/filename hint.
pub fn classify(features: &AudioFeatures, hint: Option<SampleCategory>) -> SampleCategory {
    // Trust a strong hint unless DSP strongly contradicts it.
    if let Some(h) = hint {
        if !contradicts_hint(features, h) {
            return h;
        }
    }

    if features.is_percussive {
        classify_percussive(features)
    } else {
        classify_non_percussive(features)
    }
}

/// Fingerprint-based percussive classifier.
///
/// Each instrument family has a distinct combination of:
///   centroid, flatness, sub_energy_ratio, high_freq_ratio, attack_time, decay_time.
/// Rules are ordered from most-specific (fewest ambiguous neighbours) outward.
fn classify_percussive(f: &AudioFeatures) -> SampleCategory {
    let c = f.spectral_centroid;
    let fl = f.spectral_flatness;
    let sub = f.sub_energy_ratio;
    let hf = f.high_freq_ratio;
    let decay = f.decay_time;
    let attack = f.attack_time;

    // ── KICK ──────────────────────────────────────────────────────────────────
    // Dominant sub energy, low centroid.  Very dark kicks (808s) and regular kicks
    // both anchor here; the sub check prevents bright electronic toms from matching.
    if c < 250.0 || (c < 550.0 && sub > 0.18) {
        return SampleCategory::Kick;
    }

    // ── HIHAT ─────────────────────────────────────────────────────────────────
    // Metal hi-hats: energy concentrated above 4 kHz, short-to-medium decay.
    // Must test before Cymbal since both are high-freq but hihats are shorter.
    if hf > 0.55 && c > 4000.0 && decay < 0.35 {
        return SampleCategory::Hihat;
    }

    // ── CYMBAL ────────────────────────────────────────────────────────────────
    // Crash / ride: high-freq, but longer sustain than a closed hat.
    if hf > 0.40 && c > 3500.0 && decay >= 0.25 {
        return SampleCategory::Cymbal;
    }

    // ── CLAP ──────────────────────────────────────────────────────────────────
    // Key discriminant from snare: almost no sub (hand clap ≈ pure mid/HF noise),
    // high flatness (broadband noise burst), very short body, centroid above snare body.
    if fl > 0.45 && sub < 0.05 && decay < 0.15 && c > 1500.0 {
        return SampleCategory::Clap;
    }

    // ── SNARE ─────────────────────────────────────────────────────────────────
    // Snare wire gives high flatness; drum head gives some sub body (distinguishes
    // from clap).  Wide centroid range to capture both fat and bright snares.
    if fl > 0.28 && c > 350.0 && c < 5000.0 && (sub > 0.04 || decay > 0.08) {
        return SampleCategory::Snare;
    }

    // ── TOM ───────────────────────────────────────────────────────────────────
    // Tonal, mid-range, medium-to-long decay.  Low flatness (drum head resonance).
    // centroid upper bound prevents bright electronic tom hits from landing here.
    if fl < 0.28 && c > 150.0 && c < 1600.0 && decay > 0.06 {
        return SampleCategory::Tom;
    }

    SampleCategory::Perc
}

/// Non-percussive classifier (slow attack or sustained signal).
fn classify_non_percussive(f: &AudioFeatures) -> SampleCategory {
    // Bass: dominated by sub content, tonal, low centroid.
    if f.sub_energy_ratio > 0.18 && f.spectral_centroid < 650.0 && f.spectral_flatness < 0.38 {
        return SampleCategory::Bass;
    }

    // Synth / tonal pad / lead / stab.
    if f.spectral_flatness < 0.42 {
        return SampleCategory::Synth;
    }

    SampleCategory::Other
}

/// Returns true when DSP features strongly contradict the given hint.
/// Used to decide whether to override a folder/filename hint with DSP results.
fn contradicts_hint(f: &AudioFeatures, hint: SampleCategory) -> bool {
    match hint {
        SampleCategory::Kick => {
            // A "kick" folder with a very bright, non-sub sound is likely mis-labelled.
            f.spectral_centroid > 3000.0 && f.sub_energy_ratio < 0.05
        }
        SampleCategory::Snare => {
            // "Snare" folder but clearly sub-dominant → probably a kick.
            f.spectral_centroid < 200.0 && f.sub_energy_ratio > 0.35
        }
        SampleCategory::Hihat => {
            // "Hihat" folder but energy is in the bass region → clearly wrong.
            f.spectral_centroid < 1500.0
        }
        SampleCategory::Bass => {
            // "Bass" folder but percussive and bright.
            f.is_percussive && f.spectral_centroid > 2500.0
        }
        SampleCategory::Cymbal => {
            // "Cymbal" folder but very dark / sub-heavy → probably mislabelled.
            f.spectral_centroid < 800.0 && f.high_freq_ratio < 0.1
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_features(
        centroid: f32,
        flatness: f32,
        sub: f32,
        hf: f32,
        decay: f32,
        attack: f32,
        is_perc: bool,
    ) -> AudioFeatures {
        AudioFeatures {
            attack_time: attack,
            decay_time: decay,
            spectral_centroid: centroid,
            spectral_flatness: flatness,
            sub_energy_ratio: sub,
            high_freq_ratio: hf,
            peak: 0.8,
            duration: 0.3,
            is_percussive: is_perc,
        }
    }

    #[test]
    fn classify_kick_by_sub_and_low_centroid() {
        let f = make_features(180.0, 0.15, 0.35, 0.02, 0.4, 0.003, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Kick);
    }

    #[test]
    fn classify_kick_by_centroid_alone() {
        let f = make_features(220.0, 0.2, 0.10, 0.02, 0.3, 0.003, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Kick);
    }

    #[test]
    fn classify_hihat_high_freq_short() {
        let f = make_features(6500.0, 0.55, 0.01, 0.70, 0.05, 0.001, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Hihat);
    }

    #[test]
    fn classify_cymbal_high_freq_long() {
        let f = make_features(5000.0, 0.50, 0.01, 0.55, 0.80, 0.003, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Cymbal);
    }

    #[test]
    fn classify_clap_no_sub_noisy_short() {
        let f = make_features(2800.0, 0.60, 0.02, 0.35, 0.08, 0.001, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Clap);
    }

    #[test]
    fn classify_snare_with_sub_body() {
        // Snare: mid centroid, noisy, has some sub
        let f = make_features(1800.0, 0.45, 0.08, 0.20, 0.18, 0.003, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Snare);
    }

    #[test]
    fn classify_tom_tonal_mid() {
        let f = make_features(700.0, 0.18, 0.10, 0.05, 0.25, 0.005, true);
        assert_eq!(classify_percussive(&f), SampleCategory::Tom);
    }

    #[test]
    fn classify_bass_non_percussive() {
        let f = make_features(250.0, 0.20, 0.30, 0.01, 1.5, 0.05, false);
        assert_eq!(classify_non_percussive(&f), SampleCategory::Bass);
    }

    #[test]
    fn classify_synth_tonal_non_percussive() {
        let f = make_features(1200.0, 0.25, 0.05, 0.10, 1.0, 0.03, false);
        assert_eq!(classify_non_percussive(&f), SampleCategory::Synth);
    }

    #[test]
    fn clap_not_confused_with_snare() {
        // Clap has no sub, very noisy, short
        let clap = make_features(2500.0, 0.58, 0.02, 0.38, 0.07, 0.001, true);
        // Snare has sub body
        let snare = make_features(1600.0, 0.40, 0.09, 0.18, 0.20, 0.004, true);
        assert_eq!(classify_percussive(&clap), SampleCategory::Clap);
        assert_eq!(classify_percussive(&snare), SampleCategory::Snare);
    }
}
