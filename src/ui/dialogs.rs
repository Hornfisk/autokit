use nih_plug_egui::egui;
use std::path::PathBuf;

use crate::ui::folder_browser::{self, FolderBrowser};
use crate::ui::theme;
use crate::util::config;

/// State for all modal dialogs.
pub struct DialogState {
    pub show_save: bool,
    pub save_name: String,
    pub show_load: bool,
    pub preset_list: Vec<(String, PathBuf)>,
    pub show_setup: bool,
    pub setup_path: String,
    pub folder_browser: Option<FolderBrowser>,
    pub show_save_pattern: bool,
    pub save_pattern_name: String,
    pub show_load_pattern: bool,
    pub pattern_list: Vec<(String, PathBuf)>,
}

impl Default for DialogState {
    fn default() -> Self {
        Self {
            show_save: false,
            save_name: String::new(),
            show_load: false,
            preset_list: Vec::new(),
            show_setup: false,
            setup_path: String::new(),
            folder_browser: None,
            show_save_pattern: false,
            save_pattern_name: String::new(),
            show_load_pattern: false,
            pattern_list: Vec::new(),
        }
    }
}

/// Result of showing dialogs — actions the editor should handle.
pub enum DialogAction {
    None,
    SavePreset(String),
    LoadPreset(PathBuf),
    DeletePreset(PathBuf),
    SavePattern(String),
    LoadPattern(PathBuf),
    DeletePattern(PathBuf),
    StartScan(PathBuf),
}

/// Show the save-preset dialog. Returns an action if the user confirms.
pub fn show_save_dialog(ctx: &egui::Context, state: &mut DialogState) -> DialogAction {
    let mut action = DialogAction::None;
    let mut open = true;
    egui::Window::new("Save Preset")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([240.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Preset name:")
                    .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                    .color(theme::TEXT_DIM),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.save_name)
                    .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                    .desired_width(220.0),
            );
            if response.gained_focus() || state.save_name.is_empty() {
                response.request_focus();
            }

            ui.add_space(6.0);

            let name_valid = !state.save_name.trim().is_empty();
            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        name_valid,
                        egui::Button::new(
                            egui::RichText::new("SAVE")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(if name_valid {
                                    egui::Color32::from_rgb(0x74, 0xb9, 0xff)
                                } else {
                                    theme::TEXT_DISABLED
                                }),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(60.0, 22.0)),
                    )
                    .clicked()
                    || (enter_pressed && name_valid)
                {
                    action = DialogAction::SavePreset(state.save_name.trim().to_string());
                    state.show_save = false;
                }

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("CANCEL")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DIM),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(60.0, 22.0)),
                    )
                    .clicked()
                {
                    state.show_save = false;
                }
            });
        });
    if !open {
        state.show_save = false;
    }
    action
}

/// Show the load-preset dialog. Returns an action if the user selects or deletes a preset.
pub fn show_load_dialog(ctx: &egui::Context, state: &mut DialogState) -> DialogAction {
    let mut action = DialogAction::None;
    let mut open = true;
    egui::Window::new("Load Preset")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([280.0, 300.0])
        .open(&mut open)
        .show(ctx, |ui| {
            if state.preset_list.is_empty() {
                ui.label(
                    egui::RichText::new("No presets found.")
                        .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DIM),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        let mut delete_idx = None;
                        for (idx, (name, path)) in state.preset_list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(name)
                                                .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                                                .color(theme::ACCENT),
                                        )
                                        .fill(theme::BG_ROW)
                                        .min_size(egui::vec2(230.0, 24.0)),
                                    )
                                    .clicked()
                                {
                                    action = DialogAction::LoadPreset(path.clone());
                                    state.show_load = false;
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("\u{00d7}")
                                                .font(egui::FontId::new(12.0, egui::FontFamily::Monospace))
                                                .color(theme::MUTE_RED),
                                        )
                                        .fill(theme::BG_ROW)
                                        .min_size(egui::vec2(24.0, 24.0)),
                                    )
                                    .clicked()
                                {
                                    action = DialogAction::DeletePreset(path.clone());
                                    delete_idx = Some(idx);
                                }
                            });
                        }
                        if let Some(idx) = delete_idx {
                            state.preset_list.remove(idx);
                        }
                    });
            }

            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("CANCEL")
                            .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DIM),
                    )
                    .fill(theme::BG_ROW)
                    .min_size(egui::vec2(60.0, 22.0)),
                )
                .clicked()
            {
                state.show_load = false;
            }
        });
    if !open {
        state.show_load = false;
    }
    action
}

