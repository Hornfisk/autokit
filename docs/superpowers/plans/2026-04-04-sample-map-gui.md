# Sample Map GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a 2D scatter plot view of the sample library with click-to-preview and pad assignment.

**Architecture:** New `src/ui/sample_map.rs` handles all map rendering, hit testing, and interaction. Existing `editor.rs` gains a `ViewMode` toggle to switch between pad strip and map. Preview playback uses a lightweight cursor in `plugin.rs`, separate from `VoicePool`. Library gains flat-index access methods.

**Tech Stack:** egui Painter API for dot rendering, existing nih_plug + parking_lot for audio thread preview.

---

### Task 1: Library Flat Index Methods

**Files:**
- Modify: `src/analysis/library.rs`
- Test: `src/analysis/library.rs` (inline `#[cfg(test)]` module)

- [ ] **Step 1: Write failing tests for flat index access**

Add to the existing `#[cfg(test)]` module at the bottom of `src/analysis/library.rs`:

```rust
#[test]
fn all_samples_flat_returns_all_samples() {
    let lib = test_library();
    let flat = lib.all_samples_flat();
    assert_eq!(flat.len(), lib.total);
}

#[test]
fn all_samples_flat_deterministic_order() {
    let lib = test_library();
    let flat1 = lib.all_samples_flat();
    let flat2 = lib.all_samples_flat();
    for (a, b) in flat1.iter().zip(flat2.iter()) {
        assert_eq!(a.entry.filename, b.entry.filename);
    }
}

#[test]
fn sample_by_flat_index_round_trips() {
    let lib = test_library();
    let flat = lib.all_samples_flat();
    for (i, sample) in flat.iter().enumerate() {
        let retrieved = lib.sample_by_flat_index(i).expect("should exist");
        assert_eq!(retrieved.entry.filename, sample.entry.filename);
    }
}

#[test]
fn sample_by_flat_index_out_of_bounds() {
    let lib = test_library();
    assert!(lib.sample_by_flat_index(999999).is_none());
}
```

Note: `test_library()` already exists in the test module — it creates a library with one sample per category.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo test --lib -- analysis::library::tests::all_samples_flat -v 2>&1 | tail -5`

Expected: compilation error — `all_samples_flat` and `sample_by_flat_index` don't exist.

- [ ] **Step 3: Implement flat index methods**

Add these methods to `impl SampleLibrary` in `src/analysis/library.rs`, after the existing `clone_for_dice` method:

```rust
/// Return all samples in a deterministic flat order (sorted by category discriminant).
/// The index into this vec is `library_index` used by MapPoint.
pub fn all_samples_flat(&self) -> Vec<&AnalyzedSample> {
    let mut categories: Vec<SampleCategory> = self.by_category.keys().copied().collect();
    categories.sort_by_key(|c| *c as u8);
    let mut flat = Vec::with_capacity(self.total);
    for cat in categories {
        if let Some(samples) = self.by_category.get(&cat) {
            flat.extend(samples.iter());
        }
    }
    flat
}

/// Retrieve a sample by its flat index (as returned by `all_samples_flat`).
/// Returns None if index is out of bounds.
pub fn sample_by_flat_index(&self, index: usize) -> Option<&AnalyzedSample> {
    let mut categories: Vec<SampleCategory> = self.by_category.keys().copied().collect();
    categories.sort_by_key(|c| *c as u8);
    let mut offset = 0;
    for cat in categories {
        if let Some(samples) = self.by_category.get(&cat) {
            if index < offset + samples.len() {
                return Some(&samples[index - offset]);
            }
            offset += samples.len();
        }
    }
    None
}
```

This requires `SampleCategory` to derive `Ord`/`PartialOrd` or be castable to `u8`. Check: `SampleCategory` already derives `Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize` and has a `repr` that lets us cast to `u8`. If not, add `#[repr(u8)]` to the enum in `src/engine/kit.rs`. The enum already lists variants in a stable order (Kick=0..Other=9) so `*c as u8` gives deterministic sort.

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo test --lib -- analysis::library::tests -v 2>&1 | tail -20`

Expected: all library tests pass, including the 4 new ones.

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/analysis/library.rs src/engine/kit.rs
git commit -m "feat(library): add flat index access for sample map"
```

---

### Task 2: MapPoint and Coordinate Normalization

**Files:**
- Create: `src/ui/sample_map.rs`
- Modify: `src/lib.rs` (add module)
- Modify: `src/main.rs` (add module)

- [ ] **Step 1: Write failing tests for coordinate normalization**

Create `src/ui/sample_map.rs` with the test module first:

```rust
use crate::engine::kit::SampleCategory;

/// A single point on the sample map.
#[derive(Clone, Debug)]
pub struct MapPoint {
    /// Normalized X (0.0–1.0), from spectral centroid (log scale).
    pub nx: f32,
    /// Normalized Y (0.0–1.0), from decay time (linear, 0=short/top, 1=long/bottom).
    pub ny: f32,
    /// Index into SampleLibrary::all_samples_flat() for retrieval.
    pub library_index: usize,
    /// Category for coloring.
    pub category: SampleCategory,
    /// Filename for tooltip.
    pub name: String,
    /// Original centroid in Hz.
    pub centroid_hz: f32,
    /// Original decay in seconds.
    pub decay_secs: f32,
}

/// Normalize spectral centroid to 0.0–1.0 via log scale.
/// Maps ~100Hz → 0.0, ~20kHz → 1.0.
fn normalize_centroid(hz: f32) -> f32 {
    if hz <= 0.0 {
        return 0.0;
    }
    ((hz / 100.0).log2() / (200.0_f32).log2()).clamp(0.0, 1.0)
}

/// Normalize decay time to 0.0–1.0 (linear, max 4s).
fn normalize_decay(secs: f32) -> f32 {
    (secs / 4.0).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centroid_100hz_maps_to_zero() {
        let n = normalize_centroid(100.0);
        assert!(n.abs() < 0.01, "100Hz should map near 0.0, got {n}");
    }

    #[test]
    fn centroid_20khz_maps_to_one() {
        let n = normalize_centroid(20000.0);
        assert!((n - 1.0).abs() < 0.05, "20kHz should map near 1.0, got {n}");
    }

    #[test]
    fn centroid_1khz_maps_mid() {
        let n = normalize_centroid(1000.0);
        assert!(n > 0.3 && n < 0.6, "1kHz should be mid-range, got {n}");
    }

    #[test]
    fn centroid_zero_maps_to_zero() {
        assert_eq!(normalize_centroid(0.0), 0.0);
    }

    #[test]
    fn decay_zero_maps_to_zero() {
        assert_eq!(normalize_decay(0.0), 0.0);
    }

    #[test]
    fn decay_4s_maps_to_one() {
        assert_eq!(normalize_decay(4.0), 1.0);
    }

    #[test]
    fn decay_clamps_above_4s() {
        assert_eq!(normalize_decay(10.0), 1.0);
    }

    #[test]
    fn decay_2s_maps_to_half() {
        assert!((normalize_decay(2.0) - 0.5).abs() < 0.01);
    }
}
```

