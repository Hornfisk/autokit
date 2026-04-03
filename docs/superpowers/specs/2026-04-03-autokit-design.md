# Autokit — Design Spec

## Context

Autokit is an open-source drum machine plugin for Linux, inspired by XLN Audio XO and Algonaut Atlas 2. The user (ARKITECH) produces techno in Renoise on Arch Linux + Hyprland, and needs a tool to explore, randomize, and sequence their 1.3GB sample library visually. No comparable open-source tool exists for Linux.

This is a Rust rewrite of the original C++/LV2 Autokit concept. The shift to Rust provides memory safety without GC overhead — critical for real-time audio.

**Goal:** A polished, creative VST3/CLAP drum machine that lets Linux producers visually explore, randomize, and sequence drum samples with advanced pattern generation.

---

## Architecture

### Approach: Monolith (v0.1) → Workspace (v0.2)

Single nih-plug VST3/CLAP binary for v0.1. All modules in one crate with clean internal boundaries, designed for later extraction into a cargo workspace with a standalone analysis CLI.

### Module Structure

```
autokit/
├── Cargo.toml
├── models/
│   └── drum_classifier.onnx        # v0.2: pre-trained classifier (~3MB)
├── src/
│   ├── lib.rs                       # plugin entry (nih_export_vst3!, nih_export_clap!)
│   ├── plugin.rs                    # Plugin trait impl, params, process()
│   │
│   ├── engine/
│   │   ├── mod.rs
│   │   ├── sampler.rs               # voice allocation, sample playback, mixing
│   │   ├── sequencer.rs             # 16-step sequencer, swing, velocity, rolls
│   │   ├── kit.rs                   # DrumKit (16 pads, lock state, assignments)
│   │   └── midi.rs                  # MIDI input handling + MIDI note output
│   │
│   ├── analysis/
│   │   ├── mod.rs
│   │   ├── features.rs              # audio feature extraction (spectral centroid, MFCC, RMS, ZCR)
│   │   ├── classifier.rs           # v0.2: ONNX inference → labels
│   │   ├── scanner.rs               # folder scanning, file listing, change detection
│   │   └── db.rs                    # v0.2: SQLite cache for analysis results
│   │
│   ├── ui/
│   │   ├── mod.rs                   # egui editor setup
│   │   ├── pad_strip.rs             # vertical pad list with waveforms
│   │   ├── sample_map.rs            # 2D scatter plot of samples (SPACE view)
│   │   ├── sequencer_grid.rs        # step sequencer with click/drag
│   │   ├── edit_view.rs             # per-pad editing (EDIT view, v0.2)
│   │   ├── controls.rs              # randomize, lock, undo/redo, transport
│   │   └── theme.rs                 # color palette, scaling
│   │
│   ├── logging.rs                   # ring-buffer debug logger
│   │
│   └── util/
│       ├── mod.rs
│       ├── audio_file.rs            # WAV/FLAC/OGG loading via symphonia
│       └── history.rs               # undo/redo stack for kit + pattern
│
└── tests/
    ├── engine_tests.rs
    ├── analysis_tests.rs
    └── integration_tests.rs
```

---

## Threading Model

Three threads with strict real-time safety boundaries.

### Audio Thread (~5ms deadline)
- Reads host transport (tempo, playing, position)
- Reads nih-plug Params (atomic, smoothed)
- Advances sequencer → triggers pads
- Mixes active voices → output buffer
- Sends MIDI output via `context.send_event()`
- Sends peak levels to GUI via `rtrb::Producer`
- **OWNS:** voice pool, playback positions, sequencer state
- **READS:** `Arc<Vec<f32>>` sample buffers (immutable, zero-copy)
- **NEVER:** allocates, locks, does file I/O, touches DB/ONNX
- **DEV GUARD:** `assert_no_alloc` catches violations in debug builds

### GUI Thread (~16ms frame budget)
- Reads Params for display
- Reads `rtrb::Consumer` for meters/playback state
- Draws pad strip, sample map, sequencer grid, controls
- On user action → writes Params via setter
- On kit change → sends command to background via crossbeam channel

### Background Thread (nih-plug BackgroundTask)
- Loads sample files (symphonia) → `Arc<Vec<f32>>`
- Resamples to host sample rate (rubato)
- Scans sample folders for available files
- v0.2: ONNX inference, SQLite queries
- When sample ready: wraps in `basedrop::Shared`, atomic CAS swap, old sample deferred to collector, 50ms crossfade on audio thread

### IPC Summary

