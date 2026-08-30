//! Persisted remembered DualSense pads (opt-in via tray Remember toggle).

use crate::app_log;
use crate::app_meta::PKG_NAME;
use crate::battery::ControllerStatus;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownController {
    pub serial: String,
    pub product: String,
    pub connection: String,
    pub percent: u8,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct KnownFile {
    #[serde(default)]
    controllers: Vec<KnownController>,
}

#[derive(Debug, Clone, Default)]
pub struct KnownControllers {
    by_serial: HashMap<String, KnownController>,
    dirty: bool,
}

impl KnownController {
    fn from_status(status: &ControllerStatus) -> Self {
        Self {
            serial: status.serial.clone(),
            product: status.product.to_string(),
            connection: status.connection.to_string(),
            percent: status.percent,
        }
    }

    fn update_from_status(&mut self, status: &ControllerStatus) -> bool {
        let product = status.product.to_string();
        let connection = status.connection.to_string();
        let changed = self.product != product
            || self.connection != connection
            || self.percent != status.percent;
        if changed {
            self.product = product;
            self.connection = connection;
            self.percent = status.percent;
        }
        changed
    }

    pub fn submenu_label_disconnected(&self) -> String {
        format!(
            "{} ({})  {}% — disconnected",
            self.product, self.connection, self.percent
        )
    }
}

impl KnownControllers {
    pub fn load() -> Self {
        let path = store_path();
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<KnownFile>(&bytes) {
            Ok(file) => Self::from_file(file),
            Err(err) => {
                app_log::warn(format!(
                    "failed to parse controllers at {}: {err}; starting empty",
                    path.display()
                ));
                Self::default()
            }
        }
    }

    fn from_file(file: KnownFile) -> Self {
        let mut by_serial = HashMap::new();
        for record in file.controllers {
            if Self::is_storable_serial(&record.serial) {
                by_serial.insert(record.serial.clone(), record);
            }
        }
        Self {
            by_serial,
            dirty: false,
        }
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let path = store_path();
        if let Some(parent) = path.parent() {
            if let Err(err) = fs::create_dir_all(parent) {
                app_log::warn(format!("failed to create controllers dir: {err}"));
                return;
            }
        }

        let mut controllers: Vec<_> = self.by_serial.values().cloned().collect();
        controllers.sort_by(|a, b| a.serial.cmp(&b.serial));
        let file = KnownFile { controllers };

        match serde_json::to_vec_pretty(&file) {
            Ok(bytes) => {
                if let Err(err) = fs::write(&path, bytes) {
                    app_log::warn(format!("failed to write controllers: {err}"));
                    return;
                }
                self.dirty = false;
            }
            Err(err) => app_log::warn(format!("failed to serialize controllers: {err}")),
        }
    }

    pub fn is_remembered(&self, serial: &str) -> bool {
        self.by_serial.contains_key(serial)
    }

    pub fn remember(&mut self, controller: &ControllerStatus) -> bool {
        if !Self::is_storable_serial(&controller.serial) {
            return false;
        }
        match self.by_serial.get_mut(&controller.serial) {
            Some(record) => {
                let changed = record.update_from_status(controller);
                if changed {
                    self.dirty = true;
                }
                changed
            }
            None => {
                self.by_serial.insert(
                    controller.serial.clone(),
                    KnownController::from_status(controller),
                );
                self.dirty = true;
                true
            }
        }
    }

    pub fn forget(&mut self, serial: &str) -> bool {
        if self.by_serial.remove(serial).is_some() {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Refresh last-known fields for remembered pads that are currently connected.
    pub fn sync_from_live(&mut self, live: &[ControllerStatus]) -> bool {
        let mut changed = false;
        for controller in live {
            if !Self::is_storable_serial(&controller.serial) {
                continue;
            }
            if let Some(record) = self.by_serial.get_mut(&controller.serial) {
                if record.update_from_status(controller) {
                    changed = true;
                }
            }
        }
        if changed {
            self.dirty = true;
        }
        changed
    }

    /// Remembered pads not in the live HID list.
    pub fn remembered_disconnected<'a>(
        &'a self,
        live: &'a [ControllerStatus],
    ) -> Vec<&'a KnownController> {
        let live_serials: std::collections::HashSet<&str> =
            live.iter().map(|c| c.serial.as_str()).collect();
        let mut out: Vec<_> = self
            .by_serial
            .values()
            .filter(|r| !live_serials.contains(r.serial.as_str()))
            .collect();
        out.sort_by(|a, b| a.serial.cmp(&b.serial));
        out
    }

    pub fn is_storable_serial(serial: &str) -> bool {
        !serial.is_empty() && serial != "unknown"
    }
}

fn store_path() -> PathBuf {
    prefs_dir().join("controllers.json")
}

fn prefs_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            return PathBuf::from(appdata).join(PKG_NAME);
        }
    }

    #[cfg(target_os = "macos")]
    {
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home)
                .join("Library")
                .join("Application Support")
                .join(PKG_NAME);
        }
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if let Ok(config) = std::env::var("XDG_CONFIG_HOME") {
            return PathBuf::from(config).join(PKG_NAME);
        }
        if let Some(home) = std::env::var_os("HOME") {
            return PathBuf::from(home).join(".config").join(PKG_NAME);
        }
    }

    PathBuf::from(PKG_NAME)
}

