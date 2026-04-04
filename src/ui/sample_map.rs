use crate::engine::kit::SampleCategory;

/// A single point on the sample map.
#[derive(Clone, Debug)]
pub struct MapPoint {
    /// Normalized X (0.0–1.0), from spectral centroid (log scale).
    pub nx: f32,
    /// Normalized Y (0.0–1.0), from decay time (linear, 0=short/top, 1=long/bottom).
    pub ny: f32,
    /// Index into SampleLibrary::all_samples_flat() for retrieval.
    pub library_index: usize,
    /// Category for coloring.
    pub category: SampleCategory,
    /// Filename for tooltip.
    pub name: String,
    /// Original centroid in Hz.
    pub centroid_hz: f32,
    /// Original decay in seconds.
    pub decay_secs: f32,
}

/// Normalize spectral centroid to 0.0–1.0 via log scale.
/// Maps ~100Hz → 0.0, ~20kHz → 1.0.
fn normalize_centroid(hz: f32) -> f32 {
    if hz <= 0.0 {
        return 0.0;
    }
    ((hz / 100.0).log2() / (200.0_f32).log2()).clamp(0.0, 1.0)
}

/// Normalize decay time to 0.0–1.0 (linear, max 4s).
fn normalize_decay(secs: f32) -> f32 {
    (secs / 4.0).clamp(0.0, 1.0)
}

use crate::analysis::library::SampleLibrary;

/// Build map points from the full sample library.
/// Call once when library scan completes; cache the result in EditorState.
pub fn build_map_points(library: &SampleLibrary) -> Vec<MapPoint> {
    let flat = library.all_samples_flat();
    flat.iter()
        .enumerate()
        .map(|(i, sample)| MapPoint {
            nx: normalize_centroid(sample.features.spectral_centroid),
            ny: normalize_decay(sample.features.decay_time),
            library_index: i,
            category: sample.entry.category,
            name: sample.entry.filename.clone(),
            centroid_hz: sample.features.spectral_centroid,
            decay_secs: sample.features.decay_time,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centroid_100hz_maps_to_zero() {
        let n = normalize_centroid(100.0);
        assert!(n.abs() < 0.01, "100Hz should map near 0.0, got {n}");
    }

    #[test]
    fn centroid_20khz_maps_to_one() {
        let n = normalize_centroid(20000.0);
        assert!((n - 1.0).abs() < 0.05, "20kHz should map near 1.0, got {n}");
    }

    #[test]
    fn centroid_1khz_maps_mid() {
        let n = normalize_centroid(1000.0);
        assert!(n > 0.3 && n < 0.6, "1kHz should be mid-range, got {n}");
    }

    #[test]
    fn centroid_zero_maps_to_zero() {
        assert_eq!(normalize_centroid(0.0), 0.0);
    }

    #[test]
    fn decay_zero_maps_to_zero() {
        assert_eq!(normalize_decay(0.0), 0.0);
    }

    #[test]
    fn decay_4s_maps_to_one() {
        assert_eq!(normalize_decay(4.0), 1.0);
    }

    #[test]
    fn decay_clamps_above_4s() {
        assert_eq!(normalize_decay(10.0), 1.0);
    }

    #[test]
    fn decay_2s_maps_to_half() {
        assert!((normalize_decay(2.0) - 0.5).abs() < 0.01);
    }

    #[test]
    fn build_map_points_count_matches_library() {
        use crate::analysis::library::AnalyzedSample;
        use crate::analysis::features::AudioFeatures;
        use crate::analysis::scanner::SampleEntry;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Arc;

        let mut by_category = HashMap::new();
        for cat in SampleCategory::all() {
            let entry = SampleEntry {
                path: PathBuf::from(format!("/test/{}.wav", cat.label())),
                filename: format!("{}.wav", cat.label()),
                category: *cat,
                folder_hint: None,
                duration_ms: 100,
                is_percussive: true,
            };
            let sample = AnalyzedSample {
                entry,
                features: AudioFeatures {
                    attack_time: 0.001,
                    decay_time: 0.1,
                    spectral_centroid: 1000.0,
                    spectral_flatness: 0.5,
                    peak: 1.0,
                    duration: 0.1,
                    is_percussive: true,
                },
                data: Arc::new(vec![0.5; 4410]),
            };
            by_category.entry(*cat).or_insert_with(Vec::new).push(sample);
        }
        let lib = SampleLibrary { total: 10, by_category, sample_rate: 44100.0 };
        let points = build_map_points(&lib);
        assert_eq!(points.len(), 10);
        for p in &points {
            assert!(p.nx >= 0.0 && p.nx <= 1.0, "nx out of range: {}", p.nx);
            assert!(p.ny >= 0.0 && p.ny <= 1.0, "ny out of range: {}", p.ny);
        }
    }
}
