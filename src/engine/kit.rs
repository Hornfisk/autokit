use crate::util::history::PadSnapshot;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Sample categories for classification and color coding.
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
        }
    }
}

/// The 16-pad drum kit.
pub struct DrumKit {
    pub pads: Vec<DrumPad>,
}

impl DrumKit {
    pub fn new() -> Self {
        Self {
            pads: (0..16).map(DrumPad::new).collect(),
        }
    }

    /// Get pad index for a MIDI note, if mapped.
    pub fn pad_for_note(&self, note: u8) -> Option<usize> {
        self.pads.iter().position(|p| p.midi_note == note)
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
    use crate::util::history::PadSnapshot;

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
        assert_eq!(snap.len(), 16);
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

        let snap: Vec<PadSnapshot> = (0..16)
            .map(|i| PadSnapshot {
                sample: None,
                sample_path: Some(format!("/path/{i}.wav")),
                name: format!("Restored-{i}"),
                category: SampleCategory::Snare,
                volume: 0.5,
                pan: 0.3,
                pitch: -1.0,
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
}
