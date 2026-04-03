use std::collections::VecDeque;
use std::sync::Arc;

use crate::engine::kit::SampleCategory;

const MAX_HISTORY: usize = 64;

/// Snapshot of one drum pad's undoable state.
/// `locked` and `midi_note` are excluded — they persist across undo/redo.
#[derive(Clone)]
pub struct PadSnapshot {
    pub sample: Option<Arc<Vec<f32>>>,
    pub sample_path: Option<String>,
    pub name: String,
    pub category: SampleCategory,
    pub volume: f32,
    pub pan: f32,
    pub pitch: f32,
}

/// Snapshot of one sequencer step.
#[derive(Clone, Copy)]
pub struct StepSnapshot {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
}

/// Snapshot of one sequencer lane.
#[derive(Clone)]
pub struct LaneSnapshot {
    pub steps: [StepSnapshot; 16],
    pub muted: bool,
}

/// Snapshot of the full sequencer state.
#[derive(Clone)]
pub struct SequencerSnapshot {
    pub lanes: [LaneSnapshot; 16],
    pub swing: f32,
}

/// Combined snapshot for one undo entry.
#[derive(Clone)]
pub struct HistorySnapshot {
    pub pads: Vec<PadSnapshot>,
    pub sequencer: SequencerSnapshot,
}

/// Undo/redo history using full-state snapshots.
pub struct History {
    undo_stack: VecDeque<HistorySnapshot>,
    redo_stack: Vec<HistorySnapshot>,
}

impl History {
    pub fn new() -> Self {
        Self {
            undo_stack: VecDeque::with_capacity(MAX_HISTORY),
            redo_stack: Vec::new(),
        }
    }

    /// Push a snapshot before a mutation. Clears the redo stack.
    pub fn push(&mut self, snapshot: HistorySnapshot) {
        if self.undo_stack.len() >= MAX_HISTORY {
            self.undo_stack.pop_front();
        }
        self.undo_stack.push_back(snapshot);
        self.redo_stack.clear();
    }

