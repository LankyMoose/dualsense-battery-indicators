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

    pub fn is_charging(self) -> bool {
        matches!(self, Self::Charging | Self::Complete)
    }

    fn is_complete(self) -> bool {
        matches!(self, Self::Complete)
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
}

#[derive(Debug, Clone)]
struct BatteryStatus {
    percent: u8,
    state: PowerState,
    connection: &'static str,
}

pub fn read_all_controllers() -> Result<Vec<ControllerStatus>, String> {
    let api = HidApi::new().map_err(|e| e.to_string())?;
    let devices: Vec<&DeviceInfo> = api
        .device_list()
        .filter(|d| is_dualsense_gamepad(d))
        .collect();

    if devices.is_empty() {
        return Ok(Vec::new());
    }

    let mut statuses = Vec::with_capacity(devices.len());

    for (index, info) in devices.iter().enumerate() {
        let product = product_name(info.product_id());
        let serial = info
            .serial_number()
            .filter(|s| !s.is_empty())
            .unwrap_or("unknown")
            .to_string();

        match info.open_device(&api).and_then(|device| read_battery(&device)) {
            Ok(battery) => statuses.push(ControllerStatus {
                index: index + 1,
                product,
                connection: battery.connection,
                serial,
                percent: battery.percent,
                state: battery.state,
            }),
            Err(err) => {
                // Skip controllers we can't read this cycle; still report the rest.
                eprintln!("warning: failed to read {product} ({serial}): {err}");
            }
        }
    }

    Ok(statuses)
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

    // Bluetooth starts in a truncated report mode. Requesting the calibration
    // feature report switches it to the full report that includes battery data.
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
        let mut level = power & POWER_LEVEL_MASK;
        let state = PowerState::from_nibble(power >> POWER_STATE_SHIFT);

        if state.is_complete() {
            level = MAX_POWER_LEVEL;
        }

        let percent = ((u16::from(level) * 100) / u16::from(MAX_POWER_LEVEL)).min(100) as u8;

        return Ok(BatteryStatus {
            percent,
            state,
            connection,
        });
    }

    Err(hidapi::HidError::HidApiError {
        message: "timed out waiting for a DualSense input report with battery data".into(),
    })
}

fn request_full_bt_report(device: &HidDevice) -> Result<(), hidapi::HidError> {
    let mut feature = vec![0u8; CALIBRATION_FEATURE_SIZE];
    feature[0] = CALIBRATION_FEATURE_REPORT;
    device.get_feature_report(&mut feature)?;
    Ok(())
}
