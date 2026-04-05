use nih_plug_egui::egui;
use egui::{Color32, Rect, Pos2, Vec2, Stroke, FontId, Align2};
use crate::engine::kit::SampleCategory;
use crate::engine::sequencer::{ConditionTrig, NUM_STEPS, NUM_PATTERNS};
use crate::ui::theme;
use crate::ui::knob;

/// State held in EditorState for the sequencer view.
#[derive(Default)]
pub struct SeqViewState {
    /// Currently selected step (lane, step) for parameter editing.
    pub selected: Option<(usize, usize)>,
    /// Drag-paint state: enable or disable mode.
    pub drag_paint: Option<bool>,
    /// Set of (lane, step) already painted during current drag.
    pub drag_visited: std::collections::HashSet<(usize, usize)>,
    /// Cell rects from last frame, for hit-testing during drag.
    pub cell_rects: Vec<(usize, usize, Rect)>,
    /// Where the current drag started (lane, step).
    pub drag_origin: Option<(usize, usize)>,
    /// Y position at drag start.
    pub drag_start_y: f32,
    /// True once we've determined this drag is a velocity adjustment.
    pub drag_is_velocity: bool,
    /// The velocity value at the moment we enter velocity-drag mode.
    pub drag_start_velocity: f32,
}

/// Display data for sequencer -- captured from SharedState + atomics.
pub struct SeqDisplay {
    pub current_step: usize,
    pub playing: bool,
    pub active_pattern: usize,
    pub queued_pattern: Option<usize>,
    pub fill_active: bool,
    pub pattern_has_data: [bool; NUM_PATTERNS],
    pub lanes: Vec<LaneDisplay>,
    pub swing: f32,
    pub ext_mode: bool,
}

pub struct LaneDisplay {
    pub pad_name: String,
    pub category: SampleCategory,
    pub muted: bool,
    pub solo: bool,
    pub locked: bool,
    pub volume: f32,
    pub steps: [StepDisplay; NUM_STEPS],
}

#[derive(Clone, Copy)]
pub struct StepDisplay {
    pub enabled: bool,
    pub velocity: f32,
    pub probability: f32,
    pub pan: Option<f32>,
    pub pitch: Option<f32>,
    pub condition: ConditionTrig,
}

/// Actions the sequencer UI wants to perform.
pub enum SeqAction {
    ToggleStep { lane: usize, step: usize },
    SetStepEnabled { lane: usize, step: usize, enabled: bool },
    SelectStep { lane: usize, step: usize },
    SetStepVelocity { lane: usize, step: usize, value: f32 },
    SetStepPan { lane: usize, step: usize, value: Option<f32> },
    SetStepPitch { lane: usize, step: usize, value: Option<f32> },
    SetStepProbability { lane: usize, step: usize, value: f32 },
    SetStepCondition { lane: usize, step: usize, condition: ConditionTrig },
    ToggleLaneMute { lane: usize },
    ToggleLaneSolo { lane: usize },
    ToggleLaneLock { lane: usize },
    SetLaneVolume { lane: usize, volume: f32 },
    SelectPattern { index: usize },
    SetSwing { value: f32 },
    CopyPattern,
    PastePattern,
    ClearPattern,
    DicePattern,
    SetFillActive { active: bool },
    ToggleInternalPlay,
    ExportMidi,
    OpenSavePatternDialog,
    OpenLoadPatternDialog,
    ResetLane { lane: usize },
    ResetStep { lane: usize, step: usize },
}

