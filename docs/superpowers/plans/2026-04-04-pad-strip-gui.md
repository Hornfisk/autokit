# Pad Strip GUI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an egui-based GUI to the Autokit plugin showing a vertical pad list with waveforms, per-pad controls, and a global toolbar.

**Architecture:** Move `DrumKit`, `SampleLibrary`, and `History` into an `Arc<Mutex<SharedState>>` shared between the audio thread and GUI. The GUI renders in immediate mode via `nih_plug_egui`, reading pad state and waveform caches to paint a vertical list of 16 pad rows with an expand/collapse detail panel per pad. JetBrains Mono font, neon-on-dark category-colored theme.

**Tech Stack:** nih_plug_egui (egui via nih-plug), parking_lot::Mutex, egui custom painting

---

### Task 1: SharedState + WaveformSummary structs

**Files:**
- Create: `src/ui/state.rs`
- Modify: `src/lib.rs:17-20`

- [ ] **Step 1: Create `src/ui/state.rs` with SharedState and WaveformSummary**

```rust
use parking_lot::Mutex;
use std::sync::Arc;

use crate::analysis::library::SampleLibrary;
use crate::engine::kit::DrumKit;
use crate::util::history::History;

/// Scan progress for the toolbar display.
#[derive(Clone, Debug)]
pub enum ScanStatus {
    Scanning,
    Ready { total: usize },
}

/// Pre-computed waveform display data for one pad.
/// Stores min/max pairs downsampled to `points` columns.
#[derive(Clone)]
pub struct WaveformSummary {
    /// (min, max) amplitude pairs, one per display column.
    pub points: Vec<[f32; 2]>,
}

impl WaveformSummary {
    /// Downsample raw sample data to `num_points` min/max pairs.
    pub fn from_samples(samples: &[f32], num_points: usize) -> Self {
        if samples.is_empty() || num_points == 0 {
            return Self { points: vec![] };
        }

        let chunk_size = samples.len() / num_points;
        if chunk_size == 0 {
            // Fewer samples than points — one point per sample
            return Self {
                points: samples.iter().map(|&s| [s, s]).collect(),
            };
        }

        let points = (0..num_points)
            .map(|i| {
                let start = i * chunk_size;
                let end = ((i + 1) * chunk_size).min(samples.len());
                let chunk = &samples[start..end];
                let min = chunk.iter().copied().fold(f32::INFINITY, f32::min);
                let max = chunk.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                [min, max]
            })
            .collect();

        Self { points }
    }
}

/// State shared between the audio thread and GUI thread.
/// Locked via `parking_lot::Mutex` — locks must be brief.
pub struct SharedState {
    pub kit: DrumKit,
    pub library: Option<SampleLibrary>,
    pub history: History,
    pub scan_status: ScanStatus,
    /// Pre-computed waveform summaries, one per pad. Recomputed on sample change.
    pub waveforms: [Option<WaveformSummary>; 16],
}

impl SharedState {
    pub fn new() -> Self {
        Self {
            kit: DrumKit::new(),
            library: None,
            history: History::new(),
            scan_status: ScanStatus::Scanning,
            waveforms: Default::default(),
        }
    }

    /// Recompute waveform summary for a single pad.
    pub fn update_waveform(&mut self, pad_index: usize, num_points: usize) {
        if pad_index >= self.kit.pads.len() {
            return;
        }
        self.waveforms[pad_index] = self.kit.pads[pad_index]
            .sample
            .as_ref()
            .map(|s| WaveformSummary::from_samples(s, num_points));
    }

    /// Recompute waveform summaries for all 16 pads.
    pub fn update_all_waveforms(&mut self, num_points: usize) {
        for i in 0..16 {
            self.update_waveform(i, num_points);
        }
    }
}
```

- [ ] **Step 2: Add `state` module to `src/lib.rs`**

In `src/lib.rs`, change the `mod ui` block from:

```rust
mod ui {
    pub mod theme;
}
```

to:

```rust
mod ui {
    pub mod state;
    pub mod theme;
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -5`
Expected: compiles with existing warnings only, no new errors.

- [ ] **Step 4: Write test for WaveformSummary**

Add to the bottom of `src/ui/state.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_summary_downsamples_correctly() {
        // 200 samples → 10 points = 20 samples per chunk
        let samples: Vec<f32> = (0..200).map(|i| (i as f32 / 200.0) * 2.0 - 1.0).collect();
        let summary = WaveformSummary::from_samples(&samples, 10);
        assert_eq!(summary.points.len(), 10);
        // First chunk: samples 0..20, values -1.0 to ~-0.8
        assert!(summary.points[0][0] < summary.points[0][1]); // min < max
    }

    #[test]
    fn waveform_summary_empty_input() {
        let summary = WaveformSummary::from_samples(&[], 10);
        assert!(summary.points.is_empty());
    }

    #[test]
    fn waveform_summary_fewer_samples_than_points() {
        let samples = vec![0.5, -0.3, 0.8];
        let summary = WaveformSummary::from_samples(&samples, 10);
        assert_eq!(summary.points.len(), 3);
    }
}
```

- [ ] **Step 5: Run tests**

Run: `cd /home/natalia/repos/Autokit && cargo test --lib ui::state 2>&1 | tail -10`
Expected: 3 tests pass.

- [ ] **Step 6: Commit**

```bash
git add src/ui/state.rs src/lib.rs
git commit -m "feat(ui): add SharedState and WaveformSummary structs"
```

---

### Task 2: Refactor plugin.rs to use SharedState

**Files:**
- Modify: `src/plugin.rs`

- [ ] **Step 1: Update imports and struct definition in `plugin.rs`**

Replace the imports and `Autokit` struct (lines 1-65) with:

```rust
use nih_plug::prelude::*;
use nih_plug::util::permit_alloc;
use std::path::PathBuf;
use std::sync::Arc;

use crossbeam_channel::Receiver;
use parking_lot::Mutex;

use crate::analysis::library::SampleLibrary;
use crate::engine::sampler::VoicePool;
use crate::engine::sequencer::Sequencer;
use crate::logging;
use crate::ui::state::{ScanStatus, SharedState};
use crate::util::history::HistorySnapshot;

/// Hard-coded sample library root — folder picker comes in GUI phase.
const SAMPLE_LIBRARY_ROOT: &str = "/home/natalia/Music/Samples";

/// Number of points in waveform summaries.
const WAVEFORM_POINTS: usize = 200;

#[derive(Params)]
pub struct AutokitParams {
    #[id = "master_vol"]
    pub master_volume: FloatParam,
}

impl Default for AutokitParams {
    fn default() -> Self {
        Self {
            master_volume: FloatParam::new(
                "Master Volume",
                util::db_to_gain(0.0),
                FloatRange::Skewed {
                    min: util::db_to_gain(-60.0),
                    max: util::db_to_gain(6.0),
                    factor: FloatRange::gain_skew_factor(-60.0, 6.0),
                },
            )
            .with_smoother(SmoothingStyle::Logarithmic(50.0))
            .with_unit(" dB")
            .with_value_to_string(formatters::v2s_f32_gain_to_db(2))
            .with_string_to_value(formatters::s2v_f32_gain_to_db()),
        }
    }
}

/// Messages from background thread to audio thread.
enum BgMessage {
    /// Library scan complete — assign samples to kit.
    LibraryReady(SampleLibrary),
}

pub struct Autokit {
    params: Arc<AutokitParams>,
    sample_rate: f32,
    /// Shared state between audio thread and GUI.
    pub shared: Arc<Mutex<SharedState>>,
    voices: Option<VoicePool>,
    /// Receive messages from background thread (checked in process()).
    bg_rx: Option<Receiver<BgMessage>>,
    sequencer: Sequencer,
    /// Debug: counts process() calls to log periodic status.
    #[cfg(debug_assertions)]
    process_count: u64,
}

impl Default for Autokit {
    fn default() -> Self {
        Self {
            params: Arc::new(AutokitParams::default()),
            sample_rate: 44100.0,
            shared: Arc::new(Mutex::new(SharedState::new())),
            voices: None,
            bg_rx: None,
            sequencer: Sequencer::new(),
            #[cfg(debug_assertions)]
            process_count: 0,
        }
    }
}
```

- [ ] **Step 2: Update `populate_kit_from_library` to work with SharedState**

Replace the `populate_kit_from_library` function (lines 84-109 in original) with:

```rust
/// Populate the kit from the library using the default layout.
fn populate_kit_from_library(shared: &mut SharedState) {
    let library = match &shared.library {
        Some(lib) => lib,
        None => return,
    };
    let layout = library.generate_kit();
    let mut assigned = 0u32;

    for (pad_idx, category) in layout {
        if pad_idx >= shared.kit.pads.len() {
            break;
        }

        // Skip locked pads
        if shared.kit.pads[pad_idx].locked {
            continue;
        }

        if let Some(sample) = library.random_from(category) {
            shared.kit.pads[pad_idx].sample = Some(Arc::clone(&sample.data));
            shared.kit.pads[pad_idx].sample_path =
                Some(sample.entry.path.to_string_lossy().to_string());
            shared.kit.pads[pad_idx].name = sample.entry.filename.clone();
            shared.kit.pads[pad_idx].category = sample.entry.category;
            assigned += 1;
        }
    }

    // Update waveform summaries for all pads
    shared.update_all_waveforms(WAVEFORM_POINTS);

    tracing::info!(assigned, total_pads = shared.kit.pads.len(), "kit populated from library");
}
```

- [ ] **Step 3: Update `initialize()` — no changes needed**

The `initialize` method stays the same (spawns background thread, creates VoicePool). No changes required.

- [ ] **Step 4: Update `process()` to lock SharedState**

Replace the `process()` method body with:

```rust
    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Check for background thread messages (non-blocking)
        if let Some(rx) = &self.bg_rx {
            if let Ok(msg) = rx.try_recv() {
                permit_alloc(|| {
                    match msg {
                        BgMessage::LibraryReady(library) => {
                            tracing::info!(
                                total = library.total,
                                "library received — populating kit"
                            );
                            let mut shared = self.shared.lock();
                            // Push snapshot before first population for undo support
                            let snapshot = HistorySnapshot {
                                pads: shared.kit.snapshot(),
                                sequencer: self.sequencer.snapshot(),
                            };
                            shared.history.push(snapshot);
                            shared.library = Some(library);
                            shared.scan_status = ScanStatus::Ready {
                                total: shared.library.as_ref().unwrap().total,
                            };
                            populate_kit_from_library(&mut shared);
                        }
                    }
                });
            }
        }

        // Periodic debug heartbeat (~every 5s)
        #[cfg(debug_assertions)]
        {
            self.process_count += 1;
            if self.process_count % 1000 == 1 {
                let active = self.voices.as_ref().map(|v| v.active_count()).unwrap_or(0);
                let shared = self.shared.lock();
                let has_lib = shared.library.is_some();
                let seq_step = self.sequencer.current_step();
                let seq_playing = self.sequencer.is_playing();
                drop(shared);
                permit_alloc(|| {
                    tracing::debug!(
                        call = self.process_count,
                        active_voices = active,
                        library_loaded = has_lib,
                        seq_step,
                        seq_playing,
                        "process() heartbeat"
                    );
                });
            }
        }

        let voices = match &mut self.voices {
            Some(v) => v,
            None => return ProcessStatus::KeepAlive,
        };

        // Lock shared state for MIDI + sequencer + rendering
        let shared = self.shared.lock();

        // Drain MIDI events and trigger voices
        while let Some(event) = context.next_event() {
            match event {
                NoteEvent::NoteOn { note, velocity, .. } => {
                    if let Some(pad_idx) = shared.kit.pad_for_note(note) {
                        voices.trigger(pad_idx, velocity, &shared.kit, 0);
                    }
                }
                NoteEvent::NoteOff { .. } => {}
                _ => {}
            }
        }

        // Run sequencer — triggers voices at step boundaries
        let transport = context.transport();
        self.sequencer.process_buffer(
            buffer.samples(),
            transport.playing,
            transport.tempo,
            transport.pos_beats(),
            self.sample_rate,
            voices,
            &shared.kit,
        );

        let num_samples = buffer.samples();
        let channels = buffer.as_slice();

        if channels.len() < 2 {
            return ProcessStatus::KeepAlive;
        }

        let (left_channels, right_channels) = channels.split_at_mut(1);
        let output_left = &mut left_channels[0][..num_samples];
        let output_right = &mut right_channels[0][..num_samples];

        output_left.fill(0.0);
        output_right.fill(0.0);

        voices.process(output_left, output_right, &shared.kit);

        drop(shared); // Release lock before param smoothing

        let master_gain = self.params.master_volume.smoothed.next();
        for s in output_left.iter_mut() {
            *s *= master_gain;
        }
        for s in output_right.iter_mut() {
            *s *= master_gain;
        }

        ProcessStatus::KeepAlive
    }
```

