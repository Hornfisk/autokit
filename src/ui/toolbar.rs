use std::sync::atomic::{AtomicU32, Ordering};

use nih_plug::prelude::*;
use nih_plug_egui::egui;

use crate::plugin::AutokitParams;
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
) -> ToolbarAction {
    let mut action = ToolbarAction::None;

    egui::Frame::NONE
        .fill(theme::BG_TOOLBAR)
        .inner_margin(egui::Margin { left: 16, right: 20, top: 8, bottom: 8 })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(28.0);
                ui.spacing_mut().item_spacing.x = 8.0;

                // Left: logo icon + "AUTOKIT" text + version
                let sized = egui::load::SizedTexture::new(logo_texture.id(), egui::vec2(22.0, 22.0));
                ui.image(sized);
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
                        .min_size(egui::vec2(36.0, 22.0))
                    };

                    if ui.add(tab("PADS", ViewMode::PadStrip)).clicked() && view_mode != ViewMode::PadStrip {
                        action = ToolbarAction::SetView(ViewMode::PadStrip);
                    }
                    if ui.add(tab("MAP", ViewMode::SampleMap)).clicked() && view_mode != ViewMode::SampleMap {
                        action = ToolbarAction::SetView(ViewMode::SampleMap);
                    }
                    if ui.add(tab("SEQ", ViewMode::Sequencer)).clicked() && view_mode != ViewMode::Sequencer {
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
                        .min_size(egui::vec2(22.0, 22.0)),
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
                    }
                }

                if let Some((pad_num, cat_label)) = shortcut_info {
                    ui.label(egui::RichText::new(format!("\u{2192} pad {}: {}", pad_num, cat_label))
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                        .color(theme::ACCENT));
                }

                // Tempo control — standalone only (plugin tempo is owned by host)
                if is_standalone {
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                    let mut bpm = standalone_tempo.load(Ordering::Relaxed) as f32 / 10.0;
                    let drag = egui::DragValue::new(&mut bpm)
                        .range(30.0..=300.0)
                        .speed(0.5)
                        .fixed_decimals(1)
                        .suffix(" BPM");
                    if ui.add_sized(egui::vec2(72.0, 22.0), drag).changed() {
                        standalone_tempo.store((bpm * 10.0) as u32, Ordering::Relaxed);
                    }
                    ui.add(egui::Separator::default().vertical().spacing(4.0));
                }

                // Right-aligned toolbar items — anchored to the right edge
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;

                    // Master volume knob
                    {
                        let mut gain_db = util::gain_to_db(params.master_volume.unmodulated_plain_value());
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("master_vol_knob"),
                            &mut gain_db, -60.0, 6.0, 0.0,
                            "Master volume (dB)",
                            |v| format!("{v:.1}"),
                            theme::ACCENT, 20.0,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.master_volume);
                            setter.set_parameter(&params.master_volume, util::db_to_gain(gain_db));
                            setter.end_set_parameter(&params.master_volume);
                        }
                        ui.label(
                            egui::RichText::new("VOL")
                                .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DISABLED),
                        );
                    }

                    ui.add(egui::Separator::default().vertical().spacing(4.0));

                    // ── Compressor / Drive / Limiter controls ──

                    // LIM toggle
                    {
                        let lim_val = params.limiter_on.value();
                        let lim_color = if lim_val { theme::ACCENT } else { theme::TEXT_DIM };
                        let lim_bg = if lim_val { theme::ACCENT_DIM } else { theme::BG_ROW };
                        if ui.add(
                            egui::Button::new(
                                egui::RichText::new("LIM")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(lim_color)
                                    .strong(),
                            )
                            .fill(lim_bg)
                            .min_size(egui::vec2(32.0, 22.0)),
                        ).clicked() {
                            setter.begin_set_parameter(&params.limiter_on);
                            setter.set_parameter(&params.limiter_on, !lim_val);
                            setter.end_set_parameter(&params.limiter_on);
                        }
                    }

                    // DRIVE knob
                    {
                        let mut drive = params.comp_drive.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("comp_drive_knob"),
                            &mut drive, 0.0, 1.0, 0.0,
                            "Saturation drive",
                            |v| format!("{:.0}%", v * 100.0),
                            theme::TEXT_DIM, 20.0,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.comp_drive);
                            setter.set_parameter(&params.comp_drive, drive);
                            setter.end_set_parameter(&params.comp_drive);
                        }
                        ui.label(
                            egui::RichText::new("DRV")
                                .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DISABLED),
                        );
                    }

                    // COMP threshold knob
                    {
                        let mut thr = params.comp_threshold.unmodulated_plain_value();
                        let resp = crate::ui::knob::knob_inline(
                            ui, egui::Id::new("comp_threshold_knob"),
                            &mut thr, -40.0, 0.0, -12.0,
                            "Compressor threshold (dB)",
                            |v| format!("{:.0}", v),
                            theme::ACCENT, 20.0,
                        );
                        if resp.changed {
                            setter.begin_set_parameter(&params.comp_threshold);
                            setter.set_parameter(&params.comp_threshold, thr);
                            setter.end_set_parameter(&params.comp_threshold);
                        }
                        ui.label(
                            egui::RichText::new("CMP")
                                .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DISABLED),
                        );
                    }

                    ui.add(egui::Separator::default().vertical().spacing(4.0));

                    // Load preset
                    let load_color = egui::Color32::from_rgb(0xff, 0x9f, 0x43);
                    let load_dim = egui::Color32::from_rgba_premultiplied(0x44, 0x28, 0x10, 0x44);
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

                    // Save preset
                    let save_color = egui::Color32::from_rgb(0x74, 0xb9, 0xff);
                    let save_dim = egui::Color32::from_rgba_premultiplied(0x1c, 0x2e, 0x44, 0x44);
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("S")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(save_color)
                                    .strong(),
                            )
                            .fill(save_dim)
                            .min_size(egui::vec2(22.0, 22.0)),
                        )
                        .on_hover_text("Save preset")
                        .clicked()
                    {
                        action = ToolbarAction::OpenSaveDialog;
                    }

                    ui.add(egui::Separator::default().vertical().spacing(4.0));

                    // Lock All
                    let lock_label = if all_locked { "UNLOCK ALL" } else { "LOCK ALL" };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new(lock_label)
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(theme::TEXT_DIM),
                            )
                            .fill(theme::BG_ROW)
                            .min_size(egui::vec2(60.0, 22.0)),
                        )
                        .clicked()
                    {
                        action = ToolbarAction::LockAll;
                    }

                    // Dice All
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("DICE ALL")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(theme::ACCENT)
                                    .strong(),
                            )
                            .fill(theme::ACCENT_DIM)
                            .min_size(egui::vec2(60.0, 22.0)),
                        )
                        .clicked()
                    {
                        action = ToolbarAction::DiceAll;
                    }

                    ui.add(egui::Separator::default().vertical().spacing(4.0));

                    // Redo
                    let redo_color = if can_redo { theme::TEXT_DIM } else { theme::TEXT_DISABLED };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("REDO")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(redo_color),
                            )
                            .fill(theme::BG_ROW)
                            .min_size(egui::vec2(44.0, 22.0)),
                        )
                        .clicked()
                        && can_redo
                    {
                        action = ToolbarAction::Redo;
                    }

                    // Undo
                    let undo_color = if can_undo { theme::TEXT_DIM } else { theme::TEXT_DISABLED };
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("UNDO")
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(undo_color),
                            )
                            .fill(theme::BG_ROW)
                            .min_size(egui::vec2(44.0, 22.0)),
                        )
                        .clicked()
                        && can_undo
                    {
                        action = ToolbarAction::Undo;
                    }
                });
            });
        });

    action
}
