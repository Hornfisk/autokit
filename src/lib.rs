//! Autokit — a sample-based drum machine plugin with spectral sample analysis.
//!
//! This crate builds two ways from one module tree:
//!
//! - as a `cdylib`, exporting the VST3 and CLAP entry points at the bottom of
//!   this file;
//! - as an `lib` (rlib), which `src/main.rs` links against to produce the
//!   standalone binary.
//!
//! Until 0.5.5 `main.rs` re-declared this entire module tree itself. That
//! compiled the whole crate twice, reported every warning twice, and meant a
//! new module had to be added in two places or the two builds would drift.

use nih_plug::prelude::*;

pub mod logging;
pub mod plugin;

pub mod engine {
    pub mod echo_detect;
    pub mod fx;
    pub mod kit;
    pub mod master_bus;
    pub mod sampler;
    pub mod sequencer;
    pub mod step_smoother;
}

pub mod analysis {
    pub mod cache;
    pub mod features;
    pub mod library;
    pub mod scanner;
}

pub mod ui {
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

pub mod util {
    pub mod audio_file;
    pub mod config;
    pub mod default_kit;
    pub mod history;
    pub mod preset;
    pub mod storage;
}

nih_export_vst3!(plugin::Autokit);
nih_export_clap!(plugin::Autokit);
