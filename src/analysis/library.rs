use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use rand::prelude::IndexedRandom;

use crate::analysis::features;
use crate::analysis::scanner::{self, SampleEntry};
use crate::engine::kit::SampleCategory;
use crate::util::audio_file;

/// Maximum sample duration in seconds — longer samples are filtered out.
const MAX_DURATION_SECS: f32 = 4.0;

/// A fully analyzed sample ready for use.
#[derive(Debug, Clone)]
pub struct AnalyzedSample {
    pub entry: SampleEntry,
    pub features: features::AudioFeatures,
    /// Pre-loaded audio data (mono, f32, at analysis sample rate).
    pub data: Arc<Vec<f32>>,
}

/// The full sample library, organized by category.
pub struct SampleLibrary {
    /// All analyzed samples grouped by category.
    pub by_category: HashMap<SampleCategory, Vec<AnalyzedSample>>,
    /// Total number of samples.
    pub total: usize,
    /// Sample rate used for analysis.
    pub sample_rate: f32,
}

impl SampleLibrary {
    /// Scan a folder, load all samples, extract features, classify.
    /// This is expensive — run on a background thread.
    pub fn build(root: &Path, sample_rate: f32) -> Self {
        tracing::info!(root = %root.display(), "starting library scan");

        let entries = scanner::scan_folder(root);
        let max_samples = (MAX_DURATION_SECS * sample_rate) as usize;

        let mut by_category: HashMap<SampleCategory, Vec<AnalyzedSample>> = HashMap::new();
        let mut loaded = 0u32;
        let mut skipped = 0u32;

        for entry in entries {
            let path_str = match entry.path.to_str() {
                Some(s) => s,
                None => continue,
            };

            // Load audio
            let data = match audio_file::load_wav_mono(path_str) {
                Ok(d) => d,
                Err(e) => {
                    tracing::trace!(path = path_str, error = %e, "skipping unloadable file");
                    skipped += 1;
                    continue;
                }
            };

            // Filter by duration
            if data.len() > max_samples {
                skipped += 1;
                continue;
            }

            // Filter by oneshot heuristic: must have a clear transient
            if !looks_like_oneshot(&data) {
                skipped += 1;
                continue;
            }

            // Extract features
            let feats = features::extract(&data, sample_rate);

            // Classify
            let category = features::classify(&feats, entry.folder_hint);

            let mut classified_entry = entry;
            classified_entry.category = category;
            classified_entry.duration_ms = (data.len() as f32 / sample_rate * 1000.0) as u32;
            classified_entry.is_percussive = feats.is_percussive;

            let analyzed = AnalyzedSample {
                entry: classified_entry,
                features: feats,
                data: Arc::new(data),
            };

            by_category.entry(category).or_default().push(analyzed);
            loaded += 1;
        }

        // Log category distribution
        let mut dist: Vec<(SampleCategory, usize)> = by_category
            .iter()
            .map(|(cat, samples)| (*cat, samples.len()))
            .collect();
        dist.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

        tracing::info!(
            loaded,
            skipped,
            categories = ?dist,
            "library build complete"
        );

        SampleLibrary {
            total: loaded as usize,
            by_category,
            sample_rate,
        }
    }

    /// Pick a random sample from a category. Returns None if category is empty.
    pub fn random_from(&self, category: SampleCategory) -> Option<&AnalyzedSample> {
        let samples = self.by_category.get(&category)?;
        if samples.is_empty() {
            return None;
        }
        let mut rng = rand::rng();
        samples.choose(&mut rng)
    }

    /// Generate a default techno kit layout:
    /// 0-1: Kick, 2-3: Snare, 4-5: Hihat, 6: Clap, 7: Tom,
    /// 8-9: Perc, 10: Cymbal, 11: Bass, 12-13: Synth, 14-15: Other/Perc
    pub fn generate_kit(&self) -> Vec<(usize, SampleCategory)> {
        vec![
            (0, SampleCategory::Kick),
            (1, SampleCategory::Kick),
            (2, SampleCategory::Snare),
            (3, SampleCategory::Snare),
            (4, SampleCategory::Hihat),
            (5, SampleCategory::Hihat),
            (6, SampleCategory::Clap),
            (7, SampleCategory::Tom),
            (8, SampleCategory::Perc),
            (9, SampleCategory::Perc),
            (10, SampleCategory::Cymbal),
            (11, SampleCategory::Bass),
            (12, SampleCategory::Synth),
            (13, SampleCategory::Synth),
            (14, SampleCategory::Perc),
            (15, SampleCategory::Other),
        ]
    }
}

/// Heuristic: a oneshot has a relatively high peak near the start
/// and decays toward silence. Loops have sustained energy throughout.
fn looks_like_oneshot(samples: &[f32]) -> bool {
    if samples.len() < 100 {
        return true; // Very short = definitely a oneshot
    }

    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak < 1e-5 {
        return false; // Silent
    }

    // Find where the peak is (should be in the first 25% for a oneshot)
    let peak_idx = samples
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.abs().partial_cmp(&b.abs()).unwrap())
        .map(|(i, _)| i)
        .unwrap_or(0);

    let peak_position_ratio = peak_idx as f32 / samples.len() as f32;
    if peak_position_ratio > 0.4 {
        return false; // Peak too late — probably a loop or riser
    }

    // Compare energy in first quarter vs last quarter
    let quarter = samples.len() / 4;
    let first_energy: f32 = samples[..quarter].iter().map(|s| s * s).sum::<f32>() / quarter as f32;
    let last_energy: f32 = samples[samples.len() - quarter..].iter().map(|s| s * s).sum::<f32>() / quarter as f32;

    if first_energy < 1e-10 {
        return false;
    }

    // Oneshots should decay: last quarter energy should be significantly less
    let energy_ratio = last_energy / first_energy;
    energy_ratio < 0.5 // Last quarter has less than half the energy of first quarter
}
