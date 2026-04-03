use nih_plug::prelude::*;
use nih_plug_egui::egui;
use nih_plug_egui::{create_egui_editor, EguiState};
use parking_lot::Mutex;
use std::sync::Arc;

use crate::plugin::AutokitParams;
use crate::ui::pad_row::{self, PadRowAction};
use crate::ui::state::SharedState;
use crate::ui::theme;
use crate::ui::toolbar::{self, ToolbarAction};
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
                .frame(egui::Frame::NONE.fill(theme::BG_MAIN))
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

                                let row_action =
                                    pad_row::draw_collapsed(ui, i, pad, wf, is_selected);

                                match row_action {
                                    PadRowAction::ToggleExpand => {
                                        state.selected_pad =
                                            if is_selected { None } else { Some(i) };
                                    }
                                    PadRowAction::DicePad => {
                                        if shared.library.is_some() {
                                            let snapshot = HistorySnapshot {
                                                pads: shared.kit.snapshot(),
                                                sequencer: sequencer_snapshot_fn(),
                                            };
                                            shared.history.push(snapshot);
                                            let lib =
                                                shared.library.as_ref().unwrap().clone_for_dice();
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
                                                let lib = shared
                                                    .library
                                                    .as_ref()
                                                    .unwrap()
                                                    .clone_for_dice();
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
                                                let lib = shared
                                                    .library
                                                    .as_ref()
                                                    .unwrap()
                                                    .clone_for_dice();
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
