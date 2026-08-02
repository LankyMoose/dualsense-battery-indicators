//! Developer-only controller presets (feature `dev-emulate`).

use crate::battery::{ControllerStatus, LOW_BATTERY_PERCENT, PowerState};

pub const SERIAL_PREFIX: &str = "emu-";

pub fn is_emulated(serial: &str) -> bool {
    serial.starts_with(SERIAL_PREFIX)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Discharging50,
    LowBattery,
    Charging,
    FullyCharged,
    /// Charging → Complete on successive applies when already charging.
    ChargeCompleteStep,
    TwoPads,
    Clear,
}

impl Preset {
    pub fn menu_id(self) -> &'static str {
        match self {
            Self::Discharging50 => "dev:discharging50",
            Self::LowBattery => "dev:low",
            Self::Charging => "dev:charging",
            Self::FullyCharged => "dev:complete",
            Self::ChargeCompleteStep => "dev:charge_step",
            Self::TwoPads => "dev:two",
            Self::Clear => "dev:clear",
        }
    }

    pub fn menu_label(self) -> &'static str {
        match self {
            Self::Discharging50 => "Emulate: discharging 50%",
            Self::LowBattery => "Emulate: low battery",
            Self::Charging => "Emulate: charging",
            Self::FullyCharged => "Emulate: fully charged",
            Self::ChargeCompleteStep => "Emulate: charge complete step",
            Self::TwoPads => "Emulate: two pads",
            Self::Clear => "Clear emulation",
        }
    }

    pub fn from_menu_id(id: &str) -> Option<Self> {
        [
            Self::Discharging50,
            Self::LowBattery,
            Self::Charging,
            Self::FullyCharged,
            Self::ChargeCompleteStep,
            Self::TwoPads,
            Self::Clear,
        ]
        .into_iter()
        .find(|p| p.menu_id() == id)
    }

    pub const ALL: &'static [Preset] = &[
        Self::Discharging50,
        Self::LowBattery,
        Self::Charging,
        Self::FullyCharged,
        Self::ChargeCompleteStep,
        Self::TwoPads,
        Self::Clear,
    ];
}

fn one(
    index: usize,
    serial_suffix: &str,
    percent: u8,
    state: PowerState,
    connection: &'static str,
) -> ControllerStatus {
    ControllerStatus {
        index,
        product: "DualSense",
        connection,
        serial: format!("{SERIAL_PREFIX}{serial_suffix}"),
        percent,
        state,
    }
}

/// Apply a preset. `current` is the active emulated list (may be empty).
pub fn apply_preset(preset: Preset, current: &[ControllerStatus]) -> Vec<ControllerStatus> {
    match preset {
        Preset::Clear => Vec::new(),
        Preset::Discharging50 => {
            vec![one(1, "1", 50, PowerState::Discharging, "USB")]
        }
        Preset::LowBattery => {
            vec![one(
                1,
                "1",
                LOW_BATTERY_PERCENT,
                PowerState::Discharging,
                "Bluetooth",
            )]
        }
        Preset::Charging => {
            vec![one(1, "1", 80, PowerState::Charging, "USB")]
        }
        Preset::FullyCharged => {
            vec![one(1, "1", 100, PowerState::Complete, "USB")]
        }
        Preset::ChargeCompleteStep => {
            let charging = current.len() == 1
                && current[0].state == PowerState::Charging
                && is_emulated(&current[0].serial);
            if charging {
                vec![one(1, "1", 100, PowerState::Complete, "USB")]
            } else {
                vec![one(1, "1", 80, PowerState::Charging, "USB")]
            }
        }
        Preset::TwoPads => {
            vec![
                one(
                    1,
                    "1",
                    LOW_BATTERY_PERCENT,
                    PowerState::Discharging,
                    "Bluetooth",
                ),
                one(2, "2", 80, PowerState::Charging, "USB"),
            ]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn charge_complete_step_two_phase() {
        let first = apply_preset(Preset::ChargeCompleteStep, &[]);
        assert_eq!(first.len(), 1);
        assert_eq!(first[0].state, PowerState::Charging);

        let second = apply_preset(Preset::ChargeCompleteStep, &first);
        assert_eq!(second[0].state, PowerState::Complete);
    }

    #[test]
    fn clear_empties() {
        let pads = apply_preset(Preset::LowBattery, &[]);
        assert!(!pads.is_empty());
        assert!(apply_preset(Preset::Clear, &pads).is_empty());
    }
}