- [ ] **Step 5: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -10`
Expected: compiles (may have warnings about unused imports from removed fields).

- [ ] **Step 6: Run all existing tests**

Run: `cd /home/natalia/repos/Autokit && cargo test 2>&1 | tail -15`
Expected: all 38 tests pass (35 existing + 3 new waveform tests).

- [ ] **Step 7: Commit**

```bash
git add src/plugin.rs
git commit -m "refactor(plugin): move kit/library/history into Arc<Mutex<SharedState>>"
```

---

### Task 3: Extend theme.rs with colors, font constants, and helpers

**Files:**
- Modify: `src/ui/theme.rs`

- [ ] **Step 1: Add egui color constants and helpers to theme.rs**

Replace the entire contents of `src/ui/theme.rs` with:

```rust
use crate::engine::kit::SampleCategory;

/// Color as [R, G, B] u8 values.
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn to_egui(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.0, self.1, self.2)
    }

    pub fn to_egui_alpha(&self, alpha: u8) -> egui::Color32 {
        egui::Color32::from_rgba_premultiplied(
            (self.0 as u16 * alpha as u16 / 255) as u8,
            (self.1 as u16 * alpha as u16 / 255) as u8,
            (self.2 as u16 * alpha as u16 / 255) as u8,
            alpha,
        )
    }
}

use nih_plug_egui::egui;

/// Get the display color for a sample category.
pub fn category_color(cat: SampleCategory) -> Color {
    match cat {
        SampleCategory::Kick => Color(0xff, 0x6b, 0x9d),   // magenta
        SampleCategory::Snare => Color(0x4e, 0xcd, 0xc4),   // cyan
        SampleCategory::Hihat => Color(0xff, 0x9f, 0x43),   // bright orange
        SampleCategory::Clap => Color(0xa8, 0xe6, 0xcf),    // mint
        SampleCategory::Tom => Color(0xff, 0x76, 0x75),      // coral
        SampleCategory::Perc => Color(0xc0, 0x84, 0xfc),     // purple
        SampleCategory::Cymbal => Color(0xff, 0xd1, 0x66),   // gold
        SampleCategory::Bass => Color(0x74, 0xb9, 0xff),     // deep blue
        SampleCategory::Synth => Color(0xfd, 0x79, 0xa8),    // hot pink
        SampleCategory::Other => Color(0x63, 0x6e, 0x72),    // grey
    }
}

// UI background colors
pub const BG_MAIN: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0a, 0x1a);
pub const BG_TOOLBAR: egui::Color32 = egui::Color32::from_rgb(0x0e, 0x0e, 0x20);
pub const BG_ROW: egui::Color32 = egui::Color32::from_rgb(0x11, 0x11, 0x26);
pub const BG_ROW_HOVER: egui::Color32 = egui::Color32::from_rgb(0x16, 0x16, 0x2e);
pub const BG_DETAIL: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x0d, 0x22);

// Accent
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xaa);
pub const ACCENT_DIM: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x00, 0x54, 0x44, 0x44);

// Text
pub const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(0xcc, 0xcc, 0xcc);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x63, 0x6e, 0x72);
pub const TEXT_DISABLED: egui::Color32 = egui::Color32::from_rgba_premultiplied(0x32, 0x37, 0x39, 0x66);

// Font setup
pub const FONT_NAME: &str = "JetBrains Mono";

/// Register JetBrains Mono as the default font.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/JetBrainsMono-Regular.ttf"
        ))),
    );

    // Put it first in the proportional and monospace families
    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, FONT_NAME.to_owned());

    ctx.set_fonts(fonts);
}

use std::sync::Arc;

/// Configure the egui visual style for Autokit's dark theme.
pub fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let visuals = &mut style.visuals;

    visuals.dark_mode = true;
    visuals.panel_fill = BG_MAIN;
    visuals.window_fill = BG_MAIN;
    visuals.extreme_bg_color = BG_MAIN;

    // Widget backgrounds
    visuals.widgets.inactive.bg_fill = BG_ROW;
    visuals.widgets.hovered.bg_fill = BG_ROW_HOVER;
    visuals.widgets.active.bg_fill = BG_ROW_HOVER;

    // Rounding
    visuals.widgets.inactive.rounding = egui::Rounding::same(3.0);
    visuals.widgets.hovered.rounding = egui::Rounding::same(3.0);
    visuals.widgets.active.rounding = egui::Rounding::same(3.0);

    // Stroke
    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

    // Scrollbar
    visuals.selection.bg_fill = ACCENT;

    ctx.set_style(style);
}
```

- [ ] **Step 2: Download JetBrains Mono font**

```bash
mkdir -p /home/natalia/repos/Autokit/assets
curl -L 'https://github.com/JetBrains/JetBrainsMono/raw/master/fonts/ttf/JetBrainsMono-Regular.ttf' \
  -o /home/natalia/repos/Autokit/assets/JetBrainsMono-Regular.ttf
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -5`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/ui/theme.rs assets/JetBrainsMono-Regular.ttf
git commit -m "feat(ui): extend theme with egui colors, font setup, and dark style"
```

---

### Task 4: Waveform painter widget

**Files:**
- Create: `src/ui/waveform.rs`
- Modify: `src/lib.rs:17-21`

- [ ] **Step 1: Create `src/ui/waveform.rs`**

