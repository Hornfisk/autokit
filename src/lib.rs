use nih_plug::prelude::*;

mod plugin;
mod logging;

mod engine {
    pub mod kit;
    pub mod sampler;
    pub mod sequencer;
}

mod analysis {
    pub mod features;
    pub mod library;
    pub mod scanner;
}

mod ui {
    pub mod theme;
}

mod util {
    pub mod audio_file;
    pub mod history;
}

nih_export_vst3!(plugin::Autokit);
nih_export_clap!(plugin::Autokit);
