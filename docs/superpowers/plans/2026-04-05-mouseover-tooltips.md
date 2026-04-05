# Mouseover Tooltips Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add toggleable mouseover tooltips to all interactive GUI elements so new users can learn the interface.

**Architecture:** A `tooltips_on: bool` field in `EditorState` (default `true`) is toggled by a `[?]` button in the toolbar. A `tip()` helper conditionally attaches `.on_hover_text()`. The bool is threaded through all draw functions as a parameter.

**Tech Stack:** Rust, egui (via nih_plug_egui)

---

### Task 1: Add `tip()` helper and `tooltips_on` state

**Files:**
- Modify: `src/ui/theme.rs` (add `tip()` at end of file)
- Modify: `src/ui/editor.rs:105-161` (add field + default)

- [ ] **Step 1: Add `tip()` helper to theme.rs**

Append to the end of `src/ui/theme.rs`:

```rust
/// Conditionally attach a tooltip to a response.
/// When `on` is false, returns the response unchanged.
pub fn tip(response: egui::Response, text: &str, on: bool) -> egui::Response {
    if on {
        response.on_hover_text(text)
    } else {
        response
    }
}
```

- [ ] **Step 2: Add `tooltips_on` field to `EditorState`**

In `src/ui/editor.rs`, add to the `EditorState` struct (after line 137, before the closing `}`):

```rust
    /// Whether help tooltips are shown on hover.
    pub tooltips_on: bool,
```

In the `Default` impl (after `logo_texture: None,` on line 158):

```rust
            tooltips_on: true,
```

- [ ] **Step 3: Compile check**

