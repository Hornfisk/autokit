# Per-Track LVL Knob Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an always-visible 16px inline LVL knob per track in both sequencer and pads views, wired to the existing `pad.volume` field.

**Architecture:** New `knob_inline()` widget (label-less, compact). Sequencer grid shifts right by 20px to make room — no cell size change. Pads collapsed row adds knob, waveform absorbs width. Expanded panel drops its VOL knob.

**Tech Stack:** Rust, nih-plug, egui (via nih_plug_egui)

---

### Task 1: Add `knob_inline()` to knob.rs

**Files:**
- Modify: `src/ui/knob.rs`

- [ ] **Step 1: Add the `knob_inline` function**

Add this function after the existing `knob()` function (after line 116):

```rust
/// Draw a compact inline knob — no label below, just the ring + value text.
/// Designed for use in track row headers where vertical space is tight.
pub fn knob_inline(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    tooltip: &str,
    format_value: impl Fn(f32) -> String,
    ring_color: egui::Color32,
    diameter: f32,
) -> KnobResponse {
    let mut result = KnobResponse {
        changed: false,
        reset: false,
    };

    let size = egui::vec2(diameter, diameter);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    response.clone().on_hover_text(tooltip);

    // Double-click detection (egui-baseview workaround)
    let dbl_id = id.with("last_press_inline");
    let pointer_pressed_on_knob = ui.input(|i| {
        i.events.iter().any(|e| matches!(
            e,
            egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Primary, .. }
        ))
    }) && response.contains_pointer();
    let is_double_click = if pointer_pressed_on_knob {
        let now = ui.input(|i| i.time);
        let prev: f64 = ui.data(|d| d.get_temp(dbl_id).unwrap_or(0.0));
        ui.data_mut(|d| d.insert_temp(dbl_id, now));
        now - prev < 0.4
    } else {
        false
    };

    if is_double_click || (response.clicked() && ui.input(|i| i.modifiers.ctrl)) {
        *value = default;
        result.changed = true;
        result.reset = true;
    }

    if response.dragged() && !is_double_click {
        let delta = -response.drag_delta().y;
        let speed = if ui.input(|i| i.modifiers.shift) { 0.001 } else { 0.005 };
        *value = (*value + delta * speed * (max - min)).clamp(min, max);
        result.changed = true;
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();
        let radius = diameter / 2.0 - 1.5;
        painter.circle_stroke(center, radius, egui::Stroke::new(1.5, ring_color));
        let text = format_value(*value);
        let font_size = if diameter < 20.0 { 7.0 } else { 8.0 };
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            &text,
            egui::FontId::new(font_size, egui::FontFamily::Monospace),
            ring_color,
        );
    }

    result
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: no errors (function is defined but not yet called)

- [ ] **Step 3: Commit**

```bash
git add src/ui/knob.rs
git commit -m "feat(ui): add knob_inline() compact knob widget"
```

---

### Task 2: Add LVL knob to sequencer view

**Files:**
- Modify: `src/ui/sequencer_ui.rs` (lines 42-49, 62-85, 207-210, 269-336)

- [ ] **Step 1: Add `volume` field to `LaneDisplay`**

In `src/ui/sequencer_ui.rs`, add a `volume` field to the `LaneDisplay` struct (after line 47, the `locked` field):

```rust
// Change from:
    pub locked: bool,
    pub steps: [StepDisplay; NUM_STEPS],

// Change to:
    pub locked: bool,
    pub volume: f32,
    pub steps: [StepDisplay; NUM_STEPS],
```

- [ ] **Step 2: Add `SetLaneVolume` to `SeqAction`**

In the `SeqAction` enum, add a new variant after `ToggleLaneLock` (after line 73):

```rust
// Change from:
    ToggleLaneLock { lane: usize },
    SelectPattern { index: usize },

// Change to:
    ToggleLaneLock { lane: usize },
    SetLaneVolume { lane: usize, volume: f32 },
    SelectPattern { index: usize },
