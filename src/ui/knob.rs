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

        // Double-click detection from raw pointer events. egui-baseview
        // doesn't provide click_count in PointerButton events, so egui's
        // built-in double_clicked() never fires. We detect press events on
        // the knob rect manually and track timestamps.
        let dbl_id = response.id.with("last_press");
        let pointer_pressed_on_knob = ui.input(|i| {
            i.events.iter().any(|e| matches!(
                e,
                egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Primary, .. }
            ))
        }) && response.contains_pointer();
        let is_double_click = if pointer_pressed_on_knob {
            let now = ui.input(|i| i.time);
            let prev: f64 = ui.data(|d| d.get_temp(dbl_id).unwrap_or(0.0));
            ui.data_mut(|d| d.insert_temp(dbl_id, now));
            now - prev < 0.4
        } else {
            false
        };

        // Double-click or ctrl+click to reset
        if is_double_click || (response.clicked() && ui.input(|i| i.modifiers.ctrl)) {
            *value = default;
            result.changed = true;
            result.reset = true;
        }

        // Vertical drag to change value (skip if resetting via double-click)
        if response.dragged() && !is_double_click {
            let delta = -response.drag_delta().y;
            let speed = if ui.input(|i| i.modifiers.shift) {
                0.001 // Fine control
            } else {
                0.005
            };
            *value = (*value + delta * speed * (max - min)).clamp(min, max);
            result.changed = true;
        }

        // Paint the knob (use ui.painter() to avoid clipping the stroke at rect edges)
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            let center = rect.center();
            let radius = diameter / 2.0 - 2.0;

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

/// Draw a compact inline knob — no label below, just the ring + value text.
/// Designed for use in track row headers where vertical space is tight.
pub fn knob_inline(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut f32,
    min: f32,
    max: f32,
    default: f32,
    tooltip: &str,
    format_value: impl Fn(f32) -> String,
    ring_color: egui::Color32,
    diameter: f32,
) -> KnobResponse {
    let mut result = KnobResponse {
        changed: false,
        reset: false,
    };

    let size = egui::vec2(diameter, diameter);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
    response.clone().on_hover_text(tooltip);

    // Double-click detection (egui-baseview workaround)
    let dbl_id = id.with("last_press_inline");
    let pointer_pressed_on_knob = ui.input(|i| {
        i.events.iter().any(|e| matches!(
            e,
            egui::Event::PointerButton { pressed: true, button: egui::PointerButton::Primary, .. }
        ))
    }) && response.contains_pointer();
    let is_double_click = if pointer_pressed_on_knob {
        let now = ui.input(|i| i.time);
        let prev: f64 = ui.data(|d| d.get_temp(dbl_id).unwrap_or(0.0));
        ui.data_mut(|d| d.insert_temp(dbl_id, now));
        now - prev < 0.4
    } else {
        false
    };

    if is_double_click || (response.clicked() && ui.input(|i| i.modifiers.ctrl)) {
        *value = default;
        result.changed = true;
        result.reset = true;
    }

    if response.dragged() && !is_double_click {
        let delta = -response.drag_delta().y;
        let speed = if ui.input(|i| i.modifiers.shift) { 0.001 } else { 0.005 };
        *value = (*value + delta * speed * (max - min)).clamp(min, max);
        result.changed = true;
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let center = rect.center();
        let radius = diameter / 2.0 - 1.5;
        painter.circle_stroke(center, radius, egui::Stroke::new(1.5, ring_color));
        let text = format_value(*value);
        let font_size = if diameter < 20.0 { 7.0 } else { 8.0 };
        painter.text(
            center,
            egui::Align2::CENTER_CENTER,
            &text,
            egui::FontId::new(font_size, egui::FontFamily::Monospace),
            ring_color,
        );
    }

    result
}
