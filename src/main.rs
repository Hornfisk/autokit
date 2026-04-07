use nih_plug::prelude::*;

mod plugin;
mod logging;

mod engine {
    pub mod echo_detect;
    pub mod kit;
    pub mod master_bus;
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
    pub mod dialogs;
    pub mod editor;
    pub mod folder_browser;
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
    pub mod config;
    pub mod history;
    pub mod preset;
}

fn main() {
    // WASAPI shared mode delivers variable buffer sizes that can exceed nih-plug's
    // default configured size (512), causing a panic in cpal.rs on startup.
    // If --period-size wasn't explicitly supplied, relaunch with 2048 as a safe
    // default that accommodates observed WASAPI delivery sizes (1056–1266 samples).
    #[cfg(target_os = "windows")]
    if !std::env::args().any(|a| a == "--period-size") {
        let exe = std::env::current_exe()
            .expect("could not determine executable path");
        let extra: Vec<String> = std::env::args().skip(1).collect();
        let status = std::process::Command::new(exe)
            .arg("--period-size")
            .arg("2048")
            .args(&extra)
            .status()
            .expect("failed to relaunch with WASAPI buffer workaround");
        std::process::exit(status.code().unwrap_or(1));
    }

    nih_export_standalone::<plugin::Autokit>();
}