```

- [ ] **Step 3: Increase `label_width` to accommodate knob**

In `draw_grid()`, update the label_width calculation (line 207):

```rust
// Change from:
    // Label area: 3px strip + 8px space + 46px tag + 4px space = 61px
    let label_width = 61.0;

// Change to:
    // Label area: 3px strip + 8px space + 46px tag + 4px space + 16px knob + 4px space = 81px
    let label_width = 81.0;
```

- [ ] **Step 4: Update the step numbers header offset**

The step numbers header at line 240-260 already uses `label_width + controls_width` for its left offset — no code change needed, it shifts automatically.

Verify by reading line 241: `ui.add_space(label_width + controls_width);`

- [ ] **Step 5: Add the LVL knob to each track row**

In the per-lane horizontal layout, after the tag badge and its 4px space (after line 304 `ui.add_space(4.0);`), insert the LVL knob before the M/S/L buttons:

```rust
// Change from:
            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 2.0;

            // Mute button

// Change to:
            ui.add_space(4.0);

            // LVL knob — inline, vertically centered
            {
                let mut vol = lane.volume;
                let knob_resp = ui.allocate_ui(Vec2::new(16.0, row_height), |ui| {
                    ui.centered_and_justified(|ui| {
                        knob::knob_inline(
                            ui,
                            egui::Id::new(("seq_lvl", lane_idx)),
                            &mut vol,
                            0.0, 1.0, 1.0,
                            "LVL",
                            |v| format!("{}", (v * 100.0) as u32),
                            cat_color,
                            16.0,
                        )
                    })
                });
                if knob_resp.inner.inner.changed {
                    actions.push(SeqAction::SetLaneVolume { lane: lane_idx, volume: vol });
                }
            }

            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 2.0;

            // Mute button
