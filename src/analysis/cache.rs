//! Persistent scan cache — avoids re-analyzing unchanged files on every launch.
//!
//! Cache location: `~/.cache/autokit/library_cache.json`
//!
//! Only metadata and DSP features are cached; audio data is always loaded fresh.

use std::collections::HashMap;
use std::path::Path;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};

use crate::analysis::features::AudioFeatures;
use crate::engine::kit::SampleCategory;

/// Bump this when the cache schema changes in a breaking way.
const CACHE_VERSION: u32 = 3;

/// Cached metadata for a single sample file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Last-modified time (seconds since UNIX epoch).
    pub modified_secs: u64,
    /// File size in bytes.
    pub file_size: u64,
    /// Resolved category after DSP classification.
    pub category: SampleCategory,
    /// Duration in milliseconds.
    pub duration_ms: u32,
    /// Whether the sample is percussive.
    pub is_percussive: bool,
    /// Full DSP feature set.
    pub features: AudioFeatures,
}

/// The full cache file.
#[derive(Debug, Serialize, Deserialize)]
pub struct LibraryCache {
    /// Schema version — if mismatched, cache is discarded.
    pub version: u32,
    /// Root path that was scanned (as a string for serialization).
    pub root: String,
    /// Map from absolute file path (string) to cached entry.
    pub entries: HashMap<String, CacheEntry>,
}

impl LibraryCache {
    /// Create an empty cache for the given root.
    pub fn new(root: &Path) -> Self {
        LibraryCache {
            version: CACHE_VERSION,
            root: root.to_string_lossy().into_owned(),
            entries: HashMap::new(),
        }
    }

    /// Try to load the cache from disk. Returns `None` if missing, corrupt, or wrong version.
    pub fn load(root: &Path) -> Option<Self> {
        let path = cache_file_path()?;

        let text = std::fs::read_to_string(&path).ok()?;
        let cache: LibraryCache = match serde_json::from_str(&text) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(error = %e, "cache: corrupt or unreadable — will do full scan");
                return None;
            }
        };

        if cache.version != CACHE_VERSION {
            tracing::info!(
                cached = cache.version,
                current = CACHE_VERSION,
                "cache: version mismatch — discarding"
            );
            return None;
        }

        if cache.root != root.to_string_lossy().as_ref() {
            tracing::info!("cache: root path changed — discarding");
            return None;
        }

        tracing::info!(
            entries = cache.entries.len(),
            path = %path.display(),
            "cache: loaded"
        );
        Some(cache)
    }

    /// Save the cache to disk. Silently ignores errors (non-fatal).
    pub fn save(&self) {
        let path = match cache_file_path() {
            Some(p) => p,
            None => {
                tracing::warn!("cache: could not determine cache path — not saving");
                return;
            }
        };

        if let Some(parent) = path.parent() {
            if let Err(e) = std::fs::create_dir_all(parent) {
                tracing::warn!(error = %e, "cache: could not create cache dir — not saving");
                return;
            }
        }

        match serde_json::to_string(self) {
            Ok(text) => {
                if let Err(e) = std::fs::write(&path, text) {
                    tracing::warn!(error = %e, "cache: write failed");
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "cache: serialization failed");
            }
        }
    }

    /// Look up a file by path, returning the entry only if mtime+size match.
    pub fn get_if_valid(&self, path: &Path, mtime: u64, size: u64) -> Option<&CacheEntry> {
        let key = path.to_string_lossy();
        let entry = self.entries.get(key.as_ref())?;
        if entry.modified_secs == mtime && entry.file_size == size {
            Some(entry)
        } else {
            None
        }
    }

    /// Insert or update a cache entry.
    pub fn insert(&mut self, path: &Path, entry: CacheEntry) {
        self.entries.insert(path.to_string_lossy().into_owned(), entry);
    }

    /// Remove entries for paths no longer on disk, returning the count removed.
    pub fn retain_existing(&mut self) -> usize {
        let before = self.entries.len();
        self.entries.retain(|path, _| Path::new(path).exists());
        before - self.entries.len()
    }
}

/// Return the path to the cache file, or None if $HOME is not set.
fn cache_file_path() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let mut p = std::path::PathBuf::from(home);
    p.push(".cache");
    p.push("autokit");
    p.push("library_cache.json");
    Some(p)
}

/// Read mtime (seconds since epoch) and file size from a path's metadata.
/// Returns (0, 0) on error.
pub fn file_stamp(path: &Path) -> (u64, u64) {
    match std::fs::metadata(path) {
        Ok(meta) => {
            let mtime = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_secs())
                .unwrap_or(0);
            (mtime, meta.len())
        }
        Err(_) => (0, 0),
    }
}
