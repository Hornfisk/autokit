use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use std::path::PathBuf;

use crate::engine::kit::{SampleCategory, NUM_PADS};
use crate::plugin::AutokitParams;
use crate::ui::pad_row::{self, PadRowAction};
use crate::ui::sample_map::{self, MapPoint};
use crate::ui::state::{ScanStatus, SharedState, WaveformSummary};
use crate::ui::theme;
use crate::ui::toolbar::{self, ToolbarAction};
use crate::util::history::HistorySnapshot;
use crate::util::preset;

/// Number of points in waveform summaries.
const WAVEFORM_POINTS: usize = 200;

/// Lightweight snapshot of one pad for display — no sample data, just metadata.
struct PadDisplay {
    name: String,
    category: SampleCategory,
    has_sample: bool,
    locked: bool,
    volume: f32,
    pan: f32,
    pitch: f32,
    decay: f32,
}

/// Everything the GUI needs to render one frame, captured in a brief lock.
struct DisplaySnapshot {
    pads: [PadDisplay; NUM_PADS],
    waveforms: [Option<WaveformSummary>; NUM_PADS],
    scan_status: ScanStatus,
    can_undo: bool,
    can_redo: bool,
    has_library: bool,
}

impl DisplaySnapshot {
    fn from_shared(shared: &SharedState) -> Self {
        let pads = core::array::from_fn(|i| {
            let pad = &shared.kit.pads[i];
            PadDisplay {
                name: pad.name.clone(),
                category: pad.category,
                has_sample: pad.sample.is_some(),
                locked: pad.locked,
                volume: pad.volume,
                pan: pad.pan,
                pitch: pad.pitch,
                decay: pad.decay,
            }
        });
        Self {
            pads,
            waveforms: shared.waveforms.clone(),
            scan_status: shared.scan_status.clone(),
            can_undo: shared.history.can_undo(),
            can_redo: shared.history.can_redo(),
            has_library: shared.library.is_some(),
        }
    }
}

/// Keyboard keys mapped to pads (ISO keyboard bottom row: z x c v b n m ,)
const PAD_KEYS: [egui::Key; NUM_PADS] = [
    egui::Key::Z,
    egui::Key::X,
    egui::Key::C,
    egui::Key::V,
    egui::Key::B,
    egui::Key::N,
    egui::Key::M,
    egui::Key::Comma,
];

/// Which view is currently active.
#[derive(Clone, Copy, PartialEq)]
pub enum ViewMode {
    PadStrip,
    SampleMap,
    Sequencer,
}

/// GUI-only state (not shared with audio thread).
pub struct EditorState {
    /// Which pad is expanded (None = all collapsed).
    pub selected_pad: Option<usize>,
    /// Current UI scale factor (75%-150%).
    pub scale: f32,
    /// Last-seen trigger counter values — used to detect new triggers.
    pub last_trigger: [u8; NUM_PADS],
    /// Per-pad flash brightness for play animation (0.0 = dark, 1.0 = full flash).
    pub brightness: [f32; NUM_PADS],
    /// Whether the save-preset dialog is open.
    pub show_save_dialog: bool,
    /// Text input for preset name in save dialog.
    pub save_name: String,
    /// Whether the load-preset dialog is open.
    pub show_load_dialog: bool,
    /// Cached list of available presets (refreshed when load dialog opens).
    pub preset_list: Vec<(String, PathBuf)>,
    /// Status message shown briefly after save/load.
    pub status_message: Option<String>,
    /// Which view is active (pad strip or sample map scatter plot).
    pub view_mode: ViewMode,
    /// Cached scatter plot points (built lazily when map view first opened).
    pub map_points: Vec<MapPoint>,
    /// Whether map points have been built for the current library.
    pub map_built: bool,
    /// Zoom/pan state for the sample map view.
    pub map_view: sample_map::MapViewState,
    /// Index of the hovered dot in the map view.
    pub map_hovered: Option<usize>,
    /// Assignment popup state for the sample map.
    pub map_popup: sample_map::PopupState,
    /// Which pad is in shortcut-assign mode (click dot = assign to this pad).
    pub map_shortcut_pad: Option<usize>,
    /// Sequencer view state.
    pub seq_view: crate::ui::sequencer_ui::SeqViewState,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected_pad: None,
            scale: 1.0,
            last_trigger: [0u8; NUM_PADS],
            brightness: [0.0f32; NUM_PADS],
            show_save_dialog: false,
            save_name: String::new(),
            show_load_dialog: false,
            preset_list: Vec::new(),
            status_message: None,
            view_mode: ViewMode::PadStrip,
            map_points: Vec::new(),
            map_built: false,
            map_view: sample_map::MapViewState::default(),
            map_hovered: None,
            map_popup: sample_map::PopupState::default(),
            map_shortcut_pad: None,
            seq_view: Default::default(),
        }
    }
}

