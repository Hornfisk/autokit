use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use walkdir::WalkDir;
use serde::{Deserialize, Serialize};

use crate::analysis::library::ScanProgress;
use crate::engine::kit::SampleCategory;

const AUDIO_EXTENSIONS: &[&str] = &["wav", "flac", "ogg"];
const MAX_DURATION_SECS: f32 = 4.0;

/// Folder name keywords that hint at a sample category.
const FOLDER_HINTS: &[(&[&str], SampleCategory)] = &[
    (&["kick", "kik", "kck", "bd"], SampleCategory::Kick),
    (&["snare", "snr", "sd"], SampleCategory::Snare),
    (&["hat", "hh", "hihat", "hi-hat", "hi_hat"], SampleCategory::Hihat),
    (&["clap", "clp", "cp"], SampleCategory::Clap),
    (&["tom"], SampleCategory::Tom),
    (&["perc", "percussion", "rimshot", "rim", "clave", "tambourine", "shaker", "conga", "bongo", "woodblock", "triangle", "cowbell"], SampleCategory::Perc),
    (&["cymbal", "crash", "ride"], SampleCategory::Cymbal),
    (&["bass", "808", "sub"], SampleCategory::Bass),
    (&["synth", "stab", "lead", "pad", "key", "chord", "arp"], SampleCategory::Synth),
    (&["vox", "vocal", "voice", "choir", "sing"], SampleCategory::Other),
    (&["fx", "sfx", "effect", "noise", "riser", "sweep", "impact", "transition"], SampleCategory::Other),
];

/// Filename keywords (typically short abbreviations) that hint at a sample category.
/// These are matched as whole "tokens" in the filename (split by non-alphanumeric chars)
/// to avoid false positives (e.g., "bd" inside "kbd" or "abdomen").
const FILENAME_HINTS: &[(&[&str], SampleCategory)] = &[
    (&["bd", "kick", "kik", "kck"], SampleCategory::Kick),
    (&["sd", "snare", "snr"], SampleCategory::Snare),
    (&["hh", "hihat", "hat", "oh", "ch"], SampleCategory::Hihat),
    (&["cp", "clap", "clp"], SampleCategory::Clap),
    (&["tom", "lt", "mt", "ht"], SampleCategory::Tom),
    (&["perc", "rim", "rimshot", "rs", "cb", "cowbell", "clv", "clave",
      "tamb", "shk", "shaker", "conga", "bongo", "triangle", "block"], SampleCategory::Perc),
    (&["cy", "cymbal", "crash", "cr", "ride", "rd"], SampleCategory::Cymbal),
    (&["bass", "808", "sub"], SampleCategory::Bass),
    (&["synth", "stab", "lead", "pad", "key"], SampleCategory::Synth),
    (&["vox", "vocal"], SampleCategory::Other),
    (&["fx", "sfx"], SampleCategory::Other),
];

/// Keywords in filename/path that suggest a loop (not a oneshot).
const LOOP_KEYWORDS: &[&str] = &["loop", "bpm", "_lp_", "_lp.", "groove", "break"];

/// A discovered sample file with metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SampleEntry {
    pub path: PathBuf,
    pub filename: String,
    /// Hint from folder name, if any.
    pub folder_hint: Option<SampleCategory>,
    /// Set after DSP analysis.
    pub category: SampleCategory,
    /// Duration in milliseconds (set after loading).
    pub duration_ms: u32,
    /// Whether DSP analysis considers it percussive.
    pub is_percussive: bool,
}

/// Maximum directory depth the walker will descend. Guards against
/// pathological nesting and accidental scans of `/` or `$HOME`.
const MAX_SCAN_DEPTH: usize = 16;

/// Hard cap on files considered during a scan. Prevents the walker from
/// getting lost inside an enormous library and never returning.
const MAX_SCAN_ENTRIES: usize = 200_000;

/// Recursively scan a folder for audio files, filtering out likely loops.
pub fn scan_folder(root: &Path) -> Vec<SampleEntry> {
    scan_folder_with_progress(root, None)
}

