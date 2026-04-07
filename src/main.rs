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
    let args = build_args();
    nih_plug::wrapper::standalone::nih_export_standalone_with_args::<plugin::Autokit>(
        args.into_iter(),
    );
}

/// Build the CLI args to pass to nih-plug's standalone runner.
///
/// On Windows, WASAPI shared mode delivers buffers in the audio device's native
/// period (observed: 1056–1266 samples on Windows 11), which exceeds nih-plug's
/// default configured size of 512. The CPAL backend panics on any mismatch,
/// killing the audio thread before playback starts.
///
/// We inject `--period-size 2048` unless the user already passed it explicitly,
/// so the binary works correctly on Windows without wrapper scripts or manual flags.
fn build_args() -> Vec<String> {
    let args: Vec<String> = std::env::args().collect();

    #[cfg(target_os = "windows")]
    if !args.iter().any(|a| a == "--period-size") {
        let mut patched = Vec::with_capacity(args.len() + 2);
        patched.push(args[0].clone());
        patched.push("--period-size".to_string());
        patched.push("2048".to_string());
        patched.extend_from_slice(&args[1..]);
        return patched;
    }

    args
}
