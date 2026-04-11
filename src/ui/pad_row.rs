use nih_plug_egui::egui;

use crate::engine::kit::SampleCategory;
use crate::ui::knob;
use crate::ui::state::WaveformSummary;
use crate::ui::theme::{self, category_color};
use crate::ui::waveform;

/// Actions that a pad row can trigger.
pub enum PadRowAction {
    None,
    ToggleExpand,
    DicePad,
    DiceCategory,
    ToggleLock,
    PlayPad,
    BrowseSample,
    SetVolume(f32),
    SetPan(f32),
    SetPitch(f32),
    SetDecay(f32),
}

/// Truncate a string to `max_chars`, appending "…" if truncated.
fn truncate_name(name: &str, max_chars: usize) -> String {
    if name.chars().count() <= max_chars {
        name.to_string()
    } else {
        let truncated: String = name.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Draw a collapsed pad row from snapshot data (no mutex held).
pub fn draw_collapsed_from_snapshot(
    ui: &mut egui::Ui,
    _index: usize,
    has_sample: bool,
    name: &str,
    category: SampleCategory,
    volume: f32,
    waveform_summary: Option<&WaveformSummary>,
    is_selected: bool,
    play_brightness: f32,
    locked: bool,
    row_height: f32,
    tooltips_on: bool,
) -> (PadRowAction, egui::Rect) {
    let mut action = PadRowAction::None;
    let cat_color = category_color(category);
    let cat_egui = cat_color.to_egui();
    let waveform_opacity = if is_selected { 0.85 } else { 0.5 };

    let bg = if is_selected {
        theme::BG_ROW_HOVER
    } else {
        theme::BG_ROW
    };

    // Fixed widths for right-side buttons — guarantees they're always visible.
    const DICE_BTN_W: f32 = 46.0;
    const LOCK_BTN_W: f32 = 46.0;
    const BTN_H: f32 = 22.0;

    let frame_response = egui::Frame::NONE
        .fill(bg)
        .corner_radius(egui::CornerRadius {
            nw: 0,
            ne: 3,
            se: 3,
            sw: 0,
        })
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(row_height);
                // Use clip_rect width rather than available_width() — if any
                // earlier widget (e.g. the toolbar) overflows, egui expands
                // max_rect beyond the viewport, making available_width() too
                // large.  clip_rect is always bounded by the actual viewport.
                let total_w = ui.clip_rect().width();
                ui.spacing_mut().item_spacing.x = 0.0;

                // Color strip (3px) — brightened on trigger
                let (strip_rect, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, row_height), egui::Sense::hover());
                let strip_color = if play_brightness > 0.0 {
                    theme::brighten(cat_egui, play_brightness)
                } else {
                    cat_egui
                };
                ui.painter()
                    .rect_filled(strip_rect, egui::CornerRadius::ZERO, strip_color);

                ui.add_space(8.0);

                // Category tag — vertically centered in row
                let tag_size = egui::vec2(46.0, 16.0);
                ui.allocate_ui(egui::vec2(46.0, row_height), |ui| {
                    ui.centered_and_justified(|ui| {
                        let (tag_rect, _) =
                            ui.allocate_exact_size(tag_size, egui::Sense::hover());
                        if ui.is_rect_visible(tag_rect) {
                            let painter = ui.painter_at(tag_rect);
                            painter.rect_filled(tag_rect, 2, cat_egui);
                            painter.text(
                                tag_rect.center(),
                                egui::Align2::CENTER_CENTER,
                                category.label(),
                                egui::FontId::new(8.0, egui::FontFamily::Monospace),
                                theme::BG_MAIN,
                            );
                        }
                    });
                });

                ui.add_space(4.0);

                // LVL knob — inline, vertically centered
                {
                    let mut vol = volume;
                    let knob_resp = ui.allocate_ui(egui::vec2(16.0, row_height), |ui| {
                        ui.centered_and_justified(|ui| {
                            crate::ui::knob::knob_inline(
                                ui,
                                egui::Id::new(("pad_lvl", _index)),
                                &mut vol,
                                0.0, 1.0, 1.0,
                                "Pad volume",
                                |v| format!("{}", (v * 100.0) as u32),
                                cat_egui,
                                16.0,
                                tooltips_on,
                            )
                        })
                    });
                    if knob_resp.inner.inner.changed {
                        action = PadRowAction::SetVolume(vol);
                    }
                }

                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 6.0;

                // Play button (▶)
                let play_response = ui.add(
                    egui::Button::new(
                        egui::RichText::new("▶")
                            .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                            .color(theme::ACCENT),
                    )
                    .fill(egui::Color32::TRANSPARENT)
                    .min_size(egui::vec2(18.0, row_height)),
                );
                theme::tip(play_response.clone(), "Preview this pad (or press keyboard key)", tooltips_on);
                if play_response.clicked() {
                    action = PadRowAction::PlayPad;
                }

                // Sample name
                let display_name = if has_sample {
                    truncate_name(name, 20)
                } else {
                    "—".to_string()
                };
                let name_response = ui.add_sized(
                    egui::vec2(140.0, ui.available_height()),
                    egui::Label::new(
                        egui::RichText::new(&display_name)
                            .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                            .color(egui::Color32::from_rgb(0xaa, 0xaa, 0xaa)),
                    )
                    .truncate()
                    .sense(egui::Sense::click()),
                );
                if has_sample && name.chars().count() > 20 {
                    theme::tip(name_response.clone(), name, tooltips_on);
                }
                if name_response.clicked() {
                    action = PadRowAction::ToggleExpand;
                }

                // Waveform — compute from total width minus all fixed elements.
                // Fixed: strip(3) + space(8) + tag(46) + spacing(6) + play(18+6)
                //        + name(140+6) + browse(22+6) + dice(46+6) + lock(46+6)
                // Added: 16px knob + 4px space = 20px
                const FIXED_W: f32 = 3.0 + 8.0 + 46.0 + 4.0 + 16.0 + 4.0 + 6.0 + 24.0 + 146.0 + 28.0 + 52.0 + 52.0 + 6.0 + 12.0;
                let waveform_width = (total_w - FIXED_W).max(40.0);
                let wf_color = cat_color.to_egui_alpha((waveform_opacity * 255.0) as u8);
                waveform::paint_waveform(
                    ui,
                    waveform_summary,
                    wf_color,
                    egui::vec2(waveform_width, 26.0),
                );

                ui.add_space(6.0);

                // Browse (…) button — open native file dialog for this pad
                let browse_response = ui.add(
                    egui::Button::new(
                        egui::RichText::new("…")
                            .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DIM)
                            .strong(),
                    )
                    .fill(theme::BG_DETAIL)
                    .stroke(egui::Stroke::new(1.0, theme::TEXT_DIM.linear_multiply(0.4)))
                    .corner_radius(3)
                    .min_size(egui::vec2(22.0, BTN_H)),
                );
                if browse_response.clicked() {
                    action = PadRowAction::BrowseSample;
                }
                theme::tip(browse_response, "Browse for a sample file (or drop one here)", tooltips_on);

                // DICE button — boxed text
                let dice_response = ui.add(
                    egui::Button::new(
                        egui::RichText::new("DICE")
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(theme::ACCENT)
                            .strong(),
                    )
                    .fill(theme::ACCENT_DIM)
                    .stroke(egui::Stroke::new(1.0, theme::ACCENT.linear_multiply(0.4)))
                    .corner_radius(3)
                    .min_size(egui::vec2(DICE_BTN_W, BTN_H)),
                );
                if dice_response.clicked() {
                    action = PadRowAction::DicePad;
                }
                theme::tip(dice_response, "Randomize this pad", tooltips_on);

                // LOCK button — boxed text, inverted when locked
                let lock_label = if locked { "LOCK" } else { "LOCK" };
                let lock_fg = if locked { theme::BG_MAIN } else { theme::TEXT_DIM };
                let lock_bg = if locked { theme::ACCENT } else { theme::BG_DETAIL };
                let lock_stroke = if locked {
                    egui::Stroke::new(1.0, theme::ACCENT)
                } else {
                    egui::Stroke::new(1.0, theme::TEXT_DIM.linear_multiply(0.5))
                };
                let lock_response = ui.add(
                    egui::Button::new(
                        egui::RichText::new(lock_label)
                            .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                            .color(lock_fg)
                            .strong(),
                    )
                    .fill(lock_bg)
                    .stroke(lock_stroke)
                    .corner_radius(3)
                    .min_size(egui::vec2(LOCK_BTN_W, BTN_H)),
                );
                if lock_response.clicked() {
                    action = PadRowAction::ToggleLock;
                }
                theme::tip(lock_response, if locked { "Unlock pad (sample will change on dice)" } else { "Lock pad (keep sample on dice)" }, tooltips_on);
            });
        });

    // Glow overlay on trigger
    if play_brightness > 0.0 {
        let glow_alpha = (play_brightness * 0.18 * 255.0) as u8;
        let glow_color = cat_color.to_egui_alpha(glow_alpha);
        ui.painter().rect_filled(
            frame_response.response.rect,
            egui::CornerRadius { nw: 0, ne: 3, se: 3, sw: 0 },
            glow_color,
        );
    }

    (action, frame_response.response.rect)
}

