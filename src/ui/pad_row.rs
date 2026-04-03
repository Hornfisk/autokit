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
    _index: usize,
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
    egui::Frame::NONE
        .fill(bg)
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 3,
            se: 3,
            sw: 0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(34.0);
                ui.spacing_mut().item_spacing.x = 0.0;

                // Color strip (3px)
                let (strip_rect, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, 34.0), egui::Sense::hover());
                ui.painter()
                    .rect_filled(strip_rect, egui::CornerRadius::ZERO, cat_egui);

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
                            painter.rect_filled(tag_rect, 2, cat_egui);
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
                        ui.add_sized(
                            egui::vec2(170.0, ui.available_height()),
                            egui::Label::new(
                                egui::RichText::new(name)
                                    .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                                    .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa)),
                            )
                            .truncate(),
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
                            painter.rect_filled(vol_rect, 2, theme::BG_MAIN);
                            let fill_width = vol_rect.width() * pad.volume;
                            let fill_rect = egui::Rect::from_min_size(
                                vol_rect.min,
                                egui::vec2(fill_width, vol_rect.height()),
                            );
                            let fill_color = cat_color.to_egui_alpha(0x66);
                            // Glow: wider rect behind at low opacity
                            let glow_rect = fill_rect.expand2(egui::vec2(0.0, 1.5));
                            painter.rect_filled(glow_rect, 2, cat_color.to_egui_alpha(0x18));
                            painter.rect_filled(fill_rect, 2, fill_color);
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

    egui::Frame::NONE
        .fill(theme::BG_DETAIL)
        .inner_margin(egui::Margin::symmetric(16, 10))
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 3,
            se: 3,
            sw: 0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                // Left border continuation
                let (strip_rect, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, 50.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    strip_rect,
                    egui::CornerRadius::ZERO,
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
                                egui::RichText::new(if pad.locked { "LOCKED" } else { "LOCK" })
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
                            .corner_radius(3),
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
                            .corner_radius(3),
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
