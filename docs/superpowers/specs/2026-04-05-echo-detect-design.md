# Echo Detection: Auto-Mute on MIDI Echo

## Problem

When Autokit exports a pattern to Renoise and the user plays it back, both the internal sequencer AND the echoed MIDI from Renoise trigger the same samples, causing doubled playback.

The plugin outputs MIDI NoteOn/Off on ch10 for each sequencer step. Renoise records this, then plays it back into the plugin as MIDI input. The internal sequencer is still running, so every hit fires twice.

## Solution

An `EchoDetector` on the audio thread that tracks outgoing sequencer MIDI and suppresses incoming notes that match within a short timing window. Manual pad play (different notes or different timing) passes through unaffected.

## Data Structure

```rust
pub struct EchoDetector {
    buffer: [(u8, u64); 32],  // (note, sample_timestamp) ring buffer
    write_pos: usize,
    len: usize,
    sample_clock: u64,        // monotonic sample counter
    echo_window: u64,         // max samples between send/receive to count as echo
    consecutive_echoes: u32,
    suppress_threshold: u32,  // consecutive echoes before activating EXT mode
    suppressing: bool,
    last_echo_clock: u64,     // sample_clock of last detected echo
    recovery_samples: u64,    // samples with no echoes before clearing EXT mode
}
```

- Fixed-size 32-entry ring buffer (max 8 pads per step, entries expire quickly)
- No heap allocation, audio-thread safe
- `sample_clock` incremented by `buffer_len` each `process()` call

## Constants

- `ECHO_WINDOW`: 2400 samples (~50ms at 48kHz) — max round-trip for echo matching
- `SUPPRESS_THRESHOLD`: 4 consecutive echoes before activating EXT mode
- `RECOVERY_SAMPLES`: 96000 (~2 seconds at 48kHz) with no echoes before clearing EXT mode

## Flow in `process()`

**Critical ordering**: sequencer fires and records BEFORE incoming MIDI is checked. This is necessary because Renoise echoes are phase-locked to the same transport — they arrive in the same buffer as the sequencer fires.

1. `detector.tick(buffer_len)` — advance clock, check recovery timeout
2. Sequencer runs, fires steps, triggers voices
3. MIDI output sent to host + `detector.record(note)` for each triggered note
4. Incoming MIDI NoteOn arrives:
   - `detector.check(note)` scans buffer for matching note within `echo_window`
   - **Match found**: consume the entry (set note to 0xFF), increment `consecutive_echoes`, return `true` (suppress voice trigger)
   - **No match**: reset `consecutive_echoes` to 0, return `false` (trigger voice normally)
5. After 4+ consecutive echoes: set `suppressing = true`, shared with UI via `AtomicBool`

## Edge Cases

- **Same note, different source**: If user manually hits a pad at the exact moment the sequencer fires the same note, the manual hit gets suppressed. Acceptable — the sequencer already triggered that pad.
- **Multiple pads same MIDI note**: Each outgoing note gets its own ring buffer entry. `check()` consumes only one match per incoming note.
- **Internal play (no host)**: No MIDI routing back, detector never matches, zero overhead.
- **Sequencer stopped**: No outgoing notes recorded, all incoming MIDI triggers normally.

## Additional changes implemented alongside echo detection

### Pattern switching fix
- `process_buffer_with_patterns()` now takes `&mut PatternBank` and handles queued pattern switch at bar boundary (was missing)
- When sequencer is stopped, pattern selection switches immediately instead of queuing

### DAW state persistence
- `#[persist = "plugin-state"]` field in `AutokitParams` serializes kit + patterns as JSON every ~1s
- On reload, checks for persisted state and restores instead of randomizing from library
- Serialization wrapped in `permit_alloc()` for audio thread safety

## File Changes

| File | Change |
|------|--------|
| `src/engine/echo_detect.rs` | New file: `EchoDetector` with `new()`, `record()`, `check()`, `tick()`, 8 unit tests |
| `src/lib.rs`, `src/main.rs` | Add `pub mod echo_detect;` to engine block |
| `src/engine/sequencer.rs` | `process_buffer_with_patterns` takes `&mut PatternBank`, adds queued switch at bar boundary |
| `src/plugin.rs` | EchoDetector + `seq_ext_mode` AtomicBool, reordered process() flow, `#[persist]` for kit+patterns, periodic state snapshot |
| `src/ui/editor.rs` | Pass `seq_ext_mode` to editor, immediate pattern switch when stopped |
| `src/ui/sequencer_ui.rs` | `ext_mode` field on SeqDisplay, orange "EXT" label in bottom bar |

## UI

Small "EXT" label displayed near the PLAY button when `suppressing` is true. Reads an `AtomicBool` from the plugin — no lock contention.