Run: `cargo check 2>&1 | head -20`
Expected: no errors (new field + function are unused but that's fine)

- [ ] **Step 4: Commit**

```bash
git add src/ui/theme.rs src/ui/editor.rs
git commit -m "feat(tooltips): add tip() helper and tooltips_on state"
```

---

### Task 2: Add `[?]` toggle button to toolbar

**Files:**
- Modify: `src/ui/toolbar.rs:15-26` (add `ToggleTooltips` to `ToolbarAction`)
- Modify: `src/ui/toolbar.rs:38-53` (add `tooltips_on` param)
- Modify: `src/ui/toolbar.rs:79-103` (add button after view tabs)
- Modify: `src/ui/editor.rs:318-333` (pass new param, handle new action)

- [ ] **Step 1: Add `ToggleTooltips` variant to `ToolbarAction`**

In `src/ui/toolbar.rs`, add to the `ToolbarAction` enum (after `SetView(ViewMode),` on line 24):

```rust
    ToggleTooltips,
```

- [ ] **Step 2: Add `tooltips_on` parameter to `draw_toolbar_snapshot`**

In `src/ui/toolbar.rs`, change the function signature at line 38 to add `tooltips_on: bool` as the last parameter (before the closing `)`)— i.e. after `standalone_tempo: &AtomicU32,`:

```rust
    tooltips_on: bool,
```

- [ ] **Step 3: Add `[?]` button after view tabs**

In `src/ui/toolbar.rs`, after the view tab block's closing `}` (line 103), insert:

```rust
                // Help tooltips toggle
                {
                    let tip_color = if tooltips_on { theme::ACCENT } else { theme::TEXT_DIM };
                    let tip_bg = if tooltips_on { theme::ACCENT_DIM } else { theme::BG_ROW };
                    let tip_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new("?")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(tip_color)
                                .strong(),
                        )
                        .fill(tip_bg)
                        .min_size(egui::vec2(22.0, 22.0)),
                    );
                    // This tooltip is always shown (not gated) so users can discover the toggle
                    tip_btn.on_hover_text("Toggle help tooltips");
                    if tip_btn.clicked() {
                        action = ToolbarAction::ToggleTooltips;
                    }
                }
```

- [ ] **Step 4: Update the call site in editor.rs**

In `src/ui/editor.rs`, the `draw_toolbar_snapshot` call (~line 318). Add `state.tooltips_on,` as the last argument (after `&seq_standalone_tempo,`):

```rust
                    let toolbar_action = toolbar::draw_toolbar_snapshot(
                        ui,
                        &snap.scan_status,
                        snap.can_undo,
                        snap.can_redo,
                        all_locked,
                        &params,
                        setter,
                        state.view_mode,
                        shortcut_info,
                        logo,
                        snap.scan_processed,
                        snap.scan_total,
                        is_standalone,
                        &seq_standalone_tempo,
                        state.tooltips_on,
                    );
```

- [ ] **Step 5: Handle `ToggleTooltips` action in editor.rs**

In `src/ui/editor.rs`, in the `match toolbar_action` block (~line 335), add before `ToolbarAction::None => {}`:

```rust
                        ToolbarAction::ToggleTooltips => {
                            state.tooltips_on = !state.tooltips_on;
                        }
```

- [ ] **Step 6: Compile check**

Run: `cargo check 2>&1 | head -20`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add src/ui/toolbar.rs src/ui/editor.rs
git commit -m "feat(tooltips): add [?] toggle button to toolbar"
```

---

### Task 3: Add tooltips to toolbar controls

**Files:**
- Modify: `src/ui/toolbar.rs` (wrap existing + add new tooltips)

All changes are in `src/ui/toolbar.rs`. Use `theme::tip()` — add `use crate::ui::theme;` is already imported.

- [ ] **Step 1: Wrap existing toolbar tooltips with `tip()`**

Replace line 167:
```rust
                            .on_hover_text("Click to change sample folder")
```
with:
```rust
                            ;
                        theme::tip(response_tmp, "Click to change sample folder", tooltips_on);
```

Actually, the existing code chains `.on_hover_text()` on the `if` expression. The cleanest approach: capture the response, call `tip()`, then check `.clicked()`. But that restructures the code. Instead, just replace each `.on_hover_text("...")` with a let-binding pattern. Here's the concrete approach for each:

**Sample count button (line 167-168):** The `.on_hover_text("Click to change sample folder")` is chained after the button `.clicked()` check. Replace:
```rust
                        .on_hover_text("Click to change sample folder")
```
with:
```rust
                        ;
                        // tip handled below after response capture
```

This is getting complex with the existing chaining. Simpler approach — since `tip()` returns a `Response` and the original code only reads it for `.clicked()`, we can do the tip call on a separate line after the response is captured. Let's use a practical approach:

For each existing `.on_hover_text()` in toolbar.rs, replace it with a call to `theme::tip()`. Since the existing code uses method chaining, capture the response first.

**Line 167** — sample count hover text. Currently:
```rust
                        .on_hover_text("Click to change sample folder")
                        .clicked()
```
Replace with:
```rust
                        .clicked()
```
Then after the if-block that uses this response (the one ending on line 171), add:
```rust
                        // (tip is on the response from ui.add above — but since we consumed it in the if, we skip here; the button text "N samples" is self-explanatory when tooltips are off)
```

Actually, looking more carefully at the code structure: the `.on_hover_text()` is called on the response from the `if ui.add(...)` chain. In egui, `on_hover_text` returns a `Response` but the `.clicked()` is the important part. The tooltip side-effect happens during `on_hover_text()` regardless of the return value. So we can just gate it:

The simplest, least-invasive pattern: after each existing `response.on_hover_text("...")` or `.on_hover_text("...")`, change it to use an if-guard. But since Rust doesn't allow conditional method chaining easily, we need to restructure slightly.

**Practical approach for all toolbar tooltips:** Extract each button response into a named variable, call `tip()` on it, then check `.clicked()`.

Here are the exact changes:

**Sample count button (~line 156-171):** Replace the whole block:
```rust
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(format!("{total} samples"))
                                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                        .color(theme::ACCENT),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .frame(false),
                            )
                            .on_hover_text("Click to change sample folder")
                            .clicked()
                        {
                            action = ToolbarAction::OpenSetup;
                        }
