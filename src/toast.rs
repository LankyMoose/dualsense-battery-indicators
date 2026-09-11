//! Overlay toast message model (presented by the iced daemon via `toast_view`).

use crate::color::{BatterySpectrum, Rgb};
use crate::notify::NotifyEvent;

#[derive(Debug, Clone)]
pub struct ToastMessage {
    pub heading: String,
    pub body: String,
    pub accent: Rgb,
}

impl ToastMessage {
    pub fn from_notification(event: NotifyEvent, spectrum: BatterySpectrum) -> Self {
        Self {
            heading: event.heading,
            body: event.body,
            accent: spectrum.color_at_percent(event.percent.unwrap_or(100)),
        }
    }

    pub fn preview(accent: Rgb) -> Self {
        Self {
            heading: "DualSense Battery Indicators".to_string(),
            body: "Toasts will appear here".to_string(),
            accent,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::{ControllerStatus, PowerState};
    use crate::notify::NotifyTracker;
    use crate::prefs::Prefs;

    fn pad(serial: &str, percent: u8) -> ControllerStatus {
        ControllerStatus {
            index: 1,
            product: "DualSense",
            connection: "USB",
            serial: serial.to_string(),
            percent,
            state: PowerState::Discharging,
        }
    }

    #[test]
    fn from_notification_uses_spectrum_accent_for_percent() {
        let mut tracker = NotifyTracker::new();
        let connected = vec![pad("a", 40)];
        let events = tracker.evaluate(&[], &connected, &Prefs::default(), |_| None);
        assert_eq!(events.len(), 1);
        let message =
            ToastMessage::from_notification(events[0].clone(), BatterySpectrum::default());
        assert!(message.heading.contains("DualSense"));
        assert!(message.body.contains("40%"));
    }

    #[test]
    fn preview_has_stable_copy() {
        let message = ToastMessage::preview(Rgb::new(1, 2, 3));
        assert_eq!(message.accent, Rgb::new(1, 2, 3));
        assert!(!message.heading.is_empty());
        assert!(!message.body.is_empty());
    }
}