```rust
use nih_plug_egui::egui;

use crate::ui::state::WaveformSummary;

/// Paint a line waveform from a WaveformSummary.
/// Draws a polyline of midpoints through the min/max pairs.
pub fn paint_waveform(
    ui: &mut egui::Ui,
    summary: Option<&WaveformSummary>,
    color: egui::Color32,
    desired_size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter_at(rect);

    match summary {
        Some(summary) if !summary.points.is_empty() => {
            let n = summary.points.len();
            let step = rect.width() / n as f32;

            let points: Vec<egui::Pos2> = summary
                .points
                .iter()
                .enumerate()
                .map(|(i, [min, max])| {
                    let x = rect.left() + (i as f32 + 0.5) * step;
                    let mid = (min + max) / 2.0;
                    // Map amplitude (-1..1) to y (bottom..top)
                    let y = rect.center().y - mid * (rect.height() / 2.0);
                    egui::pos2(x, y)
                })
                .collect();

            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.2, color),
            ));
        }
        _ => {
            // No sample loaded — draw a dim center line
            let center_y = rect.center().y;
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 4.0, center_y),
                    egui::pos2(rect.right() - 4.0, center_y),
                ],
                egui::Stroke::new(0.5, color.linear_multiply(0.2)),
            );
        }
    }

    response
}
```

- [ ] **Step 2: Add `waveform` module to `src/lib.rs`**

Update the `mod ui` block:

```rust
mod ui {
    pub mod state;
    pub mod theme;
    pub mod waveform;
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -5`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/ui/waveform.rs src/lib.rs
git commit -m "feat(ui): add waveform painter widget"
```

---

### Task 5: Custom knob widget

**Files:**
- Create: `src/ui/knob.rs`
- Modify: `src/lib.rs:17-22`

- [ ] **Step 1: Create `src/ui/knob.rs`**

```rust
use nih_plug_egui::egui;

/// Response from a knob interaction.
pub struct KnobResponse {
    /// The new value after dragging, if changed.
    pub changed: bool,
    /// Whether the user ctrl+clicked to reset.
    pub reset: bool,
}

