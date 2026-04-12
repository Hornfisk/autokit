use nih_plug::prelude::*;

mod plugin;
mod logging;

mod engine {
    pub mod echo_detect;
    pub mod fx;
    pub mod kit;
    pub mod master_bus;
    pub mod sampler;
    pub mod sequencer;
    pub mod step_smoother;
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
    pub mod default_kit;
    pub mod history;
    pub mod preset;
}

fn main() {
    install_panic_logger();
    let args = build_args();
    nih_plug::wrapper::standalone::nih_export_standalone_with_args::<plugin::Autokit, _>(args);
}

/// Chain a tracing logger in front of the default panic hook so that a
/// panic on the JACK audio thread leaves a breadcrumb in
/// `~/.local/share/autokit/autokit.log` instead of vanishing silently.
/// nih-plug's JACK backend does not wrap the process callback in
/// `catch_unwind`, so without this any panic there kills the callback
/// thread and leaves PipeWire holding a zombie client — recovery
/// requires restarting the audio server.
fn install_panic_logger() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(|s| s.as_str()))
            .unwrap_or("<non-string panic>");
        tracing::error!(location = %location, payload = %payload, "panic caught by hook");
        default(info);
    }));
}

/// Build the CLI args to pass to nih-plug's standalone runner.
///
/// Two things happen here:
///
/// 1. On Windows, WASAPI shared mode delivers buffers in the audio device's
///    native period (observed: 1056–1266 samples on Windows 11), which
///    exceeds nih-plug's default of 512. CPAL panics on any mismatch, so we
///    inject `--period-size 2048` unless the user already passed it.
///
/// 2. nih-plug's standalone wrapper never auto-picks a MIDI input port —
///    without `--midi-input <name>` the plugin receives no MIDI at all. We
///    enumerate available input ports and inject the first reasonable one
///    (skipping ALSA's "Midi Through" stub on Linux) so external controllers
///    work out of the box. Pass `--midi-input` explicitly to override.
fn build_args() -> Vec<String> {
    let mut args: Vec<String> = std::env::args().collect();

    #[cfg(target_os = "windows")]
    if !args.iter().any(|a| a == "--period-size") {
        let mut patched = Vec::with_capacity(args.len() + 2);
        patched.push(args[0].clone());
        patched.push("--period-size".to_string());
        patched.push("2048".to_string());
        patched.extend_from_slice(&args[1..]);
        args = patched;
    }

    if !args.iter().any(|a| a == "--midi-input") {
        if let Some(port_name) = pick_default_midi_input() {
            eprintln!("autokit: auto-selecting MIDI input '{port_name}' (pass --midi-input to override)");
            args.push("--midi-input".to_string());
            args.push(port_name);
        } else {
            eprintln!("autokit: no MIDI input ports detected; external MIDI disabled");
        }
    }

    args
}

/// Enumerate MIDI input ports via midir and return the first usable one.
/// Skips ALSA's "Midi Through" virtual stub on Linux since it never carries
/// device input.
fn pick_default_midi_input() -> Option<String> {
    let input = midir::MidiInput::new("autokit-port-scan").ok()?;
    let ports = input.ports();
    for port in &ports {
        let name = input.port_name(port).ok()?;
        if name.to_ascii_lowercase().contains("midi through") {
            continue;
        }
        return Some(name);
    }
    // Fall back to the first port even if it's "Midi Through" — better than nothing.
    ports.first().and_then(|p| input.port_name(p).ok())
}
