use crate::analysis::library::SampleLibrary;
use crate::util::history::PadSnapshot;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;

/// Sample categories for classification and color coding.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SampleCategory {
    Kick,
    Snare,
    Hihat,
    Clap,
    Tom,
    Perc,
    Cymbal,
    Bass,
    Synth,
    Other,
}

impl SampleCategory {
    pub fn all() -> &'static [SampleCategory] {
        &[
            Self::Kick,
            Self::Snare,
            Self::Hihat,
            Self::Clap,
            Self::Tom,
            Self::Perc,
            Self::Cymbal,
            Self::Bass,
            Self::Synth,
            Self::Other,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Kick => "KICK",
            Self::Snare => "SNARE",
            Self::Hihat => "HIHAT",
            Self::Clap => "CLAP",
            Self::Tom => "TOM",
            Self::Perc => "PERC",
            Self::Cymbal => "CYMBAL",
            Self::Bass => "BASS",
            Self::Synth => "SYNTH",
            Self::Other => "OTHER",
        }
    }
}

/// Number of pads in the kit.
pub const NUM_PADS: usize = 8;

/// A single drum pad in the kit.
pub struct DrumPad {
    /// Audio sample data (mono, f32, at host sample rate)
    pub sample: Option<Arc<Vec<f32>>>,
    /// Source file path
    pub sample_path: Option<String>,
    /// Display name
    pub name: String,
    /// Classification
    pub category: SampleCategory,
    /// Locked pads survive randomization
    pub locked: bool,
    /// MIDI note number (GM drum map: 36=C1 kick through 51=D#2)
    pub midi_note: u8,
    /// Volume (0.0 to 1.0)
    pub volume: f32,
    /// Pan (-1.0 left to 1.0 right)
    pub pan: f32,
    /// Pitch adjustment in semitones
    pub pitch: f32,
    /// Decay / sample length (0.0 = very short, 1.0 = full sample). Default 1.0.
    pub decay: f32,
    /// Start point (0.0 = beginning, 1.0 = end of sample). Default 0.0.
    pub start: f32,
    /// End point (0.0 = beginning, 1.0 = end of sample). Default 1.0.
    pub end: f32,
}

impl DrumPad {
    pub fn new(index: usize) -> Self {
        Self {
            sample: None,
            sample_path: None,
            name: format!("Pad {}", index + 1),
            category: SampleCategory::Other,
            locked: false,
            midi_note: 36 + index as u8, // GM drum map starting at C1
            volume: 1.0,
            pan: 0.0,
            pitch: 0.0,
            decay: 1.0,
            start: 0.0,
            end: 1.0,
        }
    }
}

/// The drum kit.
pub struct DrumKit {
    pub pads: Vec<DrumPad>,
}

impl DrumKit {
    pub fn new() -> Self {
        Self {
            pads: (0..NUM_PADS).map(DrumPad::new).collect(),
        }
    }

    /// Get pad index for a MIDI note, if mapped.
    pub fn pad_for_note(&self, note: u8) -> Option<usize> {
        self.pads.iter().position(|p| p.midi_note == note)
    }

    /// Get the MIDI note number for a pad index.
    pub fn note_for_pad(&self, index: usize) -> u8 {
        self.pads.get(index).map(|p| p.midi_note).unwrap_or(36 + index as u8)
    }

    /// Capture the undoable state of all pads.
    pub fn snapshot(&self) -> Vec<PadSnapshot> {
        self.pads
            .iter()
            .map(|p| PadSnapshot {
                sample: p.sample.clone(),
                sample_path: p.sample_path.clone(),
                name: p.name.clone(),
                category: p.category,
                volume: p.volume,
                pan: p.pan,
                pitch: p.pitch,
                decay: p.decay,
                start: p.start,
                end: p.end,
            })
            .collect()
    }

    /// Toggle lock on a pad. Locked pads survive randomization.
    /// This is NOT an undoable action.
    pub fn toggle_lock(&mut self, index: usize) {
        if index < self.pads.len() {
            self.pads[index].locked = !self.pads[index].locked;
        }
    }

