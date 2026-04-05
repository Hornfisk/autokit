//! Persistent user configuration — sample library path and preferences.
//!
//! Config location (platform-aware):
//!   Linux:   $XDG_CONFIG_HOME/autokit/config.json  (~/.config/autokit/)
//!   macOS:   ~/Library/Application Support/autokit/config.json
//!   Windows: %APPDATA%/autokit/config.json

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const CONFIG_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub version: u32,
    pub sample_library_root: String,
}

impl Config {
    pub fn new(root: &str) -> Self {
        Config {
            version: CONFIG_VERSION,
            sample_library_root: root.to_owned(),
        }
    }

    /// Load config from disk. Returns None if missing, corrupt, or wrong version.
    pub fn load() -> Option<Self> {
        let path = config_path()?;
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
        let path = match config_path() {
            Some(p) => p,
            None => {
                tracing::warn!("config: could not determine config path");
                return;
            }
        };
        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(error = %e, "config: could not create config dir");
                return;
            }
        }
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    tracing::warn!(error = %e, "config: write failed");
                }
            }
            Err(e) => tracing::warn!(error = %e, "config: serialization failed"),
        }
    }
}

/// Platform-aware config file path.
fn config_path() -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
                PathBuf::from(home).join(".config")
            });
        Some(base.join("autokit").join("config.json"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var("HOME").ok()?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support/autokit/config.json"),
        )
    }
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var("APPDATA").ok()?;
        Some(PathBuf::from(appdata).join("autokit").join("config.json"))
    }
}

/// Platform-aware default sample library root.
pub fn default_sample_root() -> PathBuf {
    let home = home_dir();
    #[cfg(target_os = "windows")]
    {
        home.join("Music").join("Samples")
    }
    #[cfg(not(target_os = "windows"))]
    {
        home.join("Music").join("Samples")
    }
}

/// Quick-discover a likely sample root. Returns the first existing path found.
pub fn discover_sample_root() -> Option<PathBuf> {
    let home = home_dir();
    let candidates = [
        home.join("Music").join("Samples"),
        home.join("Music").join("samples"),
        home.join("Samples"),
        home.join("samples"),
        home.join("Music"),
    ];
    candidates.into_iter().find(|p| p.is_dir())
}

/// Home directory for the current user.
pub fn home_dir() -> PathBuf {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("C:\\"))
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"))
    }
}