- [ ] **Step 2: Register module in lib.rs and main.rs**

In `src/lib.rs`, add `pub mod sample_map;` inside the `mod ui` block (after `toolbar`):

```rust
mod ui {
    pub mod editor;
    pub mod knob;
    pub mod pad_row;
    pub mod sample_map;
    pub mod state;
    pub mod theme;
    pub mod toolbar;
    pub mod waveform;
}
```

Apply the identical change to `src/main.rs`.

- [ ] **Step 3: Run tests to verify they pass**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo test --lib -- ui::sample_map::tests -v 2>&1 | tail -20`

Expected: all 8 normalization tests pass.

- [ ] **Step 4: Add MapPoint builder function with test**

Add to `src/ui/sample_map.rs`, after the normalization functions:

```rust
use crate::analysis::library::SampleLibrary;

/// Build map points from the full sample library.
/// Call once when library scan completes; cache the result in EditorState.
pub fn build_map_points(library: &SampleLibrary) -> Vec<MapPoint> {
    let flat = library.all_samples_flat();
    flat.iter()
        .enumerate()
        .map(|(i, sample)| MapPoint {
            nx: normalize_centroid(sample.features.spectral_centroid),
            ny: normalize_decay(sample.features.decay_time),
            library_index: i,
            category: sample.entry.category,
            name: sample.entry.filename.clone(),
            centroid_hz: sample.features.spectral_centroid,
            decay_secs: sample.features.decay_time,
        })
        .collect()
}
```

Add test:

```rust
#[test]
fn build_map_points_count_matches_library() {
    use crate::analysis::library::SampleLibrary;
    use crate::analysis::library::AnalyzedSample;
    use crate::analysis::features::AudioFeatures;
    use crate::analysis::scanner::SampleEntry;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    let mut by_category = HashMap::new();
    for cat in SampleCategory::all() {
        let entry = SampleEntry {
            path: PathBuf::from(format!("/test/{}.wav", cat.label())),
            filename: format!("{}.wav", cat.label()),
            category: *cat,
            folder_hint: None,
            duration_ms: 100,
            is_percussive: true,
        };
        let sample = AnalyzedSample {
            entry,
            features: AudioFeatures {
                attack_time: 0.001,
                decay_time: 0.1,
                spectral_centroid: 1000.0,
                spectral_flatness: 0.5,
                peak: 1.0,
                duration: 0.1,
                is_percussive: true,
            },
            data: Arc::new(vec![0.5; 4410]),
        };
        by_category.entry(*cat).or_insert_with(Vec::new).push(sample);
    }
    let lib = SampleLibrary { total: 10, by_category, sample_rate: 44100.0 };
    let points = build_map_points(&lib);
    assert_eq!(points.len(), 10);
    // All points should have valid normalized coords
    for p in &points {
        assert!(p.nx >= 0.0 && p.nx <= 1.0, "nx out of range: {}", p.nx);
        assert!(p.ny >= 0.0 && p.ny <= 1.0, "ny out of range: {}", p.ny);
    }
}
```

- [ ] **Step 5: Run all sample_map tests**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo test --lib -- ui::sample_map::tests -v 2>&1 | tail -20`

Expected: all 9 tests pass.