pub fn submenu_label_live(controller: &ControllerStatus) -> String {
    if controller.is_low_battery() {
        format!(
            "LOW {}% — {} ({})",
            controller.percent, controller.product, controller.connection
        )
    } else {
        format!(
            "{} ({})  {}%",
            controller.product, controller.connection, controller.percent
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::{ControllerStatus, PowerState};

    fn pad(serial: &str, percent: u8, connection: &'static str) -> ControllerStatus {
        ControllerStatus {
            index: 1,
            product: "DualSense",
            connection,
            serial: serial.to_string(),
            percent,
            state: PowerState::Discharging,
        }
    }

    #[test]
    fn remember_and_forget() {
        let mut store = KnownControllers::default();
        let controller = pad("abc123", 50, "Bluetooth");

        assert!(store.remember(&controller));
        assert!(store.is_remembered("abc123"));
        assert!(store.forget("abc123"));
        assert!(!store.is_remembered("abc123"));
    }

    #[test]
    fn skips_unknown_and_empty_serials() {
        let mut store = KnownControllers::default();
        assert!(!store.remember(&pad("unknown", 50, "USB")));
        assert!(!store.remember(&pad("", 50, "USB")));
        assert!(!store.is_remembered("unknown"));
        assert!(!store.is_remembered(""));
    }

    #[test]
    fn sync_from_live_updates_only_remembered() {
        let mut store = KnownControllers::default();
        store.remember(&pad("remembered", 50, "USB"));
        store.dirty = false;

        let live = vec![pad("remembered", 75, "Bluetooth"), pad("other", 30, "USB")];
        assert!(store.sync_from_live(&live));
        assert_eq!(store.by_serial["remembered"].percent, 75);
        assert_eq!(store.by_serial["remembered"].connection, "Bluetooth");
        assert!(!store.is_remembered("other"));
    }

    #[test]
    fn remembered_disconnected_excludes_live() {
        let mut store = KnownControllers::default();
        store.remember(&pad("live", 50, "USB"));
        store.remember(&pad("gone", 25, "Bluetooth"));

        let live = vec![pad("live", 60, "USB")];
        let disconnected = store.remembered_disconnected(&live);
        assert_eq!(disconnected.len(), 1);
        assert_eq!(disconnected[0].serial, "gone");
    }

    #[test]
    fn disconnected_label_includes_percent() {
        let record = KnownController {
            serial: "abc".into(),
            product: "DualSense".into(),
            connection: "Bluetooth".into(),
            percent: 75,
        };
        assert_eq!(
            record.submenu_label_disconnected(),
            "DualSense (Bluetooth)  75% — disconnected"
        );
    }
}