| Path | Mechanism | Notes |
|------|-----------|-------|
| GUI ↔ Audio params | nih-plug Params (AtomicF32, CAS) | Wait-free, built-in |
| Audio → GUI telemetry | `rtrb` SPSC ring buffer | Peak meters, playback pos |
| GUI → Background commands | `crossbeam-channel` | Load requests, scan commands |
| Background → Audio samples | `basedrop::Shared` + atomic CAS | Deferred dealloc off audio thread |

---

## UI Design

### Layout: Three-panel, XO-inspired

**Default size:** 1100×680, scalable.

```
┌─────────────────────────────────────────────────────┐
│ AUTOKIT v0.1    [SPACE] [EDIT]    ⏵ Sync  🔄 Rescan │
├────────┬────────────────────────────────────────────┤
│        │                                            │
│  PAD   │         SAMPLE MAP (SPACE view)            │
│ STRIP  │    Color-coded dots clustered by type      │
│        │    Hover preview · Click to assign          │
│  K1 ── │    Zoom/pan · Category legend              │
│  K2 ── │                                            │
│  SN ── │                                            │
│  CL ── ├────────────────────────────────────────────┤
│  OH ── │                                            │
│  CH 🔒 │         SEQUENCER (16-step grid)           │
│  P1 ── │    One row per active pad · Click toggle   │
│  P2 ── │    Drag for velocity · Playhead synced     │
│        │                                            │
│ [DICE] ├────────────────────────────────────────────┤
│ [LOCK] │ Steps:16  Vel:80%  Roll:OFF    ⏵ Synced   │
│ [↩][↪] │                                            │
└────────┴────────────────────────────────────────────┘
```

### Sample Mode

**Oneshots only.** This is a drum machine — loops are not supported. During folder scanning, samples detected as loops (by filename heuristic like `loop`, `bpm`, or by length >2s with sustained amplitude) are filtered out or flagged as "other" and excluded from auto-assignment.

### Sample Categories

Expanded beyond basic 5 to cover the full range of sounds in a techno producer's library:

| Category | Color | Hex | Includes |
|----------|-------|-----|----------|
| Kick | Magenta | `#ff6b9d` | kicks, subs, booms, thuds |
| Snare | Cyan | `#4ecdc4` | snares, rimshots, crosssticks |
| Hihat | Bright orange | `#ff9f43` | closed hats, open hats, pedal hats |
| Clap | Mint green | `#a8e6cf` | claps, snaps, finger clicks |
| Tom | Coral | `#ff7675` | toms, rototoms, bongos, congas |
| Perc | Purple | `#c084fc` | shakers, tambourines, cowbells, woodblocks, misc perc |
| Cymbal | Gold | `#ffd166` | crashes, rides, splashes, chinas |
| Bass | Deep blue | `#74b9ff` | bass hits, bass stabs, 808 bass tones |
| Synth | Hot pink | `#fd79a8` | synth stabs, one-shot chords, bleeps, FX hits |
| Other | Grey | `#636e72` | unclassified, noise, textures, foley |

v0.1: categories assigned by hybrid DSP envelope analysis + subfolder name hints. v0.2: ONNX classifier refines accuracy.

### UI Colors

| Element | Color | Hex |
|---------|-------|-----|
| UI accent | Teal | `#00d4aa` |
| Background | Near-black | `#0a0a1a` |
| Panel bg | Dark navy | `#1a1a2e` |
| Sample map bg | Deep black | `#08080f` |

### SPACE/EDIT Toggle
- **SPACE** (default): sample map in top-right — 2D scatter of all scanned samples
- **EDIT**: per-pad controls (envelope, pitch, tone, FX sends) — v0.2

### Interactions
- **Pads:** click=audition, right-click=context menu, drag file onto=load, lock icon toggle
- **Sample map:** hover=preview filename+waveform tooltip, click=assign to selected pad, scroll=zoom, drag=pan
- **Sequencer:** click=toggle step, click+drag vertical=velocity, opacity reflects velocity
- **DICE:** randomize unlocked pads from scanned folder
- **LOCK:** toggle lock on selected pad (preserved during randomize)
- **↩/↪:** undo/redo randomization history

---

## Sample Classification (v0.1 — Hybrid DSP + Folder Hints)

**Oneshots only.** Loops filtered out by filename heuristic and duration/envelope analysis.

**DSP analysis** (runs on background thread during scan):
1. Amplitude envelope (RMS over 5ms windows) → attack time, decay time
2. Spectral centroid → frequency weight (low/mid/high)
3. Spectral flatness → noise vs tonal content

