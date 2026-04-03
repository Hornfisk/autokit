# Phase 6: Pad Strip GUI — Design Spec

## Overview

Add an egui-based GUI to the Autokit plugin. The GUI displays all 16 drum pads as a vertical scrollable list with inline waveform previews, per-pad controls, and a global toolbar. The visual identity is neon-on-dark with category-colored accents and a monospace font for a futurist rave aesthetic.

## Window

- **Size:** 900x700 pixels, fixed (not resizable).
- **Scale selector:** dropdown in the toolbar offering 100% / 125% / 150%. Stored in plugin state. Applied via `egui::Context::set_pixels_per_point()`.
- **Framework:** `nih_plug_egui` — the `editor()` method on the Plugin trait returns an `egui::Editor`.

## Architecture: GUI ↔ Audio Thread Communication

The plugin currently owns `DrumKit`, `History`, `SampleLibrary`, and `Sequencer` directly in the `Autokit` struct, which lives on the audio thread. The GUI needs read/write access to kit state and read access to the library.

### Shared State

Introduce `SharedState`, a struct wrapped in `Arc<Mutex<>>` (using `parking_lot::Mutex`):

```rust
pub struct SharedState {
    pub kit: DrumKit,
    pub library: Option<SampleLibrary>,
    pub history: History,
    pub scan_status: ScanStatus,
}

pub enum ScanStatus {
    Scanning,
    Ready { total: usize },
}
```

Both the audio thread (`process()`) and the GUI thread (`update()`) lock this mutex. Locks are brief:
- Audio thread: reads pad sample data + volume/pan for voice rendering, checks `bg_rx` for scan completion.
- GUI thread: reads pad state for display, writes volume/pan/pitch changes, triggers dice/undo/redo.

### What stays on the audio thread only

- `VoicePool` — only touched by `process()`, no GUI interaction needed.
- `Sequencer` — Phase 8 (sequencer grid GUI) will share it later. For now it stays on the audio thread.
- `AutokitParams` (nih-plug params) — master volume remains a host-automatable param. The GUI reads it via `self.params.master_volume.value()` for display and can set it via the param's setter.

### Waveform cache

Computing waveform display data (downsampled min/max pairs) from `Arc<Vec<f32>>` sample data on every frame is wasteful. Pre-compute waveform summaries:

- `WaveformSummary`: a `Vec<[f32; 2]>` of min/max pairs, one per display column (200 points at default width).
- Computed once when a pad's sample changes (on scan complete, dice, undo/redo).
- Stored alongside pad data in a parallel `[Option<WaveformSummary>; 16]` array inside `SharedState`.
- The GUI reads these directly for line plot rendering.

## Layout

### Toolbar (top bar, ~40px height)