- [ ] **Step 6: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/sample_map.rs src/lib.rs src/main.rs
git commit -m "feat(ui): add MapPoint struct, normalization, and builder"
```

---

### Task 3: View Toggle in Editor

**Files:**
- Modify: `src/ui/editor.rs`
- Modify: `src/ui/toolbar.rs`

- [ ] **Step 1: Add ViewMode enum and map state to EditorState**

In `src/ui/editor.rs`, add after the existing imports:

```rust
use crate::ui::sample_map::{self, MapPoint};
```

Add the enum before `EditorState`:

```rust
/// Which view is currently active.
#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    PadStrip,
    SampleMap,
}
```

Add these fields to `EditorState`:

```rust
/// Current view mode (pad strip or sample map).
pub view_mode: ViewMode,
/// Cached map points — built once when library loads.
pub map_points: Vec<MapPoint>,
/// Whether map points have been built for the current library.
pub map_built: bool,
```

And set defaults in the `Default` impl:

```rust
view_mode: ViewMode::PadStrip,
map_points: Vec::new(),
map_built: false,
```

- [ ] **Step 2: Add ViewMode toggle to toolbar**

In `src/ui/toolbar.rs`, add a new variant to `ToolbarAction`:

```rust
pub enum ToolbarAction {
    None,
    Undo,
    Redo,
    DiceAll,
    LockAll,
    SetScale(f32),
    OpenSaveDialog,
    OpenLoadDialog,
    ToggleView,  // NEW
}
```

Update `draw_toolbar_snapshot` signature to take the current view mode. Add a parameter `view_mode: ViewMode` (import from `crate::ui::editor::ViewMode`).

Insert the MAP/PADS toggle right after the scan status label (before the spacer). Replace the version label with the view toggle:

```rust
// View toggle: MAP / PADS
{
    let map_color = if matches!(view_mode, ViewMode::SampleMap) {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    let pads_color = if matches!(view_mode, ViewMode::PadStrip) {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    let map_bg = if matches!(view_mode, ViewMode::SampleMap) {
        theme::ACCENT_DIM
    } else {
        theme::BG_ROW
    };
    let pads_bg = if matches!(view_mode, ViewMode::PadStrip) {
        theme::ACCENT_DIM
    } else {
        theme::BG_ROW
    };

    if ui
        .add(
            egui::Button::new(
                egui::RichText::new("MAP")
                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                    .color(map_color),
            )
            .fill(map_bg)
            .min_size(egui::vec2(36.0, 22.0)),
        )
        .clicked()
        && !matches!(view_mode, ViewMode::SampleMap)
    {
        action = ToolbarAction::ToggleView;
    }

    if ui
        .add(
            egui::Button::new(
                egui::RichText::new("PADS")
                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                    .color(pads_color),
            )
            .fill(pads_bg)
            .min_size(egui::vec2(36.0, 22.0)),
        )
        .clicked()
        && !matches!(view_mode, ViewMode::PadStrip)
    {
        action = ToolbarAction::ToggleView;
    }
}
```

- [ ] **Step 3: Wire view toggle in editor.rs**

In the editor `update` closure in `editor.rs`:

1. Update the `draw_toolbar_snapshot` call to pass `state.view_mode`.
2. Handle `ToolbarAction::ToggleView`:

```rust
ToolbarAction::ToggleView => {
    state.view_mode = match state.view_mode {
        ViewMode::PadStrip => ViewMode::SampleMap,
        ViewMode::SampleMap => ViewMode::PadStrip,
    };
}
```

3. Wrap the existing pad list rendering in a `match state.view_mode` block:

```rust
match state.view_mode {
    ViewMode::PadStrip => {
        // ... existing pad scroll area code ...
    }
    ViewMode::SampleMap => {
        // Build map points on first switch (lazy)
        if !state.map_built && snap.has_library {
            let shared = shared.lock();
            if let Some(ref lib) = shared.library {
                state.map_points = sample_map::build_map_points(lib);
                state.map_built = true;
            }
        }
        // Placeholder: just show dot count for now
        ui.centered_and_justified(|ui| {
            ui.label(
                egui::RichText::new(format!("Sample Map: {} points", state.map_points.len()))
                    .font(egui::FontId::new(14.0, egui::FontFamily::Monospace))
                    .color(theme::TEXT_DIM),
            );
        });
    }
}
```

- [ ] **Step 4: Build and verify**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo build --release 2>&1 | tail -5`

Expected: compiles cleanly. (No unit test for UI toggle — verified manually by running standalone and clicking MAP/PADS.)

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/editor.rs src/ui/toolbar.rs
git commit -m "feat(ui): add MAP/PADS view toggle in toolbar"
```

---

### Task 4: Dot Rendering with egui Painter

**Files:**
- Modify: `src/ui/sample_map.rs`
- Modify: `src/ui/editor.rs`

- [ ] **Step 1: Add view transform helpers to sample_map.rs**

Add after `build_map_points`:

```rust
use nih_plug_egui::egui;

/// View state for zoom/pan.
pub struct MapViewState {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Default for MapViewState {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan_x: 0.0,
            pan_y: 0.0,
        }
    }
}

/// Convert normalized (0–1) coords to screen position within the map rect.
fn to_screen(nx: f32, ny: f32, view: &MapViewState, rect: egui::Rect) -> egui::Pos2 {
    let x = (nx - view.pan_x) * view.zoom * rect.width() + rect.left();
    // Y inverted: ny=0 (short decay) at top, ny=1 (long decay) at bottom
    let y = (ny - view.pan_y) * view.zoom * rect.height() + rect.top();
    egui::pos2(x, y)
}

/// Convert screen position back to normalized coords.
fn from_screen(pos: egui::Pos2, view: &MapViewState, rect: egui::Rect) -> (f32, f32) {
    let nx = (pos.x - rect.left()) / (view.zoom * rect.width()) + view.pan_x;
    let ny = (pos.y - rect.top()) / (view.zoom * rect.height()) + view.pan_y;
    (nx, ny)
}
```

Add test:

```rust
#[test]
fn screen_transform_round_trips() {
    let view = MapViewState::default();
    let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(400.0, 200.0));
    let screen = to_screen(0.5, 0.5, &view, rect);
    let (nx, ny) = from_screen(screen, &view, rect);
    assert!((nx - 0.5).abs() < 0.001);
    assert!((ny - 0.5).abs() < 0.001);
}

#[test]
fn screen_transform_with_zoom() {
    let view = MapViewState { zoom: 2.0, pan_x: 0.25, pan_y: 0.25 };
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 200.0));
    // nx=0.5 → (0.5 - 0.25) * 2.0 * 400 + 0 = 200.0
    let screen = to_screen(0.5, 0.5, &view, rect);
    assert!((screen.x - 200.0).abs() < 0.1);
    assert!((screen.y - 100.0).abs() < 0.1);
}
```

- [ ] **Step 2: Add draw_map function**

Add the main rendering function to `src/ui/sample_map.rs`:

```rust
use crate::ui::theme;

/// Hit test result.
pub struct HitResult {
    /// Index into the map_points vec.
    pub point_index: usize,
    /// Screen position of the hit dot.
    pub screen_pos: egui::Pos2,
}

/// Find the nearest map point within `max_dist` screen pixels of `cursor`.
pub fn hit_test(
    cursor: egui::Pos2,
    points: &[MapPoint],
    view: &MapViewState,
    rect: egui::Rect,
    max_dist: f32,
) -> Option<HitResult> {
    let mut best: Option<(usize, f32, egui::Pos2)> = None;
    for (i, p) in points.iter().enumerate() {
        let screen = to_screen(p.nx, p.ny, view, rect);
        // Skip dots outside the visible rect (with margin)
        if !rect.expand(max_dist).contains(screen) {
            continue;
        }
        let dist = cursor.distance(screen);
        if dist < max_dist {
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((i, dist, screen));
            }
        }
    }
    best.map(|(i, _, pos)| HitResult {
        point_index: i,
        screen_pos: pos,
    })
}

/// Actions returned by draw_map for the editor to handle.
pub enum MapAction {
    None,
    /// User clicked a dot — preview it and show popup.
    ClickedDot { point_index: usize },
}