/// Draw the complete sequencer view. Returns a list of actions.
pub fn draw_sequencer_view(
    ui: &mut egui::Ui,
    display: &SeqDisplay,
    view_state: &mut SeqViewState,
    available_height: f32,
) -> Vec<SeqAction> {
    let mut actions: Vec<SeqAction> = Vec::new();

    // Step grid at top (may return multiple actions from drag-paint)
    let (grid_actions, grid_layout) = draw_grid(ui, display, available_height, view_state);
    actions.extend(grid_actions);
    let grid_right = grid_layout.grid_right_x;

    ui.add_space(2.0);

    // Combined row: step param knobs (left) + SWING + pattern selector (right-aligned to grid edge)
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;

        // Step parameter knobs (only if a step is selected)
        if let Some((lane_idx, step_idx)) = view_state.selected {
            if lane_idx < display.lanes.len() {
                let step = &display.lanes[lane_idx].steps[step_idx];
                let cat = display.lanes[lane_idx].category;
                actions.extend(draw_param_controls(ui, lane_idx, step_idx, step, cat));
            }
        }

        // Pattern selector — right-aligned, ending at grid right edge
        let ptrn_btn_w = 24.0;
        let ptrn_spacing = 3.0;
        let ptrn_label_w = 34.0;
        let ptrn_total_w = ptrn_label_w + ptrn_spacing + NUM_PATTERNS as f32 * ptrn_btn_w + (NUM_PATTERNS - 1) as f32 * ptrn_spacing;
        let ptrn_start_x = grid_right - ptrn_total_w;
        let cur_x = ui.cursor().left();
        let space_to_ptrn = (ptrn_start_x - cur_x).max(0.0);
        ui.add_space(space_to_ptrn);

        ui.spacing_mut().item_spacing.x = ptrn_spacing;

        ui.label(
            egui::RichText::new("PTRN")
                .font(FontId::monospace(9.0))
                .color(theme::TEXT_DIM),
        );

        for i in 0..NUM_PATTERNS {
            let is_active = i == display.active_pattern;
            let is_queued = display.queued_pattern == Some(i);
            let has_data = display.pattern_has_data[i];

            let (bg, text_color) = if is_active {
                (theme::ACCENT, Color32::BLACK)
            } else if is_queued {
                (Color32::TRANSPARENT, theme::ACCENT)
            } else if has_data {
                (Color32::TRANSPARENT, theme::PAT_HAS_DATA)
            } else {
                (Color32::TRANSPARENT, theme::PAT_EMPTY)
            };

            let btn = egui::Button::new(
                egui::RichText::new(format!("{:02}", i + 1))
                    .font(FontId::monospace(9.0))
                    .color(text_color),
            )
            .fill(bg)
            .min_size(Vec2::new(ptrn_btn_w, 20.0))
            .corner_radius(3.0);

            let response = ui.add(btn);

            if is_queued {
                let t = ui.input(|i| i.time) as f32;
                let alpha = ((t * 4.0).sin() * 0.5 + 0.5) * 200.0 + 55.0;
                let border_color = Color32::from_rgba_premultiplied(0, 212, 170, alpha as u8);
                ui.painter().rect_stroke(
                    response.rect,
                    3.0,
                    Stroke::new(1.5, border_color),
                    egui::StrokeKind::Outside,
                );
            }

            if response.clicked() && !is_active {
                actions.push(SeqAction::SelectPattern { index: i });
            }
        }
    });

    // Push bottom bar to near the bottom edge
    let remaining = ui.available_height() - 32.0;
    if remaining > 0.0 {
        ui.add_space(remaining);
    }

    // Bottom bar
    if let Some(a) = draw_bottom_bar(ui, display) {
        actions.push(a);
    }

    actions
}


/// Layout info returned by draw_grid so the caller can align elements.
struct GridLayout {
    /// Right X coordinate of the last grid cell.
    grid_right_x: f32,
}

