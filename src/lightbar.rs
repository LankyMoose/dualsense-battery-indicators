//! DualSense lightbar HID output (serialized writes).

use crate::app_log;
use crate::battery::{is_dualsense_gamepad, normalize_identity, resolve_device_identity};
use crate::color::{Rgb, color_for_battery_percent};
use hidapi::{BusType, HidApi, HidDevice};
use std::collections::HashSet;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{LazyLock, Mutex};
use std::thread;
use std::time::Duration;

const OUTPUT_REPORT_USB_ID: u8 = 0x02;
const OUTPUT_REPORT_USB_SIZE: usize = 63;
const OUTPUT_REPORT_BT_ID: u8 = 0x31;
const OUTPUT_REPORT_BT_SIZE: usize = 78;
const OUTPUT_REPORT_BT_TAG: u8 = 0x10;
const OUTPUT_CRC32_SEED: u8 = 0xA2;

/// Offsets into the DualSense common output payload (after report ID / BT header).
const OFF_VALID_FLAG1: usize = 1;
const OFF_VALID_FLAG2: usize = 38;
const OFF_LIGHTBAR_SETUP: usize = 41;
const OFF_LIGHTBAR_R: usize = 44;
const OFF_LIGHTBAR_G: usize = 45;
const OFF_LIGHTBAR_B: usize = 46;

const OUTPUT_VALID_FLAG1_LIGHTBAR: u8 = 1 << 2;
const OUTPUT_VALID_FLAG2_LIGHTBAR_SETUP: u8 = 1 << 1;
/// Reconfigure so RGB programming is accepted (Linux hid-playstation Bluetooth init).
/// Must be a **separate** report from the RGB write.
const OUTPUT_LIGHTBAR_SETUP_LIGHT_OUT: u8 = 1 << 1;

pub const IDENTIFY_FLASH_MS: u64 = 150;
pub const IDENTIFY_FLASH_COUNT: u32 = 5;
pub const LOW_BATTERY_PULSE_ON_MS: u64 = 400;
pub const LOW_BATTERY_PULSE_GAP_MS: u64 = 1600;
pub const LOW_BATTERY_ORANGE: Rgb = Rgb::ORANGE;

static LIGHTBAR_LOCK: Mutex<()> = Mutex::new(());
static BT_OUTPUT_SEQ: AtomicU8 = AtomicU8::new(0);
/// Serials that have already received a `LIGHT_OUT` claim this connection.
static CLAIMED_SERIALS: LazyLock<Mutex<HashSet<String>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

fn next_bt_seq_tag() -> u8 {
    let seq = BT_OUTPUT_SEQ.fetch_add(1, Ordering::Relaxed) & 0x0F;
    seq << 4
}

/// Drop claims for pads that are no longer present (Bluetooth reconnect needs a fresh claim).
pub fn sync_lightbar_claims(active_serials: impl IntoIterator<Item = impl AsRef<str>>) {
    let active: HashSet<String> = active_serials
        .into_iter()
        .map(|s| normalize_identity(s.as_ref()))
        .collect();
    if let Ok(mut claimed) = CLAIMED_SERIALS.lock() {
        claimed.retain(|s| active.contains(s));
    }
}

fn take_claim_if_needed(serial: &str) -> bool {
    let Ok(mut claimed) = CLAIMED_SERIALS.lock() else {
        return true;
    };
    claimed.insert(normalize_identity(serial))
}

fn forget_claim(serial: &str) {
    if let Ok(mut claimed) = CLAIMED_SERIALS.lock() {
        claimed.remove(&normalize_identity(serial));
    }
}

/// Apply a lightbar color to the controller with the given serial.
pub fn apply_lightbar_rgb(serial: &str, color: Rgb) -> Result<(), String> {
    let _guard = LIGHTBAR_LOCK
        .lock()
        .map_err(|_| "lightbar lock poisoned".to_string())?;
    apply_lightbar_rgb_unlocked(serial, color)
}