/// Create the egui editor for the Autokit plugin.
pub fn create(
    egui_state: Arc<EguiState>,
    shared: Arc<Mutex<SharedState>>,
    params: Arc<AutokitParams>,
    trigger_flags: Arc<[AtomicU8; NUM_PADS]>,
    gui_triggers: Arc<[AtomicU8; NUM_PADS]>,
    seq_current_step: Arc<std::sync::atomic::AtomicUsize>,
    seq_playing: Arc<std::sync::atomic::AtomicBool>,
    seq_active_pattern: Arc<std::sync::atomic::AtomicUsize>,
    seq_fill_active: Arc<std::sync::atomic::AtomicBool>,
    seq_internal_play: Arc<std::sync::atomic::AtomicBool>,
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
            // --- Phase 1: Brief lock to snapshot display state ---
            let snap = {
                let shared = shared.lock();
                DisplaySnapshot::from_shared(&shared)
            };
            // Lock is now released — audio thread can proceed freely.

            // --- Pad activity animation ---
            // Read lockfree trigger counters; decay brightness each frame.
            {
                let dt = ctx.input(|i| i.predicted_dt);
                let mut any_active = false;
                for i in 0..NUM_PADS {
                    let current = trigger_flags[i].load(Ordering::Relaxed);
                    if current != state.last_trigger[i] {
                        state.brightness[i] = 1.0;
                        state.last_trigger[i] = current;
                    }
                    state.brightness[i] = (state.brightness[i] - dt * 5.0).max(0.0);
                    if state.brightness[i] > 0.0 {
                        any_active = true;
                    }
                }
                if any_active {
                    ctx.request_repaint();
                }
            }

            // --- Keyboard pad triggers ---
            // Check keyboard keys mapped to pads; set GUI trigger requests.
            // Use both Key enum matching AND raw text events to handle
            // different keyboard layouts (ISO-NOR reports ',' or ';' for
            // the physical key right of M depending on shift state).
            ctx.input(|input| {
                for (i, &key) in PAD_KEYS.iter().enumerate() {
                    if input.key_pressed(key) {
                        gui_triggers[i].store(1, Ordering::Relaxed);
                    }
                }
                // Fallback: check raw text/key events for the last pad
                if input.key_pressed(egui::Key::Semicolon) {
                    gui_triggers[NUM_PADS - 1].store(1, Ordering::Relaxed);
                }
                if input.key_pressed(egui::Key::Period) {
                    gui_triggers[NUM_PADS - 1].store(1, Ordering::Relaxed);
                }
                // Also check text events — catches any layout where the
                // physical key produces a character rather than a named key.
                for event in &input.events {
                    if let egui::Event::Text(text) = event {
                        match text.as_str() {
                            "," | ";" => {
                                gui_triggers[NUM_PADS - 1].store(1, Ordering::Relaxed);
                            }
                            _ => {}
                        }
                    }
                }
            });

            // Space bar toggles internal sequencer play/stop
            if ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                let current = seq_internal_play.load(Ordering::Relaxed);
                seq_internal_play.store(!current, Ordering::Relaxed);
            }

            // Collect any actions triggered during rendering.
            let mut pending_actions: Vec<GuiAction> = Vec::new();

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(theme::BG_MAIN))
                .show(ctx, |ui| {
                    // Toolbar (uses snapshot data, no mutex held)
                    let all_locked = snap.pads.iter().all(|p| p.locked);
                    let shortcut_info = state.map_shortcut_pad.map(|i| (i + 1, snap.pads[i].category.label()));
                    let toolbar_action = toolbar::draw_toolbar_snapshot(
                        ui,
                        &snap.scan_status,
                        snap.can_undo,
                        snap.can_redo,
                        all_locked,
                        &params,
                        setter,
                        state.scale,
                        state.view_mode,
                        shortcut_info,
                    );

                    match toolbar_action {
                        ToolbarAction::Undo => pending_actions.push(GuiAction::Undo),
                        ToolbarAction::Redo => pending_actions.push(GuiAction::Redo),
                        ToolbarAction::DiceAll => {
                            if snap.has_library {
                                pending_actions.push(GuiAction::DiceAll);
                            }
                        }
                        ToolbarAction::LockAll => pending_actions.push(GuiAction::LockAll),
                        ToolbarAction::SetScale(s) => {
                            state.scale = s;
                            ctx.set_pixels_per_point(s);
                        }
                        ToolbarAction::OpenSaveDialog => {
                            state.show_save_dialog = true;
                            state.show_load_dialog = false;
                        }
                        ToolbarAction::OpenLoadDialog => {
                            state.preset_list = preset::list_presets();
                            state.show_load_dialog = true;
                            state.show_save_dialog = false;
                        }
                        ToolbarAction::ToggleView => {
                            state.view_mode = match state.view_mode {
                                ViewMode::PadStrip => ViewMode::SampleMap,
                                ViewMode::SampleMap => ViewMode::Sequencer,
                                ViewMode::Sequencer => ViewMode::PadStrip,
                            };
                        }
                        ToolbarAction::SetView(mode) => {
                            state.view_mode = mode;
                        }
                        ToolbarAction::None => {}
                    }

                    // Separator line
                    ui.add(egui::Separator::default().spacing(0.0));

                    match state.view_mode {
                        ViewMode::PadStrip => {
                            // Pad list
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;

                                    for i in 0..NUM_PADS {
                                        let is_selected = state.selected_pad == Some(i);
                                        let pad = &snap.pads[i];
                                        let wf = snap.waveforms[i].as_ref();

                                        let row_action = pad_row::draw_collapsed_from_snapshot(
                                            ui, i, pad.has_sample, &pad.name, pad.category,
                                            pad.volume, wf, is_selected, state.brightness[i],
                                            pad.locked,
                                        );

                                        match row_action {
                                            PadRowAction::ToggleExpand => {
                                                state.selected_pad =
                                                    if is_selected { None } else { Some(i) };
                                            }
                                            PadRowAction::DicePad => {
                                                if snap.has_library {
                                                    pending_actions.push(GuiAction::DicePad(i));
                                                }
                                            }
                                            PadRowAction::PlayPad => {
                                                gui_triggers[i].store(1, Ordering::Relaxed);
                                            }
                                            PadRowAction::ToggleLock => {
                                                pending_actions.push(GuiAction::ToggleLock(i));
                                            }
                                            _ => {}
                                        }

                                        // Expanded detail (knobs + dice category)
                                        if is_selected {
                                            let detail_action = pad_row::draw_expanded_from_snapshot(
                                                ui, i, pad.category, pad.volume, pad.pan,
                                                pad.pitch, pad.decay,
                                            );

                                            match detail_action {
                                                PadRowAction::SetVolume(v) => {
                                                    pending_actions.push(GuiAction::SetPadVolume(i, v));
                                                }
                                                PadRowAction::SetPan(v) => {
                                                    pending_actions.push(GuiAction::SetPadPan(i, v));
                                                }
                                                PadRowAction::SetPitch(v) => {
                                                    pending_actions.push(GuiAction::SetPadPitch(i, v));
                                                }
                                                PadRowAction::SetDecay(v) => {
                                                    pending_actions.push(GuiAction::SetPadDecay(i, v));
                                                }
                                                PadRowAction::DiceCategory => {
                                                    if snap.has_library {
                                                        pending_actions.push(
                                                            GuiAction::DiceCategory(i, pad.category));
                                                    }
                                                }
                                                _ => {}
                                            }
                                        }
                                    }
                                });
                        }
                        ViewMode::SampleMap => {
                            // Build map points lazily on first view
                            if !state.map_built && snap.has_library {
                                let shared = shared.lock();
                                if let Some(ref lib) = shared.library {
                                    state.map_points = sample_map::build_map_points(lib);
                                    state.map_built = true;
                                }
                            }

                            // Collect kit sample names from snapshot for highlighting
                            let kit_paths: Vec<Option<String>> = snap.pads.iter().map(|p| {
                                if p.has_sample { Some(p.name.clone()) } else { None }
                            }).collect();

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

                            match map_action {
                                sample_map::MapAction::ClickedDot { point_index } => {
                                    let lib_index = state.map_points[point_index].library_index;
                                    if let Some(pad) = state.map_shortcut_pad {
                                        // Shortcut mode: assign directly + preview
                                        pending_actions.push(GuiAction::AssignFromMap { pad_index: pad, library_index: lib_index });
                                    } else {
                                        // Normal mode: preview + popup
                                        pending_actions.push(GuiAction::PreviewSample(lib_index));
                                        state.map_popup.active_point = Some(point_index);
                                        if let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) {
                                            state.map_popup.anchor_pos = cursor;
                                        }
                                    }
                                }
                                sample_map::MapAction::AssignToPad { .. } => {}
                                sample_map::MapAction::None => {}
                            }

                            // Separator
                            ui.add(egui::Separator::default().spacing(0.0));

                            // Mini pad bar
                            let pad_names: [String; NUM_PADS] = core::array::from_fn(|i| snap.pads[i].name.clone());
                            let pad_categories: [SampleCategory; NUM_PADS] = core::array::from_fn(|i| snap.pads[i].category);
                            let bar_action = sample_map::draw_mini_pad_bar(ui, &pad_names, &pad_categories, state.map_shortcut_pad);
                            match bar_action {
                                sample_map::PadBarAction::ToggleShortcut(i) => {
                                    state.map_shortcut_pad = if state.map_shortcut_pad == Some(i) { None } else { Some(i) };
                                }
                                sample_map::PadBarAction::None => {}
                            }

                            // Escape exits shortcut mode and closes popup
                            ctx.input(|input| {
                                if input.key_pressed(egui::Key::Escape) {
                                    state.map_shortcut_pad = None;
                                    state.map_popup.active_point = None;
                                }
                            });
                        }
                        ViewMode::Sequencer => {
                            // Build SeqDisplay from SharedState + atomics
                            let seq_display = {
                                let shared = shared.lock();
                                let bank = &shared.pattern_bank;
                                let active = seq_active_pattern.load(Ordering::Relaxed);
                                let active = active.min(bank.patterns.len().saturating_sub(1));
                                let pat = &bank.patterns[active];

                                crate::ui::sequencer_ui::SeqDisplay {
                                    current_step: seq_current_step.load(Ordering::Relaxed),
                                    playing: seq_playing.load(Ordering::Relaxed) || seq_internal_play.load(Ordering::Relaxed),
                                    active_pattern: active,
                                    queued_pattern: bank.queued,
                                    fill_active: seq_fill_active.load(Ordering::Relaxed),
                                    pattern_has_data: core::array::from_fn(|i| bank.patterns[i].has_data()),
                                    lanes: pat.lanes.iter().enumerate().map(|(i, lane)| {
                                        crate::ui::sequencer_ui::LaneDisplay {
                                            pad_name: snap.pads[i].name.clone(),
                                            category: snap.pads[i].category,
                                            muted: lane.muted,
                                            locked: snap.pads[i].locked,
                                            steps: core::array::from_fn(|j| crate::ui::sequencer_ui::StepDisplay {
                                                enabled: lane.steps[j].enabled,
                                                velocity: lane.steps[j].velocity,
                                                probability: lane.steps[j].probability,
                                                pan: lane.steps[j].pan,
                                                pitch: lane.steps[j].pitch,
                                                condition: lane.steps[j].condition,
                                            }),
                                        }
                                    }).collect(),
                                    swing: pat.swing,
                                }
                            };

                            {
                                use crate::ui::sequencer_ui::SeqAction;
                                for seq_action in crate::ui::sequencer_ui::draw_sequencer_view(
                                    ui, &seq_display, &mut state.seq_view,
                                ) {
                                    pending_actions.push(match seq_action {
                                        SeqAction::ToggleStep { lane, step } => GuiAction::SeqToggleStep { lane, step },
                                        SeqAction::SetStepEnabled { lane, step, enabled } => GuiAction::SeqSetStepEnabled { lane, step, enabled },
                                        SeqAction::SelectStep { .. } => GuiAction::None,
                                        SeqAction::SetStepVelocity { lane, step, value } => GuiAction::SeqSetStepVelocity { lane, step, value },
                                        SeqAction::SetStepPan { lane, step, value } => GuiAction::SeqSetStepPan { lane, step, value },
                                        SeqAction::SetStepPitch { lane, step, value } => GuiAction::SeqSetStepPitch { lane, step, value },
                                        SeqAction::SetStepProbability { lane, step, value } => GuiAction::SeqSetStepProbability { lane, step, value },
                                        SeqAction::SetStepCondition { lane, step, condition } => GuiAction::SeqSetStepCondition { lane, step, condition },
                                        SeqAction::ToggleLaneMute { lane } => GuiAction::SeqToggleLaneMute { lane },
                                        SeqAction::ToggleLaneLock { lane } => GuiAction::SeqToggleLaneLock { lane },
                                        SeqAction::SelectPattern { index } => GuiAction::SeqSelectPattern { index },
                                        SeqAction::SetSwing { value } => GuiAction::SeqSetSwing { value },
                                        SeqAction::CopyPattern => GuiAction::SeqCopyPattern,
                                        SeqAction::PastePattern => GuiAction::SeqPastePattern,
                                        SeqAction::ClearPattern => GuiAction::SeqClearPattern,
                                        SeqAction::DicePattern => GuiAction::SeqDicePattern,
                                        SeqAction::SetFillActive { active } => GuiAction::SeqSetFillActive { active },
                                        SeqAction::ToggleInternalPlay => GuiAction::SeqToggleInternalPlay,
                                    });
                                }
                            }
                        }
                    }
                });

            // --- Sample map assignment popup ---
            if state.view_mode == ViewMode::SampleMap && state.map_popup.active_point.is_some() {
                let pad_categories: [SampleCategory; NUM_PADS] = core::array::from_fn(|i| snap.pads[i].category);
                let map_rect = ctx.screen_rect();
                let popup_action = sample_map::draw_popup(ctx, &mut state.map_popup, &state.map_points, &pad_categories, map_rect);
                match popup_action {
                    sample_map::MapAction::AssignToPad { point_index, pad_index } => {
                        let lib_index = state.map_points[point_index].library_index;
                        pending_actions.push(GuiAction::AssignFromMap { pad_index, library_index: lib_index });
                    }
                    _ => {}
                }
            }

            // Dismiss popup on click outside
            if state.view_mode == ViewMode::SampleMap && state.map_popup.active_point.is_some()
                && ctx.input(|i| i.pointer.any_click()) && state.map_hovered.is_none()
            {
                state.map_popup.active_point = None;
            }

            // --- Save preset dialog ---
            if state.show_save_dialog {
                let mut open = true;
                egui::Window::new("Save Preset")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .fixed_size([240.0, 0.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        ui.label(
                            egui::RichText::new("Preset name:")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DIM),
                        );
                        let response = ui.add(
                            egui::TextEdit::singleline(&mut state.save_name)
                                .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                                .desired_width(220.0),
                        );
                        // Auto-focus the text input
                        if response.gained_focus() || state.save_name.is_empty() {
                            response.request_focus();
                        }

                        ui.add_space(6.0);

                        let name_valid = !state.save_name.trim().is_empty();
                        let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

                        ui.horizontal(|ui| {
                            let save_enabled = name_valid;
                            if ui
                                .add_enabled(
                                    save_enabled,
                                    egui::Button::new(
                                        egui::RichText::new("SAVE")
                                            .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                            .color(if save_enabled {
                                                egui::Color32::from_rgb(0x74, 0xb9, 0xff)
                                            } else {
                                                theme::TEXT_DISABLED
                                            }),
                                    )
                                    .fill(theme::BG_ROW)
                                    .min_size(egui::vec2(60.0, 22.0)),
                                )
                                .clicked()
                                || (enter_pressed && save_enabled)
                            {
                                pending_actions.push(
                                    GuiAction::SavePreset(state.save_name.trim().to_string()));
                                state.show_save_dialog = false;
                            }

                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("CANCEL")
                                            .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                            .color(theme::TEXT_DIM),
                                    )
                                    .fill(theme::BG_ROW)
                                    .min_size(egui::vec2(60.0, 22.0)),
                                )
                                .clicked()
                            {
                                state.show_save_dialog = false;
                            }
                        });
                    });
                if !open {
                    state.show_save_dialog = false;
                }
            }

            // --- Load preset dialog ---
            if state.show_load_dialog {
                let mut open = true;
                egui::Window::new("Load Preset")
                    .collapsible(false)
                    .resizable(false)
                    .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                    .fixed_size([280.0, 300.0])
                    .open(&mut open)
                    .show(ctx, |ui| {
                        if state.preset_list.is_empty() {
                            ui.label(
                                egui::RichText::new("No presets found.")
                                    .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                    .color(theme::TEXT_DIM),
                            );
                        } else {
                            egui::ScrollArea::vertical()
                                .max_height(260.0)
                                .show(ui, |ui| {
                                    // Clone the list to avoid borrow conflict with state
                                    let list: Vec<(String, PathBuf)> =
                                        state.preset_list.clone();
                                    for (name, path) in &list {
                                        if ui
                                            .add(
                                                egui::Button::new(
                                                    egui::RichText::new(name)
                                                        .font(egui::FontId::new(
                                                            11.0,
                                                            egui::FontFamily::Monospace,
                                                        ))
                                                        .color(theme::ACCENT),
                                                )
                                                .fill(theme::BG_ROW)
                                                .min_size(egui::vec2(260.0, 24.0)),
                                            )
                                            .clicked()
                                        {
                                            pending_actions.push(
                                                GuiAction::LoadPreset(path.clone()));
                                            state.show_load_dialog = false;
                                        }
                                    }
                                });
                        }

                        ui.add_space(4.0);
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("CANCEL")
                                        .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                        .color(theme::TEXT_DIM),
                                )
                                .fill(theme::BG_ROW)
                                .min_size(egui::vec2(60.0, 22.0)),
                            )
                            .clicked()
                        {
                            state.show_load_dialog = false;
                        }
                    });
                if !open {
                    state.show_load_dialog = false;
                }
            }

            // --- Phase 2: Brief lock to apply any mutation ---
            if !pending_actions.is_empty() {
                let mut needs_waveform_update = false;
                {
                let mut shared = shared.lock();
                for action in pending_actions {
                match action {
                    GuiAction::Undo => {
                        let current = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        if let Some(restored) = shared.history.undo(current) {
                            shared.kit.restore(&restored.pads);
                            shared.pattern_bank.restore(&restored.sequencer);
                            needs_waveform_update = true;
                        }
                    }
                    GuiAction::Redo => {
                        let current = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        if let Some(restored) = shared.history.redo(current) {
                            shared.kit.restore(&restored.pads);
                            shared.pattern_bank.restore(&restored.sequencer);
                            needs_waveform_update = true;
                        }
                    }
                    GuiAction::DiceAll => {
                        if shared.library.is_some() {
                            {
                                let SharedState { ref library, ref mut kit, ref mut history, ref pattern_bank, .. } = *shared;
                                let lib = library.as_ref().unwrap();
                                history.push(HistorySnapshot {
                                    pads: kit.snapshot(),
                                    sequencer: pattern_bank.snapshot(),
                                });
                                kit.dice_all(lib);
                            }
                            needs_waveform_update = true;
                        }
                    }
                    GuiAction::DicePad(i) => {
                        if shared.library.is_some() {
                            {
                                let SharedState { ref library, ref mut kit, ref mut history, ref pattern_bank, .. } = *shared;
                                let lib = library.as_ref().unwrap();
                                history.push(HistorySnapshot {
                                    pads: kit.snapshot(),
                                    sequencer: pattern_bank.snapshot(),
                                });
                                kit.dice_pad(i, lib);
                            }
                            shared.update_waveform(i, WAVEFORM_POINTS);
                        }
                    }
                    GuiAction::DiceCategory(_i, cat) => {
                        if shared.library.is_some() {
                            {
                                let SharedState { ref library, ref mut kit, ref mut history, ref pattern_bank, .. } = *shared;
                                let lib = library.as_ref().unwrap();
                                history.push(HistorySnapshot {
                                    pads: kit.snapshot(),
                                    sequencer: pattern_bank.snapshot(),
                                });
                                kit.dice_category(cat, lib);
                            }
                            needs_waveform_update = true;
                        }
                    }
                    GuiAction::LockAll => {
                        let all_locked = shared.kit.pads.iter().all(|p| p.locked);
                        for pad in &mut shared.kit.pads {
                            pad.locked = !all_locked;
                        }
                    }
                    GuiAction::ToggleLock(i) => {
                        shared.kit.toggle_lock(i);
                    }
                    GuiAction::SetPadVolume(i, v) => {
                        shared.kit.pads[i].volume = v;
                    }
                    GuiAction::SetPadPan(i, v) => {
                        shared.kit.pads[i].pan = v;
                    }
                    GuiAction::SetPadPitch(i, v) => {
                        shared.kit.pads[i].pitch = v;
                    }
                    GuiAction::SetPadDecay(i, v) => {
                        shared.kit.pads[i].decay = v;
                    }
                    GuiAction::SavePreset(name) => {
                        let p = preset::from_kit(&name, &shared.kit);
                        match preset::save_preset(&p) {
                            Ok(path) => {
                                tracing::info!("Saved preset to {}", path.display());
                                state.status_message =
                                    Some(format!("Saved: {name}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to save preset: {e}");
                                state.status_message =
                                    Some(format!("Save failed: {e}"));
                            }
                        }
                    }
                    GuiAction::LoadPreset(path) => {
                        match preset::load_preset(&path) {
                            Ok(p) => {
                                // Push history snapshot before applying
                                let snap = HistorySnapshot {
                                    pads: shared.kit.snapshot(),
                                    sequencer: shared.pattern_bank.snapshot(),
                                };
                                shared.history.push(snap);
                                preset::apply_to_kit(&p, &mut shared.kit);
                                needs_waveform_update = true;
                                tracing::info!("Loaded preset: {}", p.name);
                                state.status_message =
                                    Some(format!("Loaded: {}", p.name));
                            }
                            Err(e) => {
                                tracing::error!("Failed to load preset: {e}");
                                state.status_message =
                                    Some(format!("Load failed: {e}"));
                            }
                        }
                    }
                    GuiAction::PreviewSample(lib_index) => {
                        let data = shared.library.as_ref().and_then(|lib| {
                            lib.sample_by_flat_index(lib_index).map(|s| Arc::clone(&s.data))
                        });
                        if let Some(data) = data {
                            shared.preview_sample = Some(data);
                        }
                    }
                    GuiAction::AssignFromMap { pad_index, library_index } => {
                        // Extract sample info before mutating shared state
                        let sample_info = shared.library.as_ref().and_then(|lib| {
                            lib.sample_by_flat_index(library_index).map(|s| {
                                (Arc::clone(&s.data),
                                 s.entry.path.to_string_lossy().to_string(),
                                 s.entry.filename.clone(),
                                 s.entry.category)
                            })
                        });
                        if let Some((data, path, filename, category)) = sample_info {
                            let snap = HistorySnapshot {
                                pads: shared.kit.snapshot(),
                                sequencer: shared.pattern_bank.snapshot(),
                            };
                            shared.history.push(snap);
                            let pad = &mut shared.kit.pads[pad_index];
                            pad.sample = Some(Arc::clone(&data));
                            pad.sample_path = Some(path);
                            pad.name = filename;
                            pad.category = category;
                            shared.update_waveform(pad_index, WAVEFORM_POINTS);
                            shared.preview_sample = Some(data);
                        }
                    }
                    GuiAction::None => {}
                    // Sequencer actions
                    GuiAction::SeqToggleStep { lane, step } => {
                        let pat = shared.pattern_bank.active_pattern_mut();
                        let s = &mut pat.lanes[lane].steps[step];
                        s.enabled = !s.enabled;
                        if s.enabled {
                            s.velocity = 0.8;
                            s.probability = 1.0;
                            s.pan = None;
                            s.pitch = None;
                            s.condition = crate::engine::sequencer::ConditionTrig::Always;
                        }
                    }
                    GuiAction::SeqSetStepEnabled { lane, step, enabled } => {
                        let pat = shared.pattern_bank.active_pattern_mut();
                        let s = &mut pat.lanes[lane].steps[step];
                        s.enabled = enabled;
                        if enabled && s.velocity == 0.0 {
                            s.velocity = 0.8;
                            s.probability = 1.0;
                        }
                    }
                    GuiAction::SeqSetStepVelocity { lane, step, value } => {
                        shared.pattern_bank.active_pattern_mut().lanes[lane].steps[step].velocity = value;
                    }
                    GuiAction::SeqSetStepPan { lane, step, value } => {
                        shared.pattern_bank.active_pattern_mut().lanes[lane].steps[step].pan = value;
                    }
                    GuiAction::SeqSetStepPitch { lane, step, value } => {
                        shared.pattern_bank.active_pattern_mut().lanes[lane].steps[step].pitch = value;
                    }
                    GuiAction::SeqSetStepProbability { lane, step, value } => {
                        shared.pattern_bank.active_pattern_mut().lanes[lane].steps[step].probability = value;
                    }
                    GuiAction::SeqSetStepCondition { lane, step, condition } => {
                        shared.pattern_bank.active_pattern_mut().lanes[lane].steps[step].condition = condition;
                    }
                    GuiAction::SeqToggleLaneMute { lane } => {
                        let pat = shared.pattern_bank.active_pattern_mut();
                        pat.lanes[lane].muted = !pat.lanes[lane].muted;
                    }
                    GuiAction::SeqToggleLaneLock { lane } => {
                        shared.kit.toggle_lock(lane);
                    }
                    GuiAction::SeqSelectPattern { index } => {
                        shared.pattern_bank.queued = Some(index);
                    }
                    GuiAction::SeqSetSwing { value } => {
                        shared.pattern_bank.active_pattern_mut().swing = value;
                    }
                    GuiAction::SeqCopyPattern => {
                        let pat = shared.pattern_bank.active_pattern().clone();
                        shared.pattern_clipboard = Some(pat);
                    }
                    GuiAction::SeqPastePattern => {
                        if let Some(clip) = shared.pattern_clipboard.clone() {
                            let snap = HistorySnapshot {
                                pads: shared.kit.snapshot(),
                                sequencer: shared.pattern_bank.snapshot(),
                            };
                            shared.history.push(snap);
                            *shared.pattern_bank.active_pattern_mut() = clip;
                        }
                    }
                    GuiAction::SeqClearPattern => {
                        let snap = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        shared.history.push(snap);
                        let pat = shared.pattern_bank.active_pattern_mut();
                        for lane in &mut pat.lanes {
                            for step in &mut lane.steps {
                                *step = crate::engine::sequencer::Step::default();
                            }
                            lane.muted = false;
                        }
                        pat.swing = 0.0;
                    }
                    GuiAction::SeqDicePattern => {
                        use rand::Rng;
                        use rand::seq::SliceRandom;
                        let snap = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        shared.history.push(snap);
                        let locked: Vec<bool> = shared.kit.pads.iter().map(|p| p.locked).collect();
                        let mut rng = rand::rng();
                        let pat = shared.pattern_bank.active_pattern_mut();
                        for (i, lane) in pat.lanes.iter_mut().enumerate() {
                            if locked[i] { continue; }
                            for step in &mut lane.steps {
                                *step = crate::engine::sequencer::Step::default();
                            }
                            let num_steps: usize = rng.random_range(2..=6);
                            let mut positions: Vec<usize> = (0..16).collect();
                            positions.shuffle(&mut rng);
                            for &pos in &positions[..num_steps] {
                                lane.steps[pos].enabled = true;
                                lane.steps[pos].velocity = rng.random_range(0.5..=1.0);
                                lane.steps[pos].probability = rng.random_range(0.7..=1.0);
                                if rng.random_bool(0.15) {
                                    let conds = &[
                                        crate::engine::sequencer::ConditionTrig::Every(2),
                                        crate::engine::sequencer::ConditionTrig::Every(4),
                                        crate::engine::sequencer::ConditionTrig::Fill,
                                    ];
                                    lane.steps[pos].condition = conds[rng.random_range(0..conds.len())];
                                }
                            }
                        }
                    }
                    GuiAction::SeqSetFillActive { active } => {
                        seq_fill_active.store(active, Ordering::Relaxed);
                    }
                    GuiAction::SeqToggleInternalPlay => {
                        let current = seq_internal_play.load(Ordering::Relaxed);
                        seq_internal_play.store(!current, Ordering::Relaxed);
                    }
                }
                } // end for action in pending_actions
                } // Lock drops here — held only for the mutation.
                // Update waveforms OUTSIDE the lock to avoid blocking audio
                if needs_waveform_update {
                    let mut shared = shared.lock();
                    shared.update_all_waveforms(WAVEFORM_POINTS);
                }
            }
        },
    )
}

