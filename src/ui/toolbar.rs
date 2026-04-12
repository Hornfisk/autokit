use std::sync::atomic::{AtomicU32, Ordering};

use nih_plug::prelude::*;
use nih_plug_egui::egui;

use crate::plugin::{AutokitParams, SequencerSync};
use crate::ui::editor::ViewMode;
use crate::ui::state::ScanStatus;
use crate::ui::theme;

const ICON_SIZE: usize = 64;
const ICON_RGBA: &[u8] = include_bytes!("../../assets/icon_64_rgba.bin");

/// Actions the toolbar can trigger.
pub enum ToolbarAction {
    None,
    Undo,
    Redo,
    DiceAll,
    LockAll,
    OpenSaveDialog,
    OpenLoadDialog,
    ToggleView,
    SetView(ViewMode),
    ToggleTooltips,
    OpenSetup,
    ClearAutomation,
}

/// Load the logo texture (call once, cache the handle).
pub fn load_logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    ctx.load_texture(
        "autokit_logo",
        egui::ColorImage::from_rgba_unmultiplied([ICON_SIZE, ICON_SIZE], ICON_RGBA),
        egui::TextureOptions::LINEAR,
    )
}

/// Draw the toolbar from snapshot data (no mutex held).
pub fn draw_toolbar_snapshot(
    ui: &mut egui::Ui,
    scan_status: &ScanStatus,
    can_undo: bool,
    can_redo: bool,
    all_locked: bool,
    params: &AutokitParams,
    setter: &ParamSetter,
    view_mode: ViewMode,
    shortcut_info: Option<(usize, &str)>,
    logo_texture: &egui::TextureHandle,
    scan_processed: u32,
    scan_total: u32,
    is_standalone: bool,
    standalone_tempo: &AtomicU32,
    tooltips_on: bool,
    seq_sync: &SequencerSync,
) -> ToolbarAction {
    let mut action = ToolbarAction::None;

    egui::Frame::NONE
        .fill(theme::BG_TOOLBAR)
        .inner_margin(egui::Margin { left: 10, right: 12, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                ui.set_height(44.0);
                ui.spacing_mut().item_spacing.x = 5.0;

                // Left: logo icon + "AUTOKIT" stacked above version number
                let sized = egui::load::SizedTexture::new(logo_texture.id(), egui::vec2(30.0, 30.0));
                ui.image(sized);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(
                        egui::RichText::new("AUTOKIT")
                            .font(egui::FontId::new(18.0, egui::FontFamily::Monospace))
                            .color(egui::Color32::from_rgb(0xe6, 0x58, 0x8c))
                            .strong(),
                    );
                    ui.label(
                        egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DISABLED),
                    );
                });

                // View toggle: PADS / MAP / SEQ
                {
                    let tab = |label: &str, mode: ViewMode| -> egui::Button {
                        let is_active = view_mode == mode;
                        let color = if is_active { theme::ACCENT } else { theme::TEXT_DIM };
                        let bg = if is_active { theme::ACCENT_DIM } else { theme::BG_ROW };
                        egui::Button::new(
                            egui::RichText::new(label)
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(color),
                        )
                        .fill(bg)
                    };

                    let tab_w = 32.0;
                    let resp = ui.add(tab("PADS", ViewMode::PadStrip).min_size(egui::vec2(tab_w, 20.0)));
                    theme::tip(resp.clone(), "Pad strip view", tooltips_on);
                    if resp.clicked() && view_mode != ViewMode::PadStrip {
                        action = ToolbarAction::SetView(ViewMode::PadStrip);
                    }
                    let resp = ui.add(tab("MAP", ViewMode::SampleMap).min_size(egui::vec2(tab_w, 20.0)));
                    theme::tip(resp.clone(), "Sample map scatter plot", tooltips_on);
                    if resp.clicked() && view_mode != ViewMode::SampleMap {
                        action = ToolbarAction::SetView(ViewMode::SampleMap);
                    }
                    let resp = ui.add(tab("SEQ", ViewMode::Sequencer).min_size(egui::vec2(tab_w, 20.0)));
                    theme::tip(resp.clone(), "Step sequencer", tooltips_on);
                    if resp.clicked() && view_mode != ViewMode::Sequencer {
                        action = ToolbarAction::SetView(ViewMode::Sequencer);
                    }
                }

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
                        .min_size(egui::vec2(18.0, 20.0)),
                    );
                    // Always shown so users can discover the toggle
                    let tip_btn = tip_btn.on_hover_text("Toggle help tooltips");
                    if tip_btn.clicked() {
                        action = ToolbarAction::ToggleTooltips;
                    }
                }

                match scan_status {
                    ScanStatus::NeedsSetup { .. } => {
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new("set sample folder...")
                                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                        .color(theme::TEXT_DIM),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .frame(false),
                            )
                            .clicked()
                        {
                            action = ToolbarAction::OpenSetup;
                        }
                    }
                    ScanStatus::Scanning => {
                        if scan_total > 0 {
                            let pct = (scan_processed as f32 / scan_total as f32).clamp(0.0, 1.0);
                            let label = format!("scanning... {}/{}", scan_processed, scan_total);
                            ui.label(
                                egui::RichText::new(&label)
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(theme::TEXT_DIM),
                            );
                            let bar_width = 80.0;
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(bar_width, 8.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().rect_filled(
                                rect,
                                2.0,
                                theme::BG_ROW,
                            );
                            let mut fill_rect = rect;
                            fill_rect.set_right(rect.left() + bar_width * pct);
                            ui.painter().rect_filled(
                                fill_rect,
                                2.0,
                                theme::ACCENT,
                            );
                        } else {
                            ui.label(
                                egui::RichText::new("scanning...")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(theme::TEXT_DIM),
                            );
                        }
                    }
                    ScanStatus::Ready { total } => {
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
                    }
                }

                if let Some((pad_num, cat_label)) = shortcut_info {
                    ui.label(egui::RichText::new(format!("\u{2192} pad {}: {}", pad_num, cat_label))
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                        .color(theme::ACCENT));
                }

                // Tempo control — standalone only (plugin tempo is owned by host)
                if is_standalone {
                    ui.add(egui::Separator::default().vertical().spacing(2.0));
                    let mut bpm = standalone_tempo.load(Ordering::Relaxed) as f32 / 10.0;
                    let drag = egui::DragValue::new(&mut bpm)
                        .range(30.0..=300.0)
                        .speed(0.5)
                        .fixed_decimals(1)
                        .suffix(" BPM");
                    let resp = ui.add_sized(egui::vec2(72.0, 22.0), drag);
                    theme::tip(resp.clone(), "Tempo (BPM)", tooltips_on);
                    if resp.changed() {
                        standalone_tempo.store((bpm * 10.0) as u32, Ordering::Relaxed);
                    }
                    ui.add(egui::Separator::default().vertical().spacing(2.0));
                }

                // Right-aligned toolbar items — anchored to the right edge
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;

                    // Master volume knob
                    knob_column(ui, "VOL", |ui| {
                        let mut gain_db = util::gain_to_db(params.master_volume.unmodulated_plain_value());
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("master_vol_knob"),
                            &mut gain_db, -60.0, 6.0, 0.0,
                            "Master volume (dB). Double-click to reset",
                            |v| format!("{v:.1}"),
                            theme::ACCENT, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.master_volume);
                            setter.set_parameter(&params.master_volume, util::db_to_gain(gain_db));
                            setter.end_set_parameter(&params.master_volume);
                        }
                    });

                    ui.add(egui::Separator::default().vertical().spacing(2.0));

                    // ── Compressor / Drive / Limiter controls ──

                    // LIM toggle — wrapped in a column so its top edge
                    // aligns with the knob tops, with a spacer below to
                    // compensate for the missing label.
                    ui.allocate_ui_with_layout(
                        egui::vec2(30.0, 36.0),
                        egui::Layout::top_down(egui::Align::Center),
                        |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            let lim_val = params.limiter_on.value();
                            let lim_color = if lim_val { theme::ACCENT } else { theme::TEXT_DIM };
                            let lim_bg = if lim_val { theme::ACCENT_DIM } else { theme::BG_ROW };
                            let resp = ui.add(
                                egui::Button::new(
                                    egui::RichText::new("LIM")
                                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                        .color(lim_color)
                                        .strong(),
                                )
                                .fill(lim_bg)
                                .min_size(egui::vec2(28.0, 20.0)),
                            );
                            theme::tip(resp.clone(), "Master limiter on/off", tooltips_on);
                            if resp.clicked() {
                                setter.begin_set_parameter(&params.limiter_on);
                                setter.set_parameter(&params.limiter_on, !lim_val);
                                setter.end_set_parameter(&params.limiter_on);
                            }
                        },
                    );

                    // DRIVE knob
                    knob_column(ui, "DRV", |ui| {
                        let mut drive = params.comp_drive.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("comp_drive_knob"),
                            &mut drive, 0.0, 1.0, 0.0,
                            "Saturation drive. Double-click to reset",
                            |v| format!("{:.0}%", v * 100.0),
                            theme::TEXT_DIM, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.comp_drive);
                            setter.set_parameter(&params.comp_drive, drive);
                            setter.end_set_parameter(&params.comp_drive);
                        }
                    });

                    // COMP threshold knob
                    knob_column(ui, "CMP", |ui| {
                        let mut thr = params.comp_threshold.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("comp_threshold_knob"),
                            &mut thr, -40.0, 0.0, -12.0,
                            "Master compressor threshold (dB). Double-click to reset",
                            |v| format!("{:.0}", v),
                            theme::ACCENT, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.comp_threshold);
                            setter.set_parameter(&params.comp_threshold, thr);
                            setter.end_set_parameter(&params.comp_threshold);
                        }
                    });

                    ui.add(egui::Separator::default().vertical().spacing(2.0));

                    // ── Master FX: reverb, delay, DJ filter ──

                    // DJ FILTER knob (bipolar, LP ← → HP)
                    knob_column(ui, "DJF", |ui| {
                        let mut dj = params.dj_filter.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("dj_filter_knob"),
                            &mut dj, -1.0, 1.0, 0.0,
                            "DJ filter. Left = lowpass kill, right = highpass kill, center = off. Double-click to reset",
                            |v| if v.abs() < 0.01 { "OFF".to_string() }
                                else if v < 0.0 { format!("LP {:.0}%", -v * 100.0) }
                                else { format!("HP {:.0}%", v * 100.0) },
                            theme::FX_FILTER, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.dj_filter);
                            setter.set_parameter(&params.dj_filter, dj);
                            setter.end_set_parameter(&params.dj_filter);
                            seq_sync.fx_touch_flt.fetch_add(1, Ordering::Relaxed);
                        }
                    });

                    // DELAY TIME knob (discrete quarters of a beat)
                    knob_column(ui, "DLT", |ui| {
                        let mut dt = params.delay_time.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("delay_time_knob"),
                            &mut dt, 0.0, 1.0, 0.5,
                            "Delay time. Snaps to 1/32, 1/16, 1/8, 1/4 note. Double-click to reset",
                            |v| match (v * 3.0).round() as i32 {
                                0 => "1/32".to_string(),
                                1 => "1/16".to_string(),
                                2 => "1/8".to_string(),
                                _ => "1/4".to_string(),
                            },
                            theme::FX_DELAY, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.delay_time);
                            setter.set_parameter(&params.delay_time, dt);
                            setter.end_set_parameter(&params.delay_time);
                        }
                    });

                    // DELAY MIX knob
                    knob_column(ui, "DLY", |ui| {
                        let mut dm = params.delay_mix.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("delay_mix_knob"),
                            &mut dm, 0.0, 1.0, 0.0,
                            "Delay return level. Double-click to reset",
                            |v| format!("{:.0}%", v * 100.0),
                            theme::FX_DELAY, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.delay_mix);
                            setter.set_parameter(&params.delay_mix, dm);
                            setter.end_set_parameter(&params.delay_mix);
                            seq_sync.fx_touch_dly.fetch_add(1, Ordering::Relaxed);
                        }
                    });

                    // REVERB MIX knob
                    knob_column(ui, "RVB", |ui| {
                        let mut rv = params.reverb_mix.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("reverb_mix_knob"),
                            &mut rv, 0.0, 1.0, 0.0,
                            "Reverb return level. Double-click to reset",
                            |v| format!("{:.0}%", v * 100.0),
                            theme::FX_REVERB, 20.0,
                            tooltips_on,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.reverb_mix);
                            setter.set_parameter(&params.reverb_mix, rv);
                            setter.end_set_parameter(&params.reverb_mix);
                            seq_sync.fx_touch_rvb.fetch_add(1, Ordering::Relaxed);
                        }
                    });

                    // ── REC / CLR — live automation recording for master FX ──
                    let rec_armed = seq_sync.rec_armed.load(Ordering::Relaxed);
                    let rec_on = egui::Color32::from_rgb(220, 60, 60);
                    let rec_off = egui::Color32::from_rgba_premultiplied(0x44, 0x14, 0x14, 0x44);
                    let rec_fill = if rec_armed { rec_on } else { rec_off };
                    let rec_text = if rec_armed { egui::Color32::WHITE } else { rec_on };
                    let rec_resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("REC")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(rec_text)
                                .strong(),
                        )
                        .fill(rec_fill)
                        .min_size(egui::vec2(24.0, 20.0)),
                    );
                    theme::tip(rec_resp.clone(), "Arm live recording of master FX automation into the active pattern", tooltips_on);
                    if rec_resp.clicked() {
                        seq_sync.rec_armed.store(!rec_armed, Ordering::Relaxed);
                    }

                    let clr_color = theme::FX_FILTER;
                    let clr_dim = egui::Color32::from_rgba_premultiplied(0x44, 0x28, 0x10, 0x44);
                    let clr_resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("CLR")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(clr_color)
                                .strong(),
                        )
                        .fill(clr_dim)
                        .min_size(egui::vec2(24.0, 20.0)),
                    );
                    theme::tip(clr_resp.clone(), "Clear all master FX automation in the active pattern", tooltips_on);
                    if clr_resp.clicked() {
                        action = ToolbarAction::ClearAutomation;
                    }

                    ui.add(egui::Separator::default().vertical().spacing(2.0));

                    // Load preset
                    let load_color = egui::Color32::from_rgb(0xff, 0x9f, 0x43);
                    let load_dim = egui::Color32::from_rgba_premultiplied(0x44, 0x28, 0x10, 0x44);
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("L")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(load_color)
                                .strong(),
                        )
                        .fill(load_dim)
                        .min_size(egui::vec2(18.0, 20.0)),
                    );
                    theme::tip(resp.clone(), "Load preset", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::OpenLoadDialog;
                    }

                    // Save preset
                    let save_color = egui::Color32::from_rgb(0x74, 0xb9, 0xff);
                    let save_dim = egui::Color32::from_rgba_premultiplied(0x1c, 0x2e, 0x44, 0x44);
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("S")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(save_color)
                                .strong(),
                        )
                        .fill(save_dim)
                        .min_size(egui::vec2(18.0, 20.0)),
                    );
                    theme::tip(resp.clone(), "Save preset", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::OpenSaveDialog;
                    }

                    ui.add(egui::Separator::default().vertical().spacing(2.0));

                    // Lock All
                    let lock_label = if all_locked { "UNLOCK" } else { "LOCK" };
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new(lock_label)
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(egui::Color32::WHITE),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(46.0, 20.0)),
                    );
                    theme::tip(resp.clone(), "Lock/unlock all pads (locked pads keep their sample on dice)", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::LockAll;
                    }

                    // Dice All
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("DICE")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(theme::ACCENT)
                                .strong(),
                        )
                        .fill(theme::ACCENT_DIM)
                        .min_size(egui::vec2(42.0, 20.0)),
                    );
                    theme::tip(resp.clone(), "Randomize all unlocked pads", tooltips_on);
                    if resp.clicked() {
                        action = ToolbarAction::DiceAll;
                    }

                    ui.add(egui::Separator::default().vertical().spacing(2.0));

                    // Redo — bright when available so it's visible on the dark toolbar
                    let redo_color = if can_redo { egui::Color32::WHITE } else { theme::TEXT_DISABLED };
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("REDO")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(redo_color),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(38.0, 20.0)),
                    );
                    theme::tip(resp.clone(), "Redo last undone change", tooltips_on);
                    if resp.clicked() && can_redo {
                        action = ToolbarAction::Redo;
                    }

                    // Undo
                    let undo_color = if can_undo { egui::Color32::WHITE } else { theme::TEXT_DISABLED };
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new("UNDO")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(undo_color),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(38.0, 20.0)),
                    );
                    theme::tip(resp.clone(), "Undo last change", tooltips_on);
                    if resp.clicked() && can_undo {
                        action = ToolbarAction::Undo;
                    }
                });
            });
        });

    action
}

/// Stack a knob body and a small white centered label underneath it.
/// Used to lay out all master-bus/FX knobs with labels below rather than
/// inline to their left — keeps the toolbar compact and readable.
fn knob_column(
    ui: &mut egui::Ui,
    label: &str,
    knob_body: impl FnOnce(&mut egui::Ui),
) {
    ui.allocate_ui_with_layout(
        egui::vec2(24.0, 36.0),
        egui::Layout::top_down(egui::Align::Center),
        |ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            knob_body(ui);
            ui.label(
                egui::RichText::new(label)
                    .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                    .color(egui::Color32::WHITE),
            );
        },
    );
}