/// Draw the scatter plot onto the UI. Returns any action triggered.
pub fn draw_map(
    ui: &mut egui::Ui,
    points: &[MapPoint],
    view: &mut MapViewState,
    kit_paths: &[Option<String>],
    hovered_index: &mut Option<usize>,
) -> MapAction {
    let mut action = MapAction::None;

    // Allocate the map area (fills available space)
    let available = ui.available_size();
    let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = response.rect;

    // Background
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(0x08, 0x08, 0x1a));
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(1.0, egui::Color32::from_rgb(0x1a, 0x1a, 0x3a)), egui::PathStroke::OUTSIDE);

    // Axis labels
    let label_color = egui::Color32::from_rgba_premultiplied(0x63, 0x6e, 0x72, 0x4c);
    let label_font = egui::FontId::new(8.0, egui::FontFamily::Monospace);
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_BOTTOM,
        "BRIGHTNESS →",
        label_font.clone(),
        label_color,
    );
    // Vertical label (egui doesn't rotate text, so use a vertical stack)
    let decay_label = "D\nE\nC\nA\nY\n→";
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        decay_label,
        egui::FontId::new(7.0, egui::FontFamily::Monospace),
        label_color,
    );

    // Build set of kit sample paths for highlighting
    let kit_path_set: std::collections::HashSet<&str> = kit_paths
        .iter()
        .filter_map(|p| p.as_deref())
        .collect();

    // --- Zoom (scroll wheel) ---
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            let old_zoom = view.zoom;
            view.zoom = (view.zoom * (1.0 + scroll * 0.002)).clamp(1.0, 8.0);

            // Zoom toward cursor
            if let Some(cursor) = response.hover_pos() {
                let (nx, ny) = from_screen(cursor, &MapViewState { zoom: old_zoom, ..*view }, rect);
                view.pan_x = nx - (cursor.x - rect.left()) / (view.zoom * rect.width());
                view.pan_y = ny - (cursor.y - rect.top()) / (view.zoom * rect.height());
            }
        }
    }

    // --- Pan (drag on empty space) ---
    if response.dragged() && response.drag_delta().length() > 0.0 {
        let delta = response.drag_delta();
        view.pan_x -= delta.x / (view.zoom * rect.width());
        view.pan_y -= delta.y / (view.zoom * rect.height());
    }

    // --- Double-click to reset ---
    if response.double_clicked() {
        view.zoom = 1.0;
        view.pan_x = 0.0;
        view.pan_y = 0.0;
    }

    // --- Draw dots ---
    // First pass: library dots (dim)
    for (i, p) in points.iter().enumerate() {
        let screen = to_screen(p.nx, p.ny, view, rect);
        if !rect.contains(screen) {
            continue;
        }

        let color = theme::category_color(p.category);
        let is_kit = kit_path_set.contains(p.name.as_str());

        if is_kit {
            // Kit samples drawn in second pass
            continue;
        }

        let is_hovered = *hovered_index == Some(i);
        let (radius, alpha) = if is_hovered {
            (5.0, 0.7)
        } else {
            (3.0, 0.25)
        };

        painter.circle_filled(screen, radius, color.to_egui_alpha((alpha * 255.0) as u8));
    }

    // Second pass: kit dots (bright + ring)
    for (i, p) in points.iter().enumerate() {
        let screen = to_screen(p.nx, p.ny, view, rect);
        if !rect.contains(screen) {
            continue;
        }

        // Match by checking if this point's name appears in kit paths
        // (paths include full path, names are just filenames — need to match by path from library)
        // We'll match via library_index against kit_paths contents
        // For now, check if any kit path ends with this filename
        let is_kit = kit_paths.iter().any(|kp| {
            kp.as_ref().map_or(false, |path| path.ends_with(&p.name))
        });

        if !is_kit {
            continue;
        }

        let color = theme::category_color(p.category);
        // Glow shadow
        painter.circle_filled(screen, 8.0, color.to_egui_alpha(40));
        // Filled dot
        painter.circle_filled(screen, 5.0, color.to_egui());
        // White ring
        painter.circle_stroke(screen, 5.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
    }

    // --- Hit test for hover ---
    if let Some(cursor) = response.hover_pos() {
        if let Some(hit) = hit_test(cursor, points, view, rect, 8.0) {
            *hovered_index = Some(hit.point_index);

            // Tooltip
            let p = &points[hit.point_index];
            let color = theme::category_color(p.category);
            let tooltip_text = format!("{}\n{} · {:.0}Hz · {:.2}s", p.name, p.category.label(), p.centroid_hz, p.decay_secs);

            // Position: 15px right, 10px above cursor, flip if near edge
            let mut tooltip_pos = egui::pos2(cursor.x + 15.0, cursor.y - 10.0);
            if tooltip_pos.x + 150.0 > rect.right() {
                tooltip_pos.x = cursor.x - 165.0;
            }
            if tooltip_pos.y < rect.top() + 10.0 {
                tooltip_pos.y = cursor.y + 15.0;
            }

            let tooltip_rect = egui::Rect::from_min_size(tooltip_pos, egui::vec2(150.0, 32.0));
            painter.rect_filled(tooltip_rect, 4.0, egui::Color32::from_rgba_premultiplied(0x11, 0x11, 0x26, 0xee));
            painter.rect_stroke(tooltip_rect, 4.0, egui::Stroke::new(1.0, color.to_egui_alpha(0x55)), egui::PathStroke::OUTSIDE);
            painter.text(
                tooltip_rect.min + egui::vec2(6.0, 4.0),
                egui::Align2::LEFT_TOP,
                &p.name,
                egui::FontId::new(9.0, egui::FontFamily::Monospace),
                color.to_egui(),
            );
            painter.text(
                tooltip_rect.min + egui::vec2(6.0, 17.0),
                egui::Align2::LEFT_TOP,
                format!("{} · {:.0}Hz · {:.2}s", p.category.label(), p.centroid_hz, p.decay_secs),
                egui::FontId::new(8.0, egui::FontFamily::Monospace),
                theme::TEXT_DIM,
            );
        } else {
            *hovered_index = None;
        }
    } else {
        *hovered_index = None;
    }

    // --- Click detection ---
    if response.clicked() {
        if let Some(cursor) = response.interact_pointer_pos() {
            if let Some(hit) = hit_test(cursor, points, view, rect, 8.0) {
                action = MapAction::ClickedDot {
                    point_index: hit.point_index,
                };
            }
        }
    }

    action
}
```

- [ ] **Step 3: Add map state fields to EditorState and wire draw_map**

In `src/ui/editor.rs`, add to `EditorState`:

```rust
/// Map view state (zoom/pan).
pub map_view: sample_map::MapViewState,
/// Currently hovered dot index in the map.
pub map_hovered: Option<usize>,
```

Default:

```rust
map_view: sample_map::MapViewState::default(),
map_hovered: None,
```

Replace the placeholder `SampleMap` arm in the `match state.view_mode` with:

```rust
ViewMode::SampleMap => {
    // Build map points lazily on first view
    if !state.map_built && snap.has_library {
        let shared = shared.lock();
        if let Some(ref lib) = shared.library {
            state.map_points = sample_map::build_map_points(lib);
            state.map_built = true;
        }
    }

    // Collect kit sample paths from snapshot
    let kit_paths: Vec<Option<String>> = snap.pads.iter().map(|p| {
        if p.has_sample { Some(p.name.clone()) } else { None }
    }).collect();

    let map_action = sample_map::draw_map(
        ui,
        &state.map_points,
        &mut state.map_view,
        &kit_paths,
        &mut state.map_hovered,
    );

    match map_action {
        sample_map::MapAction::ClickedDot { point_index } => {
            // Preview + popup handled in Task 6 and 7
            let _ = point_index;
        }
        sample_map::MapAction::None => {}
    }
}
```

- [ ] **Step 4: Run tests and build**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo test --lib -- ui::sample_map::tests -v 2>&1 | tail -10 && cargo build --release 2>&1 | tail -5`

