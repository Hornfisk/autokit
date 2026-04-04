use nih_plug_egui::egui;
use egui::{Color32, Rect, Pos2, Vec2, Stroke, FontId, Align2};
use crate::engine::kit::{SampleCategory, NUM_PADS};
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
}

pub struct LaneDisplay {
    pub pad_name: String,
    pub category: SampleCategory,
    pub muted: bool,
    pub locked: bool,
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
    ToggleLaneLock { lane: usize },
    SelectPattern { index: usize },
    SetSwing { value: f32 },
    CopyPattern,
    PastePattern,
    ClearPattern,
    DicePattern,
    SetFillActive { active: bool },
    ToggleInternalPlay,
}

/// Draw the complete sequencer view. Returns a list of actions.
pub fn draw_sequencer_view(
    ui: &mut egui::Ui,
    display: &SeqDisplay,
    view_state: &mut SeqViewState,
) -> Vec<SeqAction> {
    let mut actions: Vec<SeqAction> = Vec::new();

    // Pattern selector bar
    if let Some(a) = draw_pattern_bar(ui, display) {
        actions.push(a);
    }

    ui.add_space(2.0);

    // Step grid (may return multiple actions from drag-paint)
    actions.extend(draw_grid(ui, display, view_state));

    ui.add_space(2.0);

    // Step parameter bar (only if a step is selected)
    if let Some((lane_idx, step_idx)) = view_state.selected {
        if lane_idx < display.lanes.len() {
            let step = &display.lanes[lane_idx].steps[step_idx];
            let cat = display.lanes[lane_idx].category;
            if let Some(a) = draw_param_bar(ui, lane_idx, step_idx, step, cat) {
                actions.push(a);
            }
        }
    }

    // Bottom bar
    if let Some(a) = draw_bottom_bar(ui, display) {
        actions.push(a);
    }

    actions
}

fn draw_pattern_bar(ui: &mut egui::Ui, display: &SeqDisplay) -> Option<SeqAction> {
    let mut action = None;

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;

        ui.label(
            egui::RichText::new("PATTERN")
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
            .min_size(Vec2::new(30.0, 20.0))
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
                action = Some(SeqAction::SelectPattern { index: i });
            }
        }

        // Swing control (right side)
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let swing_pct = (display.swing * 100.0) as u8;
            ui.label(
                egui::RichText::new(format!("{swing_pct}%"))
                    .font(FontId::monospace(9.0))
                    .color(theme::ACCENT),
            );

            let (rect, response) = ui.allocate_exact_size(Vec2::new(60.0, 6.0), egui::Sense::drag());
            ui.painter().rect_filled(rect, 3.0, theme::STEP_BG);
            let fill_width = rect.width() * display.swing;
            let fill_rect = Rect::from_min_size(rect.min, Vec2::new(fill_width, rect.height()));
            ui.painter().rect_filled(fill_rect, 3.0, theme::ACCENT);

            if response.dragged() {
                let delta = response.drag_delta().x / 60.0;
                let new_swing = (display.swing + delta).clamp(0.0, 1.0);
                action = Some(SeqAction::SetSwing { value: new_swing });
            }

            ui.label(
                egui::RichText::new("SWING")
                    .font(FontId::monospace(9.0))
                    .color(theme::TEXT_DIM),
            );
        });
    });

    action
}

