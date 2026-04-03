use realfft::RealFftPlanner;

use crate::engine::kit::SampleCategory;

/// Audio features extracted from a sample for classification.
#[derive(Debug, Clone)]
pub struct AudioFeatures {
    /// Time from start to peak amplitude, in seconds.
    pub attack_time: f32,
    /// Time from peak to -20dB below peak, in seconds.
    pub decay_time: f32,
    /// Spectral centroid in Hz (brightness — low=kick, high=hat).
    pub spectral_centroid: f32,
    /// Spectral flatness 0..1 (1=noise/white, 0=tonal/pure tone).
    pub spectral_flatness: f32,
    /// Peak amplitude (0..1).
    pub peak: f32,
    /// Duration in seconds.
    pub duration: f32,
    /// Whether the sample has a sharp transient and decays.
    pub is_percussive: bool,
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
            peak,
            duration,
            is_percussive: false,
        };
    }

    let attack_time = compute_attack_time(samples, sample_rate);
    let decay_time = compute_decay_time(samples, sample_rate, peak);
    let (spectral_centroid, spectral_flatness) = compute_spectral_features(samples, sample_rate);
    let is_percussive = attack_time < 0.015 && decay_time < 2.0;

    AudioFeatures {
        attack_time,
        decay_time,
        spectral_centroid,
        spectral_flatness,
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
        .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
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
        .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
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

/// Compute spectral centroid (Hz) and spectral flatness (0..1) from the full sample.
fn compute_spectral_features(samples: &[f32], sample_rate: f32) -> (f32, f32) {
    let fft_size = 2048;

    if samples.len() < fft_size {
        // Too short for meaningful FFT — use a smaller window or zero-pad
        return compute_spectral_short(samples, sample_rate);
    }

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let mut scratch = fft.make_scratch_vec();

    // Analyze multiple windows and average
    let hop = fft_size / 2;
    let num_windows = ((samples.len() - fft_size) / hop).max(1);

    let mut total_centroid = 0.0f32;
    let mut total_flatness = 0.0f32;
    let mut valid_windows = 0u32;

    for w in 0..num_windows {
        let start = w * hop;
        if start + fft_size > samples.len() {
            break;
        }

        let mut buffer: Vec<f32> = samples[start..start + fft_size].to_vec();

        // Apply Hann window
        for (i, s) in buffer.iter_mut().enumerate() {
            let hann = 0.5 * (1.0 - (2.0 * std::f32::consts::PI * i as f32 / fft_size as f32).cos());
            *s *= hann;
        }

        let mut spectrum = fft.make_output_vec();
        if fft.process_with_scratch(&mut buffer, &mut spectrum, &mut scratch).is_err() {
            continue;
        }

        // Compute magnitude spectrum
        let magnitudes: Vec<f32> = spectrum.iter().map(|c| (c.re * c.re + c.im * c.im).sqrt()).collect();

        let mag_sum: f32 = magnitudes.iter().sum();
        if mag_sum < 1e-10 {
            continue;
        }

        // Spectral centroid: weighted average frequency
        let freq_resolution = sample_rate / fft_size as f32;
        let centroid: f32 = magnitudes
            .iter()
            .enumerate()
            .map(|(i, &m)| i as f32 * freq_resolution * m)
            .sum::<f32>()
            / mag_sum;

        // Spectral flatness: geometric mean / arithmetic mean of magnitudes
        let n = magnitudes.len() as f32;
        let arith_mean = mag_sum / n;

        // Geometric mean via log to avoid underflow
        let log_sum: f32 = magnitudes
            .iter()
            .map(|&m| (m + 1e-10).ln())
            .sum::<f32>();
        let geom_mean = (log_sum / n).exp();

        let flatness = if arith_mean > 1e-10 {
            (geom_mean / arith_mean).clamp(0.0, 1.0)
        } else {
            0.0
        };

        total_centroid += centroid;
        total_flatness += flatness;
        valid_windows += 1;
    }

    if valid_windows == 0 {
        return (0.0, 0.0);
    }

    (
        total_centroid / valid_windows as f32,
        total_flatness / valid_windows as f32,
    )
}

/// Fallback spectral analysis for very short samples (< 2048 samples).
fn compute_spectral_short(samples: &[f32], sample_rate: f32) -> (f32, f32) {
    // Zero-pad to 1024
    let fft_size = 1024;
    let mut buffer = vec![0.0f32; fft_size];
    let copy_len = samples.len().min(fft_size);
    buffer[..copy_len].copy_from_slice(&samples[..copy_len]);

    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(fft_size);
    let mut scratch = fft.make_scratch_vec();
    let mut spectrum = fft.make_output_vec();

    if fft.process_with_scratch(&mut buffer, &mut spectrum, &mut scratch).is_err() {
        return (0.0, 0.0);
    }

    let magnitudes: Vec<f32> = spectrum.iter().map(|c| (c.re * c.re + c.im * c.im).sqrt()).collect();
    let mag_sum: f32 = magnitudes.iter().sum();
    if mag_sum < 1e-10 {
        return (0.0, 0.0);
    }

    let freq_resolution = sample_rate / fft_size as f32;
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

    (centroid, flatness)
}

/// Classify a sample into a category using DSP features + optional folder hint.
pub fn classify(features: &AudioFeatures, folder_hint: Option<SampleCategory>) -> SampleCategory {
    // If we have a strong folder hint and the features don't contradict it, trust the folder
    if let Some(hint) = folder_hint {
        if !contradicts_hint(features, hint) {
            return hint;
        }
    }

    // Pure DSP classification
    if features.is_percussive {
        classify_percussive(features)
    } else {
        classify_non_percussive(features)
    }
}

fn classify_percussive(f: &AudioFeatures) -> SampleCategory {
    let centroid = f.spectral_centroid;
    let flatness = f.spectral_flatness;
    let decay = f.decay_time;

    // Kick: low frequency, percussive
    if centroid < 400.0 {
        return SampleCategory::Kick;
    }

    // Hi-hat: very high frequency, short decay
    if centroid > 5000.0 && decay < 0.3 {
        return SampleCategory::Hihat;
    }

    // Cymbal: high frequency, longer decay
    if centroid > 4000.0 && decay > 0.3 {
        return SampleCategory::Cymbal;
    }

    // Snare: mid frequency, noisy
    if centroid > 1000.0 && centroid < 5000.0 && flatness > 0.3 {
        return SampleCategory::Snare;
    }

    // Clap: mid frequency, noisy, very short
    if centroid > 800.0 && centroid < 4000.0 && flatness > 0.4 && decay < 0.2 {
        return SampleCategory::Clap;
    }

    // Tom: mid-low frequency, more tonal
    if centroid > 200.0 && centroid < 1500.0 && flatness < 0.3 {
        return SampleCategory::Tom;
    }

    // Default percussive
    SampleCategory::Perc
}

fn classify_non_percussive(f: &AudioFeatures) -> SampleCategory {
    let centroid = f.spectral_centroid;

    // Bass: low frequency, tonal, sustained
    if centroid < 500.0 && f.spectral_flatness < 0.3 {
        return SampleCategory::Bass;
    }

    // Synth: tonal, sustained
    if f.spectral_flatness < 0.4 {
        return SampleCategory::Synth;
    }

    SampleCategory::Other
}

/// Check if DSP features strongly contradict a folder hint.
fn contradicts_hint(f: &AudioFeatures, hint: SampleCategory) -> bool {
    match hint {
        SampleCategory::Kick => f.spectral_centroid > 3000.0, // "kick" folder but very bright
        SampleCategory::Hihat => f.spectral_centroid < 1000.0, // "hihat" folder but very dark
        SampleCategory::Bass => f.is_percussive && f.spectral_centroid > 2000.0,
        _ => false, // Trust most folder hints
    }
}