fn draw_grid(
    ui: &mut egui::Ui,
    display: &SeqDisplay,
    available_height: f32,
    view_state: &mut SeqViewState,
) -> (Vec<SeqAction>, GridLayout) {
    let mut actions: Vec<SeqAction> = Vec::new();

    // Label area: strip + space + tag + space + knob + space = 81px
    let label_width = theme::STRIP_WIDTH + 8.0 + theme::TAG_WIDTH + 4.0 + 16.0 + 4.0;
    let controls_width = theme::CONTROLS_WIDTH;
    let cell_spacing = theme::CELL_SPACING;
    let num_lanes = display.lanes.len().max(1) as f32;
    let vert_reserved = theme::GRID_VERT_RESERVED;
    let vert_avail = available_height - vert_reserved;
    let row_from_height = ((vert_avail - cell_spacing * (num_lanes - 1.0)) / num_lanes).floor();
    let available_w = ui.available_width() - label_width - controls_width;
    let cell_from_width = ((available_w - cell_spacing * 15.0) / 16.0).floor();
    let cell_size = row_from_height.min(cell_from_width).clamp(20.0, 48.0);
    let row_height = cell_size;
    // Compute grid right edge position
    let grid_start_x = ui.min_rect().left() + label_width + controls_width;
    let grid_right_x = grid_start_x + cell_size * 16.0 + cell_spacing * 15.0;

    // Track whether a drag was active before clearing — prevents the released
    // frame's clicked() from double-toggling the step the drag already toggled.
    let had_active_drag = view_state.drag_paint.is_some()
        || view_state.drag_origin.is_some();

    // Clear drag state when mouse released
    if !ui.input(|i| i.pointer.any_pressed() || i.pointer.any_down()) {
        view_state.drag_paint = None;
        view_state.drag_visited.clear();
        view_state.drag_origin = None;
        view_state.drag_is_velocity = false;
    }
    view_state.cell_rects.clear();

    // Step numbers header — offset must match track row layout exactly
    ui.horizontal(|ui| {
        ui.add_space(label_width + controls_width);
        ui.spacing_mut().item_spacing.x = cell_spacing;
        for s in 0..NUM_STEPS {
            let is_beat = s % 4 == 0;
            let is_playhead = s == display.current_step && display.playing;
            let color = if is_playhead {
                theme::ACCENT
            } else if is_beat {
                theme::TEXT_DIM
            } else {
                Color32::from_rgb(51, 51, 51)
            };
            let text = egui::RichText::new(format!("{}", s + 1))
                .font(FontId::monospace(8.0))
                .color(color);
            ui.allocate_ui(Vec2::new(cell_size, 12.0), |ui| {
                ui.centered_and_justified(|ui| ui.label(text));
            });
        }
    });

    // Track rows — 2px vertical spacing to match pads view
    ui.spacing_mut().item_spacing.y = 2.0;
    for lane_idx in 0..display.lanes.len() {
        let lane = &display.lanes[lane_idx];
        let cat_color = theme::category_color32(lane.category);
        let is_muted = lane.muted;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;

            // Color strip (3px) — matches pads view
            let strip_color = if is_muted { cat_color.linear_multiply(0.3) } else { cat_color };
            let (strip_rect, _) = ui.allocate_exact_size(Vec2::new(3.0, row_height), egui::Sense::hover());
            ui.painter().rect_filled(strip_rect, egui::CornerRadius::ZERO, strip_color);

            ui.add_space(8.0);

            // Category tag badge (double-click to reset lane) — matches pads view exactly
            let tag_color = if is_muted { cat_color.linear_multiply(0.3) } else { cat_color };
            let tag_size = Vec2::new(46.0, 16.0);
            let label_resp = ui.allocate_ui(Vec2::new(46.0, row_height), |ui| {
                ui.centered_and_justified(|ui| {
                    let (tag_rect, _) = ui.allocate_exact_size(tag_size, egui::Sense::hover());
                    if ui.is_rect_visible(tag_rect) {
                        let painter = ui.painter_at(tag_rect);
                        painter.rect_filled(tag_rect, 2.0, tag_color);
                        painter.text(
                            tag_rect.center(),
                            Align2::CENTER_CENTER,
                            lane.category.label(),
                            FontId::new(8.0, egui::FontFamily::Monospace),
                            theme::BG_MAIN,
                        );
                    }
                })
            });
            let label_rect = label_resp.response.rect;
            let label_interact = ui.interact(label_rect, egui::Id::new(("lane_label_dblclick", lane_idx)), egui::Sense::click());
            if label_interact.double_clicked() {
                actions.push(SeqAction::ResetLane { lane: lane_idx });
            }

            ui.add_space(4.0);

            // LVL knob — inline, vertically centered
            {
                let mut vol = lane.volume;
                let knob_resp = ui.allocate_ui(Vec2::new(16.0, row_height), |ui| {
                    ui.centered_and_justified(|ui| {
                        knob::knob_inline(
                            ui,
                            egui::Id::new(("seq_lvl", lane_idx)),
                            &mut vol,
                            0.0, 1.0, 1.0,
                            "Lane volume",
                            |v| format!("{}", (v * 100.0) as u32),
                            cat_color,
                            16.0,
                            false,
                        )
                    })
                });
                if knob_resp.inner.inner.changed {
                    actions.push(SeqAction::SetLaneVolume { lane: lane_idx, volume: vol });
                }
            }

            ui.add_space(4.0);
            ui.spacing_mut().item_spacing.x = 2.0;

            // Mute button
            let mute_text = egui::RichText::new("M").font(FontId::monospace(7.0));
            let mute_btn = egui::Button::new(mute_text.color(if is_muted { Color32::WHITE } else { theme::TEXT_DIM }))
                .fill(if is_muted { theme::MUTE_RED } else { theme::STEP_BG })
                .min_size(Vec2::new(13.0, 13.0))
                .corner_radius(2.0);
            if ui.add(mute_btn).clicked() {
                actions.push(SeqAction::ToggleLaneMute { lane: lane_idx });
            }

            // Solo button
            let solo_text = egui::RichText::new("S").font(FontId::monospace(7.0));
            let solo_btn = egui::Button::new(solo_text.color(if lane.solo { Color32::BLACK } else { theme::TEXT_DIM }))
                .fill(if lane.solo { theme::SOLO_YELLOW } else { theme::STEP_BG })
                .min_size(Vec2::new(13.0, 13.0))
                .corner_radius(2.0);
            if ui.add(solo_btn).clicked() {
                actions.push(SeqAction::ToggleLaneSolo { lane: lane_idx });
            }

            // Lock button
            let lock_text = egui::RichText::new("L").font(FontId::monospace(7.0));
            let lock_btn = egui::Button::new(lock_text.color(if lane.locked { Color32::BLACK } else { theme::TEXT_DIM }))
                .fill(if lane.locked { theme::LOCK_ORANGE } else { theme::STEP_BG })
                .min_size(Vec2::new(13.0, 13.0))
                .corner_radius(2.0);
            if ui.add(lock_btn).clicked() {
                actions.push(SeqAction::ToggleLaneLock { lane: lane_idx });
            }

            // Step cells
            for step_idx in 0..NUM_STEPS {
                let step = &lane.steps[step_idx];
                let is_beat = step_idx % 4 == 0;
                let is_playhead = step_idx == display.current_step && display.playing;
                let is_selected = view_state.selected == Some((lane_idx, step_idx));

                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(cell_size, row_height),
                    egui::Sense::click_and_drag(),
                );

                // Store rect for pointer hit-testing in drag pass
                view_state.cell_rects.push((lane_idx, step_idx, rect));

                // Background
                let bg = if is_beat { theme::STEP_BG_BEAT } else { theme::STEP_BG };
                let bg = if is_muted { bg.linear_multiply(0.3) } else { bg };
                ui.painter().rect_filled(rect, 3.0, bg);

                // Active step fill — velocity controls brightness/saturation
                if step.enabled {
                    let pad = 2.0;
                    let inner = Rect::from_min_max(
                        Pos2::new(rect.left() + pad, rect.top() + pad),
                        Pos2::new(rect.right() - pad, rect.bottom() - pad),
                    );
                    let vel = step.velocity * (if is_muted { 0.3 } else { 1.0 });
                    // Velocity dims the color: low vel = dark/desaturated, high vel = full color
                    let fill_alpha = 0.15 + vel * 0.85;
                    let fill_color = if step.condition != ConditionTrig::Always {
                        cat_color.linear_multiply(fill_alpha * 0.35)
                    } else {
                        cat_color.linear_multiply(fill_alpha)
                    };
                    ui.painter().rect_filled(inner, 2.0, fill_color);
                }

                // P-lock dot (top-left)
                if step.pan.is_some() || step.pitch.is_some() {
                    let dot_pos = Pos2::new(rect.left() + 4.0, rect.top() + 4.0);
                    ui.painter().circle_filled(dot_pos, 2.0, theme::PLOCK_DOT);
                }

                // Conditional trig indicator (top-right)
                if step.condition != ConditionTrig::Always {
                    ui.painter().text(
                        Pos2::new(rect.right() - 2.0, rect.top() + 2.0),
                        Align2::RIGHT_TOP,
                        step.condition.label(),
                        FontId::monospace(6.0),
                        theme::COND_TEXT,
                    );
                }

                // Playhead indicator (top border)
                if is_playhead {
                    ui.painter().line_segment(
                        [rect.left_top(), rect.right_top()],
                        Stroke::new(2.0, theme::PLAYHEAD),
                    );
                }

                // Hover highlight
                let pointer_pos = ui.input(|i| i.pointer.hover_pos());
                let is_hovered = pointer_pos.map_or(false, |p| rect.contains(p));

                // Glow borders: green=active, red=muted, purple=selected
                if is_selected {
                    let purple = Color32::from_rgb(180, 100, 255);
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.5, purple),
                        egui::StrokeKind::Inside,
                    );
                } else if step.enabled && is_muted {
                    let red = Color32::from_rgb(200, 60, 60);
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.0, red.linear_multiply(0.5)),
                        egui::StrokeKind::Inside,
                    );
                } else if step.enabled {
                    let green = Color32::from_rgb(50, 200, 100);
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.0, green.linear_multiply(0.35)),
                        egui::StrokeKind::Inside,
                    );
                } else {
                    let border_color = if is_hovered { theme::STEP_HOVER } else { theme::STEP_BORDER };
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.0, border_color),
                        egui::StrokeKind::Inside,
                    );
                }

                // Right-click: select step for param editing
                if response.secondary_clicked() {
                    if step.enabled {
                        view_state.selected = Some((lane_idx, step_idx));
                    }
                }

                // Mouse-down on a cell: record origin for drag disambiguation.
                // We don't toggle yet — wait for release (click) or drag direction.
                if response.drag_started() && view_state.drag_origin.is_none() && !had_active_drag {
                    if let Some(pos) = ui.input(|i| i.pointer.press_origin()) {
                        view_state.drag_origin = Some((lane_idx, step_idx));
                        view_state.drag_start_y = pos.y;
                        view_state.drag_is_velocity = false;
                        view_state.drag_start_velocity = step.velocity;
                    }
                }

                // Click (release without significant drag): toggle step.
                // Suppress if a drag mode was committed to prevent double-toggle.
                // Double-click: reset step to defaults
                if response.double_clicked() {
                    actions.push(SeqAction::ResetStep { lane: lane_idx, step: step_idx });
                } else if response.clicked() && view_state.drag_paint.is_none()
                    && !view_state.drag_is_velocity && !had_active_drag
                {
                    let new_state = !step.enabled;
                    view_state.drag_paint = Some(new_state);
                    view_state.drag_visited.insert((lane_idx, step_idx));
                    actions.push(SeqAction::SetStepEnabled { lane: lane_idx, step: step_idx, enabled: new_state });
                }
            }
        });
    }

    // Drag pass: disambiguate horizontal (paint) vs vertical (velocity) drag.
    let pointer_down = ui.input(|i| i.pointer.primary_down());
    if pointer_down {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            let vert_threshold = 4.0;

            // If we have a drag origin but haven't committed to a mode yet,
            // decide based on pointer movement.
            if let Some((origin_lane, origin_step)) = view_state.drag_origin {
                if !view_state.drag_is_velocity && view_state.drag_paint.is_none() {
                    // Check if pointer moved to a different step column -> paint mode
                    let moved_horizontal = view_state.cell_rects.iter()
                        .any(|&(_, si, r)| r.contains(pos) && si != origin_step);
                    if moved_horizontal {
                        // Enter paint mode: toggle origin cell, then paint the new one
                        let step_enabled = display.lanes.get(origin_lane)
                            .map(|l| l.steps[origin_step].enabled)
                            .unwrap_or(false);
                        let new_state = !step_enabled;
                        view_state.drag_paint = Some(new_state);
                        view_state.drag_visited.insert((origin_lane, origin_step));
                        actions.push(SeqAction::SetStepEnabled { lane: origin_lane, step: origin_step, enabled: new_state });
                    }
                    // Check if pointer moved vertically enough -> velocity mode
                    let delta_y = pos.y - view_state.drag_start_y;
                    if delta_y.abs() > vert_threshold {
                        view_state.drag_is_velocity = true;
                        // Select the step for visual feedback
                        view_state.selected = Some((origin_lane, origin_step));
                        // Ensure the step is enabled for velocity adjustment
                        let step_enabled = display.lanes.get(origin_lane)
                            .map(|l| l.steps[origin_step].enabled)
                            .unwrap_or(false);
                        if !step_enabled {
                            actions.push(SeqAction::SetStepEnabled { lane: origin_lane, step: origin_step, enabled: true });
                        }
                    }
                }
            }

            // Velocity drag mode: adjust velocity based on vertical delta
            if view_state.drag_is_velocity {
                if let Some((origin_lane, origin_step)) = view_state.drag_origin {
                    let delta_y = pos.y - view_state.drag_start_y;
                    // Drag up = increase velocity, drag down = decrease. 100px = full range.
                    let velocity_delta = -delta_y / 100.0;
                    let new_vel = (view_state.drag_start_velocity + velocity_delta).clamp(0.0, 1.0);
                    actions.push(SeqAction::SetStepVelocity { lane: origin_lane, step: origin_step, value: new_vel });
                }
            }

            // Paint mode: continue painting cells the pointer passes over
            if let Some(enable) = view_state.drag_paint {
                for &(lane_idx, step_idx, rect) in &view_state.cell_rects {
                    if rect.contains(pos) && !view_state.drag_visited.contains(&(lane_idx, step_idx)) {
                        view_state.drag_visited.insert((lane_idx, step_idx));
                        actions.push(SeqAction::SetStepEnabled { lane: lane_idx, step: step_idx, enabled: enable });
                    }
                }
            }
        }
    }

    // Request repaint when playing (for playhead animation)
    if display.playing {
        ui.ctx().request_repaint();
    }

    (actions, GridLayout { grid_right_x })
}

