use crate::engine::kit::SampleCategory;

/// A single point on the sample map.
#[derive(Clone, Debug)]
pub struct MapPoint {
    /// Normalized X (0.0–1.0), from spectral centroid (log scale).
    pub nx: f32,
    /// Normalized Y (0.0–1.0), from decay time (linear, 0=short/top, 1=long/bottom).
    pub ny: f32,
    /// Index into SampleLibrary::all_samples_flat() for retrieval.
    pub library_index: usize,
    /// Category for coloring.
    pub category: SampleCategory,
    /// Filename for tooltip.
    pub name: String,
    /// Original centroid in Hz.
    pub centroid_hz: f32,
    /// Original decay in seconds.
    pub decay_secs: f32,
}

/// Normalize spectral centroid to 0.0–1.0 via log scale.
/// Maps ~100Hz → 0.0, ~20kHz → 1.0.
fn normalize_centroid(hz: f32) -> f32 {
    if hz <= 0.0 {
        return 0.0;
    }
    ((hz / 100.0).log2() / (200.0_f32).log2()).clamp(0.0, 1.0)
}

/// Normalize decay time to 0.0–1.0 via log scale.
/// Maps ~0.01s → 0.0, ~4.0s → 1.0. Log scale spreads short percussion
/// decays across the full Y axis instead of clustering near the top.
fn normalize_decay(secs: f32) -> f32 {
    if secs <= 0.0 {
        return 0.0;
    }
    // log2(secs / 0.01) / log2(4.0 / 0.01) = log2(secs/0.01) / log2(400)
    ((secs / 0.01).log2() / (400.0_f32).log2()).clamp(0.0, 1.0)
}

use nih_plug_egui::egui;
use crate::analysis::library::SampleLibrary;
use crate::engine::kit::NUM_PADS;
use crate::ui::theme;

/// Build map points from the full sample library.
/// Call once when library scan completes; cache the result in EditorState.
pub fn build_map_points(library: &SampleLibrary) -> Vec<MapPoint> {
    let flat = library.all_samples_flat();
    flat.iter()
        .enumerate()
        .map(|(i, sample)| MapPoint {
            nx: normalize_centroid(sample.features.spectral_centroid),
            ny: normalize_decay(sample.features.decay_time),
            library_index: i,
            category: sample.entry.category,
            name: sample.entry.filename.clone(),
            centroid_hz: sample.features.spectral_centroid,
            decay_secs: sample.features.decay_time,
        })
        .collect()
}

/// View state for zoom/pan.
pub struct MapViewState {
    pub zoom: f32,
    pub pan_x: f32,
    pub pan_y: f32,
}

impl Default for MapViewState {
    fn default() -> Self {
        Self { zoom: 1.0, pan_x: 0.0, pan_y: 0.0 }
    }
}

/// Convert normalized (0–1) coords to screen position within the map rect.
fn to_screen(nx: f32, ny: f32, view: &MapViewState, rect: egui::Rect) -> egui::Pos2 {
    let x = (nx - view.pan_x) * view.zoom * rect.width() + rect.left();
    let y = (ny - view.pan_y) * view.zoom * rect.height() + rect.top();
    egui::pos2(x, y)
}

/// Convert screen position back to normalized coords.
fn from_screen(pos: egui::Pos2, view: &MapViewState, rect: egui::Rect) -> (f32, f32) {
    let nx = (pos.x - rect.left()) / (view.zoom * rect.width()) + view.pan_x;
    let ny = (pos.y - rect.top()) / (view.zoom * rect.height()) + view.pan_y;
    (nx, ny)
}

/// Hit test result.
pub struct HitResult {
    pub point_index: usize,
    pub screen_pos: egui::Pos2,
}

/// Find the nearest map point within `max_dist` screen pixels of `cursor`.
pub fn hit_test(
    cursor: egui::Pos2,
    points: &[MapPoint],
    view: &MapViewState,
    rect: egui::Rect,
    max_dist: f32,
) -> Option<HitResult> {
    let mut best: Option<(usize, f32, egui::Pos2)> = None;
    for (i, p) in points.iter().enumerate() {
        let screen = to_screen(p.nx, p.ny, view, rect);
        if !rect.expand(max_dist).contains(screen) {
            continue;
        }
        let dist = cursor.distance(screen);
        if dist < max_dist {
            if best.is_none() || dist < best.unwrap().1 {
                best = Some((i, dist, screen));
            }
        }
    }
    best.map(|(i, _, pos)| HitResult { point_index: i, screen_pos: pos })
}

