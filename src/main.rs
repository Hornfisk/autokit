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
    pub mod state;
    pub mod theme;
}

mod util {
    pub mod audio_file;
    pub mod history;
}

fn main() {
    nih_export_standalone::<plugin::Autokit>();
}
