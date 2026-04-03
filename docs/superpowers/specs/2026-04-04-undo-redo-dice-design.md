# Phase 5: Undo/Redo + Randomization (DICE) — Design Spec

## Overview

Add undo/redo history and kit randomization ("DICE") to Autokit. Uses full-state snapshots (not deltas) for simplicity and correctness. Lock logic prevents specific pads from being affected by randomization.

No triggers (MIDI CC, OSC, GUI) are wired up in this phase — only public APIs that future phases will call.

## Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Undoable scope | Kit changes + sequencer edits | Covers painful accidental changes; lock state excluded intentionally |
| History depth | 64 entries | Generous headroom, ~192 KB worst case |
| Architecture | Full-state snapshots | State is tiny (~3 KB per snapshot); simpler and less bug-prone than command pattern |
| Dice scope | Full kit, single pad, per-category | Per-category is the key feature — re-roll hihats without locking everything else |
| Randomization preserves | volume, pan, pitch | Only the sample changes; mix stays intact |
| Lock in undo | Not restored | Lock is a guard rail, not a creative state |
| Triggers | None (Phase 5) | Public methods only; wired up in GUI/OSC phases |

## State Snapshots

### PadSnapshot

Captures the undoable state of a single drum pad.

```rust
struct PadSnapshot {
    sample: Option<Arc<Vec<f32>>>,  // Arc — no data copy
    sample_path: Option<String>,
    name: String,
    category: SampleCategory,
    volume: f32,
    pan: f32,
    pitch: f32,
}
```

**Excluded from snapshot:**
- `locked: bool` — guard rail, persists across undo/redo
- `midi_note: u8` — structural (GM drum map), never changes

### SequencerSnapshot

Captures the full 16x16 step grid plus global sequencer settings.

```rust
struct StepSnapshot {
    enabled: bool,
    velocity: f32,
    probability: f32,
}

struct LaneSnapshot {
    steps: [StepSnapshot; 16],
    muted: bool,
}

struct SequencerSnapshot {
    lanes: [LaneSnapshot; 16],
    swing: f32,
}
```

### HistorySnapshot

Combined snapshot for one undo entry.

```rust
struct HistorySnapshot {
    pads: Vec<PadSnapshot>,          // 16 entries
    sequencer: SequencerSnapshot,
}
```

## History Stack

```rust
struct History {
    undo_stack: VecDeque<HistorySnapshot>,  // max 64, O(1) eviction from front
    redo_stack: Vec<HistorySnapshot>,
    max_entries: usize,                     // 64
}
```

### Operations

**Before any mutation:** capture current state, push onto `undo_stack`. Clear `redo_stack` (new branch invalidates redo future).

**Undo:** pop from `undo_stack`, push current state onto `redo_stack`, restore the popped snapshot to kit + sequencer.

**Redo:** pop from `redo_stack`, push current state onto `undo_stack`, restore the popped snapshot to kit + sequencer.

**Overflow:** when `undo_stack.len() > max_entries`, `pop_front()` to evict the oldest entry.

**Edge cases:** undo/redo on empty stack is a no-op (returns `false`).

### Public API

```rust
impl History {
    fn new() -> Self;
    fn push(&mut self, snapshot: HistorySnapshot);  // push + clear redo + evict if full
    fn undo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot>;
    fn redo(&mut self, current: HistorySnapshot) -> Option<HistorySnapshot>;
    fn can_undo(&self) -> bool;
    fn can_redo(&self) -> bool;
}
```

## DICE Randomization

Three operations, all push a single undo snapshot before mutating (one undo step per action, not per pad):

### dice_all()

Re-roll all unlocked pads. Each pad gets a new random sample from its current category via `library.random_from(category)`. Volume, pan, and pitch are preserved.

### dice_pad(index: usize)

Re-roll one specific pad. No-op if the pad is locked. Same category, preserves mix settings.

### dice_category(category: SampleCategory)

Re-roll all unlocked pads matching the given category. One undo step for the batch.

### Behavior

- All dice methods require a `&SampleLibrary` reference. No-op if library is `None` (scan not yet complete).
- If `random_from()` returns `None` (empty category), the pad is left unchanged.
- Locked pads are always skipped.
- Sample data is `Arc`-cloned (reference count bump, no audio data copy).

## Lock Logic

### Current state

- `DrumPad.locked: bool` already exists (default `false`)
- `populate_kit_from_library()` already skips locked pads

### Phase 5 additions

**`toggle_lock(index: usize)`** — flips `pad.locked`. This is NOT an undoable action. Lock is a deliberate guard rail that the user sets to protect pads they like.

All dice operations check `pad.locked` before modifying.

## File Layout

| File | Changes |
|------|---------|
| `src/util/history.rs` | `History`, `HistorySnapshot`, `PadSnapshot`, `SequencerSnapshot`, snapshot capture/restore |
| `src/engine/kit.rs` | `DrumKit::snapshot()`, `DrumKit::restore()`, `DrumKit::dice_all()`, `DrumKit::dice_pad()`, `DrumKit::dice_category()`, `DrumKit::toggle_lock()` |
| `src/engine/sequencer.rs` | `Sequencer::snapshot()`, `Sequencer::restore()` |
| `src/plugin.rs` | `Autokit` gets `History` field; mutations go through history push |

## Testing

### History tests
- Push → undo restores previous state
- Push → undo → redo restores the mutation
- New push after undo clears redo stack
- 65th push evicts oldest entry
- Undo on empty stack returns `None`
- Redo on empty stack returns `None`

### Dice tests
- `dice_all`: unlocked pads get new samples, locked pads unchanged
- `dice_all`: volume, pan, pitch preserved after re-roll
- `dice_all`: category stays the same for each pad
- `dice_pad`: specific pad changes, others untouched
- `dice_pad`: locked pad is no-op
- `dice_category`: only pads of that category re-rolled
- All dice: no-op when library is `None`

### Lock tests
- `toggle_lock` flips the flag
- Locked pad survives `dice_all`, `dice_pad`, `dice_category`

### Integration tests
- Dice → undo → state matches pre-dice
- Dice → undo → redo → state matches post-dice
- Multiple dice calls → multiple undos restore each step