Expected: all tests pass, release build succeeds.

- [ ] **Step 5: Deploy and verify visually**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
cp target/release/libautokit.so ~/.vst3/autokit.vst3/Contents/x86_64-linux/autokit.so
cp target/release/libautokit.so ~/.clap/autokit.clap
```

Run standalone: `./target/release/autokit-standalone`

Verify: clicking MAP in toolbar shows a scatter plot with colored dots. Kicks should cluster bottom-left, hats top-right. Scroll to zoom, drag to pan, double-click to reset.

- [ ] **Step 6: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/sample_map.rs src/ui/editor.rs
git commit -m "feat(ui): render sample map scatter plot with zoom/pan and hover tooltip"
```

---

### Task 5: Preview Voice in Audio Thread

**Files:**
- Modify: `src/ui/state.rs`
- Modify: `src/plugin.rs`

- [ ] **Step 1: Add preview_sample to SharedState**

In `src/ui/state.rs`, add to `SharedState`:

```rust
/// Sample data to preview (set by GUI, consumed by audio thread).
pub preview_sample: Option<Arc<Vec<f32>>>,
```

Add `use std::sync::Arc;` to the imports.

Initialize in `SharedState::new()`:

```rust
preview_sample: None,
```

- [ ] **Step 2: Add preview playback state to Autokit**

In `src/plugin.rs`, add a preview state struct before `struct Autokit`:

```rust
/// Lightweight preview voice — separate from VoicePool.
struct PreviewVoice {
    data: Option<Arc<Vec<f32>>>,
    position: f64,
    /// Fade-out counter (samples remaining). 0 = not fading.
    fade_remaining: usize,
    fade_length: usize,
}

impl PreviewVoice {
    fn new() -> Self {
        Self {
            data: None,
            position: 0.0,
            fade_remaining: 0,
            fade_length: 0,
        }
    }

    /// Start a new preview, fading out any current one.
    fn start(&mut self, sample: Arc<Vec<f32>>, fade_samples: usize) {
        if self.data.is_some() && self.fade_remaining == 0 {
            self.fade_remaining = fade_samples;
            self.fade_length = fade_samples;
        }
        // We'll start the new sample after the fade completes.
        // For simplicity, just replace immediately — the old fade will
        // produce a brief crossfade.
        permit_alloc(|| {
            self.data = Some(sample);
        });
        self.position = 0.0;
        self.fade_remaining = 0;
    }

    /// Render into stereo buffers (center pan, unity gain).
    fn process(&mut self, output_left: &mut [f32], output_right: &mut [f32]) {
        let data = match &self.data {
            Some(d) => d,
            None => return,
        };

        let pan_gain = (0.25 * std::f32::consts::PI).cos(); // center pan

        for (l, r) in output_left.iter_mut().zip(output_right.iter_mut()) {
            let pos = self.position as usize;
            if pos >= data.len() {
                permit_alloc(|| {
                    self.data = None;
                });
                return;
            }

            let s = data[pos] * pan_gain;
            *l += s;
            *r += s;
            self.position += 1.0;
        }
    }
}
```

Add `preview_voice: PreviewVoice` to `struct Autokit` and `preview_voice: PreviewVoice::new()` to `Default`.

- [ ] **Step 3: Wire preview into process()**

In the `process()` method, inside the `if let Some(shared) = got_lock` block, after the GUI trigger check and before the sequencer, add:

```rust
// Check for preview sample request
if let Some(preview_data) = shared.preview_sample.take() {
    let fade = (0.05 * self.sample_rate) as usize; // 50ms fade
    self.preview_voice.start(preview_data, fade);
}
```

After `voices.process(output_left, output_right, &shared.kit);`, add:

```rust
// Mix preview voice
self.preview_voice.process(output_left, output_right);
```

Also add preview processing in the `else` (lock failed) branch — the preview voice doesn't need the lock since it has its own data:

After the silence fill in the else block:

```rust
// Preview voice can still play even if we couldn't lock shared state
self.preview_voice.process(
    &mut left_channels[0][..num_samples],
    &mut right_channels[0][..num_samples],
);
```

Wait — the else block already fills silence into `left_channels` and `right_channels`. We need to restructure slightly. Actually, preview voice should just run unconditionally after the main output. Move it after the master volume section:

After the master volume application (the last block in `process()`), add:

```rust
// Preview voice runs unconditionally (doesn't need shared lock)
{
    let channels = buffer.as_slice();
    if channels.len() >= 2 {
        let num = buffer.samples();
        let (left_ch, right_ch) = channels.split_at_mut(1);
        self.preview_voice.process(&mut left_ch[0][..num], &mut right_ch[0][..num]);
    }
}
```

Actually this won't work because `buffer.as_slice()` returns immutable. Let me reconsider. The preview trigger must be picked up from shared state (needs lock), so put the `take()` inside the lock block, and the actual `process()` call inside the same place as VoicePool. That's the cleanest approach.

Revise: inside the locked block, right after `voices.process(...)`:

```rust
self.preview_voice.process(output_left, output_right);
```

And for preview trigger pickup, inside the locked block after GUI trigger check:

```rust
if let Some(preview_data) = shared.preview_sample.take() {
    let fade = (0.05 * self.sample_rate) as usize;
    self.preview_voice.start(preview_data, fade);
}
```

For the else (no lock) case — the preview voice can still render its current data since it doesn't need shared state. But we need mutable access to the output buffers. The else block already has them. Add after the silence fill:

```rust
self.preview_voice.process(
    &mut left_channels[0][..num_samples],
    &mut right_channels[0][..num_samples],
);
```

- [ ] **Step 4: Build and verify**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo build --release 2>&1 | tail -5`

Expected: compiles cleanly.

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/state.rs src/plugin.rs
git commit -m "feat(audio): add preview voice for sample map auditioning"
```

---

### Task 6: Click-to-Preview and Assignment Popup

**Files:**
- Modify: `src/ui/sample_map.rs`
- Modify: `src/ui/editor.rs`

- [ ] **Step 1: Extend MapAction and add popup state**

In `src/ui/sample_map.rs`, update `MapAction`:

```rust
pub enum MapAction {
    None,
    /// User clicked a dot — preview and show popup.
    ClickedDot { point_index: usize },
    /// User assigned a sample from the popup to a pad.
    AssignToPad { point_index: usize, pad_index: usize },
}
```

Add popup state struct:

