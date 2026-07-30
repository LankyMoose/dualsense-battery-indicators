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

const OUTPUT_REPORT_USB_ID: u8 = 0x02;
const OUTPUT_REPORT_USB_SIZE: usize = 63;
const OUTPUT_REPORT_BT_ID: u8 = 0x31;
const OUTPUT_REPORT_BT_SIZE: usize = 78;
const OUTPUT_REPORT_BT_TAG: u8 = 0x10;
const OUTPUT_CRC32_SEED: u8 = 0xA2;
const OUTPUT_VALID_FLAG1_LIGHTBAR: u8 = 1 << 2;

const POWER_LEVEL_MASK: u8 = 0x0F;
const POWER_STATE_SHIFT: u8 = 4;
const MAX_POWER_LEVEL: u8 = 0x0A;

pub const IDENTIFY_FLASH_MS: u64 = 150;
pub const IDENTIFY_FLASH_COUNT: u32 = 5;

/// Lowest DualSense reporting bucket (mid-point 5% ≈ 0–9%).
pub const LOW_BATTERY_PERCENT: u8 = 5;
pub const LOW_BATTERY_ORANGE: (u8, u8, u8) = (255, 100, 0);
pub const LOW_BATTERY_PULSE_ON_MS: u64 = 400;
pub const LOW_BATTERY_PULSE_GAP_MS: u64 = 1600;

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
    fn from_nibble(value: u8) -> Self {
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
        format!(
            "{} ({})  {}% — {}",
            self.product,
            self.connection,
            self.percent,
            self.state.as_str()
        )
    }

    pub fn is_low_battery(&self) -> bool {
        self.percent <= LOW_BATTERY_PERCENT
    }
}

#[derive(Debug, Clone)]
struct BatteryStatus {
    percent: u8,
    state: PowerState,
    connection: &'static str,
    is_bluetooth: bool,
}

/// Map battery percent to a blue → purple → red hue.
/// 100% → hue 240° (blue), 0% → hue 360° (red), passing through purple.
pub fn color_for_battery_percent(percent: u8) -> (u8, u8, u8) {
    let t = (100u8.saturating_sub(percent)) as f32 / 100.0;
    let hue = 240.0 + t * 120.0; // 240..360
    hsv_to_rgb(hue, 1.0, 1.0)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let h_prime = (h % 360.0) / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    let m = v - c;

    let (r1, g1, b1) = match h_prime as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };

    (
        ((r1 + m) * 255.0).round() as u8,
        ((g1 + m) * 255.0).round() as u8,
        ((b1 + m) * 255.0).round() as u8,
    )
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
        let serial = info
            .serial_number()
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string();

        match info.open_device(&api).and_then(|device| {
            let battery = read_battery(&device)?;
            let (r, g, b) = color_for_battery_percent(battery.percent);
            if let Err(err) = set_lightbar_rgb(&device, r, g, b, battery.is_bluetooth) {
                eprintln!("warning: failed to set lightbar on {product}: {err}");
            }
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
                eprintln!("warning: failed to read {product} ({serial}): {err}");
            }
        }
    }

    statuses.sort_by(|a, b| a.serial.cmp(&b.serial));
    for (i, status) in statuses.iter_mut().enumerate() {
        status.index = i + 1;
    }

    Ok(statuses)
}

pub fn apply_lightbar_rgb(serial: &str, r: u8, g: u8, b: u8) -> Result<(), String> {
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
    set_lightbar_rgb(&device, r, g, b, is_bluetooth).map_err(|e| e.to_string())
}

/// Flash white, then restore battery color, five times.
pub fn identify_controller(serial: &str, percent: u8) -> Result<(), String> {
    let normal = color_for_battery_percent(percent);
    let flash_for = Duration::from_millis(IDENTIFY_FLASH_MS);

    for _ in 0..IDENTIFY_FLASH_COUNT {
        apply_lightbar_rgb(serial, 255, 255, 255)?;
        thread::sleep(flash_for);
        apply_lightbar_rgb(serial, normal.0, normal.1, normal.2)?;
        thread::sleep(flash_for);
    }

    Ok(())
}

fn is_dualsense_gamepad(d: &DeviceInfo) -> bool {
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
fn percent_from_level(level: u8, state: PowerState) -> u8 {
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

fn set_lightbar_rgb(
    device: &HidDevice,
    r: u8,
    g: u8,
    b: u8,
    is_bluetooth: bool,
) -> Result<(), hidapi::HidError> {
    if is_bluetooth {
        let mut report = [0u8; OUTPUT_REPORT_BT_SIZE];
        report[0] = OUTPUT_REPORT_BT_ID;
        report[1] = 0x10;
        report[2] = OUTPUT_REPORT_BT_TAG;
        report[3 + 1] = OUTPUT_VALID_FLAG1_LIGHTBAR;
        report[3 + 44] = r;
        report[3 + 45] = g;
        report[3 + 46] = b;

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
        report[1 + 44] = r;
        report[1 + 45] = g;
        report[1 + 46] = b;
        device.write(&report)?;
    }

    Ok(())
}
