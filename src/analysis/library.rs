use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use rand::prelude::IndexedRandom;

use crate::analysis::cache::{self, CacheEntry, LibraryCache};
use crate::analysis::features;
use crate::analysis::scanner::{self, SampleEntry};
use crate::engine::kit::SampleCategory;
use crate::util::audio_file;

/// Shared scan progress counters, readable from the UI thread.
pub struct ScanProgress {
    pub processed: AtomicU32,
    pub total: AtomicU32,
}

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
    /// Uses a persistent cache to skip DSP analysis for unchanged files.
    /// This is expensive — run on a background thread.
    pub fn build(root: &Path, sample_rate: f32) -> Self {
        Self::build_with_progress(root, sample_rate, None)
    }

    pub fn build_with_progress(root: &Path, sample_rate: f32, progress: Option<&Arc<ScanProgress>>) -> Self {
        tracing::info!(root = %root.display(), "starting library scan");

        // Load existing cache (if present, valid, and for the same root)
        let mut scan_cache = LibraryCache::load(root).unwrap_or_else(|| LibraryCache::new(root));

        let entries = scanner::scan_folder(root);
        let max_samples = (MAX_DURATION_SECS * sample_rate) as usize;

        if let Some(p) = &progress {
            p.total.store(entries.len() as u32, Ordering::Relaxed);
        }

        let mut by_category: HashMap<SampleCategory, Vec<AnalyzedSample>> = HashMap::new();
        let mut loaded = 0u32;
        let mut skipped = 0u32;
        let mut cache_hits = 0u32;
        let mut cache_misses = 0u32;

        for entry in entries {
            let path_str = match entry.path.to_str() {
                Some(s) => s,
                None => continue,
            };

            // Load audio — always required for playback regardless of cache
            let data = match audio_file::load_wav_mono(path_str) {
                Ok(d) => d,
                Err(e) => {
                    tracing::trace!(path = path_str, error = %e, "skipping unloadable file");
                    skipped += 1;
                    if let Some(p) = &progress { p.processed.store(loaded + skipped, Ordering::Relaxed); }
                    continue;
                }
            };

            // Filter by duration
            if data.len() > max_samples {
                skipped += 1;
                if let Some(p) = &progress { p.processed.store(loaded + skipped, Ordering::Relaxed); }
                continue;
            }

            // Filter by oneshot heuristic: must have a clear transient
            if !looks_like_oneshot(&data) {
                skipped += 1;
                if let Some(p) = &progress { p.processed.store(loaded + skipped, Ordering::Relaxed); }
                continue;
            }

            // Check cache for this file's mtime+size
            let (mtime, fsize) = cache::file_stamp(&entry.path);
            let (feats, category, duration_ms, is_percussive) =
                if let Some(cached) = scan_cache.get_if_valid(&entry.path, mtime, fsize) {
                    // Cache hit: reuse classification and features, skip DSP
                    cache_hits += 1;
                    (
                        cached.features.clone(),
                        cached.category,
                        cached.duration_ms,
                        cached.is_percussive,
                    )
                } else {
                    // Cache miss: run full DSP analysis
                    cache_misses += 1;
                    let f = features::extract(&data, sample_rate);
                    let cat = features::classify(&f, entry.folder_hint);
                    let dur_ms = (data.len() as f32 / sample_rate * 1000.0) as u32;
                    let percussive = f.is_percussive;

                    // Update cache entry
                    scan_cache.insert(
                        &entry.path,
                        CacheEntry {
                            modified_secs: mtime,
                            file_size: fsize,
                            category: cat,
                            duration_ms: dur_ms,
                            is_percussive: percussive,
                            features: f.clone(),
                        },
                    );

                    (f, cat, dur_ms, percussive)
                };

            let mut classified_entry = entry;
            classified_entry.category = category;
            classified_entry.duration_ms = duration_ms;
            classified_entry.is_percussive = is_percussive;

            let analyzed = AnalyzedSample {
                entry: classified_entry,
                features: feats,
                data: Arc::new(data),
            };

            by_category.entry(category).or_default().push(analyzed);
            if let Some(p) = &progress {
                p.processed.store(loaded + skipped, Ordering::Relaxed);
            }
            loaded += 1;
        }

        // Mark progress complete before post-loop work (cache save)
        if let Some(p) = &progress {
            p.processed.store(p.total.load(Ordering::Relaxed), Ordering::Relaxed);
        }

        // Purge stale cache entries for files no longer on disk
        let removed = scan_cache.retain_existing();

        // Persist updated cache
        scan_cache.save();

        // Log cache stats and category distribution
        let mut dist: Vec<(SampleCategory, usize)> = by_category
            .iter()
            .map(|(cat, samples)| (*cat, samples.len()))
            .collect();
        dist.sort_by_key(|(_, count)| std::cmp::Reverse(*count));

        tracing::info!(
            loaded,
            skipped,
            cache_hits,
            cache_misses,
            cache_removed = removed,
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

    /// Pick a random sample from a category, excluding already-used paths.
    /// Falls back to any sample in the category if all are excluded (best-effort dedup).
    /// Returns None if the category is empty.
    pub fn random_from_excluding<'a>(
        &'a self,
        category: SampleCategory,
        exclude: &HashSet<String>,
    ) -> Option<&'a AnalyzedSample> {
        let samples = self.by_category.get(&category)?;
        let candidates: Vec<&AnalyzedSample> = samples
            .iter()
            .filter(|s| !exclude.contains(&s.entry.path.to_string_lossy().to_string()))
            .collect();
        let mut rng = rand::rng();
        if candidates.is_empty() {
            // All samples in this category are already used — best-effort: pick any
            samples.choose(&mut rng)
        } else {
            candidates.choose(&mut rng).copied()
        }
    }

    /// Create a reference-only clone for use in dice operations.
    /// The Arc'd sample data is shared, not copied.
    pub fn clone_for_dice(&self) -> SampleLibrary {
        SampleLibrary {
            total: self.total,
            by_category: self.by_category.clone(),
            sample_rate: self.sample_rate,
        }
    }

    /// Return all samples in a deterministic flat order (sorted by category discriminant).
    /// The index into this vec is `library_index` used by MapPoint.
    pub fn all_samples_flat(&self) -> Vec<&AnalyzedSample> {
        let mut categories: Vec<SampleCategory> = self.by_category.keys().copied().collect();
        categories.sort_by_key(|c| *c as u8);
        let mut flat = Vec::with_capacity(self.total);
        for cat in categories {
            if let Some(samples) = self.by_category.get(&cat) {
                flat.extend(samples.iter());
            }
        }
        flat
    }

    /// Retrieve a sample by its flat index (as returned by `all_samples_flat`).
    /// Returns None if index is out of bounds.
    pub fn sample_by_flat_index(&self, index: usize) -> Option<&AnalyzedSample> {
        let mut categories: Vec<SampleCategory> = self.by_category.keys().copied().collect();
        categories.sort_by_key(|c| *c as u8);
        let mut offset = 0;
        for cat in categories {
            if let Some(samples) = self.by_category.get(&cat) {
                if index < offset + samples.len() {
                    return Some(&samples[index - offset]);
                }
                offset += samples.len();
            }
        }
        None
    }

    /// Generate a default 8-pad techno kit layout.
    pub fn generate_kit(&self) -> Vec<(usize, SampleCategory)> {
        vec![
            (0, SampleCategory::Kick),
            (1, SampleCategory::Snare),
            (2, SampleCategory::Hihat),
            (3, SampleCategory::Clap),
            (4, SampleCategory::Perc),
            (5, SampleCategory::Tom),
            (6, SampleCategory::Cymbal),
            (7, SampleCategory::Synth),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::features::AudioFeatures;
    use crate::analysis::scanner::SampleEntry;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn test_library() -> SampleLibrary {
        let mut by_category: HashMap<SampleCategory, Vec<AnalyzedSample>> = HashMap::new();
        for cat in SampleCategory::all() {
            let entry = SampleEntry {
                path: PathBuf::from(format!("/test/{}.wav", cat.label())),
                filename: format!("test-{}", cat.label()),
                category: *cat,
                folder_hint: None,
                duration_ms: 100,
                is_percussive: true,
            };
            let sample = AnalyzedSample {
                entry,
                features: AudioFeatures {
                    attack_time: 0.001,
                    decay_time: 0.05,
                    spectral_centroid: 1000.0,
                    spectral_flatness: 0.5,
                    peak: 1.0,
                    duration: 0.1,
                    is_percussive: true,
                },
                data: Arc::new(vec![0.5; 4410]),
            };
            by_category.entry(*cat).or_default().push(sample);
        }
        SampleLibrary { total: 10, by_category, sample_rate: 44100.0 }
    }

    #[test]
    fn all_samples_flat_returns_all_samples() {
        let lib = test_library();
        let flat = lib.all_samples_flat();
        assert_eq!(flat.len(), lib.total);
    }

    #[test]
    fn all_samples_flat_deterministic_order() {
        let lib = test_library();
        let flat1 = lib.all_samples_flat();
        let flat2 = lib.all_samples_flat();
        for (a, b) in flat1.iter().zip(flat2.iter()) {
            assert_eq!(a.entry.filename, b.entry.filename);
        }
    }

    #[test]
    fn sample_by_flat_index_round_trips() {
        let lib = test_library();
        let flat = lib.all_samples_flat();
        for (i, sample) in flat.iter().enumerate() {
            let retrieved = lib.sample_by_flat_index(i).expect("should exist");
            assert_eq!(retrieved.entry.filename, sample.entry.filename);
        }
    }

    #[test]
    fn sample_by_flat_index_out_of_bounds() {
        let lib = test_library();
        assert!(lib.sample_by_flat_index(999999).is_none());
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