/// Actions returned by draw_map for the editor to handle.
pub enum MapAction {
    None,
    ClickedDot { point_index: usize },
    AssignToPad { point_index: usize, pad_index: usize },
}

/// State for the assignment popup.
pub struct PopupState {
    pub active_point: Option<usize>,
    pub anchor_pos: egui::Pos2,
}
impl Default for PopupState {
    fn default() -> Self { Self { active_point: None, anchor_pos: egui::Pos2::ZERO } }
}

/// Draw the scatter plot. Returns any action triggered.
pub fn draw_map(
    ui: &mut egui::Ui,
    points: &[MapPoint],
    view: &mut MapViewState,
    kit_paths: &[Option<String>],
    hovered_index: &mut Option<usize>,
    shortcut_pad: Option<usize>,
    shortcut_category: Option<SampleCategory>,
    tooltips_on: bool,
) -> MapAction {
    let mut action = MapAction::None;
    let available = ui.available_size();
    let (response, painter) = ui.allocate_painter(available, egui::Sense::click_and_drag());
    let rect = response.rect;

    // Background
    painter.rect_filled(rect, 0.0, egui::Color32::from_rgb(0x08, 0x08, 0x1a));

    // Border tint when shortcut mode is active
    if let Some(cat) = shortcut_category {
        let border_color = theme::category_color(cat);
        painter.rect_stroke(rect, 0.0, egui::Stroke::new(2.0, border_color.to_egui_alpha(0x44)), egui::StrokeKind::Inside);
    }

    // Axis labels
    let label_color = egui::Color32::from_rgba_premultiplied(0x63, 0x6e, 0x72, 0x4c);
    let label_font = egui::FontId::new(8.0, egui::FontFamily::Monospace);
    painter.text(
        egui::pos2(rect.center().x, rect.bottom() - 6.0),
        egui::Align2::CENTER_BOTTOM,
        "BRIGHTNESS \u{2192}",
        label_font.clone(),
        label_color,
    );
    painter.text(
        egui::pos2(rect.left() + 6.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        "D\nE\nC\nA\nY\n\u{2192}",
        egui::FontId::new(7.0, egui::FontFamily::Monospace),
        label_color,
    );

    // --- Zoom (scroll wheel) ---
    if response.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll.abs() > 0.0 {
            let old_zoom = view.zoom;
            view.zoom = (view.zoom * (1.0 + scroll * 0.002)).clamp(1.0, 8.0);
            if let Some(cursor) = response.hover_pos() {
                let (nx, ny) = from_screen(
                    cursor,
                    &MapViewState { zoom: old_zoom, pan_x: view.pan_x, pan_y: view.pan_y },
                    rect,
                );
                view.pan_x = nx - (cursor.x - rect.left()) / (view.zoom * rect.width());
                view.pan_y = ny - (cursor.y - rect.top()) / (view.zoom * rect.height());
            }
        }
    }

    // --- Pan (drag) ---
    if response.dragged() && response.drag_delta().length() > 0.0 {
        let delta = response.drag_delta();
        view.pan_x -= delta.x / (view.zoom * rect.width());
        view.pan_y -= delta.y / (view.zoom * rect.height());
    }

    // --- Double-click to reset ---
    if response.double_clicked() {
        view.zoom = 1.0;
        view.pan_x = 0.0;
        view.pan_y = 0.0;
    }

    // --- Draw library dots (dim) ---
    for (i, p) in points.iter().enumerate() {
        let screen = to_screen(p.nx, p.ny, view, rect);
        if !rect.contains(screen) {
            continue;
        }

        let color = theme::category_color(p.category);
        let is_kit = kit_paths
            .iter()
            .any(|kp| kp.as_ref().is_some_and(|path| path.ends_with(&p.name)));

        if is_kit {
            continue; // Kit dots drawn in second pass
        }

        let is_hovered = *hovered_index == Some(i);
        let (radius, alpha) = if is_hovered {
            (5.0, 0.7)
        } else if let Some(cat) = shortcut_category {
            if p.category == cat { (4.0, 0.5) } else { (3.0, 0.12) }
        } else {
            (3.0, 0.25)
        };
        painter.circle_filled(screen, radius, color.to_egui_alpha((alpha * 255.0) as u8));
    }

    // --- Draw kit dots (bright + ring) ---
    for (_i, p) in points.iter().enumerate() {
        let screen = to_screen(p.nx, p.ny, view, rect);
        if !rect.contains(screen) {
            continue;
        }

        let is_kit = kit_paths
            .iter()
            .any(|kp| kp.as_ref().is_some_and(|path| path.ends_with(&p.name)));
        if !is_kit {
            continue;
        }

        let color = theme::category_color(p.category);
        painter.circle_filled(screen, 8.0, color.to_egui_alpha(40));
        painter.circle_filled(screen, 5.0, color.to_egui());
        painter.circle_stroke(screen, 5.0, egui::Stroke::new(2.0, egui::Color32::WHITE));
    }

    // --- Hit test for hover ---
    if let Some(cursor) = response.hover_pos() {
        if let Some(hit) = hit_test(cursor, points, view, rect, 8.0) {
            *hovered_index = Some(hit.point_index);
            if tooltips_on {
                let p = &points[hit.point_index];
                let color = theme::category_color(p.category);

                let mut tooltip_pos = egui::pos2(cursor.x + 15.0, cursor.y - 10.0);
                if tooltip_pos.x + 150.0 > rect.right() {
                    tooltip_pos.x = cursor.x - 165.0;
                }
                if tooltip_pos.y < rect.top() + 10.0 {
                    tooltip_pos.y = cursor.y + 15.0;
                }

                let tooltip_rect =
                    egui::Rect::from_min_size(tooltip_pos, egui::vec2(150.0, 32.0));
                painter.rect_filled(
                    tooltip_rect,
                    4.0,
                    egui::Color32::from_rgba_premultiplied(0x11, 0x11, 0x26, 0xee),
                );
                painter.text(
                    tooltip_rect.min + egui::vec2(6.0, 4.0),
                    egui::Align2::LEFT_TOP,
                    &p.name,
                    egui::FontId::new(9.0, egui::FontFamily::Monospace),
                    color.to_egui(),
                );
                painter.text(
                    tooltip_rect.min + egui::vec2(6.0, 17.0),
                    egui::Align2::LEFT_TOP,
                    format!("{} \u{00b7} {:.0}Hz \u{00b7} {:.2}s", p.category.label(), p.centroid_hz, p.decay_secs),
                    egui::FontId::new(8.0, egui::FontFamily::Monospace),
                    theme::TEXT_DIM,
                );
            }
        } else {
            *hovered_index = None;
        }
    } else {
        *hovered_index = None;
    }

    // --- Click detection ---
    if response.clicked() {
        if let Some(cursor) = response.interact_pointer_pos() {
            if let Some(hit) = hit_test(cursor, points, view, rect, 8.0) {
                action = MapAction::ClickedDot { point_index: hit.point_index };
            }
        }
    }

    action
}

