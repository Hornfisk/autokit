# Phase 5: Undo/Redo + DICE Randomization — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add snapshot-based undo/redo (64-deep) and DICE randomization (full kit, single pad, per-category) to Autokit, with lock logic protecting pads from randomization.

**Architecture:** Full-state snapshots stored in a `VecDeque`-backed history stack. Each snapshot captures 16 pad states + 16x16 sequencer grid. `Arc<Vec<f32>>` sample data is reference-counted (no audio copy). All dice operations push one snapshot before mutating. Lock state is excluded from undo/redo.

**Tech Stack:** Rust, nih-plug, std::collections::VecDeque, existing DrumKit/Sequencer types

**Spec:** `docs/superpowers/specs/2026-04-04-undo-redo-dice-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|----------------|
| `src/util/history.rs` | Rewrite (currently stub) | `PadSnapshot`, `SequencerSnapshot`, `HistorySnapshot`, `History` struct with undo/redo stack |
| `src/engine/kit.rs` | Modify | Add `DrumKit::snapshot()`, `DrumKit::restore()`, `DrumKit::toggle_lock()`, `DrumKit::dice_all()`, `DrumKit::dice_pad()`, `DrumKit::dice_category()` |
| `src/engine/sequencer.rs` | Modify | Add `StepSnapshot`, `LaneSnapshot`, `SequencerSnapshot` types + `Sequencer::snapshot()`, `Sequencer::restore()` |
| `src/plugin.rs` | Modify | Add `History` field to `Autokit`, wire history push before mutations |

---

### Task 1: Snapshot types and History struct

**Files:**
- Rewrite: `src/util/history.rs`

- [ ] **Step 1: Write failing test — push and undo restores previous state**

Add to `src/util/history.rs`:

```rust
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
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib util::history::tests::push_then_undo_restores_previous 2>&1 | tail -20`

Expected: FAIL — types not defined yet.

- [ ] **Step 3: Write snapshot types and History implementation**

Replace the full contents of `src/util/history.rs` with:

```rust
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
```

Then append the `#[cfg(test)]` module after the impl block (the test from Step 1).

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib util::history::tests::push_then_undo_restores_previous 2>&1 | tail -10`

Expected: PASS

- [ ] **Step 5: Write remaining history tests**

Add these tests to the existing `mod tests` in `src/util/history.rs`:

```rust
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
    // The first restored should be "s64" (s0 was evicted)
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
```

- [ ] **Step 6: Run all history tests**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib util::history::tests 2>&1 | tail -15`

Expected: all 6 tests PASS.

- [ ] **Step 7: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/util/history.rs
git commit -m "feat(history): add snapshot types and History undo/redo stack (64-deep)"
```

---

### Task 2: DrumKit snapshot and restore

**Files:**
- Modify: `src/engine/kit.rs`

- [ ] **Step 1: Write failing test — snapshot captures pad state and restore applies it**

Add to `src/engine/kit.rs` at the bottom of the existing file. If there's no `#[cfg(test)]` module yet, create one:

```rust
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
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests 2>&1 | tail -15`

Expected: FAIL — `snapshot()` and `restore()` not defined.

- [ ] **Step 3: Implement snapshot and restore on DrumKit**

Add these methods to `impl DrumKit` in `src/engine/kit.rs`:

```rust
use crate::util::history::PadSnapshot;

impl DrumKit {
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
```

Add `use std::sync::Arc;` at the top of the file if not already present (it is — already used by `DrumPad`).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests 2>&1 | tail -15`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/engine/kit.rs
git commit -m "feat(kit): add snapshot and restore for undo/redo support"
```

---

### Task 3: Sequencer snapshot and restore

**Files:**
- Modify: `src/engine/sequencer.rs`

- [ ] **Step 1: Write failing test — sequencer snapshot captures and restore applies**

Add to the existing `mod tests` in `src/engine/sequencer.rs`:

