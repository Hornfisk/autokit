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
use crate::ui::dialogs::{self, DialogState, DialogAction};
use crate::ui::toolbar::{self, ToolbarAction};
use crate::plugin::populate_kit_from_library;
use crate::util::config;
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
    scan_processed: u32,
    scan_total: u32,
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
        let (scan_processed, scan_total) = shared.scan_progress.as_ref()
            .map(|p| {
                (p.processed.load(std::sync::atomic::Ordering::Relaxed),
                 p.total.load(std::sync::atomic::Ordering::Relaxed))
            })
            .unwrap_or((0, 0));
        Self {
            pads,
            waveforms: shared.waveforms.clone(),
            scan_status: shared.scan_status.clone(),
            scan_processed,
            scan_total,
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
    /// Last-seen trigger counter values — used to detect new triggers.
    pub last_trigger: [u8; NUM_PADS],
    /// Per-pad flash brightness for play animation (0.0 = dark, 1.0 = full flash).
    pub brightness: [f32; NUM_PADS],
    /// Modal dialog state (save, load, setup).
    pub dialogs: DialogState,
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
    /// Timestamp (frame count) when status_message was set — used for auto-dismiss.
    pub status_message_frame: u64,
    /// Frame counter.
    pub frame_count: u64,
    /// Cached logo texture handle.
    pub logo_texture: Option<egui::TextureHandle>,
    /// Whether mouseover tooltips are shown.
    pub tooltips_on: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected_pad: None,
            last_trigger: [0u8; NUM_PADS],
            brightness: [0.0f32; NUM_PADS],
            dialogs: DialogState::default(),
            status_message: None,
            view_mode: ViewMode::PadStrip,
            map_points: Vec::new(),
            map_built: false,
            map_view: sample_map::MapViewState::default(),
            map_hovered: None,
            map_popup: sample_map::PopupState::default(),
            map_shortcut_pad: None,
            seq_view: Default::default(),
            status_message_frame: 0,
            frame_count: 0,
            logo_texture: None,
            tooltips_on: true,
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
    seq_sync: Arc<crate::plugin::SequencerSync>,
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
            // Destructure sync struct into local references for readability
            let seq_current_step = &seq_sync.current_step;
            let seq_playing = &seq_sync.playing;
            let seq_active_pattern = &seq_sync.active_pattern;
            let seq_fill_active = &seq_sync.fill_active;
            let seq_internal_play = &seq_sync.internal_play;
            let seq_ext_mode = &seq_sync.ext_mode;
            let seq_host_playing = &seq_sync.host_playing;
            let seq_tempo = &seq_sync.tempo;
            let seq_standalone_tempo = &seq_sync.standalone_tempo;
            let seq_is_daw = &seq_sync.is_daw;

            state.frame_count += 1;

            // --- Phase 1: Brief lock to snapshot display state ---
            let snap = {
                let mut shared = shared.lock();

                // Fallback: if process() isn't running (JACK error), pick up
                // the library from the GUI thread so the UI still works.
                if matches!(shared.scan_status, ScanStatus::Scanning) {
                    let lib = shared.bg_rx.as_ref().and_then(|rx| rx.try_recv().ok());
                    if let Some(library) = lib {
                        tracing::info!(total = library.total, "library received via GUI fallback");
                        shared.library = Some(library);
                        populate_kit_from_library(&mut shared);
                        shared.scan_status = ScanStatus::Ready {
                            total: shared.library.as_ref().map(|l| l.total).unwrap_or(0),
                        };
                    }
                }

                // Persist state if audio thread flagged it dirty.
                if seq_sync.persist_dirty.swap(false, Ordering::Relaxed) {
                    if seq_is_daw.load(Ordering::Relaxed) {
                        // DAW mode: write to plugin param for host save/load
                        if let Some(json) = preset::serialize_state(&shared.kit, &shared.pattern_bank) {
                            *params.plugin_state.lock() = json;
                        }
                    } else {
                        // Standalone mode: write to disk
                        preset::save_standalone_state(&shared.kit, &shared.pattern_bank);
                    }
                }

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
            // Gated: ignored when the host transport is actively driving playback
            // (prevents double-trigger when Space also toggles the DAW transport)
            if ctx.input(|i| i.key_pressed(egui::Key::Space))
                && !seq_host_playing.load(Ordering::Relaxed)
            {
                let current = seq_internal_play.load(Ordering::Relaxed);
                seq_internal_play.store(!current, Ordering::Relaxed);
            }

            // Collect any actions triggered during rendering.
            // Auto-open setup dialog on first frame if no config
            if let ScanStatus::NeedsSetup { ref suggested_path } = snap.scan_status {
                if !state.dialogs.show_setup && state.frame_count == 1 {
                    state.dialogs.setup_path = suggested_path
                        .as_ref()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    state.dialogs.show_setup = true;
                }
            }

            let mut pending_actions: Vec<GuiAction> = Vec::new();

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(theme::BG_MAIN).inner_margin(egui::Margin { left: 8, right: 8, top: 0, bottom: 0 }))
                .show(ctx, |ui| {
                    // Toolbar (uses snapshot data, no mutex held)
                    let all_locked = snap.pads.iter().all(|p| p.locked);
                    let shortcut_info = state.map_shortcut_pad.map(|i| (i + 1, snap.pads[i].category.label()));
                    let logo = state.logo_texture.get_or_insert_with(|| {
                        toolbar::load_logo_texture(ctx)
                    });
                    let is_standalone = !seq_is_daw.load(Ordering::Relaxed);
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

                    match toolbar_action {
                        ToolbarAction::Undo => pending_actions.push(GuiAction::Undo),
                        ToolbarAction::Redo => pending_actions.push(GuiAction::Redo),
                        ToolbarAction::DiceAll => {
                            if snap.has_library {
                                pending_actions.push(GuiAction::DiceAll);
                            }
                        }
                        ToolbarAction::LockAll => pending_actions.push(GuiAction::LockAll),
                        ToolbarAction::OpenSaveDialog => {
                            state.dialogs.show_save = true;
                            state.dialogs.show_load = false;
                        }
                        ToolbarAction::OpenLoadDialog => {
                            state.dialogs.preset_list = preset::list_presets();
                            state.dialogs.show_load = true;
                            state.dialogs.show_save = false;
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
                        ToolbarAction::OpenSetup => {
                            // Pre-fill with discovered path if empty
                            if state.dialogs.setup_path.is_empty() {
                                if let Some(discovered) = config::discover_sample_root() {
                                    state.dialogs.setup_path = discovered.to_string_lossy().into_owned();
                                }
                            }
                            state.dialogs.show_setup = true;
                            state.dialogs.folder_browser = None;
                        }
                        ToolbarAction::ToggleTooltips => {
                            state.tooltips_on = !state.tooltips_on;
                        }
                        ToolbarAction::None => {}
                    }

                    // Separator line
                    ui.add(egui::Separator::default().spacing(0.0));

                    // Shared row height — identical in both pad and seq views
                    let shared_avail_h = ui.available_height();
                    let shared_row_height = {
                        let num_lanes = 8.0_f32;
                        let cell_spacing = theme::CELL_SPACING;
                        let vert_avail = shared_avail_h - theme::GRID_VERT_RESERVED;
                        let row_from_height = ((vert_avail - cell_spacing * (num_lanes - 1.0)) / num_lanes).floor();
                        // Pad strip label: strip + space + tag + space = 61px
                        let label_width = theme::STRIP_WIDTH + 8.0 + theme::TAG_WIDTH + 4.0;
                        let controls_width = theme::CONTROLS_WIDTH;
                        let available_w = ui.available_width() - label_width - controls_width;
                        let cell_from_width = ((available_w - cell_spacing * 15.0) / 16.0).floor();
                        row_from_height.min(cell_from_width).clamp(20.0, 48.0)
                    };

                    match state.view_mode {
                        ViewMode::PadStrip => {
                            // Step number header — matches sequencer grid layout exactly
                            {
                                use egui::{FontId, Color32, Vec2};
                                let header_offset = (theme::STRIP_WIDTH + 8.0 + theme::TAG_WIDTH + 4.0) + theme::CONTROLS_WIDTH;
                                let cell_spacing = theme::CELL_SPACING;
                                ui.horizontal(|ui| {
                                    ui.add_space(header_offset);
                                    ui.spacing_mut().item_spacing.x = cell_spacing;
                                    let avail_w = ui.available_width() - 4.0;
                                    let cell_w = ((avail_w - cell_spacing * 15.0) / 16.0).floor();
                                    for s in 0..16 {
                                        let is_beat = s % 4 == 0;
                                        let color = if is_beat {
                                            crate::ui::theme::TEXT_DIM
                                        } else {
                                            Color32::from_rgb(51, 51, 51)
                                        };
                                        let text = egui::RichText::new(format!("{}", s + 1))
                                            .font(FontId::monospace(8.0))
                                            .color(color);
                                        ui.allocate_ui(Vec2::new(cell_w, 12.0), |ui| {
                                            ui.centered_and_justified(|ui| ui.label(text));
                                        });
                                    }
                                });
                            }

                            // Pad list — row height matches seq grid cell_size exactly
                            egui::ScrollArea::vertical()
                                .auto_shrink([false, false])
                                .show(ui, |ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    let pad_row_height = shared_row_height;

                                    for i in 0..NUM_PADS {
                                        let is_selected = state.selected_pad == Some(i);
                                        let pad = &snap.pads[i];
                                        let wf = snap.waveforms[i].as_ref();

                                        let row_action = pad_row::draw_collapsed_from_snapshot(
                                            ui, i, pad.has_sample, &pad.name, pad.category,
                                            pad.volume, wf, is_selected, state.brightness[i],
                                            pad.locked, pad_row_height, state.tooltips_on,
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
                                            PadRowAction::SetVolume(v) => {
                                                pending_actions.push(GuiAction::SetPadParam(i, PadParam::Volume, v));
                                            }
                                            _ => {}
                                        }

                                        // Expanded detail (knobs + dice category)
                                        if is_selected {
                                            let detail_action = pad_row::draw_expanded_from_snapshot(
                                                ui, i, pad.category, pad.pan,
                                                pad.pitch, pad.decay, state.tooltips_on,
                                            );

                                            match detail_action {
                                                PadRowAction::SetPan(v) => {
                                                    pending_actions.push(GuiAction::SetPadParam(i, PadParam::Pan, v));
                                                }
                                                PadRowAction::SetPitch(v) => {
                                                    pending_actions.push(GuiAction::SetPadParam(i, PadParam::Pitch, v));
                                                }
                                                PadRowAction::SetDecay(v) => {
                                                    pending_actions.push(GuiAction::SetPadParam(i, PadParam::Decay, v));
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
                                state.tooltips_on,
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
                                            solo: lane.solo,
                                            locked: snap.pads[i].locked,
                                            volume: snap.pads[i].volume,
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
                                    ext_mode: seq_ext_mode.load(Ordering::Relaxed),
                                }
                            };

                            // Status message toast (export feedback etc.)
                            if let Some(ref msg) = state.status_message {
                                let age = state.frame_count.saturating_sub(state.status_message_frame);
                                if age > 600 { // ~10 seconds at 60fps
                                    state.status_message = None;
                                } else {
                                    let alpha = if age > 540 { ((600 - age) as f32 / 60.0 * 255.0) as u8 } else { 255 };
                                    ui.label(
                                        egui::RichText::new(msg)
                                            .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                            .color(egui::Color32::from_rgba_unmultiplied(0, 212, 170, alpha)),
                                    );
                                    ui.ctx().request_repaint();
                                }
                            }

                            {
                                use crate::ui::sequencer_ui::SeqAction;
                                for seq_action in crate::ui::sequencer_ui::draw_sequencer_view(
                                    ui, &seq_display, &mut state.seq_view, shared_avail_h, state.tooltips_on,
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
                                        SeqAction::ToggleLaneSolo { lane } => GuiAction::SeqToggleLaneSolo { lane },
                                        SeqAction::ToggleLaneLock { lane } => GuiAction::SeqToggleLaneLock { lane },
                                        SeqAction::SetLaneVolume { lane, volume } => GuiAction::SeqSetLaneVolume { lane, volume },
                                        SeqAction::SelectPattern { index } => GuiAction::SeqSelectPattern { index },
                                        SeqAction::SetSwing { value } => GuiAction::SeqSetSwing { value },
                                        SeqAction::CopyPattern => GuiAction::SeqCopyPattern,
                                        SeqAction::PastePattern => GuiAction::SeqPastePattern,
                                        SeqAction::ClearPattern => GuiAction::SeqClearPattern,
                                        SeqAction::DicePattern => GuiAction::SeqDicePattern,
                                        SeqAction::ShiftLeft => GuiAction::SeqShiftLeft,
                                        SeqAction::ShiftRight => GuiAction::SeqShiftRight,
                                        SeqAction::SetFillActive { active } => GuiAction::SeqSetFillActive { active },
                                        SeqAction::ToggleInternalPlay => GuiAction::SeqToggleInternalPlay,
                                        SeqAction::ExportMidi => GuiAction::SeqExportMidi,
                                        SeqAction::OpenSavePatternDialog => {
                                            state.dialogs.save_pattern_name.clear();
                                            state.dialogs.show_save_pattern = true;
                                            GuiAction::None
                                        }
                                        SeqAction::OpenLoadPatternDialog => {
                                            state.dialogs.pattern_list = preset::list_patterns();
                                            state.dialogs.show_load_pattern = true;
                                            GuiAction::None
                                        }
                                        SeqAction::ResetLane { lane } => GuiAction::SeqResetLane { lane },
                                        SeqAction::ResetStep { lane, step } => GuiAction::SeqResetStep { lane, step },
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

            // --- Modal dialogs (save, load, setup) ---
            if state.dialogs.show_save {
                match dialogs::show_save_dialog(ctx, &mut state.dialogs) {
                    DialogAction::SavePreset(name) => pending_actions.push(GuiAction::SavePreset(name)),
                    _ => {}
                }
            }
            if state.dialogs.show_load {
                match dialogs::show_load_dialog(ctx, &mut state.dialogs) {
                    DialogAction::LoadPreset(path) => pending_actions.push(GuiAction::LoadPreset(path)),
                    DialogAction::DeletePreset(path) => pending_actions.push(GuiAction::DeletePreset(path)),
                    _ => {}
                }
            }
            if state.dialogs.show_setup {
                match dialogs::show_setup_dialog(ctx, &mut state.dialogs) {
                    DialogAction::StartScan(path) => pending_actions.push(GuiAction::StartScan(path)),
                    _ => {}
                }
            }
            if state.dialogs.show_save_pattern {
                match dialogs::show_save_pattern_dialog(ctx, &mut state.dialogs) {
                    DialogAction::SavePattern(name) => pending_actions.push(GuiAction::SavePattern(name)),
                    _ => {}
                }
            }
            if state.dialogs.show_load_pattern {
                match dialogs::show_load_pattern_dialog(ctx, &mut state.dialogs) {
                    DialogAction::LoadPattern(path) => pending_actions.push(GuiAction::LoadPattern(path)),
                    DialogAction::DeletePattern(path) => pending_actions.push(GuiAction::DeletePattern(path)),
                    _ => {}
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
                    GuiAction::SetPadParam(i, param, v) => {
                        let pad = &mut shared.kit.pads[i];
                        match param {
                            PadParam::Volume => pad.volume = v,
                            PadParam::Pan => pad.pan = v,
                            PadParam::Pitch => pad.pitch = v,
                            PadParam::Decay => pad.decay = v,
                        }
                    }
                    GuiAction::SavePreset(name) => {
                        let p = preset::from_kit(&name, &shared.kit, &shared.pattern_bank);
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
                                let s = &mut *shared;
                                preset::apply_to_kit(&p, &mut s.kit, &mut s.pattern_bank);
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
                    GuiAction::DeletePreset(path) => {
                        match preset::delete_file(&path) {
                            Ok(()) => {
                                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                                tracing::info!("Deleted preset: {name}");
                                state.status_message = Some(format!("Deleted: {name}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to delete preset: {e}");
                                state.status_message = Some(format!("Delete failed: {e}"));
                            }
                        }
                    }
                    GuiAction::SavePattern(name) => {
                        match preset::save_pattern(&name, shared.pattern_bank.active_pattern()) {
                            Ok(path) => {
                                tracing::info!("Saved pattern to {}", path.display());
                                state.status_message = Some(format!("Pattern saved: {name}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to save pattern: {e}");
                                state.status_message = Some(format!("Save failed: {e}"));
                            }
                        }
                    }
                    GuiAction::LoadPattern(path) => {
                        match preset::load_pattern(&path) {
                            Ok(pat) => {
                                let snap = HistorySnapshot {
                                    pads: shared.kit.snapshot(),
                                    sequencer: shared.pattern_bank.snapshot(),
                                };
                                shared.history.push(snap);
                                *shared.pattern_bank.active_pattern_mut() = pat;
                                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                                tracing::info!("Loaded pattern: {name}");
                                state.status_message = Some(format!("Pattern loaded: {name}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to load pattern: {e}");
                                state.status_message = Some(format!("Load failed: {e}"));
                            }
                        }
                    }
                    GuiAction::DeletePattern(path) => {
                        match preset::delete_file(&path) {
                            Ok(()) => {
                                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("?");
                                tracing::info!("Deleted pattern: {name}");
                                state.status_message = Some(format!("Deleted: {name}"));
                            }
                            Err(e) => {
                                tracing::error!("Failed to delete pattern: {e}");
                                state.status_message = Some(format!("Delete failed: {e}"));
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
                    GuiAction::SeqToggleLaneSolo { lane } => {
                        let pat = shared.pattern_bank.active_pattern_mut();
                        pat.lanes[lane].solo = !pat.lanes[lane].solo;
                    }
                    GuiAction::SeqToggleLaneLock { lane } => {
                        shared.kit.toggle_lock(lane);
                    }
                    GuiAction::SeqSetLaneVolume { lane, volume } => {
                        shared.kit.pads[lane].volume = volume;
                    }
                    GuiAction::SeqResetLane { lane } => {
                        let snap = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        shared.history.push(snap);
                        let pat = shared.pattern_bank.active_pattern_mut();
                        if lane < pat.lanes.len() {
                            for step in &mut pat.lanes[lane].steps {
                                *step = crate::engine::sequencer::Step::default();
                            }
                            pat.lanes[lane].muted = false;
                            pat.lanes[lane].solo = false;
                        }
                    }
                    GuiAction::SeqResetStep { lane, step } => {
                        let pat = shared.pattern_bank.active_pattern_mut();
                        if lane < pat.lanes.len() && step < pat.lanes[lane].steps.len() {
                            pat.lanes[lane].steps[step] = crate::engine::sequencer::Step::default();
                        }
                    }
                    GuiAction::SeqSelectPattern { index } => {
                        let is_playing = seq_playing.load(Ordering::Relaxed)
                            || seq_internal_play.load(Ordering::Relaxed);
                        if is_playing {
                            // Queue for bar boundary switch
                            shared.pattern_bank.queued = Some(index);
                        } else {
                            // Switch immediately when stopped
                            shared.pattern_bank.active = index;
                            shared.pattern_bank.queued = None;
                        }
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
                    GuiAction::SeqShiftLeft => {
                        let snap = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        shared.history.push(snap);
                        let pat = shared.pattern_bank.active_pattern_mut();
                        for lane in &mut pat.lanes {
                            lane.steps.rotate_left(1);
                        }
                    }
                    GuiAction::SeqShiftRight => {
                        let snap = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: shared.pattern_bank.snapshot(),
                        };
                        shared.history.push(snap);
                        let pat = shared.pattern_bank.active_pattern_mut();
                        for lane in &mut pat.lanes {
                            lane.steps.rotate_right(1);
                        }
                    }
                    GuiAction::SeqSetFillActive { active } => {
                        seq_fill_active.store(active, Ordering::Relaxed);
                    }
                    GuiAction::SeqToggleInternalPlay => {
                        if !seq_host_playing.load(Ordering::Relaxed) {
                            let current = seq_internal_play.load(Ordering::Relaxed);
                            seq_internal_play.store(!current, Ordering::Relaxed);
                        }
                    }
                    GuiAction::SeqExportMidi => {
                        match export_pattern_to_midi(&shared.pattern_bank, &shared.kit) {
                            Ok(path) => {
                                tracing::info!(?path, "MIDI pattern exported");
                                state.status_message = Some(format!("Exported: {}", path.display()));
                                state.status_message_frame = state.frame_count;
                            }
                            Err(e) => {
                                tracing::error!(%e, "MIDI export failed");
                                state.status_message = Some(format!("Export failed: {e}"));
                                state.status_message_frame = state.frame_count;
                            }
                        }
                    }
                    GuiAction::StartScan(path) => {
                        shared.scan_status = ScanStatus::Scanning;
                        shared.pending_scan_path = Some(path);
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
/// Which pad parameter to set.
enum PadParam { Volume, Pan, Pitch, Decay }

enum GuiAction {
    None,
    Undo,
    Redo,
    DiceAll,
    DicePad(usize),
    DiceCategory(usize, SampleCategory),
    LockAll,
    ToggleLock(usize),
    SetPadParam(usize, PadParam, f32),
    SavePreset(String),
    LoadPreset(PathBuf),
    DeletePreset(PathBuf),
    SavePattern(String),
    LoadPattern(PathBuf),
    DeletePattern(PathBuf),
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
    SeqToggleLaneSolo { lane: usize },
    SeqToggleLaneLock { lane: usize },
    SeqSetLaneVolume { lane: usize, volume: f32 },
    SeqSelectPattern { index: usize },
    SeqSetSwing { value: f32 },
    SeqCopyPattern,
    SeqPastePattern,
    SeqClearPattern,
    SeqDicePattern,
    SeqShiftLeft,
    SeqShiftRight,
    SeqSetFillActive { active: bool },
    SeqToggleInternalPlay,
    SeqExportMidi,
    SeqResetLane { lane: usize },
    SeqResetStep { lane: usize, step: usize },
    StartScan(PathBuf),
}

/// Export the active pattern from a PatternBank as a Standard MIDI File (Type 0).
/// Returns the path of the written .mid file.
fn export_pattern_to_midi(
    bank: &crate::engine::sequencer::PatternBank,
    kit: &crate::engine::kit::DrumKit,
) -> Result<PathBuf, std::io::Error> {
    use std::io::Write;

    let export_dir = dirs_export_path();
    std::fs::create_dir_all(&export_dir)?;

    // Generate a timestamped filename
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("autokit_pattern_{timestamp}.mid");
    let path = export_dir.join(&filename);

    let pattern = bank.active_pattern();
    let ticks_per_quarter: u16 = 96; // Standard PPQN
    let ticks_per_step = ticks_per_quarter / 4; // 16th note = quarter / 4 = 24 ticks
    let note_duration: u16 = ticks_per_step / 2; // Note lasts half a step

    // Build MIDI track events
    let mut track_data: Vec<u8> = Vec::new();

    // Tempo meta event: 120 BPM = 500000 microseconds per quarter note
    track_data.extend_from_slice(&[0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20]);

    // Collect all note events with absolute tick positions, then sort and delta-encode
    struct MidiEvent {
        tick: u32,
        status: u8,
        note: u8,
        velocity: u8,
    }

    let mut events: Vec<MidiEvent> = Vec::new();

    for lane in &pattern.lanes {
        if lane.muted {
            continue;
        }
        let note = kit.note_for_pad(lane.pad_index);

        for (step_idx, step) in lane.steps.iter().enumerate() {
            if !step.enabled {
                continue;
            }
            let tick_on = step_idx as u32 * ticks_per_step as u32;
            let tick_off = tick_on + note_duration as u32;
            let vel = (step.velocity * 127.0).round().clamp(1.0, 127.0) as u8;

            events.push(MidiEvent {
                tick: tick_on,
                status: 0x99, // Note On, channel 10 (0-indexed = 9)
                note,
                velocity: vel,
            });
            events.push(MidiEvent {
                tick: tick_off,
                status: 0x89, // Note Off, channel 10
                note,
                velocity: 0,
            });
        }
    }

    // Sort by tick, then note-off before note-on at same tick
    events.sort_by(|a, b| {
        a.tick.cmp(&b.tick)
            .then_with(|| {
                // Note-off (0x8x) sorts before note-on (0x9x)
                a.status.cmp(&b.status)
            })
            .then_with(|| a.note.cmp(&b.note))
    });

    // Write events with delta times
    let mut last_tick: u32 = 0;
    for ev in &events {
        let delta = ev.tick - last_tick;
        last_tick = ev.tick;
        write_variable_length(&mut track_data, delta);
        track_data.push(ev.status);
        track_data.push(ev.note);
        track_data.push(ev.velocity);
    }

    // End of track meta event
    write_variable_length(&mut track_data, 0);
    track_data.extend_from_slice(&[0xFF, 0x2F, 0x00]);

    // Write the SMF file
    let mut file = std::fs::File::create(&path)?;

    // Header chunk: MThd
    file.write_all(b"MThd")?;
    file.write_all(&(6u32).to_be_bytes())?; // header length
    file.write_all(&(0u16).to_be_bytes())?; // format 0
    file.write_all(&(1u16).to_be_bytes())?; // 1 track
    file.write_all(&ticks_per_quarter.to_be_bytes())?;

    // Track chunk: MTrk
    file.write_all(b"MTrk")?;
    file.write_all(&(track_data.len() as u32).to_be_bytes())?;
    file.write_all(&track_data)?;

    Ok(path)
}

/// Write a MIDI variable-length quantity.
fn write_variable_length(buf: &mut Vec<u8>, mut value: u32) {
    if value == 0 {
        buf.push(0);
        return;
    }
    // Encode in reverse, then push in order
    let mut bytes = Vec::with_capacity(4);
    bytes.push((value & 0x7F) as u8);
    value >>= 7;
    while value > 0 {
        bytes.push((value & 0x7F) as u8 | 0x80);
        value >>= 7;
    }
    bytes.reverse();
    buf.extend_from_slice(&bytes);
}

/// Get the export directory path.
fn dirs_export_path() -> PathBuf {
    if let Some(data_dir) = std::env::var_os("XDG_DATA_HOME") {
        PathBuf::from(data_dir).join("autokit/exports")
    } else if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".local/share/autokit/exports")
    } else {
        PathBuf::from("/tmp/autokit/exports")
    }
}