/// Same as [`scan_folder`] but reports walker progress via a shared
/// atomic so the UI can show "walking… N files" during long scans.
pub fn scan_folder_with_progress(
    root: &Path,
    progress: Option<&Arc<ScanProgress>>,
) -> Vec<SampleEntry> {
    let mut entries = Vec::new();
    let mut visited: usize = 0;
    let mut hit_cap = false;

    // `follow_links(false)`: avoids symlink loops and matches how most
    // sample managers behave. `max_depth`: bounded recursion.
    for entry in WalkDir::new(root)
        .follow_links(false)
        .max_depth(MAX_SCAN_DEPTH)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        visited += 1;
        if visited > MAX_SCAN_ENTRIES {
            hit_cap = true;
            break;
        }
        // Surface liveness to the UI every 64 entries so the progress
        // bar doesn't freeze during the walker phase on huge trees.
        if let Some(p) = progress {
            if visited & 0x3F == 0 {
                p.total.store(visited as u32, Ordering::Relaxed);
            }
        }
        if !entry.file_type().is_file() {
            continue;
        }

        let path = entry.path();

        // Filter by audio extension
        let ext = match path.extension().and_then(|e| e.to_str()) {
            Some(e) => e.to_lowercase(),
            None => continue,
        };
        if !AUDIO_EXTENSIONS.contains(&ext.as_str()) {
            continue;
        }

        let filename = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();

        // Skip hidden files and macOS resource forks
        if filename.starts_with('.') || filename.starts_with("._") {
            continue;
        }

        // Skip files that look like loops based on filename
        let lower_name = filename.to_lowercase();
        if LOOP_KEYWORDS.iter().any(|kw| lower_name.contains(kw)) {
            continue;
        }

        // Extract hint from parent directory names, then fall back to filename
        let folder_hint = extract_folder_hint(path)
            .or_else(|| extract_filename_hint(&lower_name));

        entries.push(SampleEntry {
            path: path.to_path_buf(),
            filename,
            folder_hint,
            category: folder_hint.unwrap_or(SampleCategory::Other),
            duration_ms: 0,
            is_percussive: false,
        });
    }

    if hit_cap {
        tracing::warn!(
            root = %root.display(),
            cap = MAX_SCAN_ENTRIES,
            "folder scan hit entry cap — some files were not considered"
        );
    }

    tracing::info!(
        root = %root.display(),
        total = entries.len(),
        visited,
        "folder scan complete"
    );

    entries
}

/// Check parent directory names for category hints.
fn extract_folder_hint(path: &Path) -> Option<SampleCategory> {
    // Check the last 3 path components (excluding filename) for hints
    let components: Vec<&str> = path
        .parent()?
        .components()
        .filter_map(|c| c.as_os_str().to_str())
        .collect();

    // Check from deepest to shallowest (most specific first)
    for component in components.iter().rev().take(3) {
        let lower = component.to_lowercase();
        for (keywords, category) in FOLDER_HINTS {
            for kw in *keywords {
                if lower.contains(kw) {
                    return Some(*category);
                }
            }
        }
    }

    None
}

/// Classify a filename into a `SampleCategory` using only its name (no folder
/// context). Used by the per-pad file browser / drag-drop path for samples
/// outside the scanned library. Case-insensitive.
pub fn guess_category_from_filename(filename: &str) -> Option<SampleCategory> {
    extract_filename_hint(&filename.to_lowercase())
}

/// Check filename for category hints using token-based matching.
/// The filename (without extension) is split on non-alphanumeric boundaries,
/// and each token is compared against known abbreviations.
fn extract_filename_hint(lower_filename: &str) -> Option<SampleCategory> {
    // Strip extension for matching
    let stem = lower_filename.rsplit_once('.').map(|(s, _)| s).unwrap_or(lower_filename);

    // Split into tokens on non-alphanumeric boundaries (e.g., "bd_01" -> ["bd", "01"])
    let tokens: Vec<&str> = stem.split(|c: char| !c.is_alphanumeric()).filter(|t| !t.is_empty()).collect();

    for token in &tokens {
        for (keywords, category) in FILENAME_HINTS {
            if keywords.contains(token) {
                return Some(*category);
            }
        }
    }

    None
}

/// Filter entries by maximum duration (in samples at given sample rate).
pub fn filter_by_duration(entries: &mut Vec<SampleEntry>, sample_rate: f32) {
    let max_samples = (MAX_DURATION_SECS * sample_rate) as u32;
    entries.retain(|e| e.duration_ms == 0 || e.duration_ms <= (max_samples * 1000 / sample_rate as u32));
}
