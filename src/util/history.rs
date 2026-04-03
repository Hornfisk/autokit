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
}