/// Draw a circular knob with a value display.
///
/// - `value`: current value (mutable, will be clamped to min..=max)
/// - `min`, `max`: range
/// - `default`: value to reset to on ctrl+click
/// - `label`: text below the knob (e.g. "VOL")
/// - `format_value`: closure to format the display string
/// - `ring_color`: color for the knob ring
/// - `diameter`: knob diameter in pixels
pub fn knob(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    label: &str,
    format_value: impl Fn(f32) -> String,
    ring_color: egui::Color32,
    diameter: f32,
) -> KnobResponse {
    let mut result = KnobResponse {
        changed: false,
        reset: false,
    };

    ui.vertical(|ui| {
        ui.set_width(diameter + 4.0);

        // Knob circle
        let size = egui::vec2(diameter, diameter);
        let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
        let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);

        // Ctrl+click to reset
        if response.clicked() && ui.input(|i| i.modifiers.ctrl) {
            *value = default;
            result.changed = true;
            result.reset = true;
        }

        // Vertical drag to change value
        if response.dragged() {
            let delta = -response.drag_delta().y;
            let speed = if ui.input(|i| i.modifiers.shift) {
                0.001 // Fine control
            } else {
                0.005
            };
            *value = (*value + delta * speed * (max - min)).clamp(min, max);
            result.changed = true;
        }

        // Paint the knob
        if ui.is_rect_visible(rect) {
            let painter = ui.painter_at(rect);
            let center = rect.center();
            let radius = diameter / 2.0 - 1.0;

            // Ring
            painter.circle_stroke(center, radius, egui::Stroke::new(2.0, ring_color));

            // Value text
            let text = format_value(*value);
            painter.text(
                center,
                egui::Align2::CENTER_CENTER,
                &text,
                egui::FontId::new(9.0, egui::FontFamily::Monospace),
                ring_color,
            );
        }

        // Label below
        ui.add_space(3.0);
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(
                egui::RichText::new(label)
                    .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                    .color(crate::ui::theme::TEXT_DIM),
            );
        });
    });

    result
}
```

- [ ] **Step 2: Add `knob` module to `src/lib.rs`**

Update the `mod ui` block:

```rust
mod ui {
    pub mod knob;
    pub mod state;
    pub mod theme;
    pub mod waveform;
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -5`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/ui/knob.rs src/lib.rs
git commit -m "feat(ui): add custom knob widget with drag and ctrl+click reset"
```

---

### Task 6: Toolbar rendering

**Files:**
- Create: `src/ui/toolbar.rs`
- Modify: `src/lib.rs:17-23`

- [ ] **Step 1: Create `src/ui/toolbar.rs`**

```rust
use nih_plug::prelude::*;
use nih_plug_egui::egui;
use std::sync::Arc;

use crate::engine::sequencer::Sequencer;
use crate::plugin::AutokitParams;
use crate::ui::state::{ScanStatus, SharedState};
use crate::ui::theme;

/// Actions the toolbar can trigger.
pub enum ToolbarAction {
    None,
    Undo,
    Redo,
    DiceAll,
    LockAll,
    SetScale(f32),
}

/// Draw the toolbar. Returns an action if a button was clicked.
pub fn draw_toolbar(
    ui: &mut egui::Ui,
    shared: &SharedState,
    params: &AutokitParams,
    setter: &ParamSetter,
    current_scale: f32,
) -> ToolbarAction {
    let mut action = ToolbarAction::None;

    egui::Frame::none()
        .fill(theme::BG_TOOLBAR)
        .inner_margin(egui::Margin::symmetric(16.0, 8.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                ui.spacing_mut().item_spacing.x = 8.0;

                // Left: logo + scan status
                ui.label(
                    egui::RichText::new("AUTOKIT")
                        .font(egui::FontId::new(15.0, egui::FontFamily::Monospace))
                        .color(theme::ACCENT)
                        .strong(),
                );
                ui.label(
                    egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DISABLED),
                );

                match &shared.scan_status {
                    ScanStatus::Scanning => {
                        ui.label(
                            egui::RichText::new("scanning...")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DIM),
                        );
                    }
                    ScanStatus::Ready { total } => {
                        ui.label(
                            egui::RichText::new(format!("{total} samples"))
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(theme::ACCENT),
                        );
                    }
                }

                ui.add_space(ui.available_width() - 380.0); // Push center/right sections

                // Center: undo/redo + dice/lock
                let undo_color = if shared.history.can_undo() {
                    theme::TEXT_DIM
                } else {
                    theme::TEXT_DISABLED
                };
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("UNDO")
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(undo_color),
                    ).fill(theme::BG_ROW).min_size(egui::vec2(44.0, 22.0)))
                    .clicked()
                    && shared.history.can_undo()
                {
                    action = ToolbarAction::Undo;
                }

                let redo_color = if shared.history.can_redo() {
                    theme::TEXT_DIM
                } else {
                    theme::TEXT_DISABLED
                };
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("REDO")
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(redo_color),
                    ).fill(theme::BG_ROW).min_size(egui::vec2(44.0, 22.0)))
                    .clicked()
                    && shared.history.can_redo()
                {
                    action = ToolbarAction::Redo;
                }

                // Divider
                ui.add(egui::Separator::default().vertical().spacing(4.0));

                // Dice All
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new("DICE ALL")
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(theme::ACCENT)
                            .strong(),
                    ).fill(theme::ACCENT_DIM).min_size(egui::vec2(60.0, 22.0)))
                    .clicked()
                {
                    action = ToolbarAction::DiceAll;
                }

                // Lock All
                let all_locked = shared.kit.pads.iter().all(|p| p.locked);
                let lock_label = if all_locked { "UNLOCK ALL" } else { "LOCK ALL" };
                if ui
                    .add(egui::Button::new(
                        egui::RichText::new(lock_label)
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DIM),
                    ).fill(theme::BG_ROW).min_size(egui::vec2(60.0, 22.0)))
                    .clicked()
                {
                    action = ToolbarAction::LockAll;
                }

                // Divider
                ui.add(egui::Separator::default().vertical().spacing(4.0));

                // Master volume
                ui.label(
                    egui::RichText::new("MASTER")
                        .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DISABLED),
                );

                let mut gain_db = util::gain_to_db(params.master_volume.value());
                let slider = egui::Slider::new(&mut gain_db, -60.0..=6.0)
                    .show_value(false)
                    .trailing_fill(true);
                if ui.add(slider).changed() {
                    setter.begin_set_parameter(&params.master_volume);
                    setter.set_parameter(&params.master_volume, util::db_to_gain(gain_db));
                    setter.end_set_parameter(&params.master_volume);
                }

                ui.label(
                    egui::RichText::new(format!("{gain_db:.1}dB"))
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                        .color(theme::ACCENT),
                );

                // Scale selector
                let scale_label = format!("{}%", (current_scale * 100.0) as u32);
                egui::ComboBox::from_id_salt("scale")
                    .selected_text(
                        egui::RichText::new(&scale_label)
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DIM),
                    )
                    .width(50.0)
                    .show_ui(ui, |ui| {
                        for &s in &[1.0f32, 1.25, 1.5] {
                            let label = format!("{}%", (s * 100.0) as u32);
                            if ui.selectable_label((current_scale - s).abs() < 0.01, &label).clicked() {
                                action = ToolbarAction::SetScale(s);
                            }
                        }
                    });
            });
        });

    action
}
```

- [ ] **Step 2: Add `toolbar` module to `src/lib.rs`**

Update the `mod ui` block:

```rust
mod ui {
    pub mod knob;
    pub mod state;
    pub mod theme;
    pub mod toolbar;
    pub mod waveform;
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -5`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/ui/toolbar.rs src/lib.rs
git commit -m "feat(ui): add toolbar with undo/redo, dice, lock, master volume, scale"
```

---

### Task 7: Pad row rendering (collapsed + expanded)

**Files:**
- Create: `src/ui/pad_row.rs`
- Modify: `src/lib.rs:17-24`

- [ ] **Step 1: Create `src/ui/pad_row.rs`**

```rust
use nih_plug_egui::egui;

use crate::engine::kit::DrumPad;
use crate::ui::knob;
use crate::ui::state::WaveformSummary;
use crate::ui::theme::{self, category_color};
use crate::ui::waveform;

/// Actions that a pad row can trigger.
pub enum PadRowAction {
    None,
    /// Toggle expand/collapse for this pad.
    ToggleExpand,
    /// Quick-dice this pad (from the inline button).
    DicePad,
    /// Dice all pads of this pad's category.
    DiceCategory,
    /// Toggle lock on this pad.
    ToggleLock,
    /// Volume changed.
    SetVolume(f32),
    /// Pan changed.
    SetPan(f32),
    /// Pitch changed.
    SetPitch(f32),
}

/// Draw a single collapsed pad row.
/// Returns the action triggered (if any).
pub fn draw_collapsed(
    ui: &mut egui::Ui,
    index: usize,
    pad: &DrumPad,
    waveform_summary: Option<&WaveformSummary>,
    is_selected: bool,
) -> PadRowAction {
    let mut action = PadRowAction::None;
    let cat_color = category_color(pad.category);
    let cat_egui = cat_color.to_egui();
    let waveform_opacity = if is_selected { 0.85 } else { 0.5 };

    let bg = if is_selected {
        theme::BG_ROW_HOVER
    } else {
        theme::BG_ROW
    };

    // Outer frame for the row
    egui::Frame::none()
        .fill(bg)
        .rounding(egui::Rounding {
            nw: 0.0,
            ne: 3.0,
            se: 3.0,
            sw: 0.0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(34.0);
                ui.spacing_mut().item_spacing.x = 0.0;

                // Color strip (3px)
                let (strip_rect, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, 34.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(strip_rect, egui::Rounding::ZERO, cat_egui);

                ui.add_space(10.0);

                // Clickable area for the main content
                let content_response = ui
                    .horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;

                        // Category tag
                        let tag_size = egui::vec2(46.0, 16.0);
                        let (tag_rect, _) =
                            ui.allocate_exact_size(tag_size, egui::Sense::hover());
                        if ui.is_rect_visible(tag_rect) {
                            let painter = ui.painter_at(tag_rect);
                            painter.rect_filled(tag_rect, 2.0, cat_egui);
                            painter.text(
                                tag_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                pad.category.label(),
                                egui::FontId::new(8.0, egui::FontFamily::Monospace),
                                theme::BG_MAIN,
                            );
                        }

                        // Sample name
                        let name = if pad.sample.is_some() {
                            &pad.name
                        } else {
                            "—"
                        };
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(name)
                                    .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                                    .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa)),
                            )
                            .truncate()
                            .min_size(egui::vec2(170.0, 0.0)),
                        );