```rust
use crate::util::history::{StepSnapshot, LaneSnapshot, SequencerSnapshot};

#[test]
fn snapshot_captures_sequencer_state() {
    let mut seq = Sequencer::new();
    seq.lanes[0].steps[0].enabled = true;
    seq.lanes[0].steps[0].velocity = 0.6;
    seq.lanes[3].muted = true;
    seq.swing = 0.3;

    let snap = seq.snapshot();
    assert!(snap.lanes[0].steps[0].enabled);
    assert!((snap.lanes[0].steps[0].velocity - 0.6).abs() < 0.001);
    assert!(snap.lanes[3].muted);
    assert!((snap.swing - 0.3).abs() < 0.001);
}

#[test]
fn restore_applies_sequencer_snapshot() {
    let mut seq = Sequencer::new();
    seq.lanes[0].steps[0].enabled = true;
    seq.swing = 0.5;

    // Capture, then modify
    let snap = seq.snapshot();
    seq.lanes[0].steps[0].enabled = false;
    seq.swing = 0.0;

    // Restore
    seq.restore(&snap);
    assert!(seq.lanes[0].steps[0].enabled);
    assert!((seq.swing - 0.5).abs() < 0.001);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::sequencer::tests::snapshot_captures_sequencer_state 2>&1 | tail -15`

Expected: FAIL — methods not defined.

- [ ] **Step 3: Implement snapshot and restore on Sequencer**

Add to `impl Sequencer` in `src/engine/sequencer.rs`:

```rust
use crate::util::history::{StepSnapshot, LaneSnapshot, SequencerSnapshot};

impl Sequencer {
    /// Capture the undoable sequencer state (steps, lanes, swing).
    /// Excludes playback state (playing, current_step, tick_accumulator, rng).
    pub fn snapshot(&self) -> SequencerSnapshot {
        let lanes: [LaneSnapshot; 16] = core::array::from_fn(|i| {
            let steps: [StepSnapshot; 16] = core::array::from_fn(|j| StepSnapshot {
                enabled: self.lanes[i].steps[j].enabled,
                velocity: self.lanes[i].steps[j].velocity,
                probability: self.lanes[i].steps[j].probability,
            });
            LaneSnapshot {
                steps,
                muted: self.lanes[i].muted,
            }
        });
        SequencerSnapshot {
            lanes,
            swing: self.swing,
        }
    }

    /// Restore sequencer state from a snapshot. Preserves playback state.
    pub fn restore(&mut self, snapshot: &SequencerSnapshot) {
        for (lane, snap_lane) in self.lanes.iter_mut().zip(snapshot.lanes.iter()) {
            for (step, snap_step) in lane.steps.iter_mut().zip(snap_lane.steps.iter()) {
                step.enabled = snap_step.enabled;
                step.velocity = snap_step.velocity;
                step.probability = snap_step.probability;
            }
            lane.muted = snap_lane.muted;
        }
        self.swing = snapshot.swing;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::sequencer::tests 2>&1 | tail -15`