```rust
/// State for the assignment popup.
pub struct PopupState {
    /// Which map point the popup is for, or None if closed.
    pub active_point: Option<usize>,
    /// Screen position to anchor the popup near.
    pub anchor_pos: egui::Pos2,
}

impl Default for PopupState {
    fn default() -> Self {
        Self {
            active_point: None,
            anchor_pos: egui::Pos2::ZERO,
        }
    }
}
```

- [ ] **Step 2: Add draw_popup function**

Add to `src/ui/sample_map.rs`:

```rust
use crate::engine::kit::NUM_PADS;

/// Draw the assignment popup near a clicked dot. Returns an action if user assigns.
pub fn draw_popup(
    ctx: &egui::Context,
    popup: &mut PopupState,
    points: &[MapPoint],
    pad_categories: &[SampleCategory; NUM_PADS],
    map_rect: egui::Rect,
) -> MapAction {
    let point_index = match popup.active_point {
        Some(i) if i < points.len() => i,
        _ => return MapAction::None,
    };

    let p = &points[point_index];
    let color = theme::category_color(p.category);

    // Position: 15px right, 10px above anchor. Flip if near edges.
    let popup_width = 140.0;
    let popup_height = 80.0;
    let mut pos = egui::pos2(popup.anchor_pos.x + 15.0, popup.anchor_pos.y - popup_height - 10.0);
    if pos.x + popup_width > map_rect.right() {
        pos.x = popup.anchor_pos.x - popup_width - 15.0;
    }
    if pos.y < map_rect.top() {
        pos.y = popup.anchor_pos.y + 15.0;
    }

    let mut action = MapAction::None;

    egui::Area::new(egui::Id::new("map_popup"))
        .fixed_pos(pos)
        .constrain(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_rgb(0x11, 0x11, 0x26))
                .stroke(egui::Stroke::new(1.0, color.to_egui_alpha(0x77)))
                .corner_radius(egui::CornerRadius::same(5))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    // Sample name
                    ui.label(
                        egui::RichText::new(&p.name)
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(color.to_egui())
                            .strong(),
                    );
                    // Stats line
                    ui.label(
                        egui::RichText::new(format!(
                            "{} · {:.0}Hz · {:.2}s",
                            p.category.label(),
                            p.centroid_hz,
                            p.decay_secs
                        ))
                        .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DIM),
                    );
                    ui.add_space(3.0);
                    // Label
                    ui.label(
                        egui::RichText::new("ASSIGN TO PAD:")
                            .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DIM),
                    );
                    ui.add_space(2.0);
                    // 4x2 grid of pad buttons
                    egui::Grid::new("popup_pads")
                        .spacing(egui::vec2(2.0, 2.0))
                        .show(ui, |ui| {
                            for i in 0..NUM_PADS {
                                let pad_color = theme::category_color(pad_categories[i]);
                                let is_match = pad_categories[i] == p.category;
                                let bg = if is_match {
                                    pad_color.to_egui_alpha(0x44)
                                } else {
                                    pad_color.to_egui_alpha(0x22)
                                };
                                let label = format!("{}", i + 1);
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(&label)
                                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                                .color(pad_color.to_egui()),
                                        )
                                        .fill(bg)
                                        .min_size(egui::vec2(24.0, 18.0)),
                                    )
                                    .clicked()
                                {
                                    action = MapAction::AssignToPad {
                                        point_index,
                                        pad_index: i,
                                    };
                                    popup.active_point = None;
                                }
                                if i == 3 {
                                    ui.end_row();
                                }
                            }
                        });
                });
        });

    // Keyboard assign: 1-8 keys
    ctx.input(|input| {
        for i in 0..NUM_PADS {
            let key = match i {
                0 => egui::Key::Num1,
                1 => egui::Key::Num2,
                2 => egui::Key::Num3,
                3 => egui::Key::Num4,
                4 => egui::Key::Num5,
                5 => egui::Key::Num6,
                6 => egui::Key::Num7,
                7 => egui::Key::Num8,
                _ => continue,
            };
            if input.key_pressed(key) {
                action = MapAction::AssignToPad {
                    point_index,
                    pad_index: i,
                };
                popup.active_point = None;
            }
        }
        // Escape closes popup
        if input.key_pressed(egui::Key::Escape) {
            popup.active_point = None;
        }
    });

    action
}
```

- [ ] **Step 3: Add popup state to EditorState and wire in editor.rs**

In `src/ui/editor.rs`, add to `EditorState`:

```rust
/// Assignment popup state for sample map.
pub map_popup: sample_map::PopupState,
```

Default: `map_popup: sample_map::PopupState::default(),`

Add `GuiAction` variants:

```rust
PreviewSample(usize),  // library_index
AssignFromMap { pad_index: usize, library_index: usize },
```

Update the `SampleMap` arm in the view match to handle click and popup:

```rust
ViewMode::SampleMap => {
    if !state.map_built && snap.has_library {
        let shared_lock = shared.lock();
        if let Some(ref lib) = shared_lock.library {
            state.map_points = sample_map::build_map_points(lib);
            state.map_built = true;
        }
    }

    let kit_paths: Vec<Option<String>> = snap.pads.iter().map(|p| {
        if p.has_sample { Some(p.name.clone()) } else { None }
    }).collect();

    let map_action = sample_map::draw_map(
        ui,
        &state.map_points,
        &mut state.map_view,
        &kit_paths,
        &mut state.map_hovered,
    );

    match map_action {
        sample_map::MapAction::ClickedDot { point_index } => {
            // Set preview + open popup
            let lib_index = state.map_points[point_index].library_index;
            pending_action = Some(GuiAction::PreviewSample(lib_index));
            state.map_popup.active_point = Some(point_index);
            // Record screen position for popup anchor
            if let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) {
                state.map_popup.anchor_pos = cursor;
            }
        }
        sample_map::MapAction::AssignToPad { .. } => {
            // Handled by popup below
        }
        sample_map::MapAction::None => {}
    }
}
```

After the CentralPanel (outside the match, same scope as the save/load dialogs), add popup rendering:

```rust
// --- Sample map assignment popup ---
if state.view_mode == ViewMode::SampleMap && state.map_popup.active_point.is_some() {
    let pad_categories: [SampleCategory; NUM_PADS] =
        core::array::from_fn(|i| snap.pads[i].category);
    let map_rect = egui::Rect::from_min_size(
        egui::pos2(0.0, 0.0),
        egui::vec2(900.0, 365.0),
    );
    let popup_action = sample_map::draw_popup(
        ctx,
        &mut state.map_popup,
        &state.map_points,
        &pad_categories,
        map_rect,
    );
    match popup_action {
        sample_map::MapAction::AssignToPad { point_index, pad_index } => {
            let lib_index = state.map_points[point_index].library_index;
            pending_action = Some(GuiAction::AssignFromMap {
                pad_index,
                library_index: lib_index,
            });
        }
        _ => {}
    }
}

// Dismiss popup on click outside (when map is showing but no dot was hit)
if state.view_mode == ViewMode::SampleMap
    && state.map_popup.active_point.is_some()
    && ctx.input(|i| i.pointer.any_click())
{
    // If the click was a ClickedDot, the popup was already updated above.
    // If it was on empty space, dismiss.
    if state.map_hovered.is_none() {
        state.map_popup.active_point = None;
    }
}
```

