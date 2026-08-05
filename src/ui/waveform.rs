use nih_plug_egui::egui;

use crate::ui::state::WaveformSummary;

/// Paint a line waveform from a WaveformSummary.
/// Draws a polyline of midpoints through the min/max pairs.
pub fn paint_waveform(
    ui: &mut egui::Ui,
    summary: Option<&WaveformSummary>,
    color: egui::Color32,
    desired_size: egui::Vec2,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::hover());

    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter_at(rect);

    match summary {
        Some(summary) if !summary.points.is_empty() => {
            let n = summary.points.len();
            let step = rect.width() / n as f32;

            let points: Vec<egui::Pos2> = summary
                .points
                .iter()
                .enumerate()
                .map(|(i, [min, max])| {
                    let x = rect.left() + (i as f32 + 0.5) * step;
                    let mid = (min + max) / 2.0;
                    // Map amplitude (-1..1) to y (bottom..top)
                    let y = rect.center().y - mid * (rect.height() / 2.0);
                    egui::pos2(x, y)
                })
                .collect();

            painter.add(egui::Shape::line(points, egui::Stroke::new(1.2, color)));
        }
        _ => {
            // No sample loaded — draw a dim center line
            let center_y = rect.center().y;
            painter.line_segment(
                [
                    egui::pos2(rect.left() + 4.0, center_y),
                    egui::pos2(rect.right() - 4.0, center_y),
                ],
                egui::Stroke::new(0.5, color.linear_multiply(0.2)),
            );
        }
    }

    response
}
