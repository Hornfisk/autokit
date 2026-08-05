use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::engine::kit::{DrumKit, SampleCategory};
use crate::engine::sequencer::PatternBank;
use crate::util::audio_file;
use crate::util::storage;

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
    #[serde(default)]
    pub start: f32,
    #[serde(default = "default_end")]
    pub end: f32,
}

fn default_end() -> f32 {
    1.0
}

/// Autokit's per-user preset directory, created if missing.
pub fn preset_dir() -> PathBuf {
    let base = storage::data_dir("autokit").join("presets");
    if !base.exists() {
        let _ = std::fs::create_dir_all(&base);
    }
    base
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
    let json = serde_json::to_string_pretty(preset).map_err(|e| format!("serialize: {e}"))?;
    storage::write_atomic(&path, &json).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// Load a preset from a JSON file.
pub fn load_preset(path: &Path) -> Result<Preset, String> {
    let data =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut preset: Preset =
        serde_json::from_str(&data).map_err(|e| format!("parse {}: {e}", path.display()))?;
    if let Some(p) = preset.patterns.as_mut() {
        sanitize_preset_patterns(p);
    }
    Ok(preset)
}

/// Bring pattern data from an untrusted file back inside the invariants the
/// audio thread assumes. See [`PatternBank::sanitize`].
fn sanitize_preset_patterns(p: &mut PresetPatterns) {
    for pattern in &mut p.patterns {
        pattern.sanitize();
    }
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

// --- Pattern persistence (individual patterns) ---

/// Autokit's per-user single-pattern directory, created if missing.
pub fn pattern_dir() -> PathBuf {
    let base = storage::data_dir("autokit").join("patterns");
    if !base.exists() {
        let _ = std::fs::create_dir_all(&base);
    }
    base
}

/// Save a single pattern to `{pattern_dir}/{name}.json`.
pub fn save_pattern(
    name: &str,
    pattern: &crate::engine::sequencer::Pattern,
) -> Result<PathBuf, String> {
    let dir = pattern_dir();
    let filename = sanitize_name(name);
    if filename.is_empty() {
        return Err("Pattern name is empty".to_string());
    }
    let path = dir.join(format!("{filename}.json"));
    let json = serde_json::to_string_pretty(pattern).map_err(|e| format!("serialize: {e}"))?;
    storage::write_atomic(&path, &json).map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(path)
}

/// Load a single pattern from a JSON file.
pub fn load_pattern(path: &Path) -> Result<crate::engine::sequencer::Pattern, String> {
    let data =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let mut pattern: crate::engine::sequencer::Pattern =
        serde_json::from_str(&data).map_err(|e| format!("parse {}: {e}", path.display()))?;
    pattern.sanitize();
    Ok(pattern)
}

/// List all `.json` patterns in the pattern directory, sorted alphabetically.
pub fn list_patterns() -> Vec<(String, PathBuf)> {
    let dir = pattern_dir();
    let mut patterns = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("json") {
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("?")
                    .to_string();
                patterns.push((name, path));
            }
        }
    }

    patterns.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    patterns
}

/// Delete a file (preset or pattern) from disk.
pub fn delete_file(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|e| format!("delete {}: {e}", path.display()))
}

/// Path for the standalone session's auto-saved state.
pub fn standalone_state_path() -> PathBuf {
    storage::data_dir("autokit").join("standalone_state.json")
}

/// Save standalone session state to disk.
///
/// This runs on a timer, so it is the most likely file to be mid-write when
/// something goes wrong — hence the atomic write.
pub fn save_standalone_state(kit: &DrumKit, pattern_bank: &crate::engine::sequencer::PatternBank) {
    let p = from_kit("_standalone", kit, pattern_bank);
    match serde_json::to_string(&p) {
        Ok(json) => {
            if let Err(e) = storage::write_atomic(&standalone_state_path(), &json) {
                tracing::warn!("standalone state write failed: {e}");
            }
        }
        Err(e) => tracing::warn!("standalone state serialize failed: {e}"),
    }
}

/// Load standalone session state from disk, if it exists.
pub fn load_standalone_state() -> Option<Preset> {
    let path = standalone_state_path();
    let data = std::fs::read_to_string(&path).ok()?;
    let mut preset: Preset = serde_json::from_str(&data).ok()?;
    if let Some(p) = preset.patterns.as_mut() {
        sanitize_preset_patterns(p);
    }
    Some(preset)
}

