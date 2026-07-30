//! DualSense discovery and battery reading.

use crate::app_log;
use crate::color::color_for_battery_percent;
use crate::lightbar::{self, set_lightbar_on_device, with_lightbar_lock};
use hidapi::{BusType, DeviceInfo, HidApi, HidDevice};
use std::thread;
use std::time::Duration;

const SONY_VENDOR_ID: u16 = 0x054C;
const DUALSENSE_PRODUCT_ID: u16 = 0x0CE6;
const DUALSENSE_EDGE_PRODUCT_ID: u16 = 0x0DF2;

/// Generic Desktop / Game Pad — the DualSense HID interface that carries battery data.
const HID_USAGE_PAGE_GENERIC_DESKTOP: u16 = 0x01;
const HID_USAGE_GAMEPAD: u16 = 0x05;

const USB_REPORT_SIZE: usize = 64;
const BT_REPORT_SIZE: usize = 78;
const USB_POWER_OFFSET: usize = 53;
const BT_POWER_OFFSET: usize = 54;

const BT_REPORT_TRUNCATED: u8 = 0x01;
const BT_REPORT_FULL: u8 = 0x31;
const USB_REPORT_ID: u8 = 0x01;
const CALIBRATION_FEATURE_REPORT: u8 = 0x05;
const CALIBRATION_FEATURE_SIZE: usize = 41;

const POWER_LEVEL_MASK: u8 = 0x0F;
const POWER_STATE_SHIFT: u8 = 4;
const MAX_POWER_LEVEL: u8 = 0x0A;

/// Lowest DualSense reporting bucket (mid-point 5% ≈ 0–9%).
pub const LOW_BATTERY_PERCENT: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PowerState {
    Discharging,
    Charging,
    Complete,
    AbnormalVoltage,
    AbnormalTemperature,
    ChargingError,
    Unknown,
}

impl PowerState {
    pub fn from_nibble(value: u8) -> Self {
        match value {
            0x00 => Self::Discharging,
            0x01 => Self::Charging,
            0x02 => Self::Complete,
            0x0A => Self::AbnormalVoltage,
            0x0B => Self::AbnormalTemperature,
            0x0F => Self::ChargingError,
            _ => Self::Unknown,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discharging => "discharging",
            Self::Charging => "charging",
            Self::Complete => "fully charged",
            Self::AbnormalVoltage => "abnormal voltage",
            Self::AbnormalTemperature => "abnormal temperature",
            Self::ChargingError => "charging error",
            Self::Unknown => "unknown",
        }
    }

    pub fn is_discharging(self) -> bool {
        matches!(self, Self::Discharging)
    }
}

#[derive(Debug, Clone)]
pub struct ControllerStatus {
    pub index: usize,
    pub product: &'static str,
    pub connection: &'static str,
    pub serial: String,
    pub percent: u8,
    pub state: PowerState,
}

impl ControllerStatus {
    pub fn menu_label(&self) -> String {
        if self.is_low_battery() {
            format!(
                "LOW {}% — {} ({}) — {}",
                self.percent,
                self.product,
                self.connection,
                self.state.as_str()
            )
        } else {
            format!(
                "{} ({})  {}% — {}",
                self.product,
                self.connection,
                self.percent,
                self.state.as_str()
            )
        }
    }

    /// Critical low bucket while discharging (drives orange pulse).
    pub fn is_low_battery(&self) -> bool {
        self.percent <= LOW_BATTERY_PERCENT && self.state.is_discharging()
    }
}

#[derive(Debug, Clone)]
struct BatteryStatus {
    percent: u8,
    state: PowerState,
    connection: &'static str,
    is_bluetooth: bool,
}

fn device_serial(info: &DeviceInfo) -> String {
    info.serial_number()
        .filter(|s| !s.is_empty())
        .unwrap_or("unknown")
        .to_string()
}

/// Enumerate DualSense gamepad serials without opening devices (fast connect/disconnect check).
pub fn list_controller_serials() -> Result<Vec<String>, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let mut serials: Vec<String> = api
        .device_list()
        .filter(|d| is_dualsense_gamepad(d))
        .map(device_serial)
        .collect();
    serials.sort();
    serials.dedup();
    Ok(serials)
}

pub fn poll_controllers() -> Result<Vec<ControllerStatus>, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let devices: Vec<&DeviceInfo> = api
        .device_list()
        .filter(|d| is_dualsense_gamepad(d))
        .collect();

    if devices.is_empty() {
        return Ok(Vec::new());
    }

    let mut statuses = Vec::with_capacity(devices.len());

    for info in devices {
        let product = product_name(info.product_id());
        let serial = device_serial(info);

        match info.open_device(&api).and_then(|device| {
            let battery = read_battery(&device)?;
            let color = color_for_battery_percent(battery.percent);
            with_lightbar_lock(|| {
                if let Err(err) =
                    set_lightbar_on_device(&device, color, battery.is_bluetooth)
                {
                    lightbar::warn_lightbar(product, err);
                }
            });
            Ok(battery)
        }) {
            Ok(battery) => statuses.push(ControllerStatus {
                index: 0,
                product,
                connection: battery.connection,
                serial,
                percent: battery.percent,
                state: battery.state,
            }),
            Err(err) => {
                app_log::warn(format!("failed to read {product} ({serial}): {err}"));
            }
        }
    }

    statuses.sort_by(|a, b| a.serial.cmp(&b.serial));
    for (i, status) in statuses.iter_mut().enumerate() {
        status.index = i + 1;
    }

    Ok(statuses)
}