/// Apply lightbar while already holding [`LIGHTBAR_LOCK`] (poll / identify).
pub(crate) fn apply_lightbar_rgb_unlocked(serial: &str, color: Rgb) -> Result<(), String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let target = normalize_identity(serial);

    // Prefer USB when the same pad appears on both buses.
    let mut best: Option<(HidDevice, bool, bool)> = None; // device, is_bluetooth, is_usb
    for info in api.device_list().filter(|d| is_dualsense_gamepad(d)) {
        let device = match info.open_device(&api) {
            Ok(d) => d,
            Err(_) => continue,
        };
        let identity = resolve_device_identity(info, &device);
        if identity != target {
            continue;
        }
        let is_bluetooth = matches!(info.bus_type(), BusType::Bluetooth);
        let is_usb = matches!(info.bus_type(), BusType::Usb);
        let replace = match &best {
            None => true,
            Some((_, _, prev_is_usb)) => is_usb && !*prev_is_usb,
        };
        if replace {
            best = Some((device, is_bluetooth, is_usb));
        }
    }

    let (device, is_bluetooth, _) = best.ok_or_else(|| format!("controller {serial} not found"))?;
    let claim = take_claim_if_needed(&target);
    match set_lightbar_on_device(&device, color, is_bluetooth, claim) {
        Ok(()) => Ok(()),
        Err(err) => {
            if claim {
                forget_claim(&target);
            }
            Err(err.to_string())
        }
    }
}

/// Write lightbar while already holding [`LIGHTBAR_LOCK`] (used during poll).
///
/// Bluetooth DualSense ignores RGB until the lightbar is reconfigured with a
/// dedicated `LIGHT_OUT` setup report (same as Linux `hid-playstation`). Color is
/// then applied in a second report with only the lightbar RGB flag.
pub fn set_lightbar_on_device(
    device: &HidDevice,
    color: Rgb,
    is_bluetooth: bool,
    claim: bool,
) -> Result<(), hidapi::HidError> {
    if claim {
        write_lightbar_claim(device, is_bluetooth)?;
    }
    write_lightbar_rgb(device, is_bluetooth, color)
}

fn write_lightbar_claim(device: &HidDevice, is_bluetooth: bool) -> Result<(), hidapi::HidError> {
    write_output_report(device, is_bluetooth, |common| {
        common[OFF_VALID_FLAG2] = OUTPUT_VALID_FLAG2_LIGHTBAR_SETUP;
        common[OFF_LIGHTBAR_SETUP] = OUTPUT_LIGHTBAR_SETUP_LIGHT_OUT;
    })
}

fn write_lightbar_rgb(
    device: &HidDevice,
    is_bluetooth: bool,
    color: Rgb,
) -> Result<(), hidapi::HidError> {
    write_output_report(device, is_bluetooth, |common| {
        common[OFF_VALID_FLAG1] = OUTPUT_VALID_FLAG1_LIGHTBAR;
        common[OFF_LIGHTBAR_R] = color.r;
        common[OFF_LIGHTBAR_G] = color.g;
        common[OFF_LIGHTBAR_B] = color.b;
    })
}

fn write_output_report(
    device: &HidDevice,
    is_bluetooth: bool,
    fill_common: impl FnOnce(&mut [u8]),
) -> Result<(), hidapi::HidError> {
    if is_bluetooth {
        device.write(&build_bt_report(fill_common))?;
    } else {
        let mut report = [0u8; OUTPUT_REPORT_USB_SIZE];
        report[0] = OUTPUT_REPORT_USB_ID;
        fill_common(&mut report[1..]);
        device.write(&report)?;
    }
    Ok(())
}

fn build_bt_report(fill_common: impl FnOnce(&mut [u8])) -> [u8; OUTPUT_REPORT_BT_SIZE] {
    let mut report = [0u8; OUTPUT_REPORT_BT_SIZE];
    report[0] = OUTPUT_REPORT_BT_ID;
    report[1] = next_bt_seq_tag();
    report[2] = OUTPUT_REPORT_BT_TAG;
    fill_common(&mut report[3..]);

    let crc = {
        let mut data = Vec::with_capacity(OUTPUT_REPORT_BT_SIZE - 3);
        data.push(OUTPUT_CRC32_SEED);
        data.extend_from_slice(&report[..OUTPUT_REPORT_BT_SIZE - 4]);
        crc32fast::hash(&data)
    };
    report[OUTPUT_REPORT_BT_SIZE - 4..].copy_from_slice(&crc.to_le_bytes());
    report
}

/// Hold the lightbar lock for a closure (poll path).
pub fn with_lightbar_lock<T>(f: impl FnOnce() -> T) -> T {
    let _guard = LIGHTBAR_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    f()
}