- [ ] **Step 4: Handle GuiAction::PreviewSample and AssignFromMap**

In the `if let Some(action) = pending_action` block in `editor.rs`, add handlers:

```rust
GuiAction::PreviewSample(lib_index) => {
    if let Some(ref lib) = shared.library {
        if let Some(sample) = lib.sample_by_flat_index(lib_index) {
            shared.preview_sample = Some(Arc::clone(&sample.data));
        }
    }
}
GuiAction::AssignFromMap { pad_index, library_index } => {
    if let Some(ref lib) = shared.library {
        if let Some(sample) = lib.sample_by_flat_index(library_index) {
            // Push history for undo
            let snap = HistorySnapshot {
                pads: shared.kit.snapshot(),
                sequencer: seq_snap(),
            };
            shared.history.push(snap);

            // Assign sample to pad
            let pad = &mut shared.kit.pads[pad_index];
            pad.sample = Some(Arc::clone(&sample.data));
            pad.sample_path = Some(sample.entry.path.to_string_lossy().to_string());
            pad.name = sample.entry.filename.clone();
            pad.category = sample.entry.category;

            shared.update_waveform(pad_index, WAVEFORM_POINTS);

            // Rebuild map points flag (kit dots changed)
            // Actually not needed — kit highlighting uses snapshot paths,
            // which update automatically on next frame.
        }
    }
}
```

Add `use std::sync::Arc;` to `editor.rs` imports if not already present.

- [ ] **Step 5: Build and verify**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo build --release 2>&1 | tail -5`

Deploy and test: click a dot on the map, hear preview, see popup, click a pad number to assign.

- [ ] **Step 6: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/sample_map.rs src/ui/editor.rs
git commit -m "feat(ui): add click-to-preview and assignment popup in sample map"
```

---

### Task 7: Mini Pad Bar

**Files:**
- Modify: `src/ui/sample_map.rs`
- Modify: `src/ui/editor.rs`

- [ ] **Step 1: Add draw_mini_pad_bar function**

Add to `src/ui/sample_map.rs`:

```rust
/// Actions from the mini pad bar.
pub enum PadBarAction {
    None,
    /// User clicked a pad to enter/exit shortcut mode.
    ToggleShortcut(usize),
}

/// Draw the mini pad bar below the map.
pub fn draw_mini_pad_bar(
    ui: &mut egui::Ui,
    pad_names: &[String; NUM_PADS],
    pad_categories: &[SampleCategory; NUM_PADS],
    shortcut_pad: Option<usize>,
) -> PadBarAction {
    let mut action = PadBarAction::None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let bar_width = ui.clip_rect().width() - 12.0;
        let btn_width = (bar_width / NUM_PADS as f32) - 2.0;

        for i in 0..NUM_PADS {
            let color = theme::category_color(pad_categories[i]);
            let is_active = shortcut_pad == Some(i);

            let bg = if is_active {
                color.to_egui_alpha(0x44)
            } else {
                color.to_egui_alpha(0x22)
            };
            let border = if is_active {
                egui::Stroke::new(2.0, color.to_egui())
            } else {
                egui::Stroke::new(1.0, color.to_egui_alpha(0x55))
            };

            let label = format!("{} {}", i + 1, pad_categories[i].label().chars().take(3).collect::<String>().to_uppercase());

            let btn = egui::Button::new(
                egui::RichText::new(&label)
                    .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                    .color(color.to_egui()),
            )
            .fill(bg)
            .stroke(border)
            .min_size(egui::vec2(btn_width, 22.0));

            let response = ui.add(btn);
            if response.clicked() {
                action = PadBarAction::ToggleShortcut(i);
            }
            if response.hovered() {
                response.on_hover_text_at_pointer(
                    egui::RichText::new(&pad_names[i])
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace)),
                );
            }
        }
    });

    action
}
```

- [ ] **Step 2: Add shortcut_pad to EditorState and wire mini pad bar**

In `src/ui/editor.rs`, add to `EditorState`:

```rust
/// Which pad is selected for shortcut assignment mode (None = off).
pub map_shortcut_pad: Option<usize>,
```

Default: `map_shortcut_pad: None,`

In the `SampleMap` arm, after `draw_map`, add:

```rust
// Separator
ui.add(egui::Separator::default().spacing(0.0));

// Mini pad bar
let pad_names: [String; NUM_PADS] = core::array::from_fn(|i| snap.pads[i].name.clone());
let pad_categories: [SampleCategory; NUM_PADS] = core::array::from_fn(|i| snap.pads[i].category);
let bar_action = sample_map::draw_mini_pad_bar(
    ui,
    &pad_names,
    &pad_categories,
    state.map_shortcut_pad,
);

match bar_action {
    sample_map::PadBarAction::ToggleShortcut(i) => {
        state.map_shortcut_pad = if state.map_shortcut_pad == Some(i) {
            None
        } else {
            Some(i)
        };
    }
    sample_map::PadBarAction::None => {}
}
```

Update the `ClickedDot` handler to support shortcut mode:

```rust
sample_map::MapAction::ClickedDot { point_index } => {
    let lib_index = state.map_points[point_index].library_index;
    pending_action = Some(GuiAction::PreviewSample(lib_index));

    if let Some(pad) = state.map_shortcut_pad {
        // Shortcut mode: assign directly, no popup
        pending_action = Some(GuiAction::AssignFromMap {
            pad_index: pad,
            library_index: lib_index,
        });
        // Also preview
        // Since we can only set one pending_action, combine:
        // Actually we need both preview AND assign. Let's preview via
        // the assignment handler (it loads the sample data anyway).
        // Or: add a combined action.
    } else {
        // Normal mode: show popup
        state.map_popup.active_point = Some(point_index);
        if let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) {
            state.map_popup.anchor_pos = cursor;
        }
    }
}
```

To handle both preview and assign in shortcut mode, we need to either: (a) set preview_sample inside the AssignFromMap handler, or (b) use a vec of actions. Simplest: set preview inside AssignFromMap handler. Add to the `AssignFromMap` handler, after assigning the sample:

