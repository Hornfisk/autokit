# Phase 8: Step Sequencer GUI — Design Spec

## Context

Autokit's sequencer engine (`engine/sequencer.rs`) already supports 8-lane x 16-step playback with swing, per-step velocity/probability, lane muting, and host transport sync. However, there is no GUI for it — the sequencer is invisible to the user. This phase adds a full Digitakt-style sequencer view as a third tab (SEQ) alongside PADS and MAP, making the sequencer usable, inspiring, and fun.

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│ TOOLBAR   [PADS] [MAP] [SEQ]   UNDO REDO SAVE  │
├─────────────────────────────────────────────────┤
│ PATTERN  [01] 02 03 04 ...16    SWING ████ 45%  │
├─────────────────────────────────────────────────┤
│ KICK   M L  [■][ ][ ][ ][■][ ][ ][ ]...        │
│ SNARE  M L  [ ][ ][ ][ ][■][ ][ ][ ]...        │
│ HIHAT  M L  [■][■][■][■][■][■][■][■]...        │
│ CLAP   M L  [ ][ ][ ][ ][■][ ][ ][ ]...        │
│ TOM    M L  [ ][ ][ ][ ][ ][ ][ ][■]...        │
│ PERC   M L  [ ][ ][■][ ][ ][ ][■][ ]...        │
│ CYMBAL M L  [■][ ][ ][ ][ ][ ][ ][ ]...        │
│ BASS   M L  [■][ ][ ][■][■][ ][ ][■]...        │
├─────────────────────────────────────────────────┤
│ STEP 6 ▸  VEL [⟳] PAN [⟳] PITCH [⟳] PROB [⟳]  │
│           COND [1:2]                     [🔒]   │
├─────────────────────────────────────────────────┤
│ ▶ PLAY  128.0 BPM  |  🎲 DICE  FILL  COPY ... │
└─────────────────────────────────────────────────┘
```

## 1. Data Model Changes

### 1.1 Expanded Step struct

The current `Step` has `enabled`, `velocity`, `probability`. Extend it to support parameter locks and conditional trigs:

```rust
#[derive(Clone, Copy, PartialEq, Default)]
pub enum ConditionTrig {
    #[default]
    Always,        // Default — fires every loop
    Every(u8),     // 1:N — fires every Nth loop (N = 2, 4, 8)
    NotEvery(u8),  // !1:N — fires on all loops EXCEPT every Nth
    Fill,          // Fires only when FILL mode is active
    NotFill,       // Fires only when FILL mode is NOT active
}

#[derive(Clone, Copy, Default)]
pub struct Step {
    pub enabled: bool,
    pub velocity: f32,      // 0.0–1.0
    pub probability: f32,   // 0.0–1.0
    pub pan: Option<f32>,   // None = use pad default, Some(-1.0..1.0) = p-lock
    pub pitch: Option<f32>, // None = use pad default, Some(-24..24 semitones) = p-lock
    pub condition: ConditionTrig,
}
```

**Design decision:** `pan` and `pitch` use `Option<f32>` — `None` means "inherit pad default" (no p-lock), `Some(value)` means parameter-locked. This keeps the common case (no p-lock) zero-cost and makes it visually clear which steps have overrides.

### 1.2 Pattern storage

```rust
pub const NUM_STEPS: usize = 16;
pub const NUM_PATTERNS: usize = 16;

#[derive(Clone)]
pub struct Pattern {
    pub lanes: [Lane; NUM_PADS],
    pub swing: f32,
}

pub struct PatternBank {
    pub patterns: [Pattern; NUM_PATTERNS],
    pub active: usize,        // Currently playing pattern (0–15)
    pub queued: Option<usize>, // Pattern queued for next bar boundary
}
```

**Pattern switching:** The `PatternBank` replaces the current bare `lanes` array in `Sequencer`. The `queued` field enables bar-boundary switching: when the user clicks a pattern, it's queued; at the next step-0 boundary, `active` swaps to `queued` and `queued` clears.

### 1.3 Fill mode

```rust
// In Sequencer:
pub fill_active: bool, // true while FILL button is held
```

When `fill_active` is true, `ConditionTrig::Fill` steps fire and `ConditionTrig::NotFill` steps don't. When false, the reverse. This is a momentary hold — press and hold the FILL button during a live performance, release to return to normal.

### 1.4 Loop counter

For ratio conditions (1:2, 1:4, 1:8), the sequencer needs a loop counter that increments each time the pattern wraps from step 15 back to step 0:

```rust
// In Sequencer:
loop_count: u64,  // increments on each full pattern cycle
```

`ConditionTrig::Every(n)` fires when `loop_count % n == 0`. `NotEvery(n)` fires when `loop_count % n != 0`.

### 1.5 Snapshot updates

`StepSnapshot` and `LaneSnapshot` must be extended to include the new fields:

```rust
#[derive(Clone, Copy)]
pub struct StepSnapshot {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
    pub pan: Option<f32>,
    pub pitch: Option<f32>,
    pub condition: ConditionTrig,
}

