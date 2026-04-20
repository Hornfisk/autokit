//! Lightweight built-in folder browser for egui.
//!
//! Works identically in standalone and plugin contexts (no OS file dialog needed).

use nih_plug_egui::egui;
use std::path::{Path, PathBuf};

use crate::ui::theme;

/// A cached directory entry (folders only).
struct DirEntry {
    name: String,
    path: PathBuf,
}

/// Built-in folder browser state.
pub struct FolderBrowser {
    /// Currently displayed directory.
    current_path: PathBuf,
    /// Cached child directories (lazy-loaded, sorted).
    entries: Vec<DirEntry>,
    /// Whether entries need to be refreshed.
    dirty: bool,
    /// When true, dotfile directories (`.cache`, `.config`, etc.) are shown.
    /// Per-session, not persisted.
    show_hidden: bool,
}

/// What happened this frame.
pub enum BrowserAction {
    None,
    /// User confirmed this folder.
    Selected(PathBuf),
    /// User cancelled.
    Cancelled,
}

impl FolderBrowser {
    /// Create a new browser starting at the given directory.
    pub fn new(start: &Path) -> Self {
        let start = if start.is_dir() {
            start.to_path_buf()
        } else {
            crate::util::config::home_dir()
        };
        FolderBrowser {
            current_path: start,
            entries: Vec::new(),
            dirty: true,
            show_hidden: false,
        }
    }

    /// Refresh the directory listing.
    fn refresh(&mut self) {
        self.entries.clear();
        if let Ok(read_dir) = std::fs::read_dir(&self.current_path) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().into_owned();
                if !self.show_hidden && name.starts_with('.') {
                    continue;
                }
                self.entries.push(DirEntry { name, path });
            }
        }
        self.entries.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        self.dirty = false;
    }

    /// Navigate to a directory.
    fn navigate(&mut self, path: PathBuf) {
        self.current_path = path;
        self.dirty = true;
    }

    /// Draw the browser as an egui Window. Returns the action taken.
    pub fn show(&mut self, ctx: &egui::Context) -> BrowserAction {
        if self.dirty {
            self.refresh();
        }

        let mut action = BrowserAction::None;
        let mut open = true;

        egui::Window::new("Browse Folders")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .fixed_size([420.0, 360.0])
            .open(&mut open)
            .show(ctx, |ui| {
                // Breadcrumb navigation
                let mut nav_to: Option<PathBuf> = None;
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    let ancestors: Vec<PathBuf> = self.current_path.ancestors()
                        .map(|a| a.to_path_buf())
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    for (i, ancestor) in ancestors.iter().enumerate() {
                        let label = if i == 0 {
                            #[cfg(target_os = "windows")]
                            { ancestor.to_string_lossy().to_string() }
                            #[cfg(not(target_os = "windows"))]
                            { "/".to_string() }
                        } else {
                            ancestor
                                .file_name()
                                .map(|n| n.to_string_lossy().to_string())
                                .unwrap_or_default()
                        };
                        if label.is_empty() {
                            continue;
                        }
                        if i > 1 {
                            ui.label(
                                egui::RichText::new("/")
                                    .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                    .color(theme::TEXT_DISABLED),
                            );
                        }
                        let is_current = *ancestor == self.current_path;
                        let color = if is_current { theme::ACCENT } else { theme::TEXT_DIM };
                        if ui
                            .add(
                                egui::Button::new(
                                    egui::RichText::new(&label)
                                        .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                        .color(color),
                                )
                                .fill(egui::Color32::TRANSPARENT)
                                .frame(false),
                            )
                            .clicked()
                            && !is_current
                        {
                            nav_to = Some(ancestor.clone());
                        }
                    }
                });
                if let Some(path) = nav_to {
                    self.navigate(path);
                }

                ui.add_space(4.0);
                ui.separator();

                // Directory listing
                egui::ScrollArea::vertical()
                    .max_height(260.0)
                    .show(ui, |ui| {
                        // Parent directory (..)
                        if let Some(parent) = self.current_path.parent() {
                            let parent = parent.to_path_buf();
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new("\u{1F4C1} ..")
                                            .font(egui::FontId::new(
                                                11.0,
                                                egui::FontFamily::Monospace,
                                            ))
                                            .color(theme::TEXT_DIM),
                                    )
                                    .fill(theme::BG_ROW)
                                    .min_size(egui::vec2(390.0, 22.0)),
                                )
                                .clicked()
                            {
                                self.navigate(parent);
                            }
                        }

                        // Child directories
                        let paths: Vec<(String, PathBuf)> = self
                            .entries
                            .iter()
                            .map(|e| (e.name.clone(), e.path.clone()))
                            .collect();
                        for (name, path) in &paths {
                            if ui
                                .add(
                                    egui::Button::new(
                                        egui::RichText::new(format!("\u{1F4C1} {name}"))
                                            .font(egui::FontId::new(
                                                11.0,
                                                egui::FontFamily::Monospace,
                                            ))
                                            .color(theme::ACCENT),
                                    )
                                    .fill(theme::BG_ROW)
                                    .min_size(egui::vec2(390.0, 22.0)),
                                )
                                .clicked()
                            {
                                self.navigate(path.clone());
                            }
                        }

                        if self.entries.is_empty() {
                            ui.label(
                                egui::RichText::new("(no subdirectories)")
                                    .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                    .color(theme::TEXT_DISABLED),
                            );
                        }
                    });

                ui.add_space(4.0);
                ui.separator();

                // Show-hidden toggle (per-session; triggers a refresh on change)
                if ui
                    .checkbox(&mut self.show_hidden, "show hidden")
                    .changed()
                {
                    self.dirty = true;
                }

                // Bottom bar: SELECT / CANCEL
                ui.horizontal(|ui| {
                    if ui
                        .add(
                            egui::Button::new(
                                egui::RichText::new("SELECT THIS FOLDER")
                                    .font(egui::FontId::new(10.0, egui::FontFamily::Monospace))
                                    .color(theme::ACCENT)
                                    .strong(),
                            )
                            .fill(theme::ACCENT_DIM)
                            .min_size(egui::vec2(160.0, 24.0)),
                        )
                        .clicked()
                    {
                        action = BrowserAction::Selected(self.current_path.clone());
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
                        action = BrowserAction::Cancelled;
                    }
                });
            });

        if !open {
            return BrowserAction::Cancelled;
        }

        action
    }
}