                        // Waveform
                        let waveform_width = (ui.available_width() - 100.0).max(80.0);
                        let wf_color = cat_color.to_egui_alpha((waveform_opacity * 255.0) as u8);
                        waveform::paint_waveform(
                            ui,
                            waveform_summary,
                            wf_color,
                            egui::vec2(waveform_width, 26.0),
                        );

                        // Volume bar
                        let vol_size = egui::vec2(50.0, 3.0);
                        let (vol_rect, _) =
                            ui.allocate_exact_size(vol_size, egui::Sense::hover());
                        if ui.is_rect_visible(vol_rect) {
                            let painter = ui.painter_at(vol_rect);
                            painter.rect_filled(vol_rect, 2.0, theme::BG_MAIN);
                            let fill_width = vol_rect.width() * pad.volume;
                            let fill_rect = egui::Rect::from_min_size(
                                vol_rect.min,
                                egui::vec2(fill_width, vol_rect.height()),
                            );
                            let fill_color = cat_color.to_egui_alpha(0x66);
                            // Glow: wider rect behind at low opacity
                            let glow_rect = fill_rect.expand2(egui::vec2(0.0, 1.5));
                            painter.rect_filled(glow_rect, 2.0, cat_color.to_egui_alpha(0x18));
                            painter.rect_filled(fill_rect, 2.0, fill_color);
                        }
                    })
                    .response;

                // Check if the main content area was clicked
                if content_response.interact(egui::Sense::click()).clicked() {
                    action = PadRowAction::ToggleExpand;
                }

                // Dice button (right side)
                ui.add_space(2.0);
                let dice_response = ui.add(
                    egui::Button::new(
                        egui::RichText::new("⚄")
                            .font(egui::FontId::new(13.0, egui::FontFamily::Monospace))
                            .color(theme::ACCENT.linear_multiply(0.4)),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(28.0, 34.0)),
                );
                if dice_response.clicked() {
                    action = PadRowAction::DicePad;
                }
            });
        });

    action
}

/// Draw the expanded detail panel for a pad.
/// Returns the action triggered (if any).
pub fn draw_expanded(
    ui: &mut egui::Ui,
    index: usize,
    pad: &DrumPad,
) -> PadRowAction {
    let mut action = PadRowAction::None;
    let cat_color = category_color(pad.category);
    let cat_egui = cat_color.to_egui();

    egui::Frame::none()
        .fill(theme::BG_DETAIL)
        .inner_margin(egui::Margin::symmetric(16.0, 10.0))
        .rounding(egui::Rounding {
            nw: 0.0,
            ne: 3.0,
            se: 3.0,
            sw: 0.0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Left border continuation
                let (strip_rect, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, 50.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    strip_rect,
                    egui::Rounding::ZERO,
                    cat_color.to_egui_alpha(0x33),
                );

                ui.add_space(12.0);

                // Knobs — vertically centered
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;

                    // Volume knob
                    let mut vol = pad.volume;
                    let vol_result = knob::knob(
                        ui,
                        egui::Id::new(("vol", index)),
                        &mut vol,
                        0.0,
                        1.0,
                        1.0,
                        "VOL",
                        |v| format!("{}", (v * 100.0) as u32),
                        cat_egui,
                        34.0,
                    );
                    if vol_result.changed {
                        action = PadRowAction::SetVolume(vol);
                    }

                    // Pan knob
                    let mut pan = pad.pan;
                    let pan_result = knob::knob(
                        ui,
                        egui::Id::new(("pan", index)),
                        &mut pan,
                        -1.0,
                        1.0,
                        0.0,
                        "PAN",
                        |v| {
                            if v.abs() < 0.01 {
                                "C".to_string()
                            } else if v < 0.0 {
                                format!("L{}", (-v * 100.0) as u32)
                            } else {
                                format!("R{}", (v * 100.0) as u32)
                            }
                        },
                        cat_color.to_egui_alpha(0x88),
                        34.0,
                    );
                    if pan_result.changed {
                        action = PadRowAction::SetPan(pan);
                    }

                    // Pitch knob
                    let mut pitch = pad.pitch;
                    let pitch_result = knob::knob(
                        ui,
                        egui::Id::new(("pitch", index)),
                        &mut pitch,
                        -24.0,
                        24.0,
                        0.0,
                        "PITCH",
                        |v| format!("{:+.0}", v),
                        cat_color.to_egui_alpha(0x88),
                        34.0,
                    );
                    if pitch_result.changed {
                        action = PadRowAction::SetPitch(pitch);
                    }

                    // Divider
                    ui.add(egui::Separator::default().vertical().spacing(8.0));

                    // Lock checkbox
                    let lock_color = if pad.locked {
                        theme::ACCENT
                    } else {
                        theme::TEXT_DISABLED
                    };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(if pad.locked { "🔒 LOCK" } else { "LOCK" })
                                    .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                                    .color(lock_color),
                            )
                            .fill(egui::Color32::TRANSPARENT),
                        )
                        .clicked()
                    {
                        action = PadRowAction::ToggleLock;
                    }

                    // Dice pad button
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("DICE PAD")
                                    .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                                    .color(theme::ACCENT),
                            )
                            .fill(theme::ACCENT_DIM)
                            .rounding(3.0),
                        )
                        .clicked()
                    {
                        action = PadRowAction::DicePad;
                    }

                    // Dice category button
                    let cat_label = format!("DICE {}S", pad.category.label());
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(&cat_label)
                                    .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                                    .color(cat_egui),
                            )
                            .fill(cat_color.to_egui_alpha(0x11))
                            .rounding(3.0),
                        )
                        .clicked()
                    {
                        action = PadRowAction::DiceCategory;
                    }
                });
            });
        });

    action
}
```

- [ ] **Step 2: Add `pad_row` module to `src/lib.rs`**

Update the `mod ui` block:

```rust
mod ui {
    pub mod knob;
    pub mod pad_row;
    pub mod state;
    pub mod theme;
    pub mod toolbar;
    pub mod waveform;
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -5`
Expected: compiles.

- [ ] **Step 4: Commit**

```bash
git add src/ui/pad_row.rs src/lib.rs
git commit -m "feat(ui): add pad row widget with collapsed and expanded states"
```

---

### Task 8: Editor — wire up the GUI

**Files:**
- Create: `src/ui/editor.rs`
- Modify: `src/lib.rs:17-25`
- Modify: `src/plugin.rs` (add `editor()` method, persist `EguiState`)

- [ ] **Step 1: Create `src/ui/editor.rs`**

```rust
use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::plugin::AutokitParams;
use crate::ui::pad_row::{self, PadRowAction};
use crate::ui::state::SharedState;
use crate::ui::toolbar::{self, ToolbarAction};
use crate::ui::theme;
use crate::util::history::HistorySnapshot;

/// Number of points in waveform summaries.
const WAVEFORM_POINTS: usize = 200;

/// GUI-only state (not shared with audio thread).
pub struct EditorState {
    /// Which pad is expanded (None = all collapsed).
    pub selected_pad: Option<usize>,
    /// Current UI scale factor.
    pub scale: f32,
    /// Whether fonts/style have been initialized.
    pub initialized: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected_pad: None,
            scale: 1.0,
            initialized: false,
        }
    }
}

