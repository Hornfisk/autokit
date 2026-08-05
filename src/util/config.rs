//! Persistent user configuration — sample library path and preferences.
//!
//! Config location (platform-aware):
//!   Linux:   $XDG_CONFIG_HOME/autokit/config.json  (~/.config/autokit/)
//!   macOS:   ~/Library/Application Support/autokit/config.json
//!   Windows: %APPDATA%/autokit/config.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::util::storage;

const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub sample_library_root: String,
    /// Last folder browsed for per-pad sample loading, so the file dialog
    /// re-opens in the same place on next click. `#[serde(default)]` keeps
    /// v1 config files loading without a version bump.
    #[serde(default)]
    pub last_browse_dir: Option<String>,
}

impl Config {
    pub fn new(root: &str) -> Self {
        Config {
            version: CONFIG_VERSION,
            sample_library_root: root.to_owned(),
            last_browse_dir: None,
        }
    }

    /// Load from disk, update `last_browse_dir`, and save. No-op if config
    /// is missing (i.e. user hasn't finished setup) — browse still works,
    /// just won't remember across launches until setup is complete.
    pub fn update_last_browse_dir(dir: &std::path::Path) {
        if let Some(mut cfg) = Self::load() {
            cfg.last_browse_dir = Some(dir.to_string_lossy().into_owned());
            cfg.save();
        }
    }

    /// Load config from disk. Returns None if missing, corrupt, or wrong version.
    pub fn load() -> Option<Self> {
        let path = config_path();
        let text = std::fs::read_to_string(&path).ok()?;
        let cfg: Config = match serde_json::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "config: corrupt or unreadable");
                return None;
            }
        };
        if cfg.version != CONFIG_VERSION {
            tracing::info!("config: version mismatch — ignoring");
            return None;
        }
        tracing::info!(root = %cfg.sample_library_root, "config: loaded");
        Some(cfg)
    }

    /// Save config to disk. Silently logs on failure.
    pub fn save(&self) {
        let path = config_path();
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = storage::write_atomic(&path, &text) {
                    tracing::warn!(error = %e, "config: write failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "config: serialization failed"),
        }
    }
}

/// Platform-aware config file path.
fn config_path() -> PathBuf {
    storage::config_dir("autokit").join("config.json")
}

/// Quick-discover a likely sample root. Returns the first existing path found.
pub fn discover_sample_root() -> Option<PathBuf> {
    let home = storage::home_dir();
    let candidates = [
        home.join("Music").join("Samples"),
        home.join("Music").join("samples"),
        home.join("Samples"),
        home.join("samples"),
        home.join("Music"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}