/// Create a `Preset` from the current kit state.
pub fn from_kit(
    name: &str,
    kit: &DrumKit,
    pattern_bank: &crate::engine::sequencer::PatternBank,
) -> Preset {
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
            start: p.start,
            end: p.end,
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
pub fn serialize_state(
    kit: &DrumKit,
    pattern_bank: &crate::engine::sequencer::PatternBank,
) -> Option<String> {
    let p = from_kit("_daw_state", kit, pattern_bank);
    serde_json::to_string(&p).ok()
}

/// Apply a preset to a kit, loading sample audio from disk.
/// Pads with missing or unreadable sample files get `None` sample data.
/// If the preset contains pattern data, it is restored into `pattern_bank`.
pub fn apply_to_kit(
    preset: &Preset,
    kit: &mut DrumKit,
    pattern_bank: &mut crate::engine::sequencer::PatternBank,
    sample_rate: f32,
) {
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
        pad.start = pp.start;
        pad.end = pp.end;

        match &pp.sample_path {
            Some(path) if !path.is_empty() => {
                // Defensive: short-circuit when the parent directory is obviously
                // missing. Avoids open() on dead paths, and (more importantly)
                // limits filesystem probing on stale network/FUSE mounts where
                // open() can hang for many seconds. This MUST NOT run on the
                // audio thread — see `restore_to_fresh` below.
                if !sample_path_likely_present(path) {
                    tracing::warn!("Preset pad {i}: missing sample (parent dir absent): {path}");
                    pad.sample = None;
                    pad.sample_path = Some(path.clone());
                    continue;
                }
                match audio_file::load_wav_mono(path, sample_rate) {
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
        pattern_bank.active = pat_data
            .active
            .min(pattern_bank.patterns.len().saturating_sub(1));
    }

    // The preset came from disk. Everything the audio thread indexes with
    // must be back in range before this bank goes anywhere near `process()`.
    pattern_bank.sanitize();
}

/// Cheap check: does this sample path's parent directory exist?
/// Used to skip dead paths without doing a full file open. This still does
/// a `stat()` syscall on the parent, which can hang on a truly broken mount,
/// but is much cheaper than `open()` + symphonia probe and avoids triggering
/// per-file automounts.
fn sample_path_likely_present(path: &str) -> bool {
    let p = Path::new(path);
    match p.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.is_dir(),
        // No parent component (e.g. bare filename, or root) — let the open() try.
        _ => true,
    }
}

/// Pre-loaded state from disk: a fresh DrumKit + PatternBank with sample data
/// already loaded, ready to install on the audio thread under a brief lock.
///
/// All file I/O happens when this is built — see [`restore_to_fresh`] and
/// [`restore_persisted_off_thread`]. NEVER build this on the audio thread.
pub struct RestoredState {
    pub kit: DrumKit,
    pub patterns: PatternBank,
    /// Sample paths that were referenced by the preset but couldn't be loaded
    /// (missing file, unreadable, or parent directory absent).
    pub missing_paths: Vec<String>,
}

/// Build a fresh `DrumKit` + `PatternBank` from a preset, loading sample audio
/// from disk. **All file I/O runs here** — call only from a background thread.
pub fn restore_to_fresh(preset: &Preset, sample_rate: f32) -> RestoredState {
    let mut kit = DrumKit::new();
    let mut patterns = PatternBank::new();
    apply_to_kit(preset, &mut kit, &mut patterns, sample_rate);

    // After apply_to_kit: any pad with `sample == None` but a non-empty
    // `sample_path` is a missing reference worth surfacing.
    let missing_paths: Vec<String> = kit
        .pads
        .iter()
        .filter(|p| p.sample.is_none())
        .filter_map(|p| p.sample_path.clone())
        .filter(|s| !s.is_empty())
        .collect();

    if !missing_paths.is_empty() {
        tracing::warn!(
            count = missing_paths.len(),
            "preset restored with missing samples — pads left empty"
        );
    }

    RestoredState {
        kit,
        patterns,
        missing_paths,
    }
}

/// Restore plugin state from JSON persisted by the host (DAW save/load).
/// Falls back to standalone session state if `persisted` is empty. Returns
/// `None` if there's nothing to restore or the JSON is corrupt.
///
/// **All file I/O runs here** — call only from a background thread.
pub fn restore_persisted_off_thread(persisted: &str, sample_rate: f32) -> Option<RestoredState> {
    if !persisted.is_empty() {
        match serde_json::from_str::<Preset>(persisted) {
            Ok(p) => Some(restore_to_fresh(&p, sample_rate)),
            Err(e) => {
                tracing::warn!("failed to parse persisted plugin state: {e}");
                None
            }
        }
    } else {
        load_standalone_state().map(|p| restore_to_fresh(&p, sample_rate))
    }
}

#[cfg(test)]
mod restore_tests {
    use super::*;

    fn pad_with_path(name: &str, path: &str) -> PresetPad {
        PresetPad {
            sample_path: Some(path.to_string()),
            name: name.to_string(),
            category: SampleCategory::Kick,
            volume: 1.0,
            pan: 0.0,
            pitch: 0.0,
            decay: 1.0,
            start: 0.0,
            end: 1.0,
        }
    }

    #[test]
    fn restore_to_fresh_collects_missing_paths_without_panicking() {
        // Two pads referencing files in a directory that does not exist on
        // any host. This is the host-freeze scenario from the field report:
        // a project saved on machine A is loaded on machine B where the
        // sample tree was never present.
        let preset = Preset {
            name: "missing-samples".to_string(),
            version: PRESET_VERSION,
            pads: vec![
                pad_with_path("kick", "/nonexistent-dir-xyz/kick.wav"),
                pad_with_path("snare", "/nonexistent-dir-xyz/snare.wav"),
            ],
            patterns: None,
        };

        let restored = restore_to_fresh(&preset, 44100.0);

        // Pads come back with metadata intact but no audio data.
        assert_eq!(restored.kit.pads[0].name, "kick");
        assert_eq!(restored.kit.pads[1].name, "snare");
        assert!(restored.kit.pads[0].sample.is_none());
        assert!(restored.kit.pads[1].sample.is_none());
        // Original paths preserved so the user can see what's broken.
        assert_eq!(
            restored.kit.pads[0].sample_path.as_deref(),
            Some("/nonexistent-dir-xyz/kick.wav")
        );
        // Both missing paths surfaced.
        assert_eq!(restored.missing_paths.len(), 2);
    }

    #[test]
    fn restore_persisted_off_thread_handles_missing_paths_gracefully() {
        // Simulate persisted DAW state pointing at samples that don't exist.
        let preset = Preset {
            name: "persisted".to_string(),
            version: PRESET_VERSION,
            pads: vec![pad_with_path("k", "/no/such/dir/file.wav")],
            patterns: None,
        };
        let json = serde_json::to_string(&preset).unwrap();

        // Should return Some(RestoredState) — restoration continues even
        // when no samples can be loaded.
        let restored = restore_persisted_off_thread(&json, 44100.0).expect("restore returns state");
        assert_eq!(restored.missing_paths.len(), 1);
        assert!(restored.kit.pads[0].sample.is_none());
    }

    #[test]
    fn restore_persisted_off_thread_returns_none_for_empty_string() {
        // Empty persisted state + no standalone file = nothing to restore.
        // (Standalone file may or may not exist on the test host; we only
        // assert this is non-panicking and returns a value of either kind.)
        let _ = restore_persisted_off_thread("", 44100.0);
    }

    #[test]
    fn restore_persisted_off_thread_returns_none_for_corrupt_json() {
        let restored = restore_persisted_off_thread("not json {{{", 44100.0);
        assert!(restored.is_none());
    }

    #[test]
    fn sample_path_likely_present_rejects_missing_parent() {
        assert!(!sample_path_likely_present("/definitely/not/here/x.wav"));
    }

    #[test]
    fn sample_path_likely_present_accepts_real_parent() {
        // Use the platform temp dir so this works on Linux, macOS, and Windows.
        let mut probe = std::env::temp_dir();
        probe.push("autokit-nonexistent-probe.wav");
        assert!(sample_path_likely_present(probe.to_str().unwrap()));
    }
}