    /// Restore pad state from a snapshot. Preserves `locked` and `midi_note`.
    pub fn restore(&mut self, snapshot: &[PadSnapshot]) {
        for (pad, snap) in self.pads.iter_mut().zip(snapshot.iter()) {
            pad.sample = snap.sample.clone();
            pad.sample_path = snap.sample_path.clone();
            pad.name = snap.name.clone();
            pad.category = snap.category;
            pad.volume = snap.volume;
            pad.pan = snap.pan;
            pad.pitch = snap.pitch;
            pad.decay = snap.decay;
            pad.start = snap.start;
            pad.end = snap.end;
        }
    }

    /// Assign an analyzed sample to a pad, updating audio data, path, name, and category.
    /// Preserves volume, pan, pitch, decay.
    fn assign_sample(pad: &mut DrumPad, sample: &crate::analysis::library::AnalyzedSample, used: &mut HashSet<String>) {
        let path = sample.entry.path.to_string_lossy().to_string();
        used.insert(path.clone());
        pad.sample = Some(Arc::clone(&sample.data));
        pad.sample_path = Some(path);
        pad.name = sample.entry.filename.clone();
        pad.category = sample.entry.category;
    }

    /// Re-roll all unlocked pads from their current category.
    /// Preserves volume, pan, pitch. Avoids assigning the same sample to multiple pads.
    pub fn dice_all(&mut self, library: &SampleLibrary) {
        let mut used: HashSet<String> = self.pads.iter()
            .filter(|p| p.locked)
            .filter_map(|p| p.sample_path.clone())
            .collect();

        for pad in &mut self.pads {
            if pad.locked { continue; }
            if let Some(sample) = library.random_from_excluding(pad.category, &used) {
                Self::assign_sample(pad, &sample, &mut used);
            }
        }
    }

    /// Re-roll one specific pad. No-op if locked or out of range.
    /// Preserves volume, pan, pitch. Avoids paths already used by other pads.
    pub fn dice_pad(&mut self, index: usize, library: &SampleLibrary) {
        if index >= self.pads.len() || self.pads[index].locked { return; }
        let mut used: HashSet<String> = self.pads.iter()
            .enumerate()
            .filter(|(i, _)| *i != index)
            .filter_map(|(_, p)| p.sample_path.clone())
            .collect();

        let category = self.pads[index].category;
        if let Some(sample) = library.random_from_excluding(category, &used) {
            Self::assign_sample(&mut self.pads[index], &sample, &mut used);
        }
    }

    /// Re-roll all unlocked pads of a given category.
    /// Preserves volume, pan, pitch. Avoids assigning the same sample to multiple pads.
    pub fn dice_category(&mut self, category: SampleCategory, library: &SampleLibrary) {
        let mut used: HashSet<String> = self.pads.iter()
            .filter(|p| p.locked || p.category != category)
            .filter_map(|p| p.sample_path.clone())
            .collect();

        for pad in &mut self.pads {
            if pad.locked || pad.category != category { continue; }
            if let Some(sample) = library.random_from_excluding(category, &used) {
                Self::assign_sample(pad, &sample, &mut used);
            }
        }
    }
}