```rust
// Also trigger preview
shared.preview_sample = Some(Arc::clone(&sample.data));
```

And for the shortcut ClickedDot case, just set `pending_action = Some(GuiAction::AssignFromMap { ... })`.

- [ ] **Step 3: Escape exits shortcut mode**

In the keyboard input section (where pad keys Z-M are checked), add:

```rust
if input.key_pressed(egui::Key::Escape) {
    state.map_shortcut_pad = None;
    state.map_popup.active_point = None;
}
```

- [ ] **Step 4: Build and verify**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo build --release 2>&1 | tail -5`

Deploy and test: mini pad bar shows 8 buttons, clicking one enters shortcut mode, clicking a dot assigns directly.

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/sample_map.rs src/ui/editor.rs
git commit -m "feat(ui): add mini pad bar and shortcut assignment mode"
```

---

### Task 8: Shortcut Mode Visual Feedback

**Files:**
- Modify: `src/ui/sample_map.rs`
- Modify: `src/ui/toolbar.rs`

- [ ] **Step 1: Pass shortcut state into draw_map**

Update `draw_map` signature in `src/ui/sample_map.rs` to accept shortcut mode:

```rust
pub fn draw_map(
    ui: &mut egui::Ui,
    points: &[MapPoint],
    view: &mut MapViewState,
    kit_paths: &[Option<String>],
    hovered_index: &mut Option<usize>,
    shortcut_pad: Option<usize>,
    shortcut_category: Option<SampleCategory>,
) -> MapAction {
```

Update the dot rendering loop — when in shortcut mode, adjust opacity:

In the first pass (library dots), replace the opacity logic:

```rust
let (radius, alpha) = if is_hovered {
    (5.0, 0.7)
} else if let Some(cat) = shortcut_category {
    if p.category == cat {
        (4.0, 0.5)
    } else {
        (3.0, 0.12)
    }
} else {
    (3.0, 0.25)
};
```

When shortcut mode is active, tint the map border:

```rust
if let Some(cat) = shortcut_category {
    let border_color = theme::category_color(cat);
    painter.rect_stroke(rect, 0.0, egui::Stroke::new(2.0, border_color.to_egui_alpha(0x44)), egui::PathStroke::OUTSIDE);
}
```

- [ ] **Step 2: Update toolbar to show shortcut status**

Update `draw_toolbar_snapshot` signature to accept shortcut info:

```rust
pub fn draw_toolbar_snapshot(
    ui: &mut egui::Ui,
    scan_status: &ScanStatus,
    can_undo: bool,
    can_redo: bool,
    all_locked: bool,
    params: &AutokitParams,
    setter: &ParamSetter,
    current_scale: f32,
    view_mode: ViewMode,
    shortcut_info: Option<(usize, &str)>,  // (pad_number, category_label)
) -> ToolbarAction {
```

After the scan status display (inside the match), add:

```rust
if let Some((pad_num, cat_label)) = shortcut_info {
    ui.label(
        egui::RichText::new(format!("→ pad {}: {}", pad_num, cat_label))
            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
            .color(theme::ACCENT),
    );
}
```

- [ ] **Step 3: Update all call sites**

Update the `draw_map` call in `editor.rs` to pass shortcut state:

```rust
let shortcut_category = state.map_shortcut_pad.map(|i| snap.pads[i].category);
let map_action = sample_map::draw_map(
    ui,
    &state.map_points,
    &mut state.map_view,
    &kit_paths,
    &mut state.map_hovered,
    state.map_shortcut_pad,
    shortcut_category,
);
```

Update the `draw_toolbar_snapshot` call to pass shortcut info:

```rust
let shortcut_info = state.map_shortcut_pad.map(|i| {
    (i + 1, snap.pads[i].category.label())
});
let toolbar_action = toolbar::draw_toolbar_snapshot(
    ui, &snap.scan_status, snap.can_undo, snap.can_redo,
    all_locked, &params, setter, state.scale,
    state.view_mode, shortcut_info,
);
```

- [ ] **Step 4: Build and verify**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo build --release 2>&1 | tail -5`

Deploy and test: shortcut mode dims non-matching dots, tints border, toolbar shows active pad.

- [ ] **Step 5: Commit**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add src/ui/sample_map.rs src/ui/toolbar.rs src/ui/editor.rs
git commit -m "feat(ui): add shortcut mode visual feedback — dot emphasis, border tint, toolbar status"
```

---

### Task 9: Final Polish and Full Test Run

**Files:**
- Modify: `src/ui/editor.rs` (minor)
- All test files

- [ ] **Step 1: Run full test suite**

Run: `cd /home/natalia/repos/Autokit/.worktrees/phase6-gui && cargo test --lib 2>&1 | tail -20`

Expected: all existing tests (42) + new tests (normalization: 8, map builder: 1, transform: 2, library flat: 4) = ~57 tests pass.

- [ ] **Step 2: Fix any compilation issues or test failures**

Address any issues found in step 1.

- [ ] **Step 3: Build release and deploy**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
cargo build --release
cp target/release/libautokit.so ~/.vst3/autokit.vst3/Contents/x86_64-linux/autokit.so
cp target/release/libautokit.so ~/.clap/autokit.clap
strings ~/.vst3/autokit.vst3/Contents/x86_64-linux/autokit.so | grep -c egui
```

Expected: egui string count > 0.

- [ ] **Step 4: Manual verification checklist**

Run standalone: `./target/release/autokit-standalone`

Verify:
- [ ] MAP/PADS toggle switches views
- [ ] Dots cluster by category (kicks bottom-left, hats top-right)
- [ ] Kit samples have white rings
- [ ] Hover shows tooltip with sample name/stats
- [ ] Click dot → hear preview + popup appears
- [ ] Popup pad buttons assign sample (check switching to PADS view shows new sample)
- [ ] Number keys 1-8 assign while popup is open
- [ ] Escape closes popup
- [ ] Click pad in mini bar → shortcut mode
- [ ] Shortcut mode: matching category dots brighten, others dim
- [ ] Shortcut mode: click dot assigns directly
- [ ] Scroll wheel zooms (centered on cursor)
- [ ] Drag pans the view
- [ ] Double-click resets zoom/pan
- [ ] Keyboard triggers (Z-M) still play pads in map view
- [ ] Undo after assignment restores previous sample

- [ ] **Step 5: Commit final polish**

```bash
cd /home/natalia/repos/Autokit/.worktrees/phase6-gui
git add -A
git commit -m "feat(ui): complete Phase 7 sample map GUI"
```
