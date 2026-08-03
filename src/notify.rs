//! Desktop notifications for low battery and charge-complete edges.

use crate::app_log;
use crate::app_meta::DISPLAY_NAME;
use crate::battery::{ControllerStatus, PowerState};
use crate::prefs::Prefs;
use notify_rust::Notification;
use std::collections::HashMap;

#[derive(Debug, Default)]
struct SerialFlags {
    notified_low: bool,
    notified_complete: bool,
}

#[derive(Debug, Default)]
pub struct NotifyTracker {
    by_serial: HashMap<String, SerialFlags>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum NotifyKind {
    Low,
    Charged,
}

impl NotifyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fire toasts for low-battery / charge-complete transitions.
    pub fn evaluate(
        &mut self,
        previous: &[ControllerStatus],
        next: &[ControllerStatus],
        prefs: &Prefs,
    ) {
        for (controller, kind) in self.collect_events(previous, next, prefs) {
            match kind {
                NotifyKind::Low => show_low(controller),
                NotifyKind::Charged => show_charged(controller),
            }
        }
    }

    fn collect_events<'a>(
        &mut self,
        previous: &[ControllerStatus],
        next: &'a [ControllerStatus],
        prefs: &Prefs,
    ) -> Vec<(&'a ControllerStatus, NotifyKind)> {
        let prev_by_serial: HashMap<&str, &ControllerStatus> =
            previous.iter().map(|c| (c.serial.as_str(), c)).collect();

        self.by_serial
            .retain(|serial, _| next.iter().any(|c| c.serial == *serial));

        let mut events = Vec::new();

        for controller in next {
            let flags = self.by_serial.entry(controller.serial.clone()).or_default();
            let prev = prev_by_serial.get(controller.serial.as_str()).copied();

            let was_low = prev.is_some_and(|p| p.is_low_battery());
            if controller.is_low_battery() {
                if !was_low && !flags.notified_low && prefs.notify_low {
                    events.push((controller, NotifyKind::Low));
                    flags.notified_low = true;
                }
            } else {
                flags.notified_low = false;
            }

            let was_complete = prev.is_some_and(|p| p.state == PowerState::Complete);
            if controller.state == PowerState::Complete {
                // Require a known prior state so plugging in an already-full pad stays quiet.
                if prev.is_some()
                    && !was_complete
                    && !flags.notified_complete
                    && prefs.notify_charged
                {
                    events.push((controller, NotifyKind::Charged));
                    flags.notified_complete = true;
                }
            } else {
                flags.notified_complete = false;
            }
        }

        events
    }
}

fn show_low(controller: &ControllerStatus) {
    let body = format!(
        "{} ({}) is low — {}%",
        controller.product, controller.connection, controller.percent
    );
    show(DISPLAY_NAME, &body);
}

fn show_charged(controller: &ControllerStatus) {
    let body = format!(
        "{} ({}) finished charging",
        controller.product, controller.connection
    );
    show(DISPLAY_NAME, &body);
}

fn show(summary: &str, body: &str) {
    if let Err(err) = Notification::new()
        .appname(DISPLAY_NAME)
        .summary(summary)
        .body(body)
        .show()
    {
        app_log::warn(format!("notification failed: {err}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::battery::PowerState;

    fn pad(
        serial: &str,
        percent: u8,
        state: PowerState,
        connection: &'static str,
    ) -> ControllerStatus {
        ControllerStatus {
            index: 1,
            product: "DualSense",
            connection,
            serial: serial.to_string(),
            percent,
            state,
        }
    }

    fn prefs(notify_low: bool, notify_charged: bool) -> Prefs {
        Prefs {
            notify_low,
            notify_charged,
            spectrum: Default::default(),
        }
    }

    #[test]
    fn enters_low_once() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(true, true);
        let mid = vec![pad("a", 50, PowerState::Discharging, "USB")];
        let low = vec![pad("a", 5, PowerState::Discharging, "USB")];

        assert!(tracker.collect_events(&[], &mid, &p).is_empty());
        assert!(!tracker.by_serial["a"].notified_low);

        let events = tracker.collect_events(&mid, &low, &p);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, NotifyKind::Low);
        assert!(tracker.by_serial["a"].notified_low);

        assert!(tracker.collect_events(&low, &low, &p).is_empty());
    }

    #[test]
    fn leave_low_allows_reentry() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(true, true);
        let low = vec![pad("a", 5, PowerState::Discharging, "BT")];
        let mid = vec![pad("a", 50, PowerState::Discharging, "BT")];

        assert_eq!(tracker.collect_events(&[], &low, &p).len(), 1);
        assert!(tracker.collect_events(&low, &mid, &p).is_empty());
        assert!(!tracker.by_serial["a"].notified_low);
        assert_eq!(tracker.collect_events(&mid, &low, &p).len(), 1);
    }

    #[test]
    fn charging_to_complete_notifies() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(true, true);
        let charging = vec![pad("a", 80, PowerState::Charging, "USB")];
        let complete = vec![pad("a", 100, PowerState::Complete, "USB")];

        assert!(tracker.collect_events(&[], &charging, &p).is_empty());
        let events = tracker.collect_events(&charging, &complete, &p);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].1, NotifyKind::Charged);
        assert!(tracker.by_serial["a"].notified_complete);
        assert!(tracker.collect_events(&complete, &complete, &p).is_empty());
    }

    #[test]
    fn already_complete_on_connect_is_quiet() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(true, true);
        let complete = vec![pad("a", 100, PowerState::Complete, "USB")];

        assert!(tracker.collect_events(&[], &complete, &p).is_empty());
        assert!(!tracker.by_serial["a"].notified_complete);
    }

    #[test]
    fn prefs_can_disable() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(false, false);
        let mid = vec![pad("a", 50, PowerState::Discharging, "USB")];
        let low = vec![pad("a", 5, PowerState::Discharging, "USB")];
        let charging = vec![pad("a", 80, PowerState::Charging, "USB")];
        let complete = vec![pad("a", 100, PowerState::Complete, "USB")];

        assert!(tracker.collect_events(&mid, &low, &p).is_empty());
        assert!(!tracker.by_serial["a"].notified_low);
        assert!(tracker.collect_events(&charging, &complete, &p).is_empty());
        assert!(!tracker.by_serial["a"].notified_complete);
    }

    #[test]
    fn multi_controller_independent() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(true, true);
        let prev = vec![
            pad("a", 50, PowerState::Discharging, "USB"),
            pad("b", 80, PowerState::Charging, "Bluetooth"),
        ];
        let next = vec![
            pad("a", 5, PowerState::Discharging, "USB"),
            pad("b", 100, PowerState::Complete, "Bluetooth"),
        ];

        let events = tracker.collect_events(&prev, &next, &p);
        assert_eq!(events.len(), 2);
        assert!(tracker.by_serial["a"].notified_low);
        assert!(tracker.by_serial["b"].notified_complete);
    }

    #[test]
    fn disconnect_clears_flags() {
        let mut tracker = NotifyTracker::new();
        let p = prefs(true, true);
        let low = vec![pad("a", 5, PowerState::Discharging, "USB")];

        let _ = tracker.collect_events(&[], &low, &p);
        assert!(tracker.by_serial.contains_key("a"));

        assert!(tracker.collect_events(&low, &[], &p).is_empty());
        assert!(!tracker.by_serial.contains_key("a"));
    }
}
