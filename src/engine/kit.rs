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
}

impl Default for DrumKit {
    fn default() -> Self {
        Self::new()
    }
}
