# Step Sequencer Design

## Overview

A 16-step sequencer that syncs to host transport, triggers the existing voice pool, and supports per-step probability. One pattern at a time, 16 lanes (one per pad).

## Data Model

### Step

```rust
struct Step {
    enabled: bool,
    velocity: f32,      // 0.0-1.0, multiplied with pad volume
    probability: f32,   // 0.0-1.0, chance of firing (1.0 = always)
}
```

Default: `enabled: false, velocity: 0.8, probability: 1.0`.

### Lane

```rust
struct Lane {
    pad_index: usize,   // 0..15, which DrumPad this lane triggers
    steps: [Step; 16],
    muted: bool,
}
```

Lane `i` maps to pad `i` by default. `muted` silences the lane without clearing step data.

### Sequencer

```rust
struct Sequencer {
    lanes: [Lane; 16],
    swing: f32,              // 0.0-1.0 (0 = straight, shifts odd steps late)
    playing: bool,           // mirrors host transport
    current_step: usize,     // 0..15
    tick_accumulator: f64,   // fractional sample position within current step
    last_pos_beats: f64,     // last known host beat position (for jump detection)
}
```

All fixed-size. No heap allocation in the audio path.

## Playback Engine

### Timing

One step = one sixteenth note = 0.25 quarter notes.

At tempo `T` BPM and sample rate `SR`:
- Samples per step (straight) = `SR * 60.0 / T / 4.0`
- Swing offsets odd-numbered steps (1, 3, 5, ...) by `swing * samples_per_step * 0.5`

Concretely: even steps (0, 2, 4, ...) last `base + swing_offset` samples, odd steps (1, 3, 5, ...) last `base - swing_offset` samples, where `swing_offset = swing * base * 0.5`. At `swing = 0.0`, all steps are equal. At `swing = 1.0`, odd steps fire at the last possible moment before the next even step (maximum shuffle). Total cycle length stays constant.

### Host Sync

Each `process()` call:

1. Read `context.transport()` to get `playing`, `tempo`, `pos_beats`.
2. If host is not playing or tempo/pos_beats unavailable: set `self.playing = false`, skip sequencer logic.
3. If host is playing:
   - Compute `step_in_song = (pos_beats * 4.0)` (sixteenth note index from song start)
   - Derive `current_step = step_in_song as usize % 16`
   - Derive fractional position within step from `step_in_song.fract()`
   - **Jump detection:** if `pos_beats` differs from expected position (based on `last_pos_beats` + elapsed samples), the host rewound or jumped. Reset `tick_accumulator` to match the new position.
   - Update `last_pos_beats`.

### Step Scanning

After establishing position from host sync, scan the buffer for step boundaries:

```
for sample_offset in 0..buffer_length:
    tick_accumulator += 1.0
    samples_for_this_step = compute_step_duration(current_step, tempo, sample_rate, swing)
    if tick_accumulator >= samples_for_this_step:
        tick_accumulator -= samples_for_this_step
        current_step = (current_step + 1) % 16
        fire_step(current_step, sample_offset)
```

`fire_step` iterates all 16 lanes. For each lane where `!muted && step.enabled`:
- Roll `rand::random::<f32>()`. If `>= step.probability`, skip.
- Call `voices.trigger(lane.pad_index, step.velocity, &kit)` with sample-accurate offset.

### Sample-Accurate Triggering

`VoicePool::trigger()` gains a `start_offset: usize` parameter. When a voice is triggered mid-buffer, it stores this offset. During `VoicePool::process()`, voices with a `start_offset > 0` skip that many samples before producing output, ensuring the attack lands at the correct sample position within the buffer.

### Interaction with MIDI Input

MIDI note-on events and sequencer triggers coexist. Both call the same `voices.trigger()` path. MIDI events are processed first (drained from context as today), then the sequencer scans for step triggers. A manual hit on a pad that's also sequenced will layer both voices.

## Integration Points

### New file: `src/engine/sequencer.rs`

Contains `Step`, `Lane`, `Sequencer` structs and all sequencer logic. The `Sequencer::process()` method takes:
- `&mut self`
- `buffer_len: usize` (number of samples in this buffer)
- `transport: &Transport` (from nih-plug context)
- `sample_rate: f32`
- `voices: &mut VoicePool`
- `kit: &DrumKit`

Returns nothing. Side-effects: triggers voices, updates internal position state.

### Changes to `src/engine/sampler.rs`

- `VoicePool::trigger()` gains `start_offset: usize` parameter.
- `Voice` struct gains `start_offset: usize` field.
- `VoicePool::process()` respects `start_offset` — voice skips that many output samples before producing audio.

### Changes to `src/plugin.rs`

- Add `sequencer: Sequencer` field to `Autokit` struct.
- In `process()`, after draining MIDI events, call `sequencer.process(...)` before `voices.process(...)`.
- Pass `context.transport()` to the sequencer.

### Changes to `src/lib.rs`

- Add `pub mod sequencer;` to the `engine` module.

## Non-Goals (Deferred)

- Multiple patterns / pattern chaining
- Micro-timing per step
- Rolls / flams
- Freerunning clock (no host)
- Step count other than 16
- UI (Phase 5)
- Serialization / preset save (later phase)