/// Draw step parameter knobs inline (no wrapper horizontal — caller provides one).
fn draw_param_controls(
    ui: &mut egui::Ui,
    lane_idx: usize,
    step_idx: usize,
    step: &StepDisplay,
    category: SampleCategory,
) -> Vec<SeqAction> {
    let mut actions = Vec::new();
    let cat_color = theme::category_color32(category);

    // Step indicator
    ui.label(
        egui::RichText::new(format!("STEP {}", step_idx + 1))
            .font(FontId::monospace(11.0))
            .color(theme::ACCENT)
            .strong(),
    );

    // VEL knob
    let mut vel = step.velocity;
    let vel_resp = knob::knob(
        ui,
        egui::Id::new(("seq_vel", lane_idx, step_idx)),
        &mut vel,
        0.0, 1.0, 0.8,
        "VEL",
        |v| format!("{}", (v * 100.0) as u8),
        cat_color,
        34.0,
    );
    if vel_resp.changed {
        actions.push(SeqAction::SetStepVelocity { lane: lane_idx, step: step_idx, value: vel });
    }
    if vel_resp.reset {
        actions.push(SeqAction::SetStepVelocity { lane: lane_idx, step: step_idx, value: 0.8 });
    }

    // PAN knob
    let mut pan = step.pan.unwrap_or(0.0);
    let pan_color = if step.pan.is_some() { theme::PLOCK_DOT } else { Color32::from_rgb(80, 80, 80) };
    let pan_resp = knob::knob(
        ui,
        egui::Id::new(("seq_pan", lane_idx, step_idx)),
        &mut pan,
        -1.0, 1.0, 0.0,
        "PAN",
        |v| {
            if v.abs() < 0.01 { "C".to_string() }
            else if v < 0.0 { format!("L{}", (-v * 100.0) as u8) }
            else { format!("R{}", (v * 100.0) as u8) }
        },
        pan_color,
        34.0,
    );
    if pan_resp.changed {
        actions.push(SeqAction::SetStepPan { lane: lane_idx, step: step_idx, value: Some(pan) });
    }
    if pan_resp.reset {
        actions.push(SeqAction::SetStepPan { lane: lane_idx, step: step_idx, value: None });
    }

    // PITCH knob
    let mut pitch = step.pitch.unwrap_or(0.0);
    let pitch_color = if step.pitch.is_some() { theme::PLOCK_DOT } else { Color32::from_rgb(80, 80, 80) };
    let pitch_resp = knob::knob(
        ui,
        egui::Id::new(("seq_pitch", lane_idx, step_idx)),
        &mut pitch,
        -24.0, 24.0, 0.0,
        "PITCH",
        |v| {
            if v.abs() < 0.1 { "0".to_string() }
            else { format!("{:+.0}", v) }
        },
        pitch_color,
        34.0,
    );
    if pitch_resp.changed {
        actions.push(SeqAction::SetStepPitch { lane: lane_idx, step: step_idx, value: Some(pitch) });
    }
    if pitch_resp.reset {
        actions.push(SeqAction::SetStepPitch { lane: lane_idx, step: step_idx, value: None });
    }

    // PROB knob
    let mut prob = step.probability;
    let prob_resp = knob::knob(
        ui,
        egui::Id::new(("seq_prob", lane_idx, step_idx)),
        &mut prob,
        0.0, 1.0, 1.0,
        "PROB",
        |v| format!("{}", (v * 100.0) as u8),
        cat_color,
        34.0,
    );
    if prob_resp.changed {
        actions.push(SeqAction::SetStepProbability { lane: lane_idx, step: step_idx, value: prob });
    }
    if prob_resp.reset {
        actions.push(SeqAction::SetStepProbability { lane: lane_idx, step: step_idx, value: 1.0 });
    }

    // COND selector
    ui.vertical(|ui| {
        let cond_text = step.condition.label();
        let cond_color = if step.condition != ConditionTrig::Always { theme::COND_TEXT } else { theme::TEXT_DIM };
        egui::ComboBox::from_id_salt(("seq_cond", lane_idx, step_idx))
            .selected_text(
                egui::RichText::new(cond_text)
                    .font(FontId::monospace(9.0))
                    .color(cond_color),
            )
            .width(56.0)
            .show_ui(ui, |ui| {
                for &cond in ConditionTrig::CYCLE {
                    let is_active = step.condition == cond;
                    let label_color = if is_active { theme::ACCENT } else { theme::TEXT_DIM };
                    let resp = ui.selectable_label(
                        is_active,
                        egui::RichText::new(cond.label())
                            .font(FontId::monospace(9.0))
                            .color(label_color),
                    );
                    if resp.clicked() && !is_active {
                        actions.push(SeqAction::SetStepCondition { lane: lane_idx, step: step_idx, condition: cond });
                    }
                }
            });
        ui.label(
            egui::RichText::new("COND")
                .font(FontId::monospace(8.0))
                .color(theme::TEXT_DIM),
        );
    });

    actions
}

