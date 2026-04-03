use crate::engine::kit::SampleCategory;

/// Color as [R, G, B] u8 values.
pub struct Color(pub u8, pub u8, pub u8);

impl Color {
    pub fn to_egui(&self) -> [u8; 3] {
        [self.0, self.1, self.2]
    }
}

/// Get the display color for a sample category.
pub fn category_color(cat: SampleCategory) -> Color {
    match cat {
        SampleCategory::Kick => Color(0xff, 0x6b, 0x9d),   // magenta
        SampleCategory::Snare => Color(0x4e, 0xcd, 0xc4),   // cyan
        SampleCategory::Hihat => Color(0xff, 0x9f, 0x43),   // bright orange
        SampleCategory::Clap => Color(0xa8, 0xe6, 0xcf),    // mint
        SampleCategory::Tom => Color(0xff, 0x76, 0x75),      // coral
        SampleCategory::Perc => Color(0xc0, 0x84, 0xfc),     // purple
        SampleCategory::Cymbal => Color(0xff, 0xd1, 0x66),   // gold
        SampleCategory::Bass => Color(0x74, 0xb9, 0xff),     // deep blue
        SampleCategory::Synth => Color(0xfd, 0x79, 0xa8),    // hot pink
        SampleCategory::Other => Color(0x63, 0x6e, 0x72),    // grey
    }
}

// UI constants
pub const BG_COLOR: [u8; 3] = [0x0a, 0x0a, 0x1a];         // near-black
pub const PANEL_BG: [u8; 3] = [0x1a, 0x1a, 0x2e];          // dark navy
pub const MAP_BG: [u8; 3] = [0x08, 0x08, 0x0f];            // deep black
pub const ACCENT: [u8; 3] = [0x00, 0xd4, 0xaa];            // teal
