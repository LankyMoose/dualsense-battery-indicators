//! Persisted notification preferences.

use crate::app_log;
use crate::app_meta::PKG_NAME;
use crate::color::BatterySpectrum;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prefs {
    #[serde(default = "default_true")]
    pub notify_low: bool,
    #[serde(default = "default_true")]
    pub notify_charged: bool,
    #[serde(default)]
    pub spectrum: BatterySpectrum,
}

fn default_true() -> bool {
    true
}

impl Default for Prefs {
    fn default() -> Self {
        Self {
            notify_low: true,
            notify_charged: true,
            spectrum: BatterySpectrum::DEFAULT,
        }
    }
}

impl Prefs {
    pub fn load() -> Self {
        let path = prefs_path();
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<Prefs>(&bytes) {
            Ok(prefs) => prefs,
            Err(err) => {
                app_log::warn(format!(
                    "failed to parse prefs at {}: {err}; using defaults",
                    path.display()
                ));
                Self::default()
            }
        }
    }

    pub fn save(&self) {
        let path = prefs_path();
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                app_log::warn(format!("failed to create prefs dir: {err}"));
                return;
            }
        }
        match serde_json::to_vec_pretty(self) {
            Ok(bytes) => {
                if let Err(err) = fs::write(&path, bytes) {
                    app_log::warn(format!("failed to write prefs: {err}"));
                }
            }
            Err(err) => app_log::warn(format!("failed to serialize prefs: {err}")),
        }
    }
}

fn prefs_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(PKG_NAME).join("prefs.json");
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(PKG_NAME)
                .join("prefs.json");
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(config) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(config).join(PKG_NAME).join("prefs.json");
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join(".config")
                .join(PKG_NAME)
                .join("prefs.json");
        }
    }

    PathBuf::from(format!("{PKG_NAME}-prefs.json"))
}