    /// Undo: pop from undo stack, push current state to redo, return the snapshot to restore.
    pub fn undo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        let previous = self.undo_stack.pop_back()?;
        self.redo_stack.push(current);
        Some(previous)
    }

    /// Redo: pop from redo stack, push current state to undo, return the snapshot to restore.
    pub fn redo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot> {
        let next = self.redo_stack.pop()?;
        self.undo_stack.push_back(current);
        Some(next)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo_stack.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo_stack.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::kit::SampleCategory;

    /// Create a minimal snapshot with identifiable pad names.
    fn make_snapshot(label: &str) -> HistorySnapshot {
        let pads: Vec<PadSnapshot> = (0..16)
            .map(|i| PadSnapshot {
                sample: None,
                sample_path: None,
                name: format!("{label}-{i}"),
                category: SampleCategory::Kick,
                volume: 1.0,
                pan: 0.0,
                pitch: 0.0,
            })
            .collect();

        let lanes: [LaneSnapshot; 16] = core::array::from_fn(|_| LaneSnapshot {
            steps: [StepSnapshot {
                enabled: false,
                velocity: 0.8,
                probability: 1.0,
            }; 16],
            muted: false,
        });

        HistorySnapshot {
            pads,
            sequencer: SequencerSnapshot { lanes, swing: 0.0 },
        }
    }

    #[test]
    fn push_then_undo_restores_previous() {
        let mut history = History::new();
        let state_a = make_snapshot("a");
        let state_b = make_snapshot("b");

        history.push(state_a.clone());
        let restored = history.undo(state_b);
        assert!(restored.is_some());
        let restored = restored.unwrap();
        assert_eq!(restored.pads[0].name, "a-0");
    }

    #[test]
    fn undo_then_redo_restores_mutation() {
        let mut history = History::new();
        let state_a = make_snapshot("a");
        let state_b = make_snapshot("b");

        history.push(state_a);
        let restored = history.undo(state_b.clone()).unwrap();
        assert_eq!(restored.pads[0].name, "a-0");

        let redone = history.redo(restored).unwrap();
        assert_eq!(redone.pads[0].name, "b-0");
    }

    #[test]
    fn new_push_after_undo_clears_redo() {
        let mut history = History::new();
        history.push(make_snapshot("a"));
        history.push(make_snapshot("b"));

        // Undo once
        let _ = history.undo(make_snapshot("c"));
        assert!(history.can_redo());

        // New push should clear redo
        history.push(make_snapshot("d"));
        assert!(!history.can_redo());
    }

    #[test]
    fn overflow_evicts_oldest() {
        let mut history = History::new();
        for i in 0..65 {
            history.push(make_snapshot(&format!("s{i}")));
        }

        // Should have 64 entries (oldest evicted)
        let mut count = 0;
        let mut current = make_snapshot("current");
        while let Some(restored) = history.undo(current) {
            count += 1;
            current = restored;
        }
        assert_eq!(count, 64);
    }

    #[test]
    fn undo_on_empty_returns_none() {
        let mut history = History::new();
        assert!(history.undo(make_snapshot("x")).is_none());
        assert!(!history.can_undo());
    }

    #[test]
    fn redo_on_empty_returns_none() {
        let mut history = History::new();
        assert!(history.redo(make_snapshot("x")).is_none());
        assert!(!history.can_redo());
    }

    use crate::engine::kit::DrumKit;
    use crate::engine::sequencer::Sequencer;
    use std::sync::Arc;

    /// Helper: create a full HistorySnapshot from a kit and sequencer.
    fn snapshot_from(kit: &DrumKit, seq: &Sequencer) -> HistorySnapshot {
        HistorySnapshot {
            pads: kit.snapshot(),
            sequencer: seq.snapshot(),
        }
    }

    #[test]
    fn dice_then_undo_restores_original() {
        use crate::analysis::library::{AnalyzedSample, SampleLibrary};
        use crate::analysis::scanner::SampleEntry;
        use crate::analysis::features::AudioFeatures;
        use std::collections::HashMap;
        use std::path::PathBuf;

        // Build a minimal library
        let mut by_category = HashMap::new();
        let entry = SampleEntry {
            path: PathBuf::from("/test/kick.wav"),
            filename: "new-kick".to_string(),
            category: SampleCategory::Kick,
            folder_hint: None,
            duration_ms: 100,
            is_percussive: true,
        };
        by_category.entry(SampleCategory::Kick).or_insert_with(Vec::new).push(AnalyzedSample {
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
            data: Arc::new(vec![0.5; 100]),
        });
        let lib = SampleLibrary {
            total: 1,
            by_category,
            sample_rate: 44100.0,
        };

        let mut kit = DrumKit::new();
        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "original-kick".to_string();
        kit.pads[0].sample = Some(Arc::new(vec![1.0; 100]));

        let seq = Sequencer::new();
        let mut history = History::new();

        // Snapshot before dice
        let before = snapshot_from(&kit, &seq);
        history.push(before);

        // Dice
        kit.dice_all(&lib);
        assert_eq!(kit.pads[0].name, "new-kick");

        // Undo
        let current = snapshot_from(&kit, &seq);
        let restored = history.undo(current).unwrap();
        kit.restore(&restored.pads);

        assert_eq!(kit.pads[0].name, "original-kick");
    }

    #[test]
    fn dice_undo_redo_roundtrip() {
        use crate::analysis::library::{AnalyzedSample, SampleLibrary};
        use crate::analysis::scanner::SampleEntry;
        use crate::analysis::features::AudioFeatures;
        use std::collections::HashMap;
        use std::path::PathBuf;

        let mut by_category = HashMap::new();
        let entry = SampleEntry {
            path: PathBuf::from("/test/kick.wav"),
            filename: "diced-kick".to_string(),
            category: SampleCategory::Kick,
            folder_hint: None,
            duration_ms: 100,
            is_percussive: true,
        };
        by_category.entry(SampleCategory::Kick).or_insert_with(Vec::new).push(AnalyzedSample {
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
            data: Arc::new(vec![0.5; 100]),
        });
        let lib = SampleLibrary {
            total: 1,
            by_category,
            sample_rate: 44100.0,
        };

        let mut kit = DrumKit::new();
        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "before".to_string();

        let seq = Sequencer::new();
        let mut history = History::new();

        // Push pre-dice snapshot, then dice
        history.push(snapshot_from(&kit, &seq));
        kit.dice_all(&lib);
        assert_eq!(kit.pads[0].name, "diced-kick");

        // Undo -> back to "before"
        let current = snapshot_from(&kit, &seq);
        let restored = history.undo(current).unwrap();
        kit.restore(&restored.pads);
        assert_eq!(kit.pads[0].name, "before");

        // Redo -> back to "diced-kick"
        let current = snapshot_from(&kit, &seq);
        let redone = history.redo(current).unwrap();
        kit.restore(&redone.pads);
        assert_eq!(kit.pads[0].name, "diced-kick");
    }

    #[test]
    fn multiple_dice_multiple_undos() {
        use crate::analysis::library::{AnalyzedSample, SampleLibrary};
        use crate::analysis::scanner::SampleEntry;
        use crate::analysis::features::AudioFeatures;
        use std::collections::HashMap;
        use std::path::PathBuf;

        // Library with distinct kick samples
        let mut by_category: HashMap<SampleCategory, Vec<AnalyzedSample>> = HashMap::new();
        for i in 0..5 {
            let entry = SampleEntry {
                path: PathBuf::from(format!("/test/kick{i}.wav")),
                filename: format!("kick-{i}"),
                category: SampleCategory::Kick,
                folder_hint: None,
                duration_ms: 100,
                is_percussive: true,
            };
            by_category.entry(SampleCategory::Kick).or_insert_with(Vec::new).push(AnalyzedSample {
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
                data: Arc::new(vec![0.5; 100]),
            });
        }
        let lib = SampleLibrary {
            total: 5,
            by_category,
            sample_rate: 44100.0,
        };

        let mut kit = DrumKit::new();
        kit.pads[0].category = SampleCategory::Kick;
        kit.pads[0].name = "initial".to_string();

        let seq = Sequencer::new();
        let mut history = History::new();
        let mut names: Vec<String> = vec!["initial".to_string()];

        // Dice 3 times, tracking names
        for _ in 0..3 {
            history.push(snapshot_from(&kit, &seq));
            kit.dice_all(&lib);
            names.push(kit.pads[0].name.clone());
        }

        // Undo 3 times — should walk back through names
        for i in (0..3).rev() {
            let current = snapshot_from(&kit, &seq);
            let restored = history.undo(current).unwrap();
            kit.restore(&restored.pads);
            assert_eq!(kit.pads[0].name, names[i]);
        }
    }
}