fn draw_grid(
    ui: &mut egui::Ui,
    display: &SeqDisplay,
    view_state: &mut SeqViewState,
) -> Vec<SeqAction> {
    let mut actions: Vec<SeqAction> = Vec::new();

    let label_width = 56.0;
    let controls_width = 30.0;
    let cell_spacing = 2.0;
    let available = ui.clip_rect().width() - label_width - controls_width - 24.0;
    let cell_size = ((available - cell_spacing * 15.0) / 16.0).floor().max(20.0);
    let row_height = cell_size.min(30.0);

    // Track whether a drag was active before clearing — prevents the released
    // frame's clicked() from double-toggling the step the drag already toggled.
    let had_active_drag = view_state.drag_paint.is_some();

    // Clear drag state when mouse released
    if !ui.input(|i| i.pointer.any_pressed() || i.pointer.any_down()) {
        view_state.drag_paint = None;
        view_state.drag_visited.clear();
    }
    view_state.cell_rects.clear();

    // Step numbers header
    ui.horizontal(|ui| {
        ui.add_space(label_width + controls_width + 4.0);
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

    // Track rows
    for lane_idx in 0..display.lanes.len() {
        let lane = &display.lanes[lane_idx];
        let cat_color = theme::category_color32(lane.category);
        let is_muted = lane.muted;

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;

            // Track label
            let label = egui::RichText::new(lane.category.label())
                .font(FontId::monospace(9.0))
                .color(if is_muted { cat_color.linear_multiply(0.3) } else { cat_color });
            ui.allocate_ui(Vec2::new(label_width, row_height), |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(label);
                });
            });

            // Mute button
            let mute_text = egui::RichText::new("M").font(FontId::monospace(7.0));
            let mute_btn = egui::Button::new(mute_text.color(if is_muted { Color32::WHITE } else { theme::TEXT_DIM }))
                .fill(if is_muted { theme::MUTE_RED } else { theme::STEP_BG })
                .min_size(Vec2::new(13.0, 13.0))
                .corner_radius(2.0);
            if ui.add(mute_btn).clicked() {
                actions.push(SeqAction::ToggleLaneMute { lane: lane_idx });
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
                    egui::Sense::click(),
                );

                // Store rect for pointer hit-testing in drag pass
                view_state.cell_rects.push((lane_idx, step_idx, rect));

                // Background
                let bg = if is_beat { theme::STEP_BG_BEAT } else { theme::STEP_BG };
                let bg = if is_muted { bg.linear_multiply(0.3) } else { bg };
                ui.painter().rect_filled(rect, 3.0, bg);

                // Velocity bar
                if step.enabled {
                    let bar_height = rect.height() * step.velocity * (if is_muted { 0.3 } else { 1.0 });
                    let bar_rect = Rect::from_min_size(
                        Pos2::new(rect.left() + rect.width() * 0.15, rect.bottom() - bar_height),
                        Vec2::new(rect.width() * 0.7, bar_height),
                    );
                    let bar_color = if step.condition != ConditionTrig::Always {
                        cat_color.linear_multiply(0.35)
                    } else {
                        cat_color
                    };
                    ui.painter().rect_filled(bar_rect, 1.0, bar_color);
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

                // Selection border
                if is_selected {
                    ui.painter().rect_stroke(
                        rect,
                        3.0,
                        Stroke::new(1.5, theme::ACCENT),
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

                // Single click (no drag active): toggle + start drag paint.
                // Suppress if a drag was just cleared this frame to prevent double-toggle.
                if response.clicked() && view_state.drag_paint.is_none() && !had_active_drag {
                    let new_state = !step.enabled;
                    view_state.drag_paint = Some(new_state);
                    view_state.drag_visited.insert((lane_idx, step_idx));
                    actions.push(SeqAction::SetStepEnabled { lane: lane_idx, step: step_idx, enabled: new_state });
                }
            }
        });
    }

    // Drag-paint pass: check pointer against all cell rects
    let pointer_down = ui.input(|i| i.pointer.primary_down());
    if pointer_down {
        if let Some(pos) = ui.input(|i| i.pointer.hover_pos()) {
            // If no drag started yet but pointer is down over a cell, start drag
            if view_state.drag_paint.is_none() {
                for &(lane_idx, step_idx, rect) in &view_state.cell_rects {
                    if rect.contains(pos) {
                        let step_enabled = display.lanes.get(lane_idx)
                            .map(|l| l.steps[step_idx].enabled)
                            .unwrap_or(false);
                        let new_state = !step_enabled;
                        view_state.drag_paint = Some(new_state);
                        view_state.drag_visited.insert((lane_idx, step_idx));
                        actions.push(SeqAction::SetStepEnabled { lane: lane_idx, step: step_idx, enabled: new_state });
                        break;
                    }
                }
            }
            // Continue painting: any cell the pointer passes over
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

    actions
}

fn draw_param_bar(
    ui: &mut egui::Ui,
    lane_idx: usize,
    step_idx: usize,
    step: &StepDisplay,
    category: SampleCategory,
) -> Option<SeqAction> {
    let mut action = None;
    let cat_color = theme::category_color32(category);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 12.0;

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
            action = Some(SeqAction::SetStepVelocity { lane: lane_idx, step: step_idx, value: vel });
        }
        if vel_resp.reset {
            action = Some(SeqAction::SetStepVelocity { lane: lane_idx, step: step_idx, value: 0.8 });
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
            action = Some(SeqAction::SetStepPan { lane: lane_idx, step: step_idx, value: Some(pan) });
        }
        if pan_resp.reset {
            action = Some(SeqAction::SetStepPan { lane: lane_idx, step: step_idx, value: None });
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
            action = Some(SeqAction::SetStepPitch { lane: lane_idx, step: step_idx, value: Some(pitch) });
        }
        if pitch_resp.reset {
            action = Some(SeqAction::SetStepPitch { lane: lane_idx, step: step_idx, value: None });
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
            action = Some(SeqAction::SetStepProbability { lane: lane_idx, step: step_idx, value: prob });
        }
        if prob_resp.reset {
            action = Some(SeqAction::SetStepProbability { lane: lane_idx, step: step_idx, value: 1.0 });
        }

        // COND selector — dropdown showing all options
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
                            action = Some(SeqAction::SetStepCondition { lane: lane_idx, step: step_idx, condition: cond });
                        }
                    }
                });
            ui.label(
                egui::RichText::new("COND")
                    .font(FontId::monospace(8.0))
                    .color(theme::TEXT_DIM),
            );
        });
    });

    action
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
                .min_size(Vec2::new(46.0, 22.0))
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
        });
    });

    action
}