/// Show the sample folder setup dialog. Returns an action if the user confirms.
pub fn show_setup_dialog(ctx: &egui::Context, state: &mut DialogState) -> DialogAction {
    let mut action = DialogAction::None;

    // If folder browser is active, show it
    if let Some(browser) = &mut state.folder_browser {
        match browser.show(ctx) {
            folder_browser::BrowserAction::Selected(path) => {
                state.setup_path = path.to_string_lossy().into_owned();
                state.folder_browser = None;
            }
            folder_browser::BrowserAction::Cancelled => {
                state.folder_browser = None;
            }
            folder_browser::BrowserAction::None => {}
        }
        return action;
    }

    // Show the setup dialog
    let mut open = true;
    egui::Window::new("Sample Library")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([400.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Select your samples folder:")
                    .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                    .color(theme::TEXT_DIM),
            );
            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut state.setup_path)
                        .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                        .desired_width(300.0),
                );
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("BROWSE")
                                .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                                .color(theme::ACCENT),
                        )
                        .fill(theme::ACCENT_DIM)
                        .min_size(egui::vec2(60.0, 22.0)),
                    )
                    .clicked()
                {
                    let start = if state.setup_path.is_empty() {
                        config::home_dir()
                    } else {
                        PathBuf::from(&state.setup_path)
                    };
                    state.folder_browser = Some(FolderBrowser::new(&start));
                }
            });

            // Validation hint
            let path = PathBuf::from(&state.setup_path);
            let path_valid = !state.setup_path.is_empty() && path.is_dir();
            if !state.setup_path.is_empty() && !path_valid {
                ui.label(
                    egui::RichText::new("folder not found")
                        .font(egui::FontId::new(9.0, egui::FontFamily::Monospace))
                        .color(egui::Color32::from_rgb(0xff, 0x6b, 0x6b)),
                );
            }

            ui.add_space(8.0);

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        path_valid,
                        egui::Button::new(
                            egui::RichText::new("SCAN")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(if path_valid { theme::ACCENT } else { theme::TEXT_DISABLED })
                                .strong(),
                        )
                        .fill(if path_valid { theme::ACCENT_DIM } else { theme::BG_ROW })
                        .min_size(egui::vec2(60.0, 24.0)),
                    )
                    .clicked()
                {
                    let cfg = config::Config::new(&state.setup_path);
                    cfg.save();
                    action = DialogAction::StartScan(path);
                    state.show_setup = false;
                }

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("CANCEL")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DIM),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(60.0, 24.0)),
                    )
                    .clicked()
                {
                    state.show_setup = false;
                }
            });
        });
    if !open {
        state.show_setup = false;
    }
    action
}

/// Show the save-pattern dialog. Returns an action if the user confirms.
pub fn show_save_pattern_dialog(ctx: &egui::Context, state: &mut DialogState) -> DialogAction {
    let mut action = DialogAction::None;
    let mut open = true;
    egui::Window::new("Save Pattern")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([240.0, 0.0])
        .open(&mut open)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new("Pattern name:")
                    .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                    .color(theme::TEXT_DIM),
            );
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.save_pattern_name)
                    .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                    .desired_width(220.0),
            );
            if response.gained_focus() || state.save_pattern_name.is_empty() {
                response.request_focus();
            }

            ui.add_space(6.0);

            let name_valid = !state.save_pattern_name.trim().is_empty();
            let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));

            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        name_valid,
                        egui::Button::new(
                            egui::RichText::new("SAVE")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(if name_valid {
                                    egui::Color32::from_rgb(0x74, 0xb9, 0xff)
                                } else {
                                    theme::TEXT_DISABLED
                                }),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(60.0, 22.0)),
                    )
                    .clicked()
                    || (enter_pressed && name_valid)
                {
                    action = DialogAction::SavePattern(state.save_pattern_name.trim().to_string());
                    state.show_save_pattern = false;
                }

                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new("CANCEL")
                                .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                .color(theme::TEXT_DIM),
                        )
                        .fill(theme::BG_ROW)
                        .min_size(egui::vec2(60.0, 22.0)),
                    )
                    .clicked()
                {
                    state.show_save_pattern = false;
                }
            });
        });
    if !open {
        state.show_save_pattern = false;
    }
    action
}

/// Show the load-pattern dialog. Returns an action if the user selects or deletes a pattern.
pub fn show_load_pattern_dialog(ctx: &egui::Context, state: &mut DialogState) -> DialogAction {
    let mut action = DialogAction::None;
    let mut open = true;
    egui::Window::new("Load Pattern")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .fixed_size([280.0, 300.0])
        .open(&mut open)
        .show(ctx, |ui| {
            if state.pattern_list.is_empty() {
                ui.label(
                    egui::RichText::new("No patterns found.")
                        .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                        .color(theme::TEXT_DIM),
                );
            } else {
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        let mut delete_idx = None;
                        for (idx, (name, path)) in state.pattern_list.iter().enumerate() {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new(name)
                                                .font(egui::FontId::new(11.0, egui::FontFamily::Monospace))
                                                .color(theme::ACCENT),
                                        )
                                        .fill(theme::BG_ROW)
                                        .min_size(egui::vec2(230.0, 24.0)),
                                    )
                                    .clicked()
                                {
                                    action = DialogAction::LoadPattern(path.clone());
                                    state.show_load_pattern = false;
                                }
                                if ui
                                    .add(
                                        egui::Button::new(
                                            egui::RichText::new("\u{00d7}")
                                                .font(egui::FontId::new(12.0, egui::FontFamily::Monospace))
                                                .color(theme::MUTE_RED),
                                        )
                                        .fill(theme::BG_ROW)
                                        .min_size(egui::vec2(24.0, 24.0)),
                                    )
                                    .clicked()
                                {
                                    action = DialogAction::DeletePattern(path.clone());
                                    delete_idx = Some(idx);
                                }
                            });
                        }
                        if let Some(idx) = delete_idx {
                            state.pattern_list.remove(idx);
                        }
                    });
            }

            ui.add_space(4.0);
            if ui
                .add(
                    egui::Button::new(
                        egui::RichText::new("CANCEL")
                            .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                            .color(theme::TEXT_DIM),
                    )
                    .fill(theme::BG_ROW)
                    .min_size(egui::vec2(60.0, 22.0)),
                )
                .clicked()
            {
                state.show_load_pattern = false;
            }
        });
    if !open {
        state.show_load_pattern = false;
    }
    action
}