fn draw_bottom_bar(ui: &mut egui::Ui, display: &SeqDisplay) -> Option<SeqAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;

        // Play/Stop toggle button
        let play_label = if display.playing { "\u{25A0} STOP" } else { "\u{25B6} PLAY" };
        let play_color = if display.playing { Color32::BLACK } else { theme::ACCENT };
        let play_bg = if display.playing { theme::ACCENT } else { theme::STEP_BG };
        if ui.add(
            egui::Button::new(
                egui::RichText::new(play_label).font(FontId::monospace(10.0)).color(play_color).strong(),
            )
            .fill(play_bg)
            .min_size(Vec2::new(60.0, 22.0))
            .corner_radius(3.0),
        ).clicked() {
            action = Some(SeqAction::ToggleInternalPlay);
        }

        // EXT indicator — shown when echo suppression is active
        if display.ext_mode {
            ui.add(
                egui::Label::new(
                    egui::RichText::new("EXT")
                        .font(FontId::monospace(10.0))
                        .color(Color32::from_rgb(255, 165, 0))
                        .strong()
                )
            );
        }

        ui.separator();

        // DICE button
        let dice_color = theme::category_color32(SampleCategory::Kick);
        let dice_btn = egui::Button::new(
            egui::RichText::new("DICE")
                .font(FontId::monospace(10.0))
                .color(dice_color),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, dice_color))
        .min_size(Vec2::new(60.0, 22.0))
        .corner_radius(3.0);
        if ui.add(dice_btn).clicked() {
            action = Some(SeqAction::DicePattern);
        }

        // FILL button (momentary)
        let fill_bg = if display.fill_active { theme::FILL_PURPLE } else { Color32::TRANSPARENT };
        let fill_text_color = if display.fill_active { Color32::WHITE } else { theme::FILL_PURPLE };
        let fill_btn = egui::Button::new(
            egui::RichText::new("FILL")
                .font(FontId::monospace(10.0))
                .color(fill_text_color),
        )
        .fill(fill_bg)
        .stroke(Stroke::new(1.0, theme::FILL_PURPLE))
        .min_size(Vec2::new(46.0, 22.0))
        .corner_radius(3.0);
        let fill_resp = ui.add(fill_btn);
        if fill_resp.is_pointer_button_down_on() && !display.fill_active {
            action = Some(SeqAction::SetFillActive { active: true });
        }
        if !fill_resp.is_pointer_button_down_on() && display.fill_active {
            action = Some(SeqAction::SetFillActive { active: false });
        }

        // Right side: COPY PASTE CLEAR
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let btn_style = |text: &str| {
                egui::Button::new(
                    egui::RichText::new(text).font(FontId::monospace(10.0)).color(theme::TEXT_DIM),
                )
                .fill(theme::STEP_BG)
                .min_size(Vec2::new(42.0, 22.0))
                .corner_radius(3.0)
            };

            if ui.add(btn_style("CLEAR")).clicked() {
                action = Some(SeqAction::ClearPattern);
            }
            if ui.add(btn_style("PASTE")).clicked() {
                action = Some(SeqAction::PastePattern);
            }
            if ui.add(btn_style("COPY")).clicked() {
                action = Some(SeqAction::CopyPattern);
            }
            if ui.add(btn_style("LOAD")).clicked() {
                action = Some(SeqAction::OpenLoadPatternDialog);
            }
            if ui.add(btn_style("SAVE")).clicked() {
                action = Some(SeqAction::OpenSavePatternDialog);
            }

            // EXPORT button — write active pattern to .mid file
            let export_btn = egui::Button::new(
                egui::RichText::new("EXPORT").font(FontId::monospace(10.0)).color(theme::ACCENT),
            )
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, theme::ACCENT))
            .min_size(Vec2::new(52.0, 22.0))
            .corner_radius(3.0);
            if ui.add(export_btn).clicked() {
                action = Some(SeqAction::ExportMidi);
            }
        });
    });

    action
}