/// Create the egui editor for the Autokit plugin.
pub fn create(
    egui_state: Arc<EguiState>,
    shared: Arc<Mutex<SharedState>>,
    params: Arc<AutokitParams>,
    sequencer_snapshot_fn: Arc<dyn Fn() -> crate::util::history::SequencerSnapshot + Send + Sync>,
) -> Option<Box<dyn Editor>> {
    create_egui_editor(
        egui_state,
        EditorState::default(),
        // Build (called once when GUI opens)
        |ctx, _state| {
            theme::setup_fonts(ctx);
            theme::setup_style(ctx);
        },
        // Update (called every frame)
        move |ctx, setter, state| {
            if !state.initialized {
                ctx.set_pixels_per_point(state.scale);
                state.initialized = true;
            }

            egui::CentralPanel::default()
                .frame(egui::Frame::none().fill(theme::BG_MAIN))
                .show(ctx, |ui| {
                    let mut shared = shared.lock();

                    // Toolbar
                    let toolbar_action = toolbar::draw_toolbar(
                        ui,
                        &shared,
                        &params,
                        setter,
                        state.scale,
                    );

                    match toolbar_action {
                        ToolbarAction::Undo => {
                            let current = HistorySnapshot {
                                pads: shared.kit.snapshot(),
                                sequencer: sequencer_snapshot_fn(),
                            };
                            if let Some(restored) = shared.history.undo(current) {
                                shared.kit.restore(&restored.pads);
                                shared.update_all_waveforms(WAVEFORM_POINTS);
                                // Note: sequencer restore happens on audio thread
                            }
                        }
                        ToolbarAction::Redo => {
                            let current = HistorySnapshot {
                                pads: shared.kit.snapshot(),
                                sequencer: sequencer_snapshot_fn(),
                            };
                            if let Some(restored) = shared.history.redo(current) {
                                shared.kit.restore(&restored.pads);
                                shared.update_all_waveforms(WAVEFORM_POINTS);
                            }
                        }
                        ToolbarAction::DiceAll => {
                            if shared.library.is_some() {
                                let snapshot = HistorySnapshot {
                                    pads: shared.kit.snapshot(),
                                    sequencer: sequencer_snapshot_fn(),
                                };
                                shared.history.push(snapshot);
                                // Borrow workaround: clone library ref
                                let lib = shared.library.as_ref().unwrap().clone_for_dice();
                                shared.kit.dice_all(&lib);
                                shared.update_all_waveforms(WAVEFORM_POINTS);
                            }
                        }
                        ToolbarAction::LockAll => {
                            let all_locked = shared.kit.pads.iter().all(|p| p.locked);
                            for pad in &mut shared.kit.pads {
                                pad.locked = !all_locked;
                            }
                        }
                        ToolbarAction::SetScale(s) => {
                            state.scale = s;
                            ctx.set_pixels_per_point(s);
                        }
                        ToolbarAction::None => {}
                    }

                    // Separator line
                    ui.add(egui::Separator::default().spacing(0.0));

                    // Pad list
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;

                            for i in 0..16 {
                                let is_selected = state.selected_pad == Some(i);
                                let pad = &shared.kit.pads[i];
                                let wf = shared.waveforms[i].as_ref();

                                let row_action = pad_row::draw_collapsed(
                                    ui, i, pad, wf, is_selected,
                                );

                                match row_action {
                                    PadRowAction::ToggleExpand => {
                                        state.selected_pad = if is_selected { None } else { Some(i) };
                                    }
                                    PadRowAction::DicePad => {
                                        if shared.library.is_some() {
                                            let snapshot = HistorySnapshot {
                                                pads: shared.kit.snapshot(),
                                                sequencer: sequencer_snapshot_fn(),
                                            };
                                            shared.history.push(snapshot);
                                            let lib = shared.library.as_ref().unwrap().clone_for_dice();
                                            shared.kit.dice_pad(i, &lib);
                                            shared.update_waveform(i, WAVEFORM_POINTS);
                                        }
                                    }
                                    _ => {}
                                }

                                // Expanded detail
                                if is_selected {
                                    let pad = &shared.kit.pads[i];
                                    let detail_action = pad_row::draw_expanded(ui, i, pad);

                                    match detail_action {
                                        PadRowAction::SetVolume(v) => {
                                            shared.kit.pads[i].volume = v;
                                        }
                                        PadRowAction::SetPan(v) => {
                                            shared.kit.pads[i].pan = v;
                                        }
                                        PadRowAction::SetPitch(v) => {
                                            shared.kit.pads[i].pitch = v;
                                        }
                                        PadRowAction::ToggleLock => {
                                            shared.kit.toggle_lock(i);
                                        }
                                        PadRowAction::DicePad => {
                                            if shared.library.is_some() {
                                                let snapshot = HistorySnapshot {
                                                    pads: shared.kit.snapshot(),
                                                    sequencer: sequencer_snapshot_fn(),
                                                };
                                                shared.history.push(snapshot);
                                                let lib = shared.library.as_ref().unwrap().clone_for_dice();
                                                shared.kit.dice_pad(i, &lib);
                                                shared.update_waveform(i, WAVEFORM_POINTS);
                                            }
                                        }
                                        PadRowAction::DiceCategory => {
                                            if shared.library.is_some() {
                                                let cat = shared.kit.pads[i].category;
                                                let snapshot = HistorySnapshot {
                                                    pads: shared.kit.snapshot(),
                                                    sequencer: sequencer_snapshot_fn(),
                                                };
                                                shared.history.push(snapshot);
                                                let lib = shared.library.as_ref().unwrap().clone_for_dice();
                                                shared.kit.dice_category(cat, &lib);
                                                shared.update_all_waveforms(WAVEFORM_POINTS);
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        });
                });
        },
    )
}
```

- [ ] **Step 2: Add a `clone_for_dice()` method to SampleLibrary**

In `src/analysis/library.rs`, add this method to `impl SampleLibrary` (after `generate_kit()`):

```rust
    /// Create a reference-only clone for use in dice operations.
    /// The Arc'd sample data is shared, not copied.
    pub fn clone_for_dice(&self) -> SampleLibrary {
        SampleLibrary {
            total: self.total,
            by_category: self.by_category.clone(),
            sample_rate: self.sample_rate,
        }
    }
```

- [ ] **Step 3: Add `editor` module to `src/lib.rs`**

Update the `mod ui` block:

```rust
mod ui {
    pub mod editor;
    pub mod knob;
    pub mod pad_row;
    pub mod state;
    pub mod theme;
    pub mod toolbar;
    pub mod waveform;
}
```

- [ ] **Step 4: Add `editor()` method to plugin.rs**

Add to `AutokitParams`:

```rust
#[derive(Params)]
pub struct AutokitParams {
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,

    #[id = "master_vol"]
    pub master_volume: FloatParam,
}
```

Update `Default for AutokitParams`:

```rust
impl Default for AutokitParams {
    fn default() -> Self {
        Self {
            editor_state: EguiState::from_size(900, 700),
            master_volume: FloatParam::new(
                // ... (same as before)
            )
            // ... (same as before)
        }
    }
}
```

Add the `editor()` method to `impl Plugin for Autokit` (after `fn params()`):

```rust
    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        let shared = Arc::clone(&self.shared);
        let params = Arc::clone(&self.params);
        let sequencer = &self.sequencer as *const _ as usize; // Will use a proper snapshot fn

        // Create a snapshot function that captures sequencer state.
        // The sequencer lives on the audio thread; we snapshot it via a closure.
        // Since undo/redo is triggered from GUI, and the sequencer state is only
        // read (not mutated) by the GUI, we store a snapshot each time the GUI opens.
        let seq_snapshot = {
            let seq = self.sequencer.snapshot();
            Arc::new(move || seq.clone())
        };

        crate::ui::editor::create(
            self.params.editor_state.clone(),
            shared,
            params,
            seq_snapshot,
        )
    }
```

Add the necessary import at the top of `plugin.rs`:

```rust
use nih_plug_egui::EguiState;
```

- [ ] **Step 5: Verify it compiles**

Run: `cd /home/natalia/repos/Autokit && cargo check 2>&1 | tail -15`
Expected: compiles. Fix any type mismatches or borrow issues.

- [ ] **Step 6: Run all tests**

Run: `cd /home/natalia/repos/Autokit && cargo test 2>&1 | tail -15`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
git add src/ui/editor.rs src/analysis/library.rs src/plugin.rs src/lib.rs
git commit -m "feat(ui): wire up egui editor with toolbar, pad list, and all interactions"
```

---

### Task 9: Build, bundle, and smoke test

**Files:**
- No new files — integration testing.

- [ ] **Step 1: Build debug bundle**

```bash
cd /home/natalia/repos/Autokit
cargo xtask bundle autokit 2>&1 | tail -10
```

Expected: bundle completes, `.vst3` and `.clap` in `target/bundled/`.

- [ ] **Step 2: Install to plugin dirs**

```bash
cp -r target/bundled/autokit.vst3 ~/.vst3/
cp -r target/bundled/autokit.clap ~/.clap/
```

- [ ] **Step 3: Test standalone**

```bash
cargo run --bin autokit-standalone -- --backend alsa --output-device pipewire 2>&1 | head -20
```

Expected: window opens at 900x700 with the pad strip GUI. Toolbar visible with AUTOKIT logo, scanning status. 16 pad rows appear (initially empty, then populated when scan completes). Close with Ctrl+C.

- [ ] **Step 4: Verify in Renoise**

Open Renoise, load Autokit as VST3 instrument. Verify:
- GUI opens and displays correctly.
- Category colors visible on pad rows.
- Clicking a row expands the detail panel with knobs.
- Dice buttons work (samples change, waveforms update).
- Undo/redo restores previous state.
- Scale dropdown changes UI size.
- Master volume slider affects output.

- [ ] **Step 5: Fix any issues found during smoke test**

Address any visual glitches, interaction bugs, or crashes found during testing.

- [ ] **Step 6: Commit fixes if any**

```bash
git add -u
git commit -m "fix(ui): address smoke test issues"
```

---

### Task 10: Update memory and project docs

**Files:**
- Modify: `/home/natalia/.claude/projects/-home-natalia-repos/memory/project_autokit.md`

- [ ] **Step 1: Update project memory**

Update the Autokit memory file to reflect Phase 6 completion:
- Phase 6 status: DONE
- New source files added (`src/ui/editor.rs`, `knob.rs`, `pad_row.rs`, `toolbar.rs`, `waveform.rs`, `state.rs`)
- Architecture note: `SharedState` in `Arc<Mutex<>>` for GUI↔audio communication
- Font: JetBrains Mono bundled in `assets/`

- [ ] **Step 2: Commit**

```bash
git add -u
git commit -m "docs: update project status for Phase 6 completion"
```