```
with:
```rust
                        let resp = ui.add(
                            egui::Button::new(
                                egui::RichText::new(format!("{total} samples"))
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(theme::ACCENT),
                            )
                            .fill(egui::Color32::TRANSPARENT)
                            .frame(false),
                        );
                        theme::tip(resp.clone(), "Click to change sample folder", tooltips_on);
                        if resp.clicked() {
                            action = ToolbarAction::OpenSetup;
                        }
```

**Load button (~line 296-311):** Replace:
```rust
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("L")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(load_color)
                                    .strong(),
                            )
                            .fill(load_dim)
                            .min_size(egui::vec2(22.0, 22.0)),
                        )
                        .on_hover_text("Load preset")
                        .clicked()
                    {
                        action = ToolbarAction::OpenLoadDialog;
                    }
```
with:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("L")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(load_color)
                                .strong(),
                        )
                        .fill(load_dim)
                        .min_size(egui::vec2(22.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Load preset", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::OpenLoadDialog;
                    }
```

**Save button (~line 314-332):** Same pattern — extract response, `tip()`, then `.clicked()`:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("S")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(save_color)
                                .strong(),
                        )
                        .fill(save_dim)
                        .min_size(egui::vec2(22.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Save preset", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::OpenSaveDialog;
                    }
```

- [ ] **Step 2: Add new tooltips to toolbar controls that don't have them**

After each button's `.clicked()` check, add a `theme::tip()` call. These are all in the right-to-left section of the toolbar:

**VOL knob (~line 203):** After the `if resp.changed {` block for master volume, add:
```rust
                        // tip is already on the knob_inline tooltip param — will be gated in Task 5
```
(The knob_inline already has a tooltip param — we'll gate it in Task 5.)

**LIM button (~line 240):** After the `.clicked()` if-block, add:
```rust
                    theme::tip(resp.clone(), "Master limiter on/off", tooltips_on);
```
Note: need to capture the response first. Change the `if ui.add(...).clicked()` to:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("LIM")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(lim_color)
                                .strong(),
                        )
                        .fill(lim_bg)
                        .min_size(egui::vec2(32.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Master limiter on/off", tooltips_on);
                    if resp.clicked() {
                        setter.begin_set_parameter(&params.limiter_on);
                        setter.set_parameter(&params.limiter_on, !lim_val);
                        setter.end_set_parameter(&params.limiter_on);
                    }
```

**LOCK ALL button (~line 336-349):** Capture response, add tip:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new(lock_label)
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DIM),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(60.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Lock/unlock all pads (locked pads keep their sample on dice)", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::LockAll;
                    }
```

**DICE ALL button (~line 352-367):** Same pattern:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("DICE ALL")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(theme::ACCENT)
                                .strong(),
                        )
                        .fill(theme::ACCENT_DIM)
                        .min_size(egui::vec2(60.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Randomize all unlocked pads", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::DiceAll;
                    }
```

**UNDO button (~line 389-405):** Capture response, add tip:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("UNDO")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(undo_color),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(44.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Undo last change", tooltips_on);
                    if resp.clicked() && can_undo {
                        action = ToolbarAction::Undo;
                    }
```

**REDO button (~line 372-388):** Same:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("REDO")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(redo_color),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(44.0, 22.0)),
                    );
                    theme::tip(resp.clone(), "Redo last undone change", tooltips_on);
                    if resp.clicked() && can_redo {
                        action = ToolbarAction::Redo;
                    }
```

**View tabs (~line 94-102):** Add tips after each tab button:
```rust
                    let resp = ui.add(tab("PADS", ViewMode::PadStrip));
                    theme::tip(resp.clone(), "Pad strip view", tooltips_on);
                    if resp.clicked() && view_mode != ViewMode::PadStrip {
                        action = ToolbarAction::SetView(ViewMode::PadStrip);
                    }
                    let resp = ui.add(tab("MAP", ViewMode::SampleMap));
                    theme::tip(resp.clone(), "Sample map scatter plot", tooltips_on);
                    if resp.clicked() && view_mode != ViewMode::SampleMap {
                        action = ToolbarAction::SetView(ViewMode::SampleMap);
                    }
                    let resp = ui.add(tab("SEQ", ViewMode::Sequencer));
                    theme::tip(resp.clone(), "Step sequencer", tooltips_on);
                    if resp.clicked() && view_mode != ViewMode::Sequencer {
                        action = ToolbarAction::SetView(ViewMode::Sequencer);
                    }
```

**BPM drag (~line 190):** After the drag value, add:
```rust
                    let resp = ui.add_sized(egui::vec2(72.0, 22.0), drag);
                    theme::tip(resp.clone(), "Tempo (BPM)", tooltips_on);
                    if resp.changed() {
                        standalone_tempo.store((bpm * 10.0) as u32, Ordering::Relaxed);
                    }
```

- [ ] **Step 3: Compile check**

Run: `cargo check 2>&1 | head -20`
Expected: compiles with no errors

- [ ] **Step 4: Commit**

```bash
git add src/ui/toolbar.rs
git commit -m "feat(tooltips): add tooltips to all toolbar controls"
```

---

### Task 4: Add tooltips to pad row controls

**Files:**
- Modify: `src/ui/pad_row.rs:34-46` (add `tooltips_on` param to `draw_collapsed_from_snapshot`)
- Modify: `src/ui/pad_row.rs:254-262` (add `tooltips_on` param to `draw_expanded_from_snapshot`)
- Modify: `src/ui/pad_row.rs` (wrap existing tooltips, add new ones)
- Modify: `src/ui/editor.rs:435-438` (pass `tooltips_on` to collapsed)
- Modify: `src/ui/editor.rs:465-468` (pass `tooltips_on` to expanded)

- [ ] **Step 1: Add `tooltips_on` param to both pad row draw functions**

In `src/ui/pad_row.rs`, add `tooltips_on: bool,` as the last parameter to `draw_collapsed_from_snapshot` (after `row_height: f32,` on line 46):

```rust
    tooltips_on: bool,
```

Same for `draw_expanded_from_snapshot` (after line 262, the closing paren has `decay: f32,`):

```rust
    tooltips_on: bool,
```

- [ ] **Step 2: Gate existing tooltips in collapsed row**

In `draw_collapsed_from_snapshot`:

**Sample name hover (line 171-172):** Replace:
```rust
                if has_sample && name.chars().count() > 20 {
                    name_response.clone().on_hover_text(name);
                }
```
with:
```rust
                if has_sample && name.chars().count() > 20 {
                    theme::tip(name_response.clone(), name, tooltips_on);
                }
```

**Dice button (line 210):** Replace:
```rust
                dice_response.on_hover_text("Randomize pad");
```
with:
```rust
                theme::tip(dice_response, "Randomize this pad", tooltips_on);
```

**Lock button (line 236):** Replace:
```rust
                lock_response.on_hover_text(if locked { "Unlock pad" } else { "Lock pad" });
```
with:
```rust
                theme::tip(lock_response, if locked { "Unlock pad (sample will change on dice)" } else { "Lock pad (keep sample on dice)" }, tooltips_on);
```

- [ ] **Step 3: Add new tooltips to collapsed row**

**Play button (after line 152):** Add:
```rust
                theme::tip(play_response, "Preview this pad (or press keyboard key)", tooltips_on);
```

**Category tag:** The tag is painted directly on a rect, not via a button widget, so there's no `Response` to attach a tooltip to. Skip — the tag label is self-explanatory.

- [ ] **Step 4: Add tooltips to expanded row**

In `draw_expanded_from_snapshot`, after the dice category button block (~line 361, after `.clicked()`):
```rust
                    // Tooltip for dice category button
                    // (The knob tooltips are handled through knob::knob — see Task 5)
```

Actually, the `knob::knob` function doesn't have a `tooltip` parameter (only `knob_inline` does). The expanded row uses `knob::knob` which shows a label below. We should add tooltips by calling `tip()` on the enclosing response. But `knob::knob` is called inside a `ui.vertical()` and doesn't return a usable response for hovering.

Simpler: leave the expanded row knobs without tooltips — they already have visible labels (PAN, PITCH, DECAY) and the format_value closures show the current value. The abbreviations are clear enough for an expanded detail panel.

For the dice category button, capture the response:
```rust
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new(&cat_label)
                                .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                                .color(cat_egui),
                        )
                        .fill(cat_color.to_egui_alpha(0x11))
                        .corner_radius(3),
                    );
                    theme::tip(resp.clone(), "Randomize within this category only", tooltips_on);
                    if resp.clicked() {
                        action = PadRowAction::DiceCategory;
                    }
```

- [ ] **Step 5: Update call sites in editor.rs**

In `src/ui/editor.rs`, the `draw_collapsed_from_snapshot` call (~line 435-439), add `state.tooltips_on,` as last arg:

```rust
                                        let row_action = pad_row::draw_collapsed_from_snapshot(
                                            ui, i, pad.has_sample, &pad.name, pad.category,
                                            pad.volume, wf, is_selected, state.brightness[i],
                                            pad.locked, pad_row_height, state.tooltips_on,
                                        );
```

The `draw_expanded_from_snapshot` call (~line 465-468), add `state.tooltips_on,`:

```rust
                                            let detail_action = pad_row::draw_expanded_from_snapshot(
                                                ui, i, pad.category, pad.pan,
                                                pad.pitch, pad.decay, state.tooltips_on,
                                            );
```

- [ ] **Step 6: Compile check**

Run: `cargo check 2>&1 | head -20`
Expected: compiles with no errors

- [ ] **Step 7: Commit**

```bash
git add src/ui/pad_row.rs src/ui/editor.rs
git commit -m "feat(tooltips): add tooltips to pad row controls"
```

---

### Task 5: Gate knob_inline tooltip on tooltips_on

**Files:**
- Modify: `src/ui/knob.rs:120-140` (add `tooltips_on` param)
- Modify: `src/ui/pad_row.rs` (pass `tooltips_on` to knob_inline calls)
- Modify: `src/ui/toolbar.rs` (pass `tooltips_on` to knob_inline calls)
- Modify: `src/ui/sequencer_ui.rs` (pass `tooltips_on` to knob_inline calls)

- [ ] **Step 1: Add `tooltips_on` param to `knob_inline`**

In `src/ui/knob.rs`, change the `knob_inline` signature (line 120) to add `tooltips_on: bool,` after `diameter: f32,`:

```rust
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
    tooltips_on: bool,
) -> KnobResponse {
```

Replace line 140:
```rust
    response.clone().on_hover_text(tooltip);
```
with:
```rust
    crate::ui::theme::tip(response.clone(), tooltip, tooltips_on);
```

- [ ] **Step 2: Update all knob_inline call sites**

There are 4 call sites for `knob_inline`:

1. **`src/ui/pad_row.rs` collapsed row (~line 121-130):** Add `tooltips_on,` after `16.0,`:
```rust
                            crate::ui::knob::knob_inline(
                                ui,
                                egui::Id::new(("pad_lvl", _index)),
                                &mut vol,
                                0.0, 1.0, 1.0,
                                "Pad volume",
                                |v| format!("{}", (v * 100.0) as u32),
                                cat_egui,
                                16.0,
                                tooltips_on,
                            )
```
Note: also change the tooltip text from `"LVL"` to `"Pad volume"` for clarity.

2. **`src/ui/toolbar.rs` master volume knob (~line 203):** Add `tooltips_on,` after `20.0,`:
```rust
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("master_vol_knob"),
                            &mut gain_db, -60.0, 6.0, 0.0,
                            "Master volume (dB). Double-click to reset",
                            |v| format!("{v:.1}"),
                            theme::ACCENT, 20.0,
                            tooltips_on,
                        );
```

3. **`src/ui/toolbar.rs` drive knob (~line 250):** Add `tooltips_on,` after `20.0,`:
```rust
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("comp_drive_knob"),
                            &mut drive, 0.0, 1.0, 0.0,
                            "Saturation drive. Double-click to reset",
                            |v| format!("{:.0}%", v * 100.0),
                            theme::TEXT_DIM, 20.0,
                            tooltips_on,
                        );
```

4. **`src/ui/toolbar.rs` compressor threshold knob (~line 272):** Add `tooltips_on,` after `20.0,`:
```rust
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("comp_threshold_knob"),
                            &mut thr, -40.0, 0.0, -12.0,
                            "Master compressor threshold (dB). Double-click to reset",
                            |v| format!("{:.0}", v),
                            theme::ACCENT, 20.0,
                            tooltips_on,
                        );
```

5. **`src/ui/sequencer_ui.rs` lane LVL knob (~line 313):** Add `tooltips_on,` after `16.0,`. But `draw_grid` doesn't have `tooltips_on` yet — this will be handled in Task 6.

For now, just update the 4 call sites in `pad_row.rs` and `toolbar.rs`.

- [ ] **Step 3: Compile check**

Run: `cargo check 2>&1 | head -30`
Expected: error about the sequencer_ui call site missing the new param — that's expected, will fix in Task 6.

- [ ] **Step 4: Temporarily add `false` to sequencer_ui knob_inline call**

In `src/ui/sequencer_ui.rs` (~line 313), add `false,` after `16.0,` as a placeholder:

```rust
                        knob::knob_inline(
                            ui,
                            egui::Id::new(("seq_lvl", lane_idx)),
                            &mut vol,
                            0.0, 1.0, 1.0,
                            "Lane volume",
                            |v| format!("{}", (v * 100.0) as u32),
                            cat_color,
                            16.0,
                            false, // will be replaced with tooltips_on in Task 6
                        )
```
Also update the tooltip text from `"LVL"` to `"Lane volume"`.

- [ ] **Step 5: Compile check**

Run: `cargo check 2>&1 | head -20`
Expected: compiles with no errors

- [ ] **Step 6: Commit**

```bash
git add src/ui/knob.rs src/ui/pad_row.rs src/ui/toolbar.rs src/ui/sequencer_ui.rs
git commit -m "feat(tooltips): gate knob_inline tooltips on tooltips_on flag"
```

---

### Task 6: Add tooltips to sequencer controls

**Files:**
- Modify: `src/ui/sequencer_ui.rs:92-97` (add `tooltips_on` param to `draw_sequencer_view`)
- Modify: `src/ui/sequencer_ui.rs:203-208` (add `tooltips_on` param to `draw_grid`)
- Modify: `src/ui/sequencer_ui.rs:572-578` (add `tooltips_on` param to `draw_param_controls`)
- Modify: `src/ui/sequencer_ui.rs:711` (add `tooltips_on` param to `draw_bottom_bar`)
- Modify: `src/ui/editor.rs:616-617` (pass `tooltips_on`)

- [ ] **Step 1: Thread `tooltips_on` through all sequencer functions**

Add `tooltips_on: bool` as the last param to each function:

`draw_sequencer_view` (line 92):
```rust
pub fn draw_sequencer_view(
    ui: &mut egui::Ui,
    display: &SeqDisplay,
    view_state: &mut SeqViewState,
    available_height: f32,
    tooltips_on: bool,
) -> Vec<SeqAction> {
```

Update its internal calls to pass `tooltips_on` through:
- Line 101: `draw_grid(ui, display, available_height, view_state)` → `draw_grid(ui, display, available_height, view_state, tooltips_on)`
- Line 116: `draw_param_controls(ui, lane_idx, step_idx, step, cat)` ��� `draw_param_controls(ui, lane_idx, step_idx, step, cat, tooltips_on)`
- Line 189: `draw_bottom_bar(ui, display)` → `draw_bottom_bar(ui, display, tooltips_on)`

`draw_grid` (line 203):
```rust
fn draw_grid(
    ui: &mut egui::Ui,
    display: &SeqDisplay,
    available_height: f32,
    view_state: &mut SeqViewState,
    tooltips_on: bool,
) -> (Vec<SeqAction>, GridLayout) {
```

`draw_param_controls` (line 572):
```rust
fn draw_param_controls(
    ui: &mut egui::Ui,
    lane_idx: usize,
    step_idx: usize,
    step: &StepDisplay,
    category: SampleCategory,
    tooltips_on: bool,
) -> Vec<SeqAction> {
```

`draw_bottom_bar` (line 711):
```rust
fn draw_bottom_bar(ui: &mut egui::Ui, display: &SeqDisplay, tooltips_on: bool) -> Option<SeqAction> {
```

- [ ] **Step 2: Replace the `false` placeholder in draw_grid's knob_inline call**

In `draw_grid` (~line 313), replace `false,` with `tooltips_on,`:
```rust
                            tooltips_on,
```

- [ ] **Step 3: Add tooltips to sequencer grid controls**

In `draw_grid`:

**Mute button (~line 339):** Capture response, add tip:
```rust
            let resp = ui.add(mute_btn);
            theme::tip(resp.clone(), "Mute this lane", tooltips_on);
            if resp.clicked() {
                actions.push(SeqAction::ToggleLaneMute { lane: lane_idx });
            }
```

**Solo button (~line 349):** Same pattern:
```rust
            let resp = ui.add(solo_btn);
            theme::tip(resp.clone(), "Solo this lane", tooltips_on);
            if resp.clicked() {
                actions.push(SeqAction::ToggleLaneSolo { lane: lane_idx });
            }
```

**Lock button (~line 359):** Same:
```rust
            let resp = ui.add(lock_btn);
            theme::tip(resp.clone(), "Lock pad (keep sample on dice)", tooltips_on);
            if resp.clicked() {
                actions.push(SeqAction::ToggleLaneLock { lane: lane_idx });
            }
```

- [ ] **Step 4: Add tooltips to bottom bar controls**

In `draw_bottom_bar`:

**Play/Stop button (~line 728):** Capture response:
```rust
        let resp = ui.add(
            egui::Button::new(
                egui::RichText::new(play_label).font(FontId::monospace(10.0)).color(play_color).strong(),
            )
            .fill(play_bg)
            .min_size(Vec2::new(60.0, 22.0))
            .corner_radius(3.0),
        );
        theme::tip(resp.clone(), "Play/stop sequencer (Space)", tooltips_on);
        if resp.clicked() {
            action = Some(SeqAction::ToggleInternalPlay);
        }
```

**DICE button (~line 757):** Add after `ui.add(dice_btn)`:
```rust
        let resp = ui.add(dice_btn);
        theme::tip(resp.clone(), "Randomize pattern steps", tooltips_on);
        if resp.clicked() {
            action = Some(SeqAction::DicePattern);
        }
```

**FILL button (~line 773):** Add after `ui.add(fill_btn)`:
```rust
        let fill_resp = ui.add(fill_btn);
        theme::tip(fill_resp.clone(), "Hold to trigger fill steps", tooltips_on);
```

**EXPORT button (~line 816):** Capture:
```rust
            let resp = ui.add(export_btn);
            theme::tip(resp.clone(), "Export pattern as MIDI file", tooltips_on);
            if resp.clicked() {
                action = Some(SeqAction::ExportMidi);
            }
```

**COPY/PASTE/CLEAR/SAVE/LOAD buttons (~line 792-806):** Add tips:
```rust
            let resp = ui.add(btn_style("CLEAR"));
            theme::tip(resp.clone(), "Clear all steps in pattern", tooltips_on);
            if resp.clicked() {
                action = Some(SeqAction::ClearPattern);
            }
            let resp = ui.add(btn_style("PASTE"));
            theme::tip(resp.clone(), "Paste copied pattern", tooltips_on);
            if resp.clicked() {
                action = Some(SeqAction::PastePattern);
            }
            let resp = ui.add(btn_style("COPY"));
            theme::tip(resp.clone(), "Copy current pattern", tooltips_on);
            if resp.clicked() {
                action = Some(SeqAction::CopyPattern);
            }
            let resp = ui.add(btn_style("LOAD"));
            theme::tip(resp.clone(), "Load saved pattern", tooltips_on);
            if resp.clicked() {
                action = Some(SeqAction::OpenLoadPatternDialog);
            }
            let resp = ui.add(btn_style("SAVE"));
            theme::tip(resp.clone(), "Save current pattern", tooltips_on);
            if resp.clicked() {
                action = Some(SeqAction::OpenSavePatternDialog);
            }
```

- [ ] **Step 5: Add tooltips to pattern selector**

In `draw_sequencer_view`, the pattern button response (~line 162):
```rust
            let response = ui.add(btn);
            theme::tip(response.clone(), &format!("Pattern {:02}", i + 1), tooltips_on);
```

The PTRN label doesn't need a tooltip — it's self-explanatory.

- [ ] **Step 6: Update call site in editor.rs**

In `src/ui/editor.rs` (~line 616), change:
```rust
                                for seq_action in crate::ui::sequencer_ui::draw_sequencer_view(
                                    ui, &seq_display, &mut state.seq_view, shared_avail_h,
                                ) {
```
to:
```rust
                                for seq_action in crate::ui::sequencer_ui::draw_sequencer_view(
                                    ui, &seq_display, &mut state.seq_view, shared_avail_h, state.tooltips_on,
                                ) {
```

- [ ] **Step 7: Add `use crate::ui::theme;` to sequencer_ui.rs if not present**

Check — it's already imported at line 5: `use crate::ui::theme;`. Good, no change needed.

- [ ] **Step 8: Compile check**

Run: `cargo check 2>&1 | head -20`
Expected: compiles with no errors

- [ ] **Step 9: Commit**

```bash
git add src/ui/sequencer_ui.rs src/ui/editor.rs
git commit -m "feat(tooltips): add tooltips to sequencer controls"
```

---

### Task 7: Gate sample map tooltips

**Files:**
- Modify: `src/ui/sample_map.rs:139-147` (add `tooltips_on` param to `draw_map`)
- Modify: `src/ui/sample_map.rs:345` (gate existing tooltip)
- Modify: `src/ui/editor.rs:508-516` (pass `tooltips_on`)

- [ ] **Step 1: Add `tooltips_on` param to `draw_map`**

In `src/ui/sample_map.rs`, add `tooltips_on: bool,` as the last parameter to `draw_map` (after `shortcut_category: Option<SampleCategory>,`):

```rust
pub fn draw_map(
    ui: &mut egui::Ui,
    points: &[MapPoint],
    view: &mut MapViewState,
    kit_paths: &[Option<String>],
    hovered_index: &mut Option<usize>,
    shortcut_pad: Option<usize>,
    shortcut_category: Option<SampleCategory>,
    tooltips_on: bool,
) -> MapAction {
```

- [ ] **Step 2: Gate the existing tooltip**

Find the `on_hover_text_at_pointer` call (~line 345) and wrap it:
```rust
                if tooltips_on {
                    response.on_hover_text_at_pointer(
                        // ... existing text ...
                    );
                }
```

- [ ] **Step 3: Update call site in editor.rs**

In `src/ui/editor.rs` (~line 508), add `state.tooltips_on,` as the last argument:
```rust
                            let map_action = sample_map::draw_map(
                                ui,
                                &state.map_points,
                                &mut state.map_view,
                                &kit_paths,
                                &mut state.map_hovered,
                                state.map_shortcut_pad,
                                shortcut_category,
                                state.tooltips_on,
                            );
```

- [ ] **Step 4: Compile and build**

Run: `cargo build 2>&1 | tail -5`
Expected: compiles successfully

- [ ] **Step 5: Commit**

```bash
git add src/ui/sample_map.rs src/ui/editor.rs
git commit -m "feat(tooltips): gate sample map tooltips on toggle"
```

---

### Task 8: Final build verification

**Files:** None (verification only)

- [ ] **Step 1: Full release build**

Run: `cargo build --release 2>&1 | tail -10`
Expected: compiles with no errors

- [ ] **Step 2: Check for warnings**

Run: `cargo build --release 2>&1 | grep -i warning | head -20`
Expected: no new warnings (existing warnings are OK)

- [ ] **Step 3: Final commit (if any cleanup needed)**

If there are warnings to fix, fix them and commit:
```bash
git add -u
git commit -m "chore: fix warnings from tooltip changes"
```