pub(crate) fn is_dualsense_gamepad(d: &DeviceInfo) -> bool {
    d.vendor_id() == SONY_VENDOR_ID
        && matches!(
            d.product_id(),
            DUALSENSE_PRODUCT_ID | DUALSENSE_EDGE_PRODUCT_ID
        )
        && d.usage_page() == HID_USAGE_PAGE_GENERIC_DESKTOP
        && d.usage() == HID_USAGE_GAMEPAD
}

fn product_name(product_id: u16) -> &'static str {
    match product_id {
        DUALSENSE_EDGE_PRODUCT_ID => "DualSense Edge",
        _ => "DualSense",
    }
}

fn read_battery(device: &HidDevice) -> Result<BatteryStatus, hidapi::HidError> {
    let bus_type = device.get_device_info()?.bus_type();
    let (connection, report_size, power_offset, is_bluetooth) = match bus_type {
        BusType::Usb => ("USB", USB_REPORT_SIZE, USB_POWER_OFFSET, false),
        BusType::Bluetooth => ("Bluetooth", BT_REPORT_SIZE, BT_POWER_OFFSET, true),
        other => {
            return Err(hidapi::HidError::HidApiError {
                message: format!("unsupported connection type: {other:?}"),
            });
        }
    };

    let mut requested_full_report = false;

    for _ in 0..40 {
        let mut buf = vec![0u8; report_size];
        let n = device.read_timeout(&mut buf, 500)?;
        if n == 0 {
            continue;
        }

        if is_bluetooth && buf[0] == BT_REPORT_TRUNCATED {
            if !requested_full_report {
                request_full_bt_report(device)?;
                requested_full_report = true;
                thread::sleep(Duration::from_millis(200));
            }
            continue;
        }

        let expected_id = if is_bluetooth {
            BT_REPORT_FULL
        } else {
            USB_REPORT_ID
        };

        if buf[0] != expected_id {
            return Err(hidapi::HidError::HidApiError {
                message: format!(
                    "unexpected report id {:#04x} (expected {:#04x})",
                    buf[0], expected_id
                ),
            });
        }

        if n <= power_offset {
            return Err(hidapi::HidError::HidApiError {
                message: format!("input report too short ({n} bytes)"),
            });
        }

        let power = buf[power_offset];
        let level = (power & POWER_LEVEL_MASK).min(MAX_POWER_LEVEL);
        let state = PowerState::from_nibble(power >> POWER_STATE_SHIFT);
        let percent = percent_from_level(level, state);

        return Ok(BatteryStatus {
            percent,
            state,
            connection,
            is_bluetooth,
        });
    }

    Err(hidapi::HidError::HidApiError {
        message: "timed out waiting for a DualSense input report with battery data".into(),
    })
}

/// DualSense reports battery in 11 coarse steps (0..=10).
/// Linux maps each step to the mid-point of its 10% bucket:
/// 0 → 0–9% (5%), 1 → 10–19% (15%), …, 9 → 90–99% (95%), 10/full → 100%.
pub fn percent_from_level(level: u8, state: PowerState) -> u8 {
    match state {
        PowerState::Complete => 100,
        PowerState::AbnormalVoltage
        | PowerState::AbnormalTemperature
        | PowerState::ChargingError => 0,
        PowerState::Discharging | PowerState::Charging | PowerState::Unknown => {
            if level >= MAX_POWER_LEVEL {
                100
            } else {
                (u16::from(level) * 10 + 5).min(100) as u8
            }
        }
    }
}

fn request_full_bt_report(device: &HidDevice) -> Result<(), hidapi::HidError> {
    let mut feature = vec![0u8; CALIBRATION_FEATURE_SIZE];
    feature[0] = CALIBRATION_FEATURE_REPORT;
    device.get_feature_report(&mut feature)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_buckets_match_linux_midpoints() {
        assert_eq!(percent_from_level(0, PowerState::Discharging), 5);
        assert_eq!(percent_from_level(1, PowerState::Discharging), 15);
        assert_eq!(percent_from_level(9, PowerState::Discharging), 95);
        assert_eq!(percent_from_level(10, PowerState::Discharging), 100);
        assert_eq!(percent_from_level(10, PowerState::Charging), 100);
        assert_eq!(percent_from_level(3, PowerState::Complete), 100);
        assert_eq!(percent_from_level(5, PowerState::AbnormalVoltage), 0);
        assert_eq!(percent_from_level(5, PowerState::ChargingError), 0);
    }

    #[test]
    fn low_battery_requires_discharging() {
        let discharging = ControllerStatus {
            index: 1,
            product: "DualSense",
            connection: "Bluetooth",
            serial: "abc".into(),
            percent: 5,
            state: PowerState::Discharging,
        };
        let charging = ControllerStatus {
            state: PowerState::Charging,
            ..discharging.clone()
        };
        assert!(discharging.is_low_battery());
        assert!(!charging.is_low_battery());
    }
}
