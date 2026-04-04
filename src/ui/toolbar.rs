use nih_plug::prelude::*;
use nih_plug_egui::egui;

use crate::plugin::AutokitParams;
use crate::ui::editor::ViewMode;
use crate::ui::state::ScanStatus;
use crate::ui::theme;

/// Actions the toolbar can trigger.
pub enum ToolbarAction {
    None,
    Undo,
    Redo,
    DiceAll,
    LockAll,
    SetScale(f32),
    OpenSaveDialog,
    OpenLoadDialog,
    ToggleView,
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
    current_scale: f32,
    view_mode: ViewMode,
) -> ToolbarAction {
    let mut action = ToolbarAction::None;

    egui::Frame::NONE
        .fill(theme::BG_TOOLBAR)
        .inner_margin(egui::Margin { left: 16, right: 20, top: 8, bottom: 8 })
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

                match scan_status {
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

                // Reserve space for right-aligned toolbar items.
                // Underestimating this constant clips right-side items;
                // overestimating wastes space but is harmless.
                const RIGHT_SIDE_BUDGET: f32 = 680.0;
                let spacer = (ui.available_width() - RIGHT_SIDE_BUDGET).max(0.0);
                ui.add_space(spacer);

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

                ui.add(egui::Separator::default().vertical().spacing(4.0));

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

                ui.add(egui::Separator::default().vertical().spacing(4.0));

                // Save preset
                let save_color = egui::Color32::from_rgb(0x74, 0xb9, 0xff); // blue
                let save_dim = egui::Color32::from_rgba_premultiplied(0x1c, 0x2e, 0x44, 0x44);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("SAVE")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(save_color)
                                .strong(),
                        )
                        .fill(save_dim)
                        .min_size(egui::vec2(44.0, 22.0)),
                    )
                    .clicked()
                {
                    action = ToolbarAction::OpenSaveDialog;
                }

                // Load preset
                let load_color = egui::Color32::from_rgb(0xff, 0x9f, 0x43); // orange
                let load_dim = egui::Color32::from_rgba_premultiplied(0x44, 0x28, 0x10, 0x44);
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("LOAD")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(load_color)
                                .strong(),
                        )
                        .fill(load_dim)
                        .min_size(egui::vec2(44.0, 22.0)),
                    )
                    .clicked()
                {
                    action = ToolbarAction::OpenLoadDialog;
                }

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

                ui.add_space(8.0);

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
                            if ui
                                .selectable_label((current_scale - s).abs() < 0.01, &label)
                                .clicked()
                            {
                                action = ToolbarAction::SetScale(s);
                            }
                        }
                    });
            });
        });

    action
}