Left section:
- **AUTOKIT** logo text — teal (#00d4aa), bold, letter-spacing 3px.
- Version label — dim grey, 9px.
- Scan status — "Scanning..." (animated) or "1716 samples" (teal, 9px).

Center section:
- **UNDO** / **REDO** buttons — ghost style, grey. Disabled (dimmer) when history is empty in that direction.
- Divider.
- **DICE ALL** button — teal accent. Pushes history snapshot, calls `kit.dice_all(&library)`, invalidates waveform cache for changed pads.
- **LOCK ALL** toggle — ghost style. Toggles lock on all 16 pads. Label changes to "UNLOCK ALL" when all are locked.

Right section:
- **MASTER** label + horizontal slider + dB value. Reads/writes `AutokitParams::master_volume`.
- **Scale dropdown** — 100% / 125% / 150%.

### Pad List (fills remaining height, scrollable if needed at 150% scale)

16 pad rows, each in one of two states:

#### Collapsed row (~38px height)

From left to right:
1. **Color strip** — 3px wide vertical bar in the pad's category color.
2. **Category tag** — filled rectangle with category color background, dark text. 8px bold, 46px wide. E.g. "KICK", "SNARE".
3. **Sample name** — 11px, light grey (#aaa), fixed 170px width, ellipsis overflow.
4. **Waveform** — line plot using pre-computed min/max summary. Stroke color = category color at 50% opacity for unselected, 85% for selected. Flex-grows to fill available width.
5. **Volume bar** — 50px wide, 3px tall. Fill color = category color at 40% opacity.
6. **Dice button** — 32px wide, die icon (⚄). Teal on hover. Quick-dice: pushes snapshot, calls `kit.dice_pad(index, &library)`.

Clicking anywhere on the row (except the dice button) toggles the expanded detail. Only one row can be expanded at a time.

#### Expanded detail (~54px height, appears below the collapsed row)

Left border: 3px strip in category color at 20% opacity (visual continuation).

Contents, vertically centered:
1. **Knob group** — three circular knobs side by side:
   - **VOL** — 0–100 (maps to 0.0–1.0). Ring and value in category color.
   - **PAN** — L100–C–R100 (maps to -1.0–1.0). Ring in category color at 50%.
   - **PITCH** — -24 to +24 semitones. Ring in category color at 50%.
   - Each knob: 34px diameter circle, 2px border, value text centered inside, label below (7px, dim grey, letter-spacing 1.5px).
   - Interaction: vertical drag to change value. Ctrl+click to reset to default.
2. **Divider** — 1px vertical line, very dim.
3. **LOCK checkbox** — 12px box + "LOCK" label. When locked, box filled with teal, label brighter.
4. **DICE PAD** button — teal accent style. Pushes snapshot, dices just this pad.
5. **DICE [CATEGORY]** button — category color accent. E.g. "DICE KICKS". Pushes snapshot, calls `kit.dice_category()`.

## Visual Style

### Colors

Use existing `theme.rs` palette. Key additions:
- `BG_MAIN: #0a0a1a` — plugin background (already defined as `BG_COLOR`)
- `BG_TOOLBAR: #0e0e20` — toolbar background
- `BG_ROW: #111126` — collapsed pad row
- `BG_ROW_HOVER: #16162e` — row on hover/selected
- `BG_DETAIL: #0d0d22` — expanded detail area
- `ACCENT: #00d4aa` — teal, interactive elements (already defined)
- `TEXT_PRIMARY: #cccccc`
- `TEXT_DIM: #636e72` at various opacities

### Typography

- **Font:** JetBrains Mono (bundled as an embedded font via `egui::FontData`). Fallback to egui's default monospace.
- **Sizes:** Logo 15px, buttons/labels 8-9px, pad names 11px, knob values 9px, knob labels 7px.
- **Weight:** Bold for logo and category tags. Regular/light for everything else.
- **Letter-spacing:** Wide on labels (1-3px), normal on sample names.

### Glow effects

- Volume bar fill has a subtle `box-shadow`-equivalent glow in egui (painted as a second wider rect at low opacity behind the fill).
- Selected/expanded row has slightly brighter waveform opacity (0.85 vs 0.5).
- No other glow — keep it clean.

## Waveform Rendering

- Line plot drawn with `egui::Painter::line()` using the pre-computed summary.
- Summary: for each display column, compute min and max of the corresponding sample range. Draw a polyline through the midpoints (or min/max pairs for thicker waveforms — but for Phase 6, single midpoint line is sufficient).
- 200 points per waveform at default width. Recompute count if row width changes due to scaling.
- Stroke: 1.2px, category color, opacity varies by selection state.
- Empty pad (no sample loaded): show a flat line at center or a dim "—" placeholder.

## Knob Rendering

Custom-painted egui widget:
- Circle outline (2px stroke) as the "ring."
- Value text centered inside.
- Drag interaction: capture vertical mouse drag, map delta to value change. Shift+drag for fine control.
- Ctrl+click: reset to default (volume=1.0, pan=0.0, pitch=0.0).
- No arc/notch indicator for v1 — the text value is sufficient.

## State Persistence

GUI-specific state that needs saving/restoring with the plugin:
- **Selected pad index** (which row is expanded) — cosmetic, no need to persist.
- **Scale factor** — persist in plugin state (serde).
- Kit state, history, etc. already handled by existing snapshot system.

## Interactions Summary

| Action | Effect |
|--------|--------|
| Click pad row | Toggle expand/collapse (auto-collapse previous) |
| Drag knob vertically | Change VOL/PAN/PITCH |
| Ctrl+click knob | Reset to default |
| Click row dice (⚄) | Push snapshot, dice that pad |
| Click DICE PAD | Push snapshot, dice selected pad |
| Click DICE [CAT] | Push snapshot, dice all unlocked pads of that category |
| Click DICE ALL | Push snapshot, dice all unlocked pads |
| Click UNDO/REDO | Restore kit + sequencer from history |
| Click LOCK checkbox | Toggle pad lock (not undoable) |
| Click LOCK ALL | Toggle lock on all 16 pads |
| Drag MASTER slider | Set master volume param |
| Click scale dropdown | Change UI scale (100/125/150%) |

## File Structure

New/modified files:

| File | Purpose |
|------|---------|
| `src/ui/editor.rs` | `create_editor()` function returning `nih_plug_egui::EguiEditor`. Main `update()` loop. |
| `src/ui/toolbar.rs` | Toolbar rendering function. |
| `src/ui/pad_row.rs` | Collapsed + expanded pad row rendering. |
| `src/ui/knob.rs` | Custom knob widget (paint + drag interaction). |
| `src/ui/waveform.rs` | Waveform summary computation + line plot painter. |
| `src/ui/theme.rs` | Extended with new color constants and font setup. |
| `src/ui/state.rs` | `SharedState` struct definition + `WaveformCache`. |
| `src/plugin.rs` | Modified: move kit/library/history into `Arc<Mutex<SharedState>>`, add `editor()` method. |
| `src/lib.rs` | Add new `ui::` submodules. |

## Scope Boundaries

**In scope (Phase 6):**
- All 16 pad rows with waveform, controls, dice, lock.
- Global toolbar with undo/redo, dice-all, lock-all, master volume, scale.
- Scanning status indicator.
- Custom knob widget.

**Out of scope (later phases):**
- Sample map scatter plot (Phase 7).
- Sequencer grid (Phase 8).
- Drag-and-drop sample loading.
- Folder picker for sample library root.
- Per-pad MIDI note display/editing.
- Audio preview on pad click (requires triggering voices from GUI thread).
- Pad reordering.

## egui Considerations

- `nih_plug_egui` provides `EguiState` for window size and an `update()` callback that receives `&egui::Context` and access to plugin params/state.
- The `create_egui_editor()` function receives `Arc<Mutex<SharedState>>` and `Arc<AutokitParams>` via closure capture.
- egui's immediate mode means the full UI is rebuilt every frame. Keep the per-frame work light: read shared state, paint, handle interactions, write back changes.
- Font embedding: load JetBrains Mono `.ttf` via `include_bytes!()` in the editor setup, register with `ctx.fonts()`.
