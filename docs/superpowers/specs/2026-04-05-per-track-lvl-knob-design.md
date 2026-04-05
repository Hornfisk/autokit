# Per-Track LVL Knob

## Context

Autokit has 8 drum pads, each with a `pad.volume` field (0.0–1.0) already in the gain chain (`sample × velocity × pad.volume × master`). The pads view exposes this as a VOL knob, but only in the expanded detail panel — it's hidden in collapsed view. The sequencer view has no per-track volume control at all. Users need quick, always-visible volume adjustment per track in both views with consistent behavior and aesthetics.

## Design

### Shared inline knob

A new `knob_inline()` function in `src/ui/knob.rs` renders a compact circular knob without the label-below layout used by the existing `knob()`. It takes the same parameters (value, min, max, default, format_value, ring_color) plus a fixed diameter.

- **Diameter:** 16px — matches the category tag badge height (46×16px)
- **Value text:** Centered inside the ring, 7pt monospace (scaled down from the 9pt used at 34px)
- **Ring stroke:** 1.5px (scaled down from 2px)
- **Ring color:** Category color (`cat_egui`) — matches the adjacent tag badge
- **Label:** None rendered below; "LVL" shown as hover tooltip
- **Interaction:** Same as existing knob — vertical drag (0.005 speed, shift for 0.001 fine), double-click/ctrl+click to reset to default (1.0)
- **Value range:** 0.0–1.0, displayed as 0–100 integer
- **Default:** 1.0 (100%)

### Sequencer view (`src/ui/sequencer_ui.rs`)

**Position:** After the category tag badge + 4px space, before M/S/L buttons.

Row layout becomes: `[3px strip][8px][46px tag][4px][16px LVL knob][4px][M S L 45px][step cells×16]`

**Layout changes:**
- `label_width` increases from 61px to ~81px (adds 16px knob + 4px space)
- `controls_width` unchanged at 45px
- Step cell size calculation: `cell_from_width` decreases by 20px, but cells remain the same size because `cell_size = min(row_from_height, cell_from_width)` is height-constrained. The 20px is absorbed by the existing empty space to the right of the grid.
- Step numbers header and bottom control bar already offset by `label_width + controls_width`, so they shift right automatically.
- Row height: unchanged.

**New action:** `SeqAction::SetLaneVolume { lane: usize, volume: f32 }` — dispatched when the knob is dragged. Handler writes to `shared.kit.pads[lane].volume` and pushes an undo snapshot.

**Knob allocation:** Rendered inside the existing `ui.horizontal` per-lane block, between the tag and M/S/L buttons. The knob is vertically centered via `allocate_ui(Vec2::new(16.0, row_height), ...)` wrapping it in a centered layout, same pattern as the tag badge.

### Pads collapsed row (`src/ui/pad_row.rs`)

**Position:** After the category tag badge + 4px space, before the ▶ play button.

Row layout becomes: `[3px strip][8px][46px tag][4px][16px LVL knob][4px][▶][name][waveform][DICE][LOCK]`

**Layout changes:**
- `FIXED_W` increases by ~20px (16px knob + 4px space)
- Waveform area absorbs the 20px — it's the flexible element
- Row height: unchanged

**Action:** `PadRowAction::SetVolume(f32)` already exists and is reused.

**Knob allocation:** Same vertical-centering pattern as in sequencer. The `draw_collapsed_from_snapshot` function already receives `volume: f32` — the knob reads and writes that value.

### Pads expanded panel (`src/ui/pad_row.rs`)

**Remove the VOL knob.** The expanded panel becomes: `[strip][12px][PAN knob][PITCH knob][DECAY knob][separator][DICE category button]`

The `draw_expanded_from_snapshot` function drops the VOL knob call and its `SetVolume` dispatch. Volume is now controlled exclusively via the inline knob in the collapsed row header (which remains visible above the expanded panel).

### Header consistency

Both views use identical header sizing:
- Strip: 3px
- Space: 8px  
- Tag badge: 46×16px
- Space: 4px
- LVL knob: 16px diameter, vertically centered in row
- Space: 4px

This shared prefix ensures the tag badges and LVL knobs align at exactly the same horizontal positions in both views.

## Files to modify

| File | Change |
|------|--------|
| `src/ui/knob.rs` | Add `knob_inline()` function |
| `src/ui/sequencer_ui.rs` | Add LVL knob after tag, increase `label_width`, add `SeqAction::SetLaneVolume` |
| `src/ui/pad_row.rs` | Add LVL knob to collapsed row, remove VOL from expanded panel, update `FIXED_W` |
| `src/ui/editor.rs` | Handle `SeqAction::SetLaneVolume` in the action dispatch |

## Verification

1. `cargo build` — must compile without warnings
2. Load in Renoise (or standalone) — verify both views show the LVL knob after each tag badge
3. Drag the LVL knob in sequencer view — verify the volume change is audible and persists across pattern switches
4. Drag the LVL knob in pads view — verify same behavior
5. Change volume in one view, switch to the other — verify the value is reflected (both read `pad.volume`)
6. Verify expanded pads panel no longer shows VOL knob, only PAN/PITCH/DECAY
7. Verify step cell sizes are identical to before the change (compare at same window size)
8. Verify tag badges remain correctly sized and aligned (46×16px, not offset)
9. Double-click the LVL knob — verify it resets to 100%
10. Save/load preset — verify volumes persist (already handled by kit serialization)
