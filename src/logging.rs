use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Once;

use tracing_subscriber::fmt;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

static INIT: Once = Once::new();

/// Log directory: ~/.local/share/autokit/
fn log_dir() -> PathBuf {
    let base = std::env::var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
            PathBuf::from(home).join(".local/share")
        });
    base.join("autokit")
}

/// Initialize the ring-buffer logger.
/// Writes to ~/.local/share/autokit/autokit.log
/// Safe to call multiple times — only initializes once.
pub fn init() {
    INIT.call_once(|| {
        let dir = log_dir();
        if let Err(e) = fs::create_dir_all(&dir) {
            eprintln!("Autokit: failed to create log dir {:?}: {}", dir, e);
            return;
        }

        let log_path = dir.join("autokit.log");

        // Truncate if over ~10k lines (~500KB)
        if let Ok(meta) = fs::metadata(&log_path) {
            if meta.len() > 500_000 {
                // Keep the last ~5000 lines
                if let Ok(content) = fs::read_to_string(&log_path) {
                    let lines: Vec<&str> = content.lines().collect();
                    let keep = if lines.len() > 5000 {
                        &lines[lines.len() - 5000..]
                    } else {
                        &lines
                    };
                    let _ = fs::write(&log_path, keep.join("\n"));
                }
            }
        }

        let file = match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("Autokit: failed to open log file {:?}: {}", log_path, e);
                return;
            }
        };

        let filter = EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| {
                if cfg!(debug_assertions) {
                    EnvFilter::new("autokit=debug,autokit_standalone=debug")
                } else {
                    EnvFilter::new("autokit=info,autokit_standalone=info")
                }
            });

        let file_layer = fmt::layer()
            .with_writer(move || {
                LineCountWriter::new(file.try_clone().unwrap_or_else(|_| {
                    // Fallback: open /dev/null to avoid panic if clone fails
                    fs::OpenOptions::new().write(true).open("/dev/null")
                        .expect("/dev/null should always be openable")
                }))
            })
            .with_ansi(false)
            .with_target(true)
            .with_thread_ids(true)
            .with_level(true);

        let subscriber = tracing_subscriber::registry()
            .with(filter)
            .with(file_layer);

        let _ = tracing::subscriber::set_global_default(subscriber);
        tracing::info!("Autokit logger initialized — log file: {:?}", log_path);
    });
}

/// Wrapper that just delegates to the inner writer.
/// Can be extended later for line-counting / rotation.
struct LineCountWriter<W: Write> {
    inner: W,
}

impl<W: Write> LineCountWriter<W> {
    fn new(inner: W) -> Self {
        Self { inner }
    }
}

impl<W: Write> Write for LineCountWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
