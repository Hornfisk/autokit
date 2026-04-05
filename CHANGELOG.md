# Changelog

All notable changes to Autokit are documented here.

## [0.2.2] — 2026-04-05

### Added

- **Per-track LVL knob** — each track in seq and pads view has a 16px inline LVL knob wired to pad volume. Double-click resets to 100%. VOL knob removed from expanded pads panel (redundant).
- **Configurable sample library path** — new setup dialog with folder browser; config persisted to `~/.config/autokit/config.json`. Auto-discovers `~/Music/Samples` on first run.
- **Tempo control (standalone)** — editable BPM field in toolbar (range 30–300, drag or type). Not shown in plugin mode where host owns tempo.
- **Transport logging** — `tracing::debug!` logs at transport decision point and sequencer trigger for diagnostics.

### Fixed

- **Space-bar double-trigger in DAW** — pressing Space while the plugin GUI is focused in Renoise no longer fires both DAW transport AND internal play simultaneously. Internal play is gated: ignored when the host transport is actively driving playback.
- **Tempo change position jump** — changing BPM in standalone no longer resets the sequencer to step 1. Beat position is now accumulated incrementally instead of derived from `samples * tempo`.
- **Internal play clean start** — toggling internal play on now resets the sequencer to step 0 and clears the beat accumulator, preventing stale position jumps.
- **Standalone audio backend** — switched from JACK to ALSA backend in launch script, fixing silent `process()` failure under PipeWire's JACK emulation.

## [0.2.1] — 2026-04-05

### Fixed

- **Standalone auto-play from PipeWire/JACK transport** — the sequencer no longer auto-starts when the standalone backend reports `playing=true` without a real DAW transport. Only plays when the user presses PLAY (Space / button). DAW transport sync still works after the host stops and restarts playback.

## [0.2.0] — 2026-04-05

### Added

- **MIDI echo detection** — new `EchoDetector` suppresses doubled playback when the host routes sequencer MIDI output back as input. Activates automatically after detecting consecutive echoes; recovers after ~2s of silence. GUI shows "EXT" indicator when active.
- **DAW state persistence** — kit and pattern state now serializes to the plugin's persist block (~1s interval), so DAW save/recall restores the full session. Gated on async init completion to avoid overwriting restored state with empty defaults.
- **Lane reset** — double-click a track label in the sequencer to clear all steps, mute, and solo for that lane.
- **Step reset** — double-click a step cell to reset it to defaults.
- **Immediate pattern switch when stopped** — selecting a pattern while the sequencer is stopped now switches instantly instead of queuing for the next bar.
- **Step number header in pads view** — 1–16 column headers now appear above the pad rows, matching the sequencer grid layout.
- **Status message toast** — export feedback and errors display as a timed overlay with fade-out (~10s at 60fps).

### Changed

- **Host transport sync rewrite** — sequencer now derives step position directly from host beats every buffer, eliminating drift accumulation. Missed steps (e.g. from GUI lock contention) are caught up in order. Pattern-boundary switches happen correctly during catch-up.
- **Voice pan resolved at trigger time** — `VoicePool::process()` no longer needs a kit reference. Pan is cached when the voice fires, removing a data dependency and enabling lock-free audio rendering.
- **Audio rendering moved outside shared-state lock** — voices render after releasing the mutex, eliminating silence gaps when the GUI holds the lock.
- **Sequencer grid sizing** — cell size now scales to fill available height and width, clamped 20–48px. Pads view row height matches the sequencer grid exactly.
- **Step cell rendering** — velocity now controls fill brightness/saturation of the entire cell interior instead of a bottom-aligned bar. Color-coded glow borders: green (active), red (muted), purple (selected).
- **Sequencer layout consolidation** — pattern selector and step parameter knobs share a single row below the grid. SWING control removed from pattern bar. Bottom bar pushed to window edge.
- **Track labels** — replaced plain text labels with color-strip + category badge matching the pads view.
- **Knob double-click reset** — knobs now reset on double-click (manual detection via raw pointer events, working around egui-baseview's missing click_count). Drag suppressed on double-click frame.
- **Knob rendering** — uses `ui.painter()` instead of `painter_at(rect)` to avoid clipping strokes at rect edges; slightly smaller radius for cleaner look.
- **Pad row** — removed inline volume bar; category tag vertically centered; row height parameterized for consistency with sequencer.
- **Window width** — default editor size widened from 960 to 1060px.
- **`editor-state` persist key** renamed to `editor-state-v2` (breaking change for existing DAW sessions — window state resets once).

### Fixed

- **Host sync drift** — removed manual `last_pos_beats` accumulation that diverged from host position over time.
- **Pattern switch at bar boundary** — `process_buffer_with_patterns` now takes `&mut PatternBank` so queued pattern switches apply correctly during both catch-up and accumulator advancement.

## [0.1.0] — 2026-03-15

Initial release.

- 8-pad drum kit with per-pad volume, pan, pitch, and decay.
- Recursive sample library scan with spectral classification and 2D scatter map.
- 16-step, 8-track sequencer with per-step velocity, probability, pan/pitch p-locks, and conditional trigs.
- 16 patterns, swing, FILL mode, DICE randomization.
- Solo and mute per track; velocity drag-painting.
- Undo/redo (64-deep snapshot history).
- Save/load JSON presets; MIDI pattern export.
- Host transport sync and standalone internal transport.
- VST3, CLAP, and standalone formats on Linux, macOS, and Windows.