Expected: all sequencer tests PASS (existing + 2 new).

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/engine/sequencer.rs
git commit -m "feat(sequencer): add snapshot and restore for undo/redo support"
```

---

### Task 4: Lock toggle

**Files:**
- Modify: `src/engine/kit.rs`

- [ ] **Step 1: Write failing test — toggle_lock flips the flag**

Add to `mod tests` in `src/engine/kit.rs`:

```rust
#[test]
fn toggle_lock_flips_flag() {
    let mut kit = DrumKit::new();
    assert!(!kit.pads[0].locked);

    kit.toggle_lock(0);
    assert!(kit.pads[0].locked);

    kit.toggle_lock(0);
    assert!(!kit.pads[0].locked);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests::toggle_lock_flips_flag 2>&1 | tail -10`

Expected: FAIL — `toggle_lock` not defined.

- [ ] **Step 3: Implement toggle_lock**

Add to `impl DrumKit` in `src/engine/kit.rs`:

```rust
/// Toggle lock on a pad. Locked pads survive randomization.
/// This is NOT an undoable action.
pub fn toggle_lock(&mut self, index: usize) {
    if index < self.pads.len() {
        self.pads[index].locked = !self.pads[index].locked;
    }
}
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests::toggle_lock_flips_flag 2>&1 | tail -10`

Expected: PASS

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/engine/kit.rs
git commit -m "feat(kit): add toggle_lock method"
```

---

### Task 5: DICE randomization methods

**Files:**
- Modify: `src/engine/kit.rs`

- [ ] **Step 1: Write failing tests for dice_all**

Add to `mod tests` in `src/engine/kit.rs`:

```rust
use crate::analysis::library::SampleLibrary;
use std::collections::HashMap;
use crate::analysis::scanner::SampleEntry;
use crate::analysis::features::AudioFeatures;
use crate::analysis::library::AnalyzedSample;
use std::path::PathBuf;

/// Build a minimal SampleLibrary with known samples for testing.
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
                attack_ms: 1.0,
                decay_ms: 50.0,
                spectral_centroid: 1000.0,
                spectral_flatness: 0.5,
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

#[test]
fn dice_all_changes_unlocked_pads() {
    let mut kit = DrumKit::new();
    let lib = test_library();

    // Give pads initial samples and categories
    kit.pads[0].category = SampleCategory::Kick;
    kit.pads[0].name = "original".to_string();

    kit.dice_all(&lib);

    // Pad should have been re-rolled (name changed from "original")
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
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests::dice_all 2>&1 | tail -15`

Expected: FAIL — `dice_all` not defined.

- [ ] **Step 3: Implement dice_all, dice_pad, dice_category**

Add to `impl DrumKit` in `src/engine/kit.rs`:

```rust
use crate::analysis::library::SampleLibrary;

impl DrumKit {
    /// Re-roll all unlocked pads from their current category.
    /// Preserves volume, pan, pitch.
    pub fn dice_all(&mut self, library: &SampleLibrary) {
        for pad in &mut self.pads {
            if pad.locked {
                continue;
            }
            if let Some(sample) = library.random_from(pad.category) {
                pad.sample = Some(Arc::clone(&sample.data));
                pad.sample_path = Some(sample.entry.path.to_string_lossy().to_string());
                pad.name = sample.entry.filename.clone();
                pad.category = sample.entry.category;
            }
        }
    }

    /// Re-roll one specific pad. No-op if locked or out of range.
    /// Preserves volume, pan, pitch.
    pub fn dice_pad(&mut self, index: usize, library: &SampleLibrary) {
        if index >= self.pads.len() {
            return;
        }
        let pad = &mut self.pads[index];
        if pad.locked {
            return;
        }
        if let Some(sample) = library.random_from(pad.category) {
            pad.sample = Some(Arc::clone(&sample.data));
            pad.sample_path = Some(sample.entry.path.to_string_lossy().to_string());
            pad.name = sample.entry.filename.clone();
            pad.category = sample.entry.category;
        }
    }

    /// Re-roll all unlocked pads of a given category.
    /// Preserves volume, pan, pitch.
    pub fn dice_category(&mut self, category: SampleCategory, library: &SampleLibrary) {
        for pad in &mut self.pads {
            if pad.locked || pad.category != category {
                continue;
            }
            if let Some(sample) = library.random_from(category) {
                pad.sample = Some(Arc::clone(&sample.data));
                pad.sample_path = Some(sample.entry.path.to_string_lossy().to_string());
                pad.name = sample.entry.filename.clone();
                pad.category = sample.entry.category;
            }
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests 2>&1 | tail -20`

Expected: all kit tests PASS.

- [ ] **Step 5: Write remaining dice tests — dice_pad and dice_category**

Add to `mod tests` in `src/engine/kit.rs`:

```rust
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
```

- [ ] **Step 6: Run all kit tests**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib engine::kit::tests 2>&1 | tail -20`

Expected: all PASS.

- [ ] **Step 7: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/engine/kit.rs
git commit -m "feat(kit): add dice_all, dice_pad, dice_category randomization"
```

---

### Task 6: Integration tests — dice + undo/redo round-trip

**Files:**
- Modify: `src/util/history.rs` (add integration tests)

- [ ] **Step 1: Write integration test — dice then undo restores original state**

Add to `mod tests` in `src/util/history.rs`:

```rust
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

    // Undo → back to "before"
    let current = snapshot_from(&kit, &seq);
    let restored = history.undo(current).unwrap();
    kit.restore(&restored.pads);
    assert_eq!(kit.pads[0].name, "before");

    // Redo → back to "diced-kick"
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
                attack_ms: 1.0,
                decay_ms: 50.0,
                spectral_centroid: 1000.0,
                spectral_flatness: 0.5,
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
```

- [ ] **Step 2: Run integration tests**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib util::history::tests 2>&1 | tail -20`

Expected: all tests PASS.

- [ ] **Step 3: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/util/history.rs
git commit -m "test(history): add integration tests for dice + undo/redo round-trips"
```

---

### Task 7: Wire History into the plugin struct

**Files:**
- Modify: `src/plugin.rs`

- [ ] **Step 1: Add History field to Autokit**

In `src/plugin.rs`, add the import at the top:

```rust
use crate::util::history::{History, HistorySnapshot};
```

Add a field to `struct Autokit`:

```rust
pub struct Autokit {
    // ... existing fields ...
    /// Undo/redo history for kit + sequencer changes.
    history: History,
}
```

Update `Default for Autokit`:

```rust
history: History::new(),
```

- [ ] **Step 2: Update populate_kit_from_library to push history**

Modify the `BgMessage::LibraryReady` handler in `process()` to push a snapshot before populating:

```rust
BgMessage::LibraryReady(library) => {
    tracing::info!(
        total = library.total,
        "library received — populating kit"
    );
    // Push snapshot before first population for undo support
    let snapshot = HistorySnapshot {
        pads: self.kit.snapshot(),
        sequencer: self.sequencer.snapshot(),
    };
    self.history.push(snapshot);
    populate_kit_from_library(&mut self.kit, &library);
    self.library = Some(library);
}
```

- [ ] **Step 3: Verify build compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -15`

Expected: no errors.

- [ ] **Step 4: Commit**

```bash
cd /home/natalia/repos/Autokit
git add src/plugin.rs
git commit -m "feat(plugin): wire History into Autokit struct with snapshot on kit population"
```

---

### Task 8: Full build and test suite

**Files:** None (verification only)

- [ ] **Step 1: Run the complete test suite**

Run: `cd /home/natalia/repos/Autokit && cargo test 2>&1 | tail -30`

Expected: all tests PASS (existing sampler + sequencer tests plus all new history/kit/integration tests).

- [ ] **Step 2: Build release bundle**

Run: `cd /home/natalia/repos/Autokit && cargo xtask bundle autokit --release 2>&1 | tail -10`

Expected: builds successfully, produces `.vst3` and `.clap` bundles.

- [ ] **Step 3: Install and verify plugin loads**

Run:
```bash
cp -r /home/natalia/repos/Autokit/target/bundled/autokit.vst3 ~/.vst3/
cp -r /home/natalia/repos/Autokit/target/bundled/autokit.clap ~/.clap/
```

Then verify the log after loading in Renoise or standalone:
```bash
cd /home/natalia/repos/Autokit && cargo run --bin autokit-standalone -- --backend alsa --output-device pipewire 2>&1 | head -20
```

- [ ] **Step 4: Commit if any fixes were needed**

Only if previous steps required fixes. Otherwise, Phase 5 is complete.
