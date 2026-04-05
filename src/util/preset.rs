use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::kit::{DrumKit, SampleCategory};
use crate::util::audio_file;

const PRESET_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    pub version: u32,
    pub pads: Vec<PresetPad>,
    #[serde(default)]
    pub patterns: Option<PresetPatterns>,
}

#[derive(Serialize, Deserialize)]
pub struct PresetPatterns {
    pub patterns: Vec<crate::engine::sequencer::Pattern>,
    pub active: usize,
}

#[derive(Serialize, Deserialize)]
pub struct PresetPad {
    pub sample_path: Option<String>,
    pub name: String,
    pub category: SampleCategory,
    pub volume: f32,
    pub pan: f32,
    pub pitch: f32,
    pub decay: f32,
}

/// Returns `~/.local/share/autokit/presets/`, creating it if missing.
pub fn preset_dir() -> PathBuf {
    let base = dirs_next().join("autokit").join("presets");
    if !base.exists() {
        let _ = std::fs::create_dir_all(&base);
    }
    base
}

/// Platform data dir: `$XDG_DATA_HOME` or `~/.local/share`.
fn dirs_next() -> PathBuf {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local").join("share")
    } else {
        PathBuf::from(".local/share")
    }
}

/// Sanitize a preset name for use as a filename.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            _ => c,
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// Save a preset to `{preset_dir}/{name}.json`.
pub fn save_preset(preset: &Preset) -> Result<PathBuf, String> {
    let dir = preset_dir();
    let filename = sanitize_name(&preset.name);
    if filename.is_empty() {
        return Err("Preset name is empty".to_string());
    }
    let path = dir.join(format!("{filename}.json"));
    let json = serde_json::to_string_pretty(preset)
        .map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(&path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// Load a preset from a JSON file.
pub fn load_preset(path: &Path) -> Result<Preset, String> {
    let data = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    serde_json::from_str(&data).map_err(|e| format!("parse {}: {e}", path.display()))
}

/// List all `.json` presets in the preset directory, sorted alphabetically.
pub fn list_presets() -> Vec<(String, PathBuf)> {
    let dir = preset_dir();
    let mut presets = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                presets.push((name, path));
            }
        }
    }

    presets.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    presets
}

/// Create a `Preset` from the current kit state.
pub fn from_kit(name: &str, kit: &DrumKit, pattern_bank: &crate::engine::sequencer::PatternBank) -> Preset {
    let pads = kit
        .pads
        .iter()
        .map(|p| PresetPad {
            sample_path: p.sample_path.clone(),
            name: p.name.clone(),
            category: p.category,
            volume: p.volume,
            pan: p.pan,
            pitch: p.pitch,
            decay: p.decay,
        })
        .collect();

    Preset {
        name: name.to_string(),
        version: PRESET_VERSION,
        pads,
        patterns: Some(PresetPatterns {
            patterns: pattern_bank.patterns.clone(),
            active: pattern_bank.active,
        }),
    }
}

/// Serialize the current kit + pattern state to a JSON string for DAW persistence.
pub fn serialize_state(kit: &DrumKit, pattern_bank: &crate::engine::sequencer::PatternBank) -> Option<String> {
    let p = from_kit("_daw_state", kit, pattern_bank);
    serde_json::to_string(&p).ok()
}

/// Apply a preset to a kit, loading sample audio from disk.
/// Pads with missing or unreadable sample files get `None` sample data.
/// If the preset contains pattern data, it is restored into `pattern_bank`.
pub fn apply_to_kit(preset: &Preset, kit: &mut DrumKit, pattern_bank: &mut crate::engine::sequencer::PatternBank) {
    for (i, pp) in preset.pads.iter().enumerate() {
        if i >= kit.pads.len() {
            break;
        }
        let pad = &mut kit.pads[i];
        pad.name = pp.name.clone();
        pad.category = pp.category;
        pad.volume = pp.volume;
        pad.pan = pp.pan;
        pad.pitch = pp.pitch;
        pad.decay = pp.decay;

        match &pp.sample_path {
            Some(path) if !path.is_empty() => {
                match audio_file::load_wav_mono(path) {
                    Ok(samples) => {
                        pad.sample = Some(Arc::new(samples));
                        pad.sample_path = Some(path.clone());
                    }
                    Err(e) => {
                        tracing::warn!("Preset pad {i}: could not load {path}: {e}");
                        pad.sample = None;
                        pad.sample_path = Some(path.clone());
                    }
                }
            }
            _ => {
                pad.sample = None;
                pad.sample_path = None;
            }
        }
    }

    if let Some(ref pat_data) = preset.patterns {
        for (i, pat) in pat_data.patterns.iter().enumerate() {
            if i < pattern_bank.patterns.len() {
                pattern_bank.patterns[i] = pat.clone();
            }
        }
        pattern_bank.active = pat_data.active.min(pattern_bank.patterns.len().saturating_sub(1));
    }
}