/// Draw the expanded detail panel from snapshot data (no mutex held).
pub fn draw_expanded_from_snapshot(
    ui: &mut egui::Ui,
    index: usize,
    category: SampleCategory,
    pan: f32,
    pitch: f32,
    decay: f32,
    tooltips_on: bool,
) -> PadRowAction {
    let mut action = PadRowAction::None;
    let cat_color = category_color(category);
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
                let (strip_rect, _) =
                    ui.allocate_exact_size(egui::vec2(3.0, 50.0), egui::Sense::hover());
                ui.painter().rect_filled(
                    strip_rect,
                    egui::CornerRadius::ZERO,
                    cat_color.to_egui_alpha(0x33),
                );

                ui.add_space(12.0);

                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 16.0;

                    // Pan knob
                    let mut p = pan;
                    let pan_result = knob::knob(
                        ui,
                        egui::Id::new(("pan", index)),
                        &mut p,
                        -1.0, 1.0, 0.0,
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
                        action = PadRowAction::SetPan(p);
                    }

                    // Pitch knob
                    let mut pt = pitch;
                    let pitch_result = knob::knob(
                        ui,
                        egui::Id::new(("pitch", index)),
                        &mut pt,
                        -24.0, 24.0, 0.0,
                        "PITCH",
                        |v| format!("{:+.0}", v),
                        cat_color.to_egui_alpha(0x88),
                        34.0,
                    );
                    if pitch_result.changed {
                        action = PadRowAction::SetPitch(pt);
                    }

                    // Decay knob
                    let mut dc = decay;
                    let decay_result = knob::knob(
                        ui,
                        egui::Id::new(("decay", index)),
                        &mut dc,
                        0.01, 1.0, 1.0,
                        "DECAY",
                        |v| format!("{}%", (v * 100.0) as u32),
                        cat_color.to_egui_alpha(0x88),
                        34.0,
                    );
                    if decay_result.changed {
                        action = PadRowAction::SetDecay(dc);
                    }

                    ui.add(egui::Separator::default().vertical().spacing(8.0));

                    // Dice category button
                    let cat_label = format!("DICE {}S", category.label());
                    let resp = ui.add(
                        egui::Button::new(
                            egui::RichText::new(&cat_label)
                                .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                                .color(cat_egui),
                        )
                        .fill(cat_color.to_egui_alpha(0x11))
                        .corner_radius(3),
                    );
                    theme::tip(resp.clone(), "Randomize within this category only", tooltips_on);
                    if resp.clicked() {
                        action = PadRowAction::DiceCategory;
                    }
                });
            });
        });

    action
}