/// Actions returned by the mini pad bar.
pub enum PadBarAction {
    None,
    ToggleShortcut(usize),
}

/// Draw a compact row of 8 pad buttons below the map.
pub fn draw_mini_pad_bar(
    ui: &mut egui::Ui,
    pad_names: &[String; NUM_PADS],
    pad_categories: &[SampleCategory; NUM_PADS],
    shortcut_pad: Option<usize>,
) -> PadBarAction {
    let mut action = PadBarAction::None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        let bar_width = ui.clip_rect().width() - 12.0;
        let btn_width = (bar_width / NUM_PADS as f32) - 2.0;

        for i in 0..NUM_PADS {
            let color = theme::category_color(pad_categories[i]);
            let is_active = shortcut_pad == Some(i);
            let bg = if is_active { color.to_egui_alpha(0x44) } else { color.to_egui_alpha(0x22) };
            let border = if is_active { egui::Stroke::new(2.0, color.to_egui()) } else { egui::Stroke::new(1.0, color.to_egui_alpha(0x55)) };
            let label = format!("{} {}", i + 1, pad_categories[i].label().chars().take(3).collect::<String>().to_uppercase());
            let btn = egui::Button::new(
                egui::RichText::new(&label).font(egui::FontId::new(8.0, egui::FontFamily::Monospace)).color(color.to_egui()))
                .fill(bg).stroke(border).min_size(egui::vec2(btn_width, 22.0));
            let response = ui.add(btn);
            if response.clicked() { action = PadBarAction::ToggleShortcut(i); }
            if response.hovered() {
                response.on_hover_text_at_pointer(
                    egui::RichText::new(&pad_names[i]).font(egui::FontId::new(9.0, egui::FontFamily::Monospace)));
            }
        }
    });
    action
}

