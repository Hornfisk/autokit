use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::engine::kit::SampleCategory;
use crate::plugin::AutokitParams;
use crate::ui::pad_row::{self, PadRowAction};
use crate::ui::state::{ScanStatus, SharedState, WaveformSummary};
use crate::ui::theme;
use crate::ui::toolbar::{self, ToolbarAction};
use crate::util::history::HistorySnapshot;

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
}

/// Everything the GUI needs to render one frame, captured in a brief lock.
struct DisplaySnapshot {
    pads: [PadDisplay; 16],
    waveforms: [Option<WaveformSummary>; 16],
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

            // --- Phase 1: Brief lock to snapshot display state ---
            let snap = {
                let shared = shared.lock();
                DisplaySnapshot::from_shared(&shared)
            };
            // Lock is now released — audio thread can proceed freely.

            // Collect any actions triggered during rendering.
            let mut pending_action: Option<GuiAction> = None;

            egui::CentralPanel::default()
                .frame(egui::Frame::NONE.fill(theme::BG_MAIN))
                .show(ctx, |ui| {
                    // Toolbar (uses snapshot data, no mutex held)
                    let all_locked = snap.pads.iter().all(|p| p.locked);
                    let toolbar_action = toolbar::draw_toolbar_snapshot(
                        ui,
                        &snap.scan_status,
                        snap.can_undo,
                        snap.can_redo,
                        all_locked,
                        &params,
                        setter,
                        state.scale,
                    );

                    match toolbar_action {
                        ToolbarAction::Undo => pending_action = Some(GuiAction::Undo),
                        ToolbarAction::Redo => pending_action = Some(GuiAction::Redo),
                        ToolbarAction::DiceAll => {
                            if snap.has_library {
                                pending_action = Some(GuiAction::DiceAll);
                            }
                        }
                        ToolbarAction::LockAll => pending_action = Some(GuiAction::LockAll),
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
                                let pad = &snap.pads[i];
                                let wf = snap.waveforms[i].as_ref();

                                let row_action = pad_row::draw_collapsed_from_snapshot(
                                    ui, i, pad.has_sample, &pad.name, pad.category,
                                    pad.volume, wf, is_selected,
                                );

                                match row_action {
                                    PadRowAction::ToggleExpand => {
                                        state.selected_pad =
                                            if is_selected { None } else { Some(i) };
                                    }
                                    PadRowAction::DicePad => {
                                        if snap.has_library {
                                            pending_action = Some(GuiAction::DicePad(i));
                                        }
                                    }
                                    _ => {}
                                }

                                // Expanded detail
                                if is_selected {
                                    let detail_action = pad_row::draw_expanded_from_snapshot(
                                        ui, i, pad.category, pad.volume, pad.pan,
                                        pad.pitch, pad.locked,
                                    );

                                    match detail_action {
                                        PadRowAction::SetVolume(v) => {
                                            pending_action = Some(GuiAction::SetPadVolume(i, v));
                                        }
                                        PadRowAction::SetPan(v) => {
                                            pending_action = Some(GuiAction::SetPadPan(i, v));
                                        }
                                        PadRowAction::SetPitch(v) => {
                                            pending_action = Some(GuiAction::SetPadPitch(i, v));
                                        }
                                        PadRowAction::ToggleLock => {
                                            pending_action = Some(GuiAction::ToggleLock(i));
                                        }
                                        PadRowAction::DicePad => {
                                            if snap.has_library {
                                                pending_action = Some(GuiAction::DicePad(i));
                                            }
                                        }
                                        PadRowAction::DiceCategory => {
                                            if snap.has_library {
                                                pending_action =
                                                    Some(GuiAction::DiceCategory(i, pad.category));
                                            }
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        });
                });

            // --- Phase 2: Brief lock to apply any mutation ---
            if let Some(action) = pending_action {
                let mut shared = shared.lock();
                let seq_snap = &sequencer_snapshot_fn;
                match action {
                    GuiAction::Undo => {
                        let current = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: seq_snap(),
                        };
                        if let Some(restored) = shared.history.undo(current) {
                            shared.kit.restore(&restored.pads);
                            shared.update_all_waveforms(WAVEFORM_POINTS);
                        }
                    }
                    GuiAction::Redo => {
                        let current = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: seq_snap(),
                        };
                        if let Some(restored) = shared.history.redo(current) {
                            shared.kit.restore(&restored.pads);
                            shared.update_all_waveforms(WAVEFORM_POINTS);
                        }
                    }
                    GuiAction::DiceAll => {
                        let snapshot = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: seq_snap(),
                        };
                        shared.history.push(snapshot);
                        let lib = shared.library.as_ref().unwrap().clone_for_dice();
                        shared.kit.dice_all(&lib);
                        shared.update_all_waveforms(WAVEFORM_POINTS);
                    }
                    GuiAction::DicePad(i) => {
                        let snapshot = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: seq_snap(),
                        };
                        shared.history.push(snapshot);
                        let lib = shared.library.as_ref().unwrap().clone_for_dice();
                        shared.kit.dice_pad(i, &lib);
                        shared.update_waveform(i, WAVEFORM_POINTS);
                    }
                    GuiAction::DiceCategory(i, cat) => {
                        let snapshot = HistorySnapshot {
                            pads: shared.kit.snapshot(),
                            sequencer: seq_snap(),
                        };
                        shared.history.push(snapshot);
                        let lib = shared.library.as_ref().unwrap().clone_for_dice();
                        shared.kit.dice_category(cat, &lib);
                        shared.update_all_waveforms(WAVEFORM_POINTS);
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
                }
                // Lock drops here — held only for the mutation.
            }
        },
    )
}

/// Actions that the GUI can trigger, applied in a brief second lock.
enum GuiAction {
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
}