impl Default for DrumKit {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::features::AudioFeatures;
    use crate::analysis::library::{AnalyzedSample, SampleLibrary};
    use crate::analysis::scanner::SampleEntry;
    use crate::util::history::PadSnapshot;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// Build a minimal SampleLibrary with one sample per category.
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
                    sub_energy_ratio: 0.1,
                    high_freq_ratio: 0.1,
                    peak: 1.0,
                    duration: 0.1,
                    is_percussive: true,
                },
                data: Arc::new(vec![0.5; 4410]),
            };
            by_category.entry(*cat).or_default().push(sample);
        }

        SampleLibrary {
            total: 10,
            by_category,
            sample_rate: 44100.0,
        }
    }

    /// Build a library with N unique samples per category.
    fn test_library_multi(samples_per_cat: usize) -> SampleLibrary {
        let mut by_category: HashMap<SampleCategory, Vec<AnalyzedSample>> = HashMap::new();
        let mut total = 0;

        for cat in SampleCategory::all() {
            for i in 0..samples_per_cat {
                let entry = SampleEntry {
                    path: PathBuf::from(format!("/test/{}-{}.wav", cat.label(), i)),
                    filename: format!("test-{}-{}", cat.label(), i),
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
                        sub_energy_ratio: 0.1,
                        high_freq_ratio: 0.1,
                        peak: 1.0,
                        duration: 0.1,
                        is_percussive: true,
                    },
                    data: Arc::new(vec![0.5 * i as f32 / samples_per_cat as f32; 4410]),
                };
                by_category.entry(*cat).or_default().push(sample);
                total += 1;
            }
        }

        SampleLibrary {
            total,
            by_category,
            sample_rate: 44100.0,
        }
    }

    #[test]
    fn snapshot_captures_pad_state() {
        let mut kit = DrumKit::new();
        kit.pads[0].name = "MyKick".to_string();
        kit.pads[0].volume = 0.75;
        kit.pads[0].pan = -0.5;
        kit.pads[0].pitch = 2.0;
        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].sample = Some(Arc::new(vec![1.0; 100]));
        kit.pads[0].locked = true;

        let snap = kit.snapshot();
        assert_eq!(snap.len(), NUM_PADS);
        assert_eq!(snap[0].name, "MyKick");
        assert!((snap[0].volume - 0.75).abs() < 0.001);
        assert!((snap[0].pan - -0.5).abs() < 0.001);
        assert!((snap[0].pitch - 2.0).abs() < 0.001);
        assert_eq!(snap[0].category, SampleCategory::Kick);
        assert!(snap[0].sample.is_some());
    }

    #[test]
    fn restore_applies_snapshot_but_preserves_lock_and_midi() {
        let mut kit = DrumKit::new();
        kit.pads[0].locked = true;
        kit.pads[0].midi_note = 42;

        let snap: Vec<PadSnapshot> = (0..NUM_PADS)
            .map(|i| PadSnapshot {
                sample: None,
                sample_path: Some(format!("/path/{i}.wav")),
                name: format!("Restored-{i}"),
                category: SampleCategory::Snare,
                volume: 0.5,
                pan: 0.3,
                pitch: -1.0,
                decay: 1.0,
                start: 0.0,
                end: 1.0,
            })
            .collect();

        kit.restore(&snap);

        assert_eq!(kit.pads[0].name, "Restored-0");
        assert!((kit.pads[0].volume - 0.5).abs() < 0.001);
        assert_eq!(kit.pads[0].category, SampleCategory::Snare);
        // locked and midi_note should be untouched
        assert!(kit.pads[0].locked);
        assert_eq!(kit.pads[0].midi_note, 42);
    }

    #[test]
    fn toggle_lock_flips_flag() {
        let mut kit = DrumKit::new();
        assert!(!kit.pads[0].locked);

        kit.toggle_lock(0);
        assert!(kit.pads[0].locked);

        kit.toggle_lock(0);
        assert!(!kit.pads[0].locked);
    }

    #[test]
    fn dice_all_changes_unlocked_pads() {
        let mut kit = DrumKit::new();
        let lib = test_library();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "original".to_string();

        kit.dice_all(&lib);

        assert_ne!(kit.pads[0].name, "original");
    }

    #[test]
    fn dice_all_skips_locked_pads() {
        let mut kit = DrumKit::new();
        let lib = test_library();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "locked-kick".to_string();
        kit.pads[0].locked = true;

        kit.dice_all(&lib);

        assert_eq!(kit.pads[0].name, "locked-kick");
    }

    #[test]
    fn dice_all_preserves_volume_pan_pitch() {
        let mut kit = DrumKit::new();
        let lib = test_library();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].volume = 0.42;
        kit.pads[0].pan = -0.7;
        kit.pads[0].pitch = 3.5;

        kit.dice_all(&lib);

        assert!((kit.pads[0].volume - 0.42).abs() < 0.001);
        assert!((kit.pads[0].pan - -0.7).abs() < 0.001);
        assert!((kit.pads[0].pitch - 3.5).abs() < 0.001);
    }

    #[test]
    fn dice_pad_changes_specific_pad_only() {
        let mut kit = DrumKit::new();
        let lib = test_library();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "original-0".to_string();
        kit.pads[1].category = SampleCategory::Snare;
        kit.pads[1].name = "original-1".to_string();

        kit.dice_pad(0, &lib);

        assert_ne!(kit.pads[0].name, "original-0");
        assert_eq!(kit.pads[1].name, "original-1");
    }

    #[test]
    fn dice_pad_locked_is_noop() {
        let mut kit = DrumKit::new();
        let lib = test_library();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "locked".to_string();
        kit.pads[0].locked = true;

        kit.dice_pad(0, &lib);

        assert_eq!(kit.pads[0].name, "locked");
    }

    #[test]
    fn dice_category_only_affects_matching_unlocked_pads() {
        let mut kit = DrumKit::new();
        let lib = test_library();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "kick-0".to_string();
        kit.pads[1].category = SampleCategory::Kick;
        kit.pads[1].name = "kick-1".to_string();
        kit.pads[1].locked = true;
        kit.pads[2].category = SampleCategory::Snare;
        kit.pads[2].name = "snare-2".to_string();

        kit.dice_category(SampleCategory::Kick, &lib);

        // Pad 0 (kick, unlocked) should change
        assert_ne!(kit.pads[0].name, "kick-0");
        // Pad 1 (kick, locked) should NOT change
        assert_eq!(kit.pads[1].name, "kick-1");
        // Pad 2 (snare) should NOT change
        assert_eq!(kit.pads[2].name, "snare-2");
    }

    #[test]
    fn dice_all_no_duplicate_samples() {
        // Library with enough unique samples per category to fill the kit without repeats
        let lib = test_library_multi(4);
        let mut kit = DrumKit::new();

        // Assign categories matching the default 8-pad layout
        let layout = [
            SampleCategory::Kick,
            SampleCategory::Snare,
            SampleCategory::Hihat,
            SampleCategory::Clap,
            SampleCategory::Perc,
            SampleCategory::Tom,
            SampleCategory::Cymbal,
            SampleCategory::Synth,
        ];
        for (pad, cat) in kit.pads.iter_mut().zip(layout.iter()) {
            pad.category = *cat;
        }

        kit.dice_all(&lib);

        // Collect all assigned paths
        let paths: Vec<String> = kit
            .pads
            .iter()
            .filter_map(|p| p.sample_path.clone())
            .collect();

        // All pads should have a sample
        assert_eq!(paths.len(), NUM_PADS, "all pads should have a sample");

        // No two pads should share the same path
        let unique: std::collections::HashSet<&String> = paths.iter().collect();
        assert_eq!(unique.len(), paths.len(), "duplicate samples found across pads");
    }

    #[test]
    fn dice_category_no_duplicate_samples() {
        // 3 unique kick samples, 2 kick pads — should be different
        let lib = test_library_multi(3);
        let mut kit = DrumKit::new();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[1].category = SampleCategory::Kick;

        kit.dice_category(SampleCategory::Kick, &lib);

        let path0 = kit.pads[0].sample_path.as_deref().unwrap_or("");
        let path1 = kit.pads[1].sample_path.as_deref().unwrap_or("");

        assert!(!path0.is_empty(), "pad 0 should have a sample");
        assert!(!path1.is_empty(), "pad 1 should have a sample");
        assert_ne!(path0, path1, "pad 0 and pad 1 should not share a sample");
    }

    #[test]
    fn dice_pad_no_duplicate_with_other_pads() {
        // Set pad 0 and pad 1 to the same kick, then dice pad 1 — should pick a different one
        let lib = test_library_multi(2);
        let mut kit = DrumKit::new();

        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].sample_path = Some("/test/KICK-0.wav".to_string());
        kit.pads[1].category = SampleCategory::Kick;
        kit.pads[1].sample_path = Some("/test/KICK-0.wav".to_string());

        kit.dice_pad(1, &lib);

        let path0 = kit.pads[0].sample_path.as_deref().unwrap_or("");
        let path1 = kit.pads[1].sample_path.as_deref().unwrap_or("");
        assert_ne!(path0, path1, "dice_pad should not reuse pad 0's sample");
    }
}
