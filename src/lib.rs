use nih_plug::prelude::*;

mod plugin;
mod logging;

mod engine {
    pub mod kit;
    pub mod sampler;
    pub mod sequencer;
}

mod analysis {
    pub mod cache;
    pub mod features;
    pub mod library;
    pub mod scanner;
}

mod ui {
    pub mod editor;
    pub mod knob;
    pub mod pad_row;
    pub mod sample_map;
    pub mod sequencer_ui;
    pub mod state;
    pub mod theme;
    pub mod toolbar;
    pub mod waveform;
}

mod util {
    pub mod audio_file;
    pub mod history;
    pub mod preset;
}

nih_export_vst3!(plugin::Autokit);
nih_export_clap!(plugin::Autokit);
