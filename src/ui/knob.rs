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
    _id: egui::Id,
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
