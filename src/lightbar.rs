//! DualSense lightbar HID output (serialized writes).

use crate::app_log;
use crate::battery::is_dualsense_gamepad;
use crate::color::{color_for_battery_percent, Rgb};
use hidapi::{BusType, HidApi, HidDevice};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const OUTPUT_REPORT_USB_ID: u8 = 0x02;
const OUTPUT_REPORT_USB_SIZE: usize = 63;
const OUTPUT_REPORT_BT_ID: u8 = 0x31;
const OUTPUT_REPORT_BT_SIZE: usize = 78;
const OUTPUT_REPORT_BT_TAG: u8 = 0x10;
const OUTPUT_CRC32_SEED: u8 = 0xA2;
const OUTPUT_VALID_FLAG1_LIGHTBAR: u8 = 1 << 2;

pub const IDENTIFY_FLASH_MS: u64 = 150;
pub const IDENTIFY_FLASH_COUNT: u32 = 5;
pub const LOW_BATTERY_PULSE_ON_MS: u64 = 400;
pub const LOW_BATTERY_PULSE_GAP_MS: u64 = 1600;
pub const LOW_BATTERY_ORANGE: Rgb = Rgb::ORANGE;

static LIGHTBAR_LOCK: Mutex<()> = Mutex::new(());
static BT_OUTPUT_SEQ: AtomicU8 = AtomicU8::new(0);

fn next_bt_seq_tag() -> u8 {
    let seq = BT_OUTPUT_SEQ.fetch_add(1, Ordering::Relaxed) & 0x0F;
    seq << 4
}

/// Apply a lightbar color to the controller with the given serial.
pub fn apply_lightbar_rgb(serial: &str, color: Rgb) -> Result<(), String> {
    let _guard = LIGHTBAR_LOCK
        .lock()
        .map_err(|_| "lightbar lock poisoned".to_string())?;
    apply_lightbar_rgb_unlocked(serial, color)
}

fn apply_lightbar_rgb_unlocked(serial: &str, color: Rgb) -> Result<(), String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let info = api
        .device_list()
        .find(|d| {
            is_dualsense_gamepad(d)
                && d.serial_number()
                    .filter(|s| !s.is_empty())
                    .unwrap_or("unknown")
                    == serial
        })
        .ok_or_else(|| format!("controller {serial} not found"))?;

    let device = info.open_device(&api).map_err(|e| e.to_string())?;
    let is_bluetooth = matches!(
        device
            .get_device_info()
            .map_err(|e| e.to_string())?
            .bus_type(),
        BusType::Bluetooth
    );
    set_lightbar_on_device(&device, color, is_bluetooth).map_err(|e| e.to_string())
}

/// Write lightbar while already holding [`LIGHTBAR_LOCK`] (used during poll).
pub fn set_lightbar_on_device(
    device: &HidDevice,
    color: Rgb,
    is_bluetooth: bool,
) -> Result<(), hidapi::HidError> {
    if is_bluetooth {
        let mut report = [0u8; OUTPUT_REPORT_BT_SIZE];
        report[0] = OUTPUT_REPORT_BT_ID;
        report[1] = next_bt_seq_tag();
        report[2] = OUTPUT_REPORT_BT_TAG;
        report[3 + 1] = OUTPUT_VALID_FLAG1_LIGHTBAR;
        report[3 + 44] = color.r;
        report[3 + 45] = color.g;
        report[3 + 46] = color.b;

        let crc = {
            let mut data = Vec::with_capacity(OUTPUT_REPORT_BT_SIZE - 3);
            data.push(OUTPUT_CRC32_SEED);
            data.extend_from_slice(&report[..OUTPUT_REPORT_BT_SIZE - 4]);
            crc32fast::hash(&data)
        };
        report[OUTPUT_REPORT_BT_SIZE - 4..].copy_from_slice(&crc.to_le_bytes());
        device.write(&report)?;
    } else {
        let mut report = [0u8; OUTPUT_REPORT_USB_SIZE];
        report[0] = OUTPUT_REPORT_USB_ID;
        report[1 + 1] = OUTPUT_VALID_FLAG1_LIGHTBAR;
        report[1 + 44] = color.r;
        report[1 + 45] = color.g;
        report[1 + 46] = color.b;
        device.write(&report)?;
    }

    Ok(())
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

#[cfg(test)]
pub fn build_bt_output_report_for_test(color: Rgb) -> [u8; OUTPUT_REPORT_BT_SIZE] {
    let mut report = [0u8; OUTPUT_REPORT_BT_SIZE];
    report[0] = OUTPUT_REPORT_BT_ID;
    report[1] = next_bt_seq_tag();
    report[2] = OUTPUT_REPORT_BT_TAG;
    report[3 + 1] = OUTPUT_VALID_FLAG1_LIGHTBAR;
    report[3 + 44] = color.r;
    report[3 + 45] = color.g;
    report[3 + 46] = color.b;
    let crc = {
        let mut data = Vec::with_capacity(OUTPUT_REPORT_BT_SIZE - 3);
        data.push(OUTPUT_CRC32_SEED);
        data.extend_from_slice(&report[..OUTPUT_REPORT_BT_SIZE - 4]);
        crc32fast::hash(&data)
    };
    report[OUTPUT_REPORT_BT_SIZE - 4..].copy_from_slice(&crc.to_le_bytes());
    report
}

pub fn warn_lightbar(product: &str, err: impl std::fmt::Display) {
    app_log::warn(format!("failed to set lightbar on {product}: {err}"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bt_report_has_expected_size_and_crc_tail() {
        let report = build_bt_output_report_for_test(Rgb::ORANGE);
        assert_eq!(report.len(), 78);
        assert_eq!(report[0], OUTPUT_REPORT_BT_ID);
        assert_eq!(report[2], OUTPUT_REPORT_BT_TAG);
        assert_eq!(report[3 + 44], 255);
        assert_eq!(report[3 + 45], 100);
        assert_eq!(report[3 + 46], 0);
        // CRC bytes should not all be zero after hashing non-empty payload.
        assert!(report[74..78].iter().any(|&b| b != 0));
    }
}
