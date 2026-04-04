# Phase 7: Sample Map GUI — Design Spec

## Overview

A 2D scatter plot view of the entire sample library, toggled from the existing pad strip view. Each dot represents a sample, positioned by spectral centroid (X, brightness) and decay time (Y, length), colored by category. Users browse by clicking dots to preview sounds, then assign them to kit pads via a compact popup or keyboard shortcut.

## Goals

- Visual exploration of the full sample library (1716+ samples)
- Click-to-preview for rapid auditioning
- Effortless 1-click assignment to any of the 8 kit pads
- Current kit samples highlighted so users can see where their kit sits in the landscape
- Basic zoom/pan for navigating dense clusters
- Scalable architecture that handles large libraries (10k+ samples) with incremental optimization

## View Toggle

- Toolbar gains a MAP / PADS toggle (two clickable labels, active one styled with accent color)
- `ViewMode` enum: `PadStrip` | `SampleMap`
- Stored in `EditorState` (GUI-only, not shared with audio thread)
- Window size unchanged: 900x365
- Keyboard pad triggers (Z-M row) work in both views

## Layout (Map View)

```
┌─────────────────────────────────────────┐
│ AUTOKIT   [MAP] [PADS]     1716 samples │  ← toolbar
├─────────────────────────────────────────┤
│                                         │
│            scatter plot                 │
│         (egui Painter area)             │  ← ~290px tall
│                                         │
├─────────────────────────────────────────┤
│ [1 KCK][2 SNR][3 HH][4 CLP]...        │  ← mini pad bar (~28px)
└─────────────────────────────────────────┘
```

## Data Architecture

### MapPoint

Pre-computed when library scan completes. Stored in `EditorState` (GUI-only).

```rust
struct MapPoint {
    /// Normalized X coordinate (0.0–1.0), from spectral_centroid.
    nx: f32,
    /// Normalized Y coordinate (0.0–1.0), from decay_time.
    ny: f32,
    /// Index into the flat sample list for retrieval.
    library_index: usize,
    /// Category for coloring.
    category: SampleCategory,
    /// Filename for tooltip.
    name: String,
    /// Original feature values for tooltip display.
    centroid_hz: f32,
    decay_secs: f32,
}
```

### Axis Mapping

- **X axis**: `spectral_centroid` (Hz). Normalized via log scale: `log2(centroid / 100.0) / log2(200.0)` clamped to 0.0–1.0. Log scale because centroid spans ~50Hz to ~20kHz and perceptual brightness is logarithmic.
- **Y axis**: `decay_time` (seconds). Normalized linearly: `decay / 4.0` clamped to 0.0–1.0 (max sample duration is 4s). Short decays at top, long at bottom — so kicks cluster bottom-left, hats top-right.

### Coordinate System

- Normalized coords (0.0–1.0) represent the full data range
- View transform: `screen_pos = (normalized - pan_offset) * zoom * area_size + area_origin`
- Inverse for hit testing: `normalized = (screen_pos - area_origin) / (zoom * area_size) + pan_offset`

### No New Shared State

Map points are computed from the existing `SampleLibrary` and cached in GUI-only state. The audio thread is unaffected. Only existing `SharedState` fields are read (library, kit) via the snapshot pattern.

## Map Rendering

New file: `src/ui/sample_map.rs`

### Dot Rendering (egui Painter)

All dots painted via `painter.circle_filled()` and `painter.circle_stroke()`:

| Dot type | Radius | Color | Opacity | Extra |
|----------|--------|-------|---------|-------|
| Library (idle) | 3px | category color | 25% | — |
| Library (hovered) | 5px | category color | 70% | — |
| Library (shortcut mode, matching cat) | 4px | category color | 50% | — |
| Library (shortcut mode, other cat) | 3px | category color | 12% | — |
| Kit sample | 5px | category color | 100% | 2px white stroke + glow shadow |
| Clicked/selected | 6px | category color | 100% | 2px category stroke + outer glow |

### Axis Labels

Subtle text painted on the painter: "BRIGHTNESS →" centered below, "DECAY →" rotated on the left. Color: `TEXT_DIM` at 30% opacity.

### Zoom

- Scroll wheel adjusts zoom level: range 1.0 (fit all) to 8.0
- Zoom centers on cursor position (not viewport center)
- Zoom level displayed in toolbar if > 1.0

### Pan

- Click-drag on empty space (no dot within hit radius) pans the view
- Pan offset clamped so the data area doesn't scroll entirely off-screen
- Reset to default (zoom 1.0, pan 0,0) via double-click on empty space or toolbar button

## Hit Testing

- On mouse move: find nearest dot within 8px screen radius for hover tooltip (tooltip follows cursor, offset 15px right / 10px above)
- On click: find nearest dot within 8px for selection
- Algorithm: linear scan of all MapPoints, transform to screen coords, compute distance to cursor, keep nearest
- 1716 points: ~0.01ms per frame, no optimization needed
- Future scaling (10k+): add viewport culling (skip dots outside visible area) — the normalized coords + zoom/pan make this a simple bounds check

