use crate::engine::kit::SampleCategory;
use nih_plug_egui::egui;
use std::sync::Arc;

/// Color as [R, G, B] u8 values.
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn to_egui(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.0, self.1, self.2)
    }

    pub fn to_egui_alpha(&self, alpha: u8) -> egui::Color32 {
        egui::Color32::from_rgba_premultiplied(
            (self.0 as u16 * alpha as u16 / 255) as u8,
            (self.1 as u16 * alpha as u16 / 255) as u8,
            (self.2 as u16 * alpha as u16 / 255) as u8,
            alpha,
        )
    }
}

/// Get the display color for a sample category.
pub fn category_color(cat: SampleCategory) -> Color {
    match cat {
        SampleCategory::Kick => Color(0xff, 0x6b, 0x9d),
        SampleCategory::Snare => Color(0x4e, 0xcd, 0xc4),
        SampleCategory::Hihat => Color(0xff, 0x9f, 0x43),
        SampleCategory::Clap => Color(0xa8, 0xe6, 0xcf),
        SampleCategory::Tom => Color(0xff, 0x76, 0x75),
        SampleCategory::Perc => Color(0xc0, 0x84, 0xfc),
        SampleCategory::Cymbal => Color(0xff, 0xd1, 0x66),
        SampleCategory::Bass => Color(0x74, 0xb9, 0xff),
        SampleCategory::Synth => Color(0xfd, 0x79, 0xa8),
        SampleCategory::Other => Color(0x63, 0x6e, 0x72),
    }
}

// UI background colors
pub const BG_MAIN: egui::Color32 = egui::Color32::from_rgb(0x0a, 0x0a, 0x1a);
pub const BG_TOOLBAR: egui::Color32 = egui::Color32::from_rgb(0x0e, 0x0e, 0x20);
pub const BG_ROW: egui::Color32 = egui::Color32::from_rgb(0x11, 0x11, 0x26);
pub const BG_ROW_HOVER: egui::Color32 = egui::Color32::from_rgb(0x16, 0x16, 0x2e);
pub const BG_DETAIL: egui::Color32 = egui::Color32::from_rgb(0x0d, 0x0d, 0x22);

// Accent
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x00, 0xd4, 0xaa);
pub const ACCENT_DIM: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x00, 0x54, 0x44, 0x44);

// Text
pub const TEXT_PRIMARY: egui::Color32 = egui::Color32::from_rgb(0xcc, 0xcc, 0xcc);
pub const TEXT_DIM: egui::Color32 = egui::Color32::from_rgb(0x63, 0x6e, 0x72);
pub const TEXT_DISABLED: egui::Color32 =
    egui::Color32::from_rgba_premultiplied(0x32, 0x37, 0x39, 0x66);

// Sequencer-specific colors
pub const PLAYHEAD: egui::Color32 = egui::Color32::from_rgb(0, 212, 170);     // same as ACCENT
pub const STEP_BG: egui::Color32 = egui::Color32::from_rgb(17, 17, 38);       // #111126
pub const STEP_BG_BEAT: egui::Color32 = egui::Color32::from_rgb(19, 19, 48);  // #131330
pub const STEP_BORDER: egui::Color32 = egui::Color32::from_rgb(26, 26, 53);   // #1a1a35
pub const STEP_HOVER: egui::Color32 = egui::Color32::from_rgb(51, 51, 102);   // #333366
pub const PLOCK_DOT: egui::Color32 = egui::Color32::from_rgb(0, 170, 255);    // #00aaff
pub const COND_TEXT: egui::Color32 = egui::Color32::from_rgb(255, 204, 0);     // #ffcc00
pub const MUTE_RED: egui::Color32 = egui::Color32::from_rgb(255, 68, 68);     // #ff4444
pub const SOLO_YELLOW: egui::Color32 = egui::Color32::from_rgb(0xE8, 0xC5, 0x30); // #e8c530
pub const LOCK_ORANGE: egui::Color32 = egui::Color32::from_rgb(255, 159, 67); // #ff9f43
pub const FILL_PURPLE: egui::Color32 = egui::Color32::from_rgb(153, 102, 255); // #9966ff
pub const PAT_HAS_DATA: egui::Color32 = egui::Color32::from_rgb(136, 136, 136); // #888
pub const PAT_EMPTY: egui::Color32 = egui::Color32::from_rgb(85, 85, 85);     // #555
pub const PATTERN_BAR_BG: egui::Color32 = egui::Color32::from_rgb(12, 12, 30); // #0c0c1e
pub const PARAM_BAR_BG: egui::Color32 = egui::Color32::from_rgb(12, 12, 30);  // #0c0c1e

/// Get display color for a sample category as egui Color32.
pub fn category_color32(cat: SampleCategory) -> egui::Color32 {
    let c = category_color(cat);
    c.to_egui()
}

// Font
pub const FONT_NAME: &str = "JetBrains Mono";

/// Register JetBrains Mono as the default font.
pub fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();

    fonts.font_data.insert(
        FONT_NAME.to_owned(),
        Arc::new(egui::FontData::from_static(include_bytes!(
            "../../assets/JetBrainsMono-Regular.ttf"
        ))),
    );

    fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default()
        .insert(0, FONT_NAME.to_owned());
    fonts
        .families
        .entry(egui::FontFamily::Monospace)
        .or_default()
        .insert(0, FONT_NAME.to_owned());

    ctx.set_fonts(fonts);
}

/// Configure the egui visual style for Autokit's dark theme.
pub fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let visuals = &mut style.visuals;

    visuals.dark_mode = true;
    visuals.panel_fill = BG_MAIN;
    visuals.window_fill = BG_MAIN;
    visuals.extreme_bg_color = BG_MAIN;

    visuals.widgets.inactive.bg_fill = BG_ROW;
    visuals.widgets.hovered.bg_fill = BG_ROW_HOVER;
    visuals.widgets.active.bg_fill = BG_ROW_HOVER;

    visuals.widgets.inactive.corner_radius = egui::CornerRadius::same(3);
    visuals.widgets.hovered.corner_radius = egui::CornerRadius::same(3);
    visuals.widgets.active.corner_radius = egui::CornerRadius::same(3);

    visuals.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    visuals.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, TEXT_PRIMARY);

    visuals.selection.bg_fill = ACCENT;

    ctx.set_style(style);
}