#[derive(Clone)]
pub struct PatternSnapshot {
    pub lanes: [LaneSnapshot; NUM_PADS],
    pub swing: f32,
}

#[derive(Clone)]
pub struct SequencerSnapshot {
    pub patterns: [PatternSnapshot; NUM_PATTERNS],
    pub active_pattern: usize,
}
```

## 2. GUI Layout — Sequencer View

### 2.1 View integration

Add `ViewMode::Sequencer` alongside `PadStrip` and `SampleMap`. The toolbar gets a third tab button: **SEQ**.

### 2.2 Pattern selector bar

A horizontal row of 16 numbered buttons (01–16) below the toolbar.

**Visual states:**
- **Active:** Filled accent color (#00d4aa) with dark text — the currently playing pattern
- **Has data:** Brighter text + slightly highlighted border — pattern has at least one enabled step
- **Queued:** Pulsing accent border animation — pattern is queued for next bar
- **Empty:** Dim text, default border — no steps enabled

**Interaction:** Click to queue. The pattern switches at the next step-0 boundary.

**Swing control:** To the right of pattern buttons. A horizontal drag bar (0–100%) with numeric readout. Drag horizontally to adjust; Ctrl+click to reset to 0%.

### 2.3 Step grid (8 rows x 16 columns)

The main area. Each row is one pad lane, each column is one sixteenth-note step.

**Track labels (left side):**
- Pad category name in its category color (KICK, SNARE, etc.)
- Click label to select that track (for keyboard step entry)
- Two small buttons per track:
  - **M** (mute) — red when active, dims the entire row
  - **L** (lock) — orange when active, protects track from DICE randomization

**Step cells:**
- **Empty step:** Dark background (`#111126`), subtle border
- **Active step:** Filled with category color, velocity shown by bar height (taller = louder)
- **Beat markers:** Steps 1, 5, 9, 13 (downbeats) have slightly lighter backgrounds
- **Playhead:** The current step column gets a bright top border (#00d4aa) that moves in real-time
- **Selected step:** Bright accent border + glow shadow — this step's params are shown in the parameter bar
- **Conditional trig indicator:** Small yellow text in the top-right corner of the cell (e.g., "1:2", "FIL")
- **Parameter lock indicator:** Small blue dot in the top-left corner when pan or pitch is p-locked
- **Muted track:** Entire row dims to ~30% opacity

**Interaction:**
- **Click** empty step: Enable it (default velocity 0.8)
- **Click** active step: Select it (show params below). Click again to disable.
- **Shift+click**: Toggle step without changing selection
- Playhead position comes from `sequencer.current_step()` via the DisplaySnapshot

### 2.4 Step parameter bar

Appears below the grid when a step is selected. Shows the selected step's parameters with knobs.

**Layout (left to right):**
1. **Step indicator:** "STEP N" in accent color (N = 1–16, 1-indexed for display)
2. **VEL knob:** 0–100%, ring color = category color
3. **PAN knob:** L100–C–R100, ring color = blue when p-locked, gray when inheriting pad default
4. **PITCH knob:** -24st to +24st, ring color = blue when p-locked, gray when inheriting pad default
5. **PROB knob:** 0–100%
6. **COND selector:** Clickable button cycling through: `——` (always), `1:2`, `1:4`, `1:8`, `!1:2`, `!1:4`, `!1:8`, `FIL`, `!FIL`. Yellow text when not "always".
_(No step-level lock — randomization respects track-level locks only, per design decision.)_

**Knob behavior** (reuses existing `knob.rs` widget):
- Vertical drag to change value (0.005 per pixel, 0.001 with Shift for fine control)
- Ctrl+click to reset to default
- Pan/pitch knobs show different ring color when p-locked (blue) vs inheriting (gray)

**P-lock creation:** When the user changes pan or pitch on a step, it becomes a p-lock (the `Option` goes from `None` to `Some`). To clear a p-lock: Ctrl+click the knob (resets to None/inherit).

### 2.5 Bottom bar

Below the parameter bar. Contains transport controls and pattern operations.

**Left side:**
- **PLAY button:** Reflects host transport state (green when playing). Not directly controllable (host-driven), but shows status.
- **BPM display:** Shows host tempo (e.g., "128.0 BPM"). Read-only.

**Center:**
- **DICE button:** Randomizes the active pattern. Respects track-level locks (locked tracks are skipped). Randomizes step placement, velocity, and probability. Pushes undo before randomizing.
- **FILL button:** Momentary hold — activates fill mode while held.

**Right side:**
- **COPY:** Copies the active pattern to clipboard (internal state, not OS clipboard)
- **PASTE:** Pastes clipboard pattern over the active pattern (pushes undo first)
- **CLEAR:** Clears all steps in the active pattern (pushes undo first)

## 3. Engine Changes

### 3.1 fire_step() updates

The `fire_step()` method needs to evaluate conditional trigs and apply per-step pan/pitch overrides:

```rust
fn fire_step(&mut self, sample_offset: usize, voices: &mut VoicePool, kit: &DrumKit) -> usize {
    let step_idx = self.current_step;
    let mut count = 0;

    for i in 0..self.lanes().len() {
        let lane = &self.active_pattern().lanes[i];
        if lane.muted { continue; }

        let step = &lane.steps[step_idx];
        if !step.enabled { continue; }

        // Conditional trig gate
        if !self.evaluate_condition(step.condition) { continue; }

        // Probability gate
        if step.probability < 1.0 {
            let roll: f32 = self.rng.random();
            if roll >= step.probability { continue; }
        }

        let pad = &kit.pads[lane.pad_index];
        let velocity = step.velocity;
        let pan = step.pan.unwrap_or(pad.pan);
        let pitch = step.pitch.unwrap_or(pad.pitch);

        voices.trigger_with_overrides(lane.pad_index, velocity, pan, pitch, kit, sample_offset);
        count += 1;
    }
    count
}
```

### 3.2 evaluate_condition()

```rust
fn evaluate_condition(&self, cond: ConditionTrig) -> bool {
    match cond {
        ConditionTrig::Always => true,
        ConditionTrig::Every(n) => self.loop_count % n as u64 == 0,
        ConditionTrig::NotEvery(n) => self.loop_count % n as u64 != 0,
        ConditionTrig::Fill => self.fill_active,
        ConditionTrig::NotFill => !self.fill_active,
    }
}
```

### 3.3 Pattern switching at bar boundary

In `process_buffer()`, when `current_step` wraps from 15 to 0:

```rust
if self.current_step == 0 {
    self.loop_count += 1;
    if let Some(queued) = self.bank.queued.take() {
        self.bank.active = queued;
    }
}
```

### 3.4 Voice trigger with overrides

Modify `VoicePool::trigger()` to accept optional pan/pitch overrides: `trigger(pad_index, velocity, kit, sample_offset, pan_override: Option<f32>, pitch_override: Option<f32>)`. When `Some`, the voice uses the override instead of the pad-level value. The existing call sites pass `None, None` to preserve current behavior.

## 4. GUI ↔ Audio Communication

### 4.1 DisplaySnapshot extension

Add sequencer state to the snapshot taken in Phase 1 of the render loop:

```rust
pub struct SeqDisplay {
    pub current_step: usize,
    pub playing: bool,
    pub active_pattern: usize,
    pub queued_pattern: Option<usize>,
    pub fill_active: bool,
    pub loop_count: u64,
    pub pattern: PatternDisplay, // Snapshot of active pattern's lanes + swing
}

pub struct PatternDisplay {
    pub lanes: [LaneDisplay; NUM_PADS],
    pub swing: f32,
}

pub struct LaneDisplay {
    pub pad_index: usize,
    pub steps: [StepDisplay; NUM_STEPS],
    pub muted: bool,
}

pub struct StepDisplay {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
    pub pan: Option<f32>,
    pub pitch: Option<f32>,
    pub condition: ConditionTrig,
}
```

### 4.2 New GuiActions

```rust
enum GuiAction {
    // ... existing actions ...

    // Step operations
    ToggleStep { lane: usize, step: usize },
    SetStepVelocity { lane: usize, step: usize, value: f32 },
    SetStepPan { lane: usize, step: usize, value: Option<f32> },
    SetStepPitch { lane: usize, step: usize, value: Option<f32> },
    SetStepProbability { lane: usize, step: usize, value: f32 },
    SetStepCondition { lane: usize, step: usize, condition: ConditionTrig },

    // Lane operations
    ToggleLaneMute { lane: usize },
    ToggleLaneLock { lane: usize },

    // Pattern operations
    SelectPattern { index: usize },
    SetSwing { value: f32 },
    CopyPattern,
    PastePattern,
    ClearPattern,
    DicePattern,

    // Fill mode
    SetFillActive { active: bool },
}
```

### 4.3 Undo integration

Pattern-mutating actions (ToggleStep, SetStep*, ClearPattern, PastePattern, DicePattern) push a history snapshot before applying. Lane mute/unmute and pattern selection do NOT push history (they're non-destructive navigation).

## 5. Randomize (DICE) Behavior

When DICE is pressed on the sequencer view:

1. Push undo snapshot
2. For each unlocked track in the active pattern:
   - Clear all steps
   - Randomly enable 2–6 steps (uniform random)
   - Set velocity to 0.5–1.0 (uniform random per step)
   - Set probability to 0.7–1.0 (uniform random per step)
   - Small chance (15%) of adding a conditional trig to each step
   - pan/pitch p-locks are NOT randomized (left as None)
3. Locked tracks are completely untouched

## 6. Pattern Save/Load (Serialization)

Patterns serialize to JSON via serde for save/load. The full pattern bank is included in the plugin state.

```rust
#[derive(Serialize, Deserialize)]
pub struct PatternBankState {
    pub patterns: Vec<PatternState>,  // 16 patterns
    pub active: usize,
}

#[derive(Serialize, Deserialize)]
pub struct PatternState {
    pub lanes: Vec<LaneState>,
    pub swing: f32,
}

#[derive(Serialize, Deserialize)]
pub struct LaneState {
    pub steps: Vec<StepState>,
    pub muted: bool,
}

#[derive(Serialize, Deserialize)]
pub struct StepState {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
    pub pan: Option<f32>,
    pub pitch: Option<f32>,
    pub condition: String,  // "always"|"1:2"|"1:4"|"1:8"|"!1:2"|"!1:4"|"!1:8"|"fill"|"!fill"
}
```

## 7. New UI Files

- `src/ui/sequencer.rs` — Main sequencer view: grid rendering, step cell drawing, pattern bar, parameter bar, bottom bar. Split into clear functions: `draw_pattern_bar()`, `draw_grid()`, `draw_param_bar()`, `draw_bottom_bar()`.
- No other new files — types go in `engine/sequencer.rs`, snapshots in `util/history.rs`, actions in `ui/editor.rs`.

## 8. Key Interactions Summary

| Action | Result |
|--------|--------|
| Click empty step | Enable step (vel 0.8, prob 1.0) |
| Click active step | Select it (shows params below) |
| Click selected step | Disable it |
| Shift+click step | Toggle without changing selection |
| Click pattern button | Queue pattern for next bar |
| Drag swing bar | Adjust swing 0–100% |
| Click M on track | Toggle mute (red) |
| Click L on track | Toggle lock for DICE (orange) |
| Drag VEL/PAN/PITCH/PROB knob | Adjust selected step's parameter |
| Ctrl+click PAN/PITCH knob | Clear p-lock (inherit pad default) |
| Ctrl+click VEL/PROB knob | Reset to default (0.8 / 1.0) |
| Click COND button | Cycle through conditions |
| Click DICE | Randomize unlocked tracks |
| Hold FILL | Activate fill mode |
| Click COPY/PASTE/CLEAR | Pattern clipboard operations |

## 9. Verification

1. **Build:** `cargo build` — no warnings
2. **Tests:** `cargo test` — all existing + new tests pass
3. **Visual:** Load in a DAW, open SEQ tab, verify:
   - Grid renders 8 rows × 16 columns with correct category colors
   - Clicking steps toggles them, playhead moves with transport
   - Parameter knobs adjust selected step values
   - Pattern switching queues and switches at bar boundary
   - DICE randomizes unlocked tracks, locked tracks untouched
   - FILL mode activates conditional steps
   - Mute dims tracks, muted tracks don't trigger
   - Copy/paste/clear work with undo support
4. **RT safety:** No allocations on audio thread (assert_process_allocs in debug)
