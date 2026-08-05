//! Where Autokit keeps its files, and how it writes them.
//!
//! Two things live here because both were previously duplicated and wrong in
//! one of the copies:
//!
//! 1. **Platform directories.** `config.rs` had correct per-platform logic
//!    while `preset.rs` had a Linux-only `XDG_DATA_HOME`/`HOME` lookup that
//!    fell through to the *relative* path `.local/share` when `HOME` was
//!    unset — which on Windows meant presets were written into whatever
//!    directory the DAW happened to be running from.
//!
//! 2. **Durable writes.** Every save used `std::fs::write`, which truncates
//!    the target before writing. A crash or power loss mid-write left a
//!    truncated JSON file and the user's kit was gone. `save_standalone_state`
//!    runs automatically on a timer, so this was not a rare path.

use std::io;
use std::path::{Path, PathBuf};

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

/// Per-user config directory for `app`.
///
/// - Linux: `$XDG_CONFIG_HOME/<app>` (default `~/.config/<app>`)
/// - macOS: `~/Library/Application Support/<app>`
/// - Windows: `%APPDATA%\<app>`
pub fn config_dir(app: &str) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join(".config"))
            .join(app)
    }
    #[cfg(target_os = "macos")]
    {
        home_dir().join("Library/Application Support").join(app)
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join("AppData").join("Roaming"))
            .join(app)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        home_dir().join(".config").join(app)
    }
}

/// Per-user data directory for `app` — presets, patterns, logs, session state.
///
/// - Linux: `$XDG_DATA_HOME/<app>` (default `~/.local/share/<app>`)
/// - macOS: `~/Library/Application Support/<app>`
/// - Windows: `%APPDATA%\<app>`
pub fn data_dir(app: &str) -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        std::env::var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| home_dir().join(".local").join("share"))
            .join(app)
    }
    #[cfg(not(target_os = "linux"))]
    {
        config_dir(app)
    }
}

/// Write `contents` to `path` so that a crash can never leave the file
/// half-written.
///
/// Writes to a sibling `.tmp` file, fsyncs it, then renames over the target.
/// `rename` within a directory is atomic on every platform Autokit ships to,
/// so a reader sees either the old file or the new one, never a truncated mix.
///
/// Windows caveat: `fs::rename` fails if the destination exists, so the old
/// file is removed first. That leaves a window where the target is missing —
/// still strictly better than the truncate-in-place that `fs::write` does,
/// where the window contains a *corrupt* file that parses as garbage.
pub fn write_atomic(path: &Path, contents: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let tmp = path.with_extension("tmp");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(contents.as_bytes())?;
        // Durability: without this the rename can land before the data does,
        // so a power loss leaves a correctly-named but empty file.
        f.sync_all()?;
    }

    #[cfg(target_os = "windows")]
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }

    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Don't leave debris behind if the rename failed.
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!("autokit-storage-test-{name}"));
        p
    }

    #[test]
    fn write_atomic_creates_the_file_and_its_parent() {
        let dir = temp_path("create-parent");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("nested").join("state.json");

        write_atomic(&path, "{\"a\":1}").expect("write should succeed");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{\"a\":1}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_atomic_replaces_existing_content_completely() {
        let path = temp_path("replace.json");
        write_atomic(&path, "aaaaaaaaaaaaaaaaaaaaaaaaaaa").unwrap();
        // Shorter than the original — a truncating writer that failed halfway
        // would leave trailing bytes of the old content behind.
        write_atomic(&path, "bb").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "bb");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn write_atomic_leaves_no_tmp_file_behind() {
        let path = temp_path("no-debris.json");
        write_atomic(&path, "{}").unwrap();
        assert!(
            !path.with_extension("tmp").exists(),
            "tmp file should be renamed away"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn data_and_config_dirs_end_with_the_app_name() {
        assert!(data_dir("autokit").ends_with("autokit"));
        assert!(config_dir("autokit").ends_with("autokit"));
    }

    #[test]
    fn dirs_are_absolute() {
        // The bug this replaces produced a *relative* `.local/share` path when
        // HOME was unset, which resolved against the DAW's working directory.
        assert!(
            data_dir("autokit").is_absolute(),
            "data dir must be absolute"
        );
        assert!(
            config_dir("autokit").is_absolute(),
            "config dir must be absolute"
        );
    }
}