## Interaction States

### State 1: Browsing (default)

- Mouse over map: hover tooltip appears near cursor showing sample name, category, centroid Hz, decay seconds
- Tooltip positioned 15px right, 10px above cursor. Flips if near edge.
- All dots at normal opacity, kit samples highlighted with rings

### State 2: Clicked Dot → Preview + Popup

- Clicking a dot: plays a preview of the sample and shows an assignment popup
- **Preview playback**: sets `SharedState.preview_sample` (brief lock), audio thread plays it on a dedicated preview voice
- **Popup**: small floating `egui::Area`, no window frame
  - Shows: sample name (category color), stats line (category · centroid · decay)
  - 4x2 grid of pad buttons (1-8), each colored by that pad's current category
  - Pad matching the sample's category gets a subtle highlight/suggestion arrow
- **Popup placement**: 15px right, 10px above the clicked dot. Flips left/below if near edges. Never overlaps the clicked dot.
- **Dismiss**: click another dot (new popup opens), click empty space, press Escape
- **Keyboard assign**: while popup is open, keys 1-8 assign to that pad and dismiss
- Clicking another dot: dismisses current popup, previews new dot, opens new popup. Enables rapid click-through browsing without popups blocking nearby dots.

### State 3: Shortcut Mode (pad pre-selected)

- Activated by clicking a pad in the mini pad bar
- Toolbar shows: "Assigning to pad N: CAT"
- Map border tints to selected pad's category color
- Dots of matching category brighten (50%), others fade (12%)
- Clicking a dot: assigns directly to the pre-selected pad (no popup), plays preview, pushes history snapshot
- Click same pad in bar again or press Escape: exits shortcut mode
- After assignment: stays in shortcut mode so user can keep clicking to try different samples on that pad

## Assignment Action

Uses the existing `GuiAction` pattern:

```rust
GuiAction::AssignFromMap {
    pad_index: usize,
    library_index: usize,
}
```

Handler (in the Phase 2 brief lock):
1. Push history snapshot (undo support)
2. Look up `AnalyzedSample` by library index
3. Set pad's sample, sample_path, name, category
4. Update waveform summary for that pad
5. In shortcut mode: update the kit dot position on the map

## Preview Voice

- `SharedState` gains: `preview_sample: Option<Arc<Vec<f32>>>`
- Audio thread: on each buffer, check if `preview_sample` is Some. If so, play it on a dedicated preview playback position (a simple `f64` cursor + the `Arc<Vec<f32>>` data, separate from `VoicePool`). Clear the `preview_sample` trigger after starting. The preview state lives alongside VoicePool in the Autokit struct, not inside it.
- Preview plays at unity gain, center pan, no pitch/decay adjustment
- If a new preview is triggered while one is playing, the old one fades out (50ms) and the new one starts

## Mini Pad Bar

- 8 buttons in a horizontal `ui.horizontal()` below the map area
- Each button: pad number + 3-letter category abbreviation, colored background matching category
- Shows current sample name on hover (tooltip)
- Click: toggle shortcut mode for that pad
- Active pad (shortcut mode): bright border + glow, bolder text
- After assignment from map: brief flash animation on the assigned pad (reuse existing brightness mechanism)

## DisplaySnapshot Extension

Add to `DisplaySnapshot`:

```rust
map_ready: bool,          // true when library is loaded
preview_playing: bool,    // true when preview voice is active (for visual feedback)
```

Map points themselves are NOT in the snapshot — they're cached in EditorState and recomputed only when the library changes.

## New Files

- `src/ui/sample_map.rs` — map rendering, hit testing, zoom/pan, popup, mini pad bar

## Modified Files

- `src/ui/editor.rs` — `ViewMode` enum, map state fields in `EditorState`, view toggle routing, `GuiAction::AssignFromMap`, preview trigger
- `src/ui/toolbar.rs` — MAP/PADS toggle buttons, zoom indicator, shortcut mode status text
- `src/ui/state.rs` — `preview_sample` field on `SharedState`
- `src/plugin.rs` — preview voice handling in audio process loop
- `src/analysis/library.rs` — `all_samples_flat()` method that returns a `Vec<&AnalyzedSample>` by iterating all categories in deterministic order (sorted by category discriminant). The index into this flat vec is `library_index` used by `MapPoint`. Also add `sample_by_flat_index()` to retrieve a sample by that index.

## Unchanged Files

kit.rs, sampler.rs, sequencer.rs, history.rs, theme.rs (category colors already defined), cache.rs, preset.rs, scanner.rs, features.rs

## Testing

- `sample_map.rs` unit tests:
  - Coordinate normalization: known centroid/decay values produce expected normalized coords
  - Hit testing: nearest-dot lookup returns correct index for known positions
  - Zoom transform: screen↔normalized round-trip preserves coordinates
- Integration: map points count matches library total after scan
- Manual: verify dot positions visually cluster by category (kicks bottom-left, hats top-right)
