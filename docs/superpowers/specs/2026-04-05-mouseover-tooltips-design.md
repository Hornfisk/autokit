# Mouseover Tooltips — Design Spec

**Date:** 2026-04-05
**Scope:** Add toggleable mouseover tooltips that explain the GUI to new users.

## Overview

A `[?]` toggle button in the toolbar enables/disables short functional tooltips on all interactive GUI elements. Tooltips prioritize non-obvious controls (acronyms like CMP, DRV, LIM) and operations that aren't self-explanatory.

## Toggle Mechanism

- New field `tooltips_on: bool` in `EditorState`, default `true`
- `[?]` button in the toolbar, always visible, positioned in the left section after the view tabs
- When active: ACCENT color + ACCENT_DIM background (matches active tab style)
- When inactive: TEXT_DIM color + BG_ROW background
- Clicking toggles `tooltips_on`

## Implementation

### Helper function (in `ui/theme.rs` or a new `ui/tips.rs`)

```rust
/// Conditionally attach a tooltip to a response.
pub fn tip(response: egui::Response, text: &str, on: bool) -> egui::Response {
    if on {
        response.on_hover_text(text)
    } else {
        response
    }
}
```

### Threading `tooltips_on` through draw functions

The bool is passed as a parameter to each draw function that needs it:
- `toolbar::draw_toolbar_snapshot()` — already has many params, one more is fine
- `pad_row::draw_collapsed_from_snapshot()` / `draw_expanded_from_snapshot()`
- `sequencer_ui::draw_sequencer()`
- `sample_map::draw_map()`
- `knob::knob_inline()` — already has a `tooltip` param; gate it on the bool

### Existing `.on_hover_text()` calls

Migrate all existing calls to use `tip()` so they respect the toggle:
- `toolbar.rs:167` — "Click to change sample folder"
- `toolbar.rs:307` — "Load preset"
- `toolbar.rs:327` — "Save preset"
- `pad_row.rs:172` — sample name hover
- `pad_row.rs:210` — "Randomize pad"
- `pad_row.rs:236` — "Unlock pad" / "Lock pad"
- `sample_map.rs:345` — sample info on hover
- `knob.rs:140` — knob tooltip

## Tooltip Text

### Toolbar (right-to-left layout)
| Element | Tooltip |
|---------|---------|
| VOL knob | "Master volume (dB). Ctrl+click to reset" |
| LIM button | "Master limiter on/off" |
| DRV knob | "Saturation drive. Ctrl+click to reset" |
| CMP knob | "Master compressor threshold (dB). Ctrl+click to reset" |
| S button | "Save preset" (existing) |
| L button | "Load preset" (existing) |
| LOCK ALL | "Lock/unlock all pads (locked pads keep their sample on dice)" |
| DICE ALL | "Randomize all unlocked pads" |
| UNDO / REDO | "Undo last change" / "Redo last undone change" |
| PADS / MAP / SEQ tabs | "Pad strip view" / "Sample map scatter plot" / "Step sequencer" |
| BPM drag | "Tempo (standalone only)" |
| `[?]` button | "Toggle help tooltips" (always shown, not gated) |
| Sample count | "Click to change sample folder" (existing) |

### Pad Row (collapsed)
| Element | Tooltip |
|---------|---------|
| Category tag | "Sample category (detected by spectral analysis)" |
| Sample name | Full sample name (existing, for truncated names) |
| Waveform | "Click to expand pad details" |
| LVL knob | "Pad volume. Ctrl+click to reset" |
| Dice button | "Randomize this pad" |
| Lock button | "Lock pad (keep sample on dice)" / "Unlock pad" |

### Pad Row (expanded)
| Element | Tooltip |
|---------|---------|
| VOL knob | "Pad volume (dB). Ctrl+click to reset" |
| PAN knob | "Stereo panning. Ctrl+click to center" |
| PITCH knob | "Pitch shift (semitones). Ctrl+click to reset" |
| DECAY knob | "Amplitude decay. Ctrl+click to reset" |
| Dice category | "Randomize within this category only" |

### Sequencer
| Element | Tooltip |
|---------|---------|
| Step cell | "Click to toggle. Drag up/down to set velocity" |
| Mute (M) | "Mute this lane" |
| Solo (S) | "Solo this lane" |
| Pattern slot | "Click to switch pattern. Right-click to copy/paste" |
| FILL button | "Hold to trigger fill steps" |
| Swing knob | "Swing amount. Shifts even steps late" |
| EXT button | "External transport sync (follow host)" |
| P-lock dot | "Parameter lock active on this step" |

### Sample Map
| Element | Tooltip |
|---------|---------|
| Dot (existing) | Sample info (already implemented) |

## Files Modified

1. **`src/ui/editor.rs`** — add `tooltips_on` to `EditorState`, add `[?]` button or pass to toolbar, thread bool to all draw calls
2. **`src/ui/toolbar.rs`** — add `tooltips_on` param, add `[?]` button, wrap existing tooltips with `tip()`
3. **`src/ui/pad_row.rs`** — add `tooltips_on` param, add new tooltips, wrap existing with `tip()`
4. **`src/ui/knob.rs`** — add `tooltips_on` param to `knob_inline()`, gate the existing tooltip
5. **`src/ui/sequencer_ui.rs`** — add `tooltips_on` param, add tooltips to step cells, mute/solo, pattern slots
6. **`src/ui/sample_map.rs`** — add `tooltips_on` param, gate existing sample info tooltip
7. **`src/ui/theme.rs`** — add `tip()` helper function

## Non-goals

- No rich/multi-line tooltips or tutorial mode
- No tooltip styling customization (uses egui default tooltip appearance)
- No persistence of the toggle across sessions (resets to `true` each launch)
- No delay/animation customization