/// Actions that the GUI can trigger, applied in a brief second lock.
enum GuiAction {
    None,
    Undo,
    Redo,
    DiceAll,
    DicePad(usize),
    DiceCategory(usize, SampleCategory),
    LockAll,
    ToggleLock(usize),
    SetPadVolume(usize, f32),
    SetPadPan(usize, f32),
    SetPadPitch(usize, f32),
    SetPadDecay(usize, f32),
    SavePreset(String),
    LoadPreset(PathBuf),
    PreviewSample(usize),
    AssignFromMap { pad_index: usize, library_index: usize },
    // Sequencer actions
    SeqToggleStep { lane: usize, step: usize },
    SeqSetStepEnabled { lane: usize, step: usize, enabled: bool },
    SeqSetStepVelocity { lane: usize, step: usize, value: f32 },
    SeqSetStepPan { lane: usize, step: usize, value: Option<f32> },
    SeqSetStepPitch { lane: usize, step: usize, value: Option<f32> },
    SeqSetStepProbability { lane: usize, step: usize, value: f32 },
    SeqSetStepCondition { lane: usize, step: usize, condition: crate::engine::sequencer::ConditionTrig },
    SeqToggleLaneMute { lane: usize },
    SeqToggleLaneLock { lane: usize },
    SeqSelectPattern { index: usize },
    SeqSetSwing { value: f32 },
    SeqCopyPattern,
    SeqPastePattern,
    SeqClearPattern,
    SeqDicePattern,
    SeqSetFillActive { active: bool },
    SeqToggleInternalPlay,
}