```

- [ ] **Step 6: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: error about missing `volume` field in `LaneDisplay` construction (in editor.rs) and unhandled `SetLaneVolume` in the match. That's expected — we wire it up in Task 3.

- [ ] **Step 7: Commit**

```bash
git add src/ui/sequencer_ui.rs
git commit -m "feat(seq): add per-lane LVL knob and SetLaneVolume action"
```

---

### Task 3: Wire sequencer LVL knob through editor.rs

**Files:**
- Modify: `src/ui/editor.rs` (lines 509-514, 551-573, 1092-1131)

- [ ] **Step 1: Add `volume` to `LaneDisplay` construction**

In editor.rs, where `LaneDisplay` is built (around line 509), add the volume field:

```rust
// Change from:
                                        crate::ui::sequencer_ui::LaneDisplay {
                                            pad_name: snap.pads[i].name.clone(),
                                            category: snap.pads[i].category,
                                            muted: lane.muted,
                                            solo: lane.solo,
                                            locked: snap.pads[i].locked,
                                            steps: core::array::from_fn(|j| crate::ui::sequencer_ui::StepDisplay {

// Change to:
                                        crate::ui::sequencer_ui::LaneDisplay {
                                            pad_name: snap.pads[i].name.clone(),
                                            category: snap.pads[i].category,
                                            muted: lane.muted,
                                            solo: lane.solo,
                                            locked: snap.pads[i].locked,
                                            volume: snap.pads[i].volume,
                                            steps: core::array::from_fn(|j| crate::ui::sequencer_ui::StepDisplay {
```

- [ ] **Step 2: Add `SeqSetLaneVolume` to `GuiAction` enum**

In the `GuiAction` enum (around line 1119), add the new variant after `SeqToggleLaneLock`:

```rust
// Change from:
    SeqToggleLaneLock { lane: usize },
    SeqSelectPattern { index: usize },

// Change to:
    SeqToggleLaneLock { lane: usize },
    SeqSetLaneVolume { lane: usize, volume: f32 },
    SeqSelectPattern { index: usize },
```

- [ ] **Step 3: Map `SeqAction::SetLaneVolume` to `GuiAction`**

In the seq action match block (around line 562), add the mapping after the `ToggleLaneLock` line:

```rust
// Change from:
                                        SeqAction::ToggleLaneLock { lane } => GuiAction::SeqToggleLaneLock { lane },
                                        SeqAction::SelectPattern { index } => GuiAction::SeqSelectPattern { index },

// Change to:
                                        SeqAction::ToggleLaneLock { lane } => GuiAction::SeqToggleLaneLock { lane },
                                        SeqAction::SetLaneVolume { lane, volume } => GuiAction::SeqSetLaneVolume { lane, volume },
                                        SeqAction::SelectPattern { index } => GuiAction::SeqSelectPattern { index },
```

- [ ] **Step 4: Handle `SeqSetLaneVolume` in the action dispatch**

Find the `GuiAction::SeqToggleLaneLock` handler and add the volume handler after it. Search for `SeqToggleLaneLock` in the match block (around line 870-880):

```rust
// Add after the SeqToggleLaneLock handler:
                    GuiAction::SeqSetLaneVolume { lane, volume } => {
                        shared.kit.pads[lane].volume = volume;
                    }
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles cleanly

- [ ] **Step 6: Commit**

```bash
git add src/ui/editor.rs
git commit -m "feat(editor): wire SeqSetLaneVolume action to pad volume"
```

---

### Task 4: Add LVL knob to pads collapsed row and remove expanded VOL

**Files:**
- Modify: `src/ui/pad_row.rs` (lines 34-46, 120-164, 237-288)

- [ ] **Step 1: Add LVL knob to collapsed row after tag badge**

In `draw_collapsed_from_snapshot`, after the tag badge allocation block (after line 119, the closing `});` of the tag `allocate_ui`), and after the existing `ui.add_space(4.0);` on line 121, add the knob before `ui.spacing_mut().item_spacing.x = 6.0;`:

```rust
// Change from:
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 6.0;

                // Play button (▶)

// Change to:
                ui.add_space(4.0);

                // LVL knob — inline, vertically centered
                {
                    let mut vol = volume;
                    let knob_resp = ui.allocate_ui(egui::vec2(16.0, row_height), |ui| {
                        ui.centered_and_justified(|ui| {
                            crate::ui::knob::knob_inline(
                                ui,
                                egui::Id::new(("pad_lvl", _index)),
                                &mut vol,
                                0.0, 1.0, 1.0,
                                "LVL",
                                |v| format!("{}", (v * 100.0) as u32),
                                cat_egui,
                                16.0,
                            )
                        })
                    });
                    if knob_resp.inner.inner.changed {
                        action = PadRowAction::SetVolume(vol);
                    }
                }

                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 6.0;

                // Play button (▶)
```

- [ ] **Step 2: Update `FIXED_W` to account for the knob**

Update the `FIXED_W` constant (line 164) to include the knob width:

```rust
// Change from:
                const FIXED_W: f32 = 3.0 + 8.0 + 46.0 + 6.0 + 24.0 + 146.0 + 52.0 + 52.0 + 6.0 + 12.0;

// Change to:
                // Added: 16px knob + 4px space = 20px
                const FIXED_W: f32 = 3.0 + 8.0 + 46.0 + 4.0 + 16.0 + 4.0 + 6.0 + 24.0 + 146.0 + 52.0 + 52.0 + 6.0 + 12.0;
```

- [ ] **Step 3: Remove VOL knob from expanded panel**

In `draw_expanded_from_snapshot` (starting line 237), remove the volume parameter from the function signature and the VOL knob rendering:

Change the function signature:

```rust
// Change from:
pub fn draw_expanded_from_snapshot(
    ui: &mut egui::Ui,
    index: usize,
    category: SampleCategory,
    volume: f32,
    pan: f32,
    pitch: f32,
    decay: f32,
) -> PadRowAction {

// Change to:
pub fn draw_expanded_from_snapshot(
    ui: &mut egui::Ui,
    index: usize,
    category: SampleCategory,
    pan: f32,
    pitch: f32,
    decay: f32,
) -> PadRowAction {
```

Remove the VOL knob block (lines 274-288):

```rust
// Delete this block entirely:
                    // Volume knob
                    let mut vol = volume;
                    let vol_result = knob::knob(
                        ui,
                        egui::Id::new(("vol", index)),
                        &mut vol,
                        0.0, 1.0, 1.0,
                        "VOL",
                        |v| format!("{}", (v * 100.0) as u32),
                        cat_egui,
                        34.0,
                    );
                    if vol_result.changed {
                        action = PadRowAction::SetVolume(vol);
                    }
```

- [ ] **Step 4: Verify it compiles (expect caller error)**

Run: `cargo check 2>&1 | tail -10`
Expected: error in editor.rs where `draw_expanded_from_snapshot` is called with the old signature. We fix that in the next step.

- [ ] **Step 5: Commit**

```bash
git add src/ui/pad_row.rs
git commit -m "feat(pads): add inline LVL knob to collapsed row, remove expanded VOL"
```

---

### Task 5: Update editor.rs call site for expanded panel

**Files:**
- Modify: `src/ui/editor.rs` (line 395-397)

- [ ] **Step 1: Remove `volume` from the expanded panel call**

Update the call to `draw_expanded_from_snapshot` in editor.rs:

```rust
// Change from:
                                            let detail_action = pad_row::draw_expanded_from_snapshot(
                                                ui, i, pad.category, pad.volume, pad.pan,
                                                pad.pitch, pad.decay,
                                            );

// Change to:
                                            let detail_action = pad_row::draw_expanded_from_snapshot(
                                                ui, i, pad.category, pad.pan,
                                                pad.pitch, pad.decay,
                                            );
```

- [ ] **Step 2: Handle `SetVolume` from collapsed row action**

The collapsed row can now return `PadRowAction::SetVolume`. Currently the match at line 374-391 has a `_ => {}` catch-all. Add explicit handling before the catch-all:

```rust
// Change from:
                                        PadRowAction::ToggleLock => {
                                            pending_actions.push(GuiAction::ToggleLock(i));
                                        }
                                        _ => {}

// Change to:
                                        PadRowAction::ToggleLock => {
                                            pending_actions.push(GuiAction::ToggleLock(i));
                                        }
                                        PadRowAction::SetVolume(v) => {
                                            pending_actions.push(GuiAction::SetPadVolume(i, v));
                                        }
                                        _ => {}
```

- [ ] **Step 3: Remove `SetVolume` from expanded panel match (now impossible)**

In the expanded panel action match (around line 400-418), the `SetVolume` arm is now dead code since the expanded panel can't return it. Remove it:

```rust
// Delete this arm:
                                                PadRowAction::SetVolume(v) => {
                                                    pending_actions.push(GuiAction::SetPadVolume(i, v));
                                                }
```

- [ ] **Step 4: Verify full build compiles**

Run: `cargo check 2>&1 | tail -5`
Expected: compiles cleanly, no warnings

- [ ] **Step 5: Full build**

Run: `cargo build --release 2>&1 | tail -5`
Expected: compiles successfully

- [ ] **Step 6: Commit**

```bash
git add src/ui/editor.rs
git commit -m "feat(editor): wire collapsed-row LVL knob, update expanded panel call"
```

---

### Task 6: Install and verify

- [ ] **Step 1: Install the VST3 bundle**

```bash
rm -rf ~/.vst3/Autokit.vst3 && cp -r target/release/bundle/vst3/Autokit.vst3 ~/.vst3/
```

- [ ] **Step 2: Verify in standalone or DAW**

Launch standalone or reload in Renoise. Check:
1. Both views show a 16px LVL knob after each category tag badge
2. Tags remain correctly sized (46×16px) and aligned — not offset
3. Dragging the LVL knob changes track volume audibly
4. Changing volume in pads view is reflected in sequencer view (and vice versa)
5. Step cell sizes in sequencer are unchanged
6. Expanded pads panel shows only PAN, PITCH, DECAY (no VOL)
7. Double-click on LVL knob resets to 100%

- [ ] **Step 3: Commit any fixes if needed**