/// Draw the assignment popup near a clicked dot.
pub fn draw_popup(
    ctx: &egui::Context,
    popup: &mut PopupState,
    points: &[MapPoint],
    pad_categories: &[SampleCategory; NUM_PADS],
    map_rect: egui::Rect,
) -> MapAction {
    let point_index = match popup.active_point {
        Some(i) if i < points.len() => i,
        _ => return MapAction::None,
    };

    let p = &points[point_index];
    let color = theme::category_color(p.category);

    // Position popup near anchor, flip if near edges
    let popup_width = 140.0;
    let popup_height = 80.0;
    let mut pos = egui::pos2(popup.anchor_pos.x + 15.0, popup.anchor_pos.y - popup_height - 10.0);
    if pos.x + popup_width > map_rect.right() { pos.x = popup.anchor_pos.x - popup_width - 15.0; }
    if pos.y < map_rect.top() { pos.y = popup.anchor_pos.y + 15.0; }

    let mut action = MapAction::None;

    egui::Area::new(egui::Id::new("map_popup"))
        .fixed_pos(pos)
        .constrain(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(egui::Color32::from_rgb(0x11, 0x11, 0x26))
                .stroke(egui::Stroke::new(1.0, color.to_egui_alpha(0x77)))
                .corner_radius(egui::CornerRadius::same(5))
                .inner_margin(egui::Margin::same(6))
                .show(ui, |ui| {
                    ui.label(egui::RichText::new(&p.name)
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                        .color(color.to_egui()).strong());
                    ui.label(egui::RichText::new(format!("{} \u{00b7} {:.0}Hz \u{00b7} {:.2}s", p.category.label(), p.centroid_hz, p.decay_secs))
                        .font(egui::FontId::new(8.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DIM));
                    ui.add_space(3.0);
                    ui.label(egui::RichText::new("ASSIGN TO PAD:")
                        .font(egui::FontId::new(7.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DIM));
                    ui.add_space(2.0);
                    egui::Grid::new("popup_pads").spacing(egui::vec2(2.0, 2.0)).show(ui, |ui| {
                        for i in 0..NUM_PADS {
                            let pad_color = theme::category_color(pad_categories[i]);
                            let is_match = pad_categories[i] == p.category;
                            let bg = if is_match { pad_color.to_egui_alpha(0x44) } else { pad_color.to_egui_alpha(0x22) };
                            if ui.add(egui::Button::new(
                                egui::RichText::new(format!("{}", i + 1))
                                    .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                    .color(pad_color.to_egui()))
                                .fill(bg).min_size(egui::vec2(24.0, 18.0)))
                                .clicked()
                            {
                                action = MapAction::AssignToPad { point_index, pad_index: i };
                                popup.active_point = None;
                            }
                            if i == 3 { ui.end_row(); }
                        }
                    });
                });
        });

    // Keyboard: 1-8 assign, Escape closes
    ctx.input(|input| {
        for i in 0..NUM_PADS {
            let key = match i {
                0 => egui::Key::Num1, 1 => egui::Key::Num2, 2 => egui::Key::Num3, 3 => egui::Key::Num4,
                4 => egui::Key::Num5, 5 => egui::Key::Num6, 6 => egui::Key::Num7, 7 => egui::Key::Num8,
                _ => continue,
            };
            if input.key_pressed(key) {
                action = MapAction::AssignToPad { point_index, pad_index: i };
                popup.active_point = None;
            }
        }
        if input.key_pressed(egui::Key::Escape) { popup.active_point = None; }
    });

    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn centroid_100hz_maps_to_zero() {
        let n = normalize_centroid(100.0);
        assert!(n.abs() < 0.01, "100Hz should map near 0.0, got {n}");
    }

    #[test]
    fn centroid_20khz_maps_to_one() {
        let n = normalize_centroid(20000.0);
        assert!((n - 1.0).abs() < 0.05, "20kHz should map near 1.0, got {n}");
    }

    #[test]
    fn centroid_1khz_maps_mid() {
        let n = normalize_centroid(1000.0);
        assert!(n > 0.3 && n < 0.6, "1kHz should be mid-range, got {n}");
    }

    #[test]
    fn centroid_zero_maps_to_zero() {
        assert_eq!(normalize_centroid(0.0), 0.0);
    }

    #[test]
    fn decay_zero_maps_to_zero() {
        assert_eq!(normalize_decay(0.0), 0.0);
    }

    #[test]
    fn decay_4s_maps_to_one() {
        assert!((normalize_decay(4.0) - 1.0).abs() < 0.01);
    }

    #[test]
    fn decay_clamps_above_4s() {
        assert_eq!(normalize_decay(10.0), 1.0);
    }

    #[test]
    fn decay_short_spreads_well() {
        // Key fix: short decays (typical percussion) should NOT cluster near 0
        let d005 = normalize_decay(0.05); // 50ms — typical kick
        let d02 = normalize_decay(0.2);   // 200ms — typical snare
        let d1 = normalize_decay(1.0);    // 1s — longer perc
        assert!(d005 > 0.15, "50ms should be well off the top, got {d005}");
        assert!(d02 > 0.3, "200ms should be mid-low, got {d02}");
        assert!(d1 > 0.6, "1s should be in the lower half, got {d1}");
        // All should be distinct and spread out
        assert!(d02 > d005 + 0.1, "should have clear separation");
        assert!(d1 > d02 + 0.1, "should have clear separation");
    }

    #[test]
    fn build_map_points_count_matches_library() {
        use crate::analysis::library::AnalyzedSample;
        use crate::analysis::features::AudioFeatures;
        use crate::analysis::scanner::SampleEntry;
        use std::collections::HashMap;
        use std::path::PathBuf;
        use std::sync::Arc;

        let mut by_category = HashMap::new();
        for cat in SampleCategory::all() {
            let entry = SampleEntry {
                path: PathBuf::from(format!("/test/{}.wav", cat.label())),
                filename: format!("{}.wav", cat.label()),
                category: *cat,
                folder_hint: None,
                duration_ms: 100,
                is_percussive: true,
            };
            let sample = AnalyzedSample {
                entry,
                features: AudioFeatures {
                    attack_time: 0.001,
                    decay_time: 0.1,
                    spectral_centroid: 1000.0,
                    spectral_flatness: 0.5,
                    peak: 1.0,
                    duration: 0.1,
                    is_percussive: true,
                },
                data: Arc::new(vec![0.5; 4410]),
            };
            by_category.entry(*cat).or_insert_with(Vec::new).push(sample);
        }
        let lib = SampleLibrary { total: 10, by_category, sample_rate: 44100.0 };
        let points = build_map_points(&lib);
        assert_eq!(points.len(), 10);
        for p in &points {
            assert!(p.nx >= 0.0 && p.nx <= 1.0, "nx out of range: {}", p.nx);
            assert!(p.ny >= 0.0 && p.ny <= 1.0, "ny out of range: {}", p.ny);
        }
    }

    #[test]
    fn screen_transform_round_trips() {
        let view = MapViewState::default();
        let rect = egui::Rect::from_min_size(egui::pos2(50.0, 50.0), egui::vec2(400.0, 200.0));
        let screen = to_screen(0.5, 0.5, &view, rect);
        let (nx, ny) = from_screen(screen, &view, rect);
        assert!((nx - 0.5).abs() < 0.001);
        assert!((ny - 0.5).abs() < 0.001);
    }

    #[test]
    fn screen_transform_with_zoom() {
        let view = MapViewState { zoom: 2.0, pan_x: 0.25, pan_y: 0.25 };
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(400.0, 200.0));
        let screen = to_screen(0.5, 0.5, &view, rect);
        assert!((screen.x - 200.0).abs() < 0.1);
        assert!((screen.y - 100.0).abs() < 0.1);
    }
}