/// Flash white, then restore battery color, five times (lock held for the sequence).
pub fn identify_controller(serial: &str, percent: u8) -> Result<(), String> {
    let normal = color_for_battery_percent(percent);
    let flash_for = Duration::from_millis(IDENTIFY_FLASH_MS);
    let _guard = LIGHTBAR_LOCK
        .lock()
        .map_err(|_| "lightbar lock poisoned".to_string())?;

    for _ in 0..IDENTIFY_FLASH_COUNT {
        apply_lightbar_rgb_unlocked(serial, Rgb::WHITE)?;
        thread::sleep(flash_for);
        apply_lightbar_rgb_unlocked(serial, normal)?;
        thread::sleep(flash_for);
    }

    Ok(())
}

/// Apply a color to every connected DualSense (CLI / debug). Always re-claims.
pub fn apply_lightbar_all(color: Rgb) -> Result<usize, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let mut applied = 0usize;
    let _guard = LIGHTBAR_LOCK
        .lock()
        .map_err(|_| "lightbar lock poisoned".to_string())?;

    if let Ok(mut claimed) = CLAIMED_SERIALS.lock() {
        claimed.clear();
    }

    for info in api.device_list().filter(|d| is_dualsense_gamepad(d)) {
        let device = match info.open_device(&api) {
            Ok(d) => d,
            Err(err) => {
                app_log::warn(format!("lightbar open failed: {err}"));
                continue;
            }
        };
        let serial = resolve_device_identity(info, &device);
        let is_bluetooth = matches!(info.bus_type(), BusType::Bluetooth);
        let claim = take_claim_if_needed(&serial);
        match set_lightbar_on_device(&device, color, is_bluetooth, claim) {
            Ok(()) => applied += 1,
            Err(err) => {
                if claim {
                    forget_claim(&serial);
                }
                app_log::warn(format!("lightbar write failed: {err}"));
            }
        }
    }

    Ok(applied)
}

pub fn warn_lightbar(product: &str, err: impl std::fmt::Display) {
    app_log::warn(format!("failed to set lightbar on {product}: {err}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt_claim_report_has_setup_without_rgb() {
        let report = build_bt_report(|common| {
            common[OFF_VALID_FLAG2] = OUTPUT_VALID_FLAG2_LIGHTBAR_SETUP;
            common[OFF_LIGHTBAR_SETUP] = OUTPUT_LIGHTBAR_SETUP_LIGHT_OUT;
        });
        assert_eq!(report.len(), 78);
        assert_eq!(report[0], OUTPUT_REPORT_BT_ID);
        assert_eq!(report[2], OUTPUT_REPORT_BT_TAG);
        assert_eq!(report[3 + OFF_VALID_FLAG1], 0);
        assert_eq!(
            report[3 + OFF_VALID_FLAG2],
            OUTPUT_VALID_FLAG2_LIGHTBAR_SETUP
        );
        assert_eq!(
            report[3 + OFF_LIGHTBAR_SETUP],
            OUTPUT_LIGHTBAR_SETUP_LIGHT_OUT
        );
        assert_eq!(report[3 + OFF_LIGHTBAR_R], 0);
        assert!(report[74..78].iter().any(|&b| b != 0));
    }

    #[test]
    fn bt_rgb_report_sets_color_without_setup() {
        let report = build_bt_report(|common| {
            common[OFF_VALID_FLAG1] = OUTPUT_VALID_FLAG1_LIGHTBAR;
            common[OFF_LIGHTBAR_R] = 255;
            common[OFF_LIGHTBAR_G] = 100;
            common[OFF_LIGHTBAR_B] = 0;
        });
        assert_eq!(report[3 + OFF_VALID_FLAG1], OUTPUT_VALID_FLAG1_LIGHTBAR);
        assert_eq!(report[3 + OFF_VALID_FLAG2], 0);
        assert_eq!(report[3 + OFF_LIGHTBAR_SETUP], 0);
        assert_eq!(report[3 + OFF_LIGHTBAR_R], 255);
        assert_eq!(report[3 + OFF_LIGHTBAR_G], 100);
        assert_eq!(report[3 + OFF_LIGHTBAR_B], 0);
        assert!(report[74..78].iter().any(|&b| b != 0));
    }

    #[test]
    fn claim_tracking_inserts_once_then_sync_clears() {
        sync_lightbar_claims(std::iter::empty::<&str>());
        assert!(take_claim_if_needed("aabbcc"));
        assert!(!take_claim_if_needed("aabbcc"));
        assert!(take_claim_if_needed("ddeeff"));
        sync_lightbar_claims(["ddeeff"]);
        assert!(!take_claim_if_needed("ddeeff"));
        assert!(take_claim_if_needed("aabbcc"));
        sync_lightbar_claims(std::iter::empty::<&str>());
    }
}
