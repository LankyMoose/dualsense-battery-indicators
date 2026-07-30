//! Simple append-only file logger (visible under windows_subsystem = "windows").

use crate::app_meta::{DISPLAY_NAME, PKG_NAME, PKG_VERSION};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

const MAX_LOG_BYTES: u64 = 1_000_000;

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

pub fn init() {
    let path = log_file_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    rotate_if_needed(&path);
    if let Ok(mut guard) = LOG_PATH.lock() {
        *guard = Some(path);
    }
    info(format!(
        "{DISPLAY_NAME} ({PKG_NAME}) {PKG_VERSION} starting"
    ));
}

fn log_file_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(PKG_NAME).join("app.log");
        }
    }

    #[cfg(not(windows))]
    {
        if let Ok(state) = std::env::var("XDG_STATE_HOME") {
            return PathBuf::from(state).join(PKG_NAME).join("app.log");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".local")
                .join("state")
                .join(PKG_NAME)
                .join("app.log");
        }
    }

    PathBuf::from(format!("{PKG_NAME}.log"))
}

fn rotate_if_needed(path: &Path) {
    if let Ok(meta) = fs::metadata(path) {
        if meta.len() >= MAX_LOG_BYTES {
            let bak = path.with_extension("log.1");
            let _ = fs::remove_file(&bak);
            let _ = fs::rename(path, bak);
        }
    }
}

fn write_line(level: &str, message: impl AsRef<str>) {
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let line = format!("[{ts}] {level}: {}\n", message.as_ref());

    let path = LOG_PATH.lock().ok().and_then(|g| g.clone());
    if let Some(path) = path {
        if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
            let _ = file.write_all(line.as_bytes());
        }
    }
}

pub fn info(message: impl AsRef<str>) {
    write_line("INFO", message);
}

pub fn warn(message: impl AsRef<str>) {
    write_line("WARN", message);
}

pub fn error(message: impl AsRef<str>) {
    write_line("ERROR", message);
}