**Classification rules** (pure Rust, no ML):
- Fast attack (<10ms) + short decay = percussive
- Low centroid + percussive → Kick
- Mid centroid + noisy + percussive → Snare
- High centroid + very short → Hihat
- High centroid + longer decay → Cymbal
- Mid centroid + tonal + percussive → Tom
- Low centroid + tonal + longer sustain → Bass
- Tonal + non-percussive → Synth
- Remainder → Perc or Other

**Folder name hints** boost confidence (e.g. sample in "kicks/" biases toward Kick). Non-percussive samples still appear on map but dimmer.

## Sample Map

For v0.1, map positions use **DSP-classified categories** as cluster anchors. Each category gets a region, samples scatter within by spectral centroid (x) and decay time (y). Samples close in timbre appear close on the map.

For v0.2, positions come from UMAP/t-SNE embeddings computed by the ONNX analysis pipeline, giving true high-dimensional sonic similarity clustering.

---

## Sequencer

- 16 steps (expandable to 32/64 in future)
- Syncs to host transport (play/stop/tempo/position from Renoise)
- Per-step: on/off, velocity (0.0–1.0), roll (off/2x/4x)
- Global swing control (0–100%)
- Pattern randomization with density/complexity controls
- MIDI output: each pad triggers its assigned MIDI note when sequencer fires
- Internal audio: simultaneously plays the loaded sample

---

## Sample Management

- Configured sample folder(s) via Browse button (persisted in plugin state)
- **Rescan** button re-reads folders, lists .wav/.flac/.ogg files
- v0.1: metadata stored as JSON (serde) — paths, pad assignments, labels
- v0.2: SQLite cache with audio features, classification labels, UMAP coords
- **Future:** auto-rescan on startup if changes detected, with toggle to disable

---

## Debug Logging

Ring-buffer log at `~/.local/share/autokit/autokit.log`. ~10,000 lines max, overwrites oldest.

**Uses:** `tracing` crate with custom rolling file subscriber.

**Captures:**
- Sample load/swap events (path, duration, pad, success/failure)
- Audio thread underruns or missed deadlines
- Host transport state changes (play/stop, tempo)
- MIDI events in/out
- GUI command dispatches
- Errors with full context
- v0.2: ONNX inference timing

**Modes:** Always-on in debug builds, togglable via config in release.

---

## Dependencies (v0.1)

| Crate | Purpose |
|-------|---------|
| `nih_plug` | VST3 + CLAP plugin framework |
| `nih_plug_egui` | egui GUI integration |
| `egui` | Immediate-mode GUI |
| `symphonia` | Audio file decoding (WAV/FLAC/OGG) |
| `rubato` | Sample rate conversion |
| `basedrop` | RT-safe deferred memory deallocation |
| `rtrb` | Wait-free SPSC ring buffer (audio→GUI) |
| `crossbeam-channel` | GUI→background command channel |
| `serde` + `serde_json` | Config/state serialization |
| `tracing` + `tracing-subscriber` | Structured logging |
| `assert_no_alloc` | Dev-time RT safety guard |

### v0.2 additions
| `ort` | ONNX Runtime inference |
| `rusqlite` | SQLite analysis cache |
| `umap-rs` or custom | 2D embedding |

---

## Plugin Formats

- **VST3** (primary) — native in Renoise
- **CLAP** (bonus) — free via nih-plug, works in Bitwig/Reaper
- LV2 not targeted (Renoise doesn't support it)

---

## Testing Strategy

- **Unit tests:** sequencer logic, voice allocation, undo/redo history, audio file loading
- **Integration tests:** full plugin process() cycle with mock host context
- **Host testing:** Renoise 3.5.4 (primary), Ardour 9.2, Reaper 7.66
- **RT safety:** `assert_no_alloc` in debug, manual profiling with cargo-flamegraph
- **UI:** manual testing on Hyprland (Wayland) and X11

---

## Verification Plan

1. `cargo build --release` produces `.vst3` and `.clap` bundles
2. Load in Renoise → plugin UI opens without crash
3. Browse to `~/Music/Samples/` → samples appear in map
4. Click sample in map → assigns to pad, plays on click
5. DICE randomizes unlocked pads → undo restores previous kit
6. Sequencer plays in sync with Renoise transport
7. MIDI output triggers Renoise instruments
8. Check `~/.local/share/autokit/autokit.log` for structured debug output
9. No audio glitches during sample swap (crossfade)
10. UI responsive at 60fps with 1000+ samples in map

---

## License

GPLv3 — keeps the tool open-source and ensures derivative works stay open.

---

## Repo & Distribution

- GitHub: `github.com/<user>/autokit`
- AUR package for Arch Linux
- GitHub Releases with pre-built `.vst3` + `.clap` bundles
- Demo kits with freely-licensed samples
