//! Opt-in local battery cycle analytics (charge / play duration estimates).
//!
//! Cycle boundaries come from DualSense power-state edges — not a separate poller.
//! Persistence is a compact per-serial summary (last-5 duration rings + one open
//! session with waypoints). Nothing leaves the machine.

use crate::app_log;
use crate::app_meta::PKG_NAME;
use crate::battery::{ControllerStatus, LOW_BATTERY_PERCENT, PowerState};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// How many qualifying durations to keep per metric.
pub const SAMPLE_RING_CAP: usize = 5;
/// Max waypoints for an in-progress timeline chart.
pub const WAYPOINT_CAP: usize = 64;
/// Max controller serials retained in the store.
pub const SERIAL_CAP: usize = 32;
/// Drop pads not seen for this long (unless remembered / nicknamed).
pub const STALE_SERIAL_DAYS: u64 = 180;
/// Drop an abandoned open session after this long.
pub const ABANDONED_SESSION_DAYS: u64 = 30;

/// Accept a charge sample only if it started near empty.
const CHARGE_START_MAX_PERCENT: u8 = 15;
/// Accept a play sample if at least this much percent was consumed.
const PLAY_MIN_PERCENT_USED: u8 = 50;
/// Or if it ended in / below the empty DualSense bucket.
const PLAY_EMPTY_PERCENT: u8 = LOW_BATTERY_PERCENT;

const MIN_CHARGE_SECS: u64 = 10 * 60;
const MAX_CHARGE_SECS: u64 = 8 * 60 * 60;
const MIN_PLAY_SECS: u64 = 30 * 60;
const MAX_PLAY_SECS: u64 = 24 * 60 * 60;

/// Heartbeat gaps larger than this are treated as "app was closed" and skipped.
pub const HEARTBEAT_MAX_GAP: Duration = Duration::from_secs(150);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    Charging,
    DischargingFromFull,
    PausedDischarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaypointKind {
    Start,
    Percent,
    Pause,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Waypoint {
    /// Unix milliseconds (wall clock) for the timeline X axis.
    pub at_ms: u64,
    pub percent: u8,
    pub kind: WaypointKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenSession {
    pub kind: SessionKind,
    pub start_percent: u8,
    pub last_percent: u8,
    /// Active (on + discharging/charging) milliseconds accumulated so far.
    pub active_ms: u64,
    pub waypoints: Vec<Waypoint>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct SerialRecord {
    /// Unix milliseconds of last observation.
    last_seen_ms: u64,
    #[serde(default)]
    charge_samples_ms: Vec<u64>,
    #[serde(default)]
    play_samples_ms: Vec<u64>,
    #[serde(default)]
    open: Option<OpenSession>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
struct AnalyticsFile {
    #[serde(default)]
    controllers: HashMap<String, SerialRecord>,
}

/// Snapshot of estimates / open session for UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControllerAnalytics {
    pub serial: String,
    pub typical_charge: Option<Duration>,
    pub typical_play: Option<Duration>,
    pub open: Option<OpenSession>,
}

/// Remaining-time hint for the tray popup meta line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EtaHint {
    PlayLeft(Duration),
    ChargeToFull(Duration),
}

#[derive(Debug, Clone, Default)]
pub struct AnalyticsStore {
    by_serial: HashMap<String, SerialRecord>,
    dirty: bool,
    /// Last wall-clock observation used for heartbeat gap detection.
    last_tick_ms: Option<u64>,
}

impl AnalyticsStore {
    pub fn load() -> Self {
        let path = store_path();
        let Ok(bytes) = fs::read(&path) else {
            return Self::default();
        };
        match serde_json::from_slice::<AnalyticsFile>(&bytes) {
            Ok(file) => Self {
                by_serial: file.controllers,
                dirty: false,
                last_tick_ms: None,
            },
            Err(err) => {
                app_log::warn(format!(
                    "failed to parse analytics at {}: {err}; starting empty",
                    path.display()
                ));
                Self::default()
            }
        }
    }

    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let path = store_path();
        if let Some(parent) = path.parent()
            && let Err(err) = fs::create_dir_all(parent)
        {
            app_log::warn(format!("failed to create analytics dir: {err}"));
            return;
        }
        let file = AnalyticsFile {
            controllers: self.by_serial.clone(),
        };
        match serde_json::to_vec_pretty(&file) {
            Ok(bytes) => {
                if let Err(err) = fs::write(&path, bytes) {
                    app_log::warn(format!("failed to write analytics: {err}"));
                    return;
                }
                self.dirty = false;
            }
            Err(err) => app_log::warn(format!("failed to serialize analytics: {err}")),
        }
    }

    pub fn clear(&mut self) {
        self.by_serial.clear();
        self.last_tick_ms = None;
        self.dirty = true;
        self.save();
    }

    pub fn is_storable_serial(serial: &str) -> bool {
        if serial.is_empty() || serial == "unknown" {
            return false;
        }
        // Emulated pads are only compiled into `--features dev-emulate` builds; allow
        // them there so Developer presets can exercise analytics end-to-end.
        #[cfg(not(feature = "dev-emulate"))]
        if serial.starts_with("emu-") {
            return false;
        }
        true
    }

    /// Developer helper: inject typical charge/play samples so ETA UI can be checked.
    #[cfg(feature = "dev-emulate")]
    pub fn dev_seed_estimates(&mut self, serial: &str) {
        if !Self::is_storable_serial(serial) {
            return;
        }
        let now_ms = system_time_ms(SystemTime::now());
        self.ensure_record(serial, now_ms);
        let record = self.by_serial.get_mut(serial).expect("ensured");
        push_sample(&mut record.charge_samples_ms, 2 * 60 * 60 * 1000);
        push_sample(&mut record.play_samples_ms, 8 * 60 * 60 * 1000);
        record.last_seen_ms = now_ms;
        self.dirty = true;
    }

    /// Developer helper: add active time to the open session (bypasses heartbeat gaps).
    #[cfg(feature = "dev-emulate")]
    pub fn dev_credit_active(&mut self, serial: &str, extra: Duration) {
        let Some(record) = self.by_serial.get_mut(serial) else {
            return;
        };
        let Some(open) = record.open.as_mut() else {
            return;
        };
        if matches!(
            open.kind,
            SessionKind::Charging | SessionKind::DischargingFromFull
        ) {
            open.active_ms = open.active_ms.saturating_add(extra.as_millis() as u64);
            self.dirty = true;
        }
    }

    /// Observe a poll snapshot. When `enabled` is false, only prune/save nothing.
    pub fn observe(
        &mut self,
        previous: &[ControllerStatus],
        next: &[ControllerStatus],
        enabled: bool,
        keep_serial: impl Fn(&str) -> bool,
        now: SystemTime,
    ) {
        if !enabled {
            return;
        }
        let now_ms = system_time_ms(now);
        let gap_ms = self
            .last_tick_ms
            .map(|prev| now_ms.saturating_sub(prev))
            .unwrap_or(0);
        let heartbeat_ok = gap_ms > 0 && gap_ms <= HEARTBEAT_MAX_GAP.as_millis() as u64;
        self.last_tick_ms = Some(now_ms);

        let prev_by: HashMap<&str, &ControllerStatus> =
            previous.iter().map(|c| (c.serial.as_str(), c)).collect();
        let next_by: HashMap<&str, &ControllerStatus> =
            next.iter().map(|c| (c.serial.as_str(), c)).collect();

        // Heartbeat active time for pads still present with an open active session.
        if heartbeat_ok {
            for controller in next {
                if !Self::is_storable_serial(&controller.serial) {
                    continue;
                }
                if let Some(record) = self.by_serial.get_mut(&controller.serial)
                    && let Some(open) = record.open.as_mut()
                    && matches!(
                        open.kind,
                        SessionKind::Charging | SessionKind::DischargingFromFull
                    )
                {
                    open.active_ms = open.active_ms.saturating_add(gap_ms);
                    record.last_seen_ms = now_ms;
                    self.dirty = true;
                }
            }
        }

        // Disconnects: pause discharge sessions.
        for prev in previous {
            if !Self::is_storable_serial(&prev.serial) {
                continue;
            }
            if next_by.contains_key(prev.serial.as_str()) {
                continue;
            }
            self.on_disconnect(&prev.serial, now_ms);
        }

        for controller in next {
            if !Self::is_storable_serial(&controller.serial) {
                continue;
            }
            let prev = prev_by.get(controller.serial.as_str()).copied();
            self.on_controller(prev, controller, now_ms);
        }

        self.prune(now_ms, keep_serial);
    }

    fn on_disconnect(&mut self, serial: &str, now_ms: u64) {
        let Some(record) = self.by_serial.get_mut(serial) else {
            return;
        };
        let Some(open) = record.open.as_mut() else {
            return;
        };
        if open.kind == SessionKind::DischargingFromFull {
            open.kind = SessionKind::PausedDischarge;
            push_waypoint(
                &mut open.waypoints,
                Waypoint {
                    at_ms: now_ms,
                    percent: open.last_percent,
                    kind: WaypointKind::Pause,
                },
            );
            record.last_seen_ms = now_ms;
            self.dirty = true;
        } else if open.kind == SessionKind::Charging {
            // Charge interrupted by disconnect — abort without recording.
            record.open = None;
            record.last_seen_ms = now_ms;
            self.dirty = true;
        }
    }

    fn on_controller(
        &mut self,
        prev: Option<&ControllerStatus>,
        next: &ControllerStatus,
        now_ms: u64,
    ) {
        let serial = next.serial.as_str();
        self.ensure_record(serial, now_ms);

        // Abandoned / percent-rose while paused.
        if let Some(record) = self.by_serial.get_mut(serial)
            && let Some(open) = record.open.as_ref()
        {
            let abandoned = now_ms.saturating_sub(record.last_seen_ms)
                > Duration::from_secs(ABANDONED_SESSION_DAYS * 24 * 60 * 60).as_millis() as u64;
            if abandoned {
                record.open = None;
                self.dirty = true;
            } else if open.kind == SessionKind::PausedDischarge
                && next.percent > open.last_percent
            {
                // Charged while away.
                record.open = None;
                self.dirty = true;
            }
        }

        let open_kind = self
            .by_serial
            .get(serial)
            .and_then(|r| r.open.as_ref())
            .map(|o| o.kind);

        // Resume paused discharge.
        if open_kind == Some(SessionKind::PausedDischarge) {
            if next.state.is_discharging() && next.percent <= self.last_open_percent(serial) {
                self.resume_discharge(serial, next.percent, now_ms);
            } else if next.state == PowerState::Charging || next.state == PowerState::Complete {
                // Plugging in finishes the paused play cycle.
                self.finish_play(serial, next.percent, now_ms);
                if next.state == PowerState::Charging {
                    self.begin_charge(serial, next.percent, now_ms);
                }
                return;
            } else if !next.state.is_discharging() {
                // Unexpected state — abort.
                if let Some(record) = self.by_serial.get_mut(serial) {
                    record.open = None;
                    record.last_seen_ms = now_ms;
                    self.dirty = true;
                }
            }
        }

        let prev_state = prev.map(|p| p.state);
        let prev_percent = prev.map(|p| p.percent);

        // State transitions.
        match (prev_state, next.state) {
            (Some(PowerState::Discharging) | Some(PowerState::Unknown) | None, PowerState::Charging) => {
                // Finish open play if any, then begin charge.
                if matches!(
                    self.open_kind(serial),
                    Some(SessionKind::DischargingFromFull) | Some(SessionKind::PausedDischarge)
                ) {
                    self.finish_play(serial, next.percent, now_ms);
                }
                self.begin_charge(serial, next.percent, now_ms);
            }
            (Some(PowerState::Charging), PowerState::Complete) => {
                self.finish_charge(serial, now_ms);
            }
            (Some(PowerState::Charging), PowerState::Discharging) => {
                // Unplugged before complete — abort charge; maybe start from-full if at 100.
                self.abort_charge(serial, now_ms);
                if next.percent >= 100 {
                    self.begin_discharge_from_full(serial, next.percent, now_ms);
                }
            }
            (Some(PowerState::Complete), PowerState::Discharging) => {
                self.begin_discharge_from_full(serial, next.percent, now_ms);
            }
            (Some(PowerState::Complete), PowerState::Charging) => {
                // Still on charger after complete — ignore.
            }
            (Some(PowerState::Discharging), PowerState::Complete) => {
                // Unusual (USB complete without charging edge); treat as plugged full.
            }
            _ => {}
        }

        // Empty-bucket finish while discharging from full.
        if matches!(self.open_kind(serial), Some(SessionKind::DischargingFromFull))
            && next.state.is_discharging()
            && next.percent <= PLAY_EMPTY_PERCENT
            && prev_percent.is_some_and(|p| p > PLAY_EMPTY_PERCENT)
        {
            self.finish_play(serial, next.percent, now_ms);
            return;
        }

        // Percent change waypoint on active sessions.
        if let Some(record) = self.by_serial.get_mut(serial)
            && let Some(open) = record.open.as_mut()
            && matches!(
                open.kind,
                SessionKind::Charging | SessionKind::DischargingFromFull
            )
            && open.last_percent != next.percent
        {
            open.last_percent = next.percent;
            push_waypoint(
                &mut open.waypoints,
                Waypoint {
                    at_ms: now_ms,
                    percent: next.percent,
                    kind: WaypointKind::Percent,
                },
            );
            record.last_seen_ms = now_ms;
            self.dirty = true;
        } else if let Some(record) = self.by_serial.get_mut(serial) {
            record.last_seen_ms = now_ms;
        }
    }

    fn ensure_record(&mut self, serial: &str, now_ms: u64) {
        self.by_serial.entry(serial.to_string()).or_insert_with(|| {
            self.dirty = true;
            SerialRecord {
                last_seen_ms: now_ms,
                ..SerialRecord::default()
            }
        });
    }

    fn open_kind(&self, serial: &str) -> Option<SessionKind> {
        self.by_serial
            .get(serial)
            .and_then(|r| r.open.as_ref())
            .map(|o| o.kind)
    }

    fn last_open_percent(&self, serial: &str) -> u8 {
        self.by_serial
            .get(serial)
            .and_then(|r| r.open.as_ref())
            .map(|o| o.last_percent)
            .unwrap_or(0)
    }

    fn begin_charge(&mut self, serial: &str, percent: u8, now_ms: u64) {
        let record = self.by_serial.get_mut(serial).expect("ensured");
        if matches!(
            record.open.as_ref().map(|o| o.kind),
            Some(SessionKind::Charging)
        ) {
            return;
        }
        let mut waypoints = Vec::new();
        push_waypoint(
            &mut waypoints,
            Waypoint {
                at_ms: now_ms,
                percent,
                kind: WaypointKind::Start,
            },
        );
        record.open = Some(OpenSession {
            kind: SessionKind::Charging,
            start_percent: percent,
            last_percent: percent,
            active_ms: 0,
            waypoints,
        });
        record.last_seen_ms = now_ms;
        self.dirty = true;
    }

    fn abort_charge(&mut self, serial: &str, now_ms: u64) {
        if let Some(record) = self.by_serial.get_mut(serial)
            && matches!(
                record.open.as_ref().map(|o| o.kind),
                Some(SessionKind::Charging)
            )
        {
            record.open = None;
            record.last_seen_ms = now_ms;
            self.dirty = true;
        }
    }

    fn finish_charge(&mut self, serial: &str, now_ms: u64) {
        let Some(record) = self.by_serial.get_mut(serial) else {
            return;
        };
        let Some(open) = record.open.take() else {
            return;
        };
        if open.kind != SessionKind::Charging {
            record.open = Some(open);
            return;
        }
        let secs = open.active_ms / 1000;
        if open.start_percent <= CHARGE_START_MAX_PERCENT
            && secs >= MIN_CHARGE_SECS
            && secs <= MAX_CHARGE_SECS
        {
            push_sample(&mut record.charge_samples_ms, open.active_ms);
        }
        record.last_seen_ms = now_ms;
        self.dirty = true;
    }

    fn begin_discharge_from_full(&mut self, serial: &str, percent: u8, now_ms: u64) {
        let record = self.by_serial.get_mut(serial).expect("ensured");
        if matches!(
            record.open.as_ref().map(|o| o.kind),
            Some(SessionKind::DischargingFromFull) | Some(SessionKind::PausedDischarge)
        ) {
            return;
        }
        let start = percent.max(100);
        let mut waypoints = Vec::new();
        push_waypoint(
            &mut waypoints,
            Waypoint {
                at_ms: now_ms,
                percent: start,
                kind: WaypointKind::Start,
            },
        );
        record.open = Some(OpenSession {
            kind: SessionKind::DischargingFromFull,
            start_percent: start,
            last_percent: percent,
            active_ms: 0,
            waypoints,
        });
        record.last_seen_ms = now_ms;
        self.dirty = true;
    }

    fn resume_discharge(&mut self, serial: &str, percent: u8, now_ms: u64) {
        let Some(record) = self.by_serial.get_mut(serial) else {
            return;
        };
        let Some(open) = record.open.as_mut() else {
            return;
        };
        if open.kind != SessionKind::PausedDischarge {
            return;
        }
        open.kind = SessionKind::DischargingFromFull;
        open.last_percent = percent;
        push_waypoint(
            &mut open.waypoints,
            Waypoint {
                at_ms: now_ms,
                percent,
                kind: WaypointKind::Resume,
            },
        );
        record.last_seen_ms = now_ms;
        self.dirty = true;
    }

    fn finish_play(&mut self, serial: &str, end_percent: u8, now_ms: u64) {
        let Some(record) = self.by_serial.get_mut(serial) else {
            return;
        };
        let Some(open) = record.open.take() else {
            return;
        };
        if !matches!(
            open.kind,
            SessionKind::DischargingFromFull | SessionKind::PausedDischarge
        ) {
            record.open = Some(open);
            return;
        }
        let start = open.start_percent.max(end_percent);
        let used = start.saturating_sub(end_percent);
        let empty_finish = end_percent <= PLAY_EMPTY_PERCENT;
        let secs = open.active_ms / 1000;
        if (used >= PLAY_MIN_PERCENT_USED || empty_finish)
            && secs >= MIN_PLAY_SECS
            && secs <= MAX_PLAY_SECS
            && used > 0
        {
            // Scale to a full 0–100% drain estimate.
            let scaled = open
                .active_ms
                .saturating_mul(100)
                .checked_div(u64::from(used))
                .unwrap_or(open.active_ms);
            let scaled_secs = scaled / 1000;
            if scaled_secs >= MIN_PLAY_SECS && scaled_secs <= MAX_PLAY_SECS {
                push_sample(&mut record.play_samples_ms, scaled);
            }
        }
        record.last_seen_ms = now_ms;
        self.dirty = true;
    }

    fn prune(&mut self, now_ms: u64, keep_serial: impl Fn(&str) -> bool) {
        let stale_ms = Duration::from_secs(STALE_SERIAL_DAYS * 24 * 60 * 60).as_millis() as u64;
        let before = self.by_serial.len();
        self.by_serial.retain(|serial, record| {
            if keep_serial(serial) {
                return true;
            }
            now_ms.saturating_sub(record.last_seen_ms) <= stale_ms
        });
        if self.by_serial.len() != before {
            self.dirty = true;
        }

        while self.by_serial.len() > SERIAL_CAP {
            let oldest = self
                .by_serial
                .iter()
                .filter(|(serial, _)| !keep_serial(serial))
                .min_by_key(|(_, r)| r.last_seen_ms)
                .map(|(s, _)| s.clone());
            let Some(serial) = oldest else {
                break;
            };
            self.by_serial.remove(&serial);
            self.dirty = true;
        }
    }

    pub fn typical_charge(&self, serial: &str) -> Option<Duration> {
        self.by_serial
            .get(serial)
            .and_then(|r| median_ms(&r.charge_samples_ms))
            .map(Duration::from_millis)
    }

    pub fn typical_play(&self, serial: &str) -> Option<Duration> {
        self.by_serial
            .get(serial)
            .and_then(|r| median_ms(&r.play_samples_ms))
            .map(Duration::from_millis)
    }

    #[allow(dead_code)] // Used by unit tests and handy for future UI.
    pub fn open_session(&self, serial: &str) -> Option<&OpenSession> {
        self.by_serial.get(serial).and_then(|r| r.open.as_ref())
    }

    pub fn panel_rows(&self) -> Vec<ControllerAnalytics> {
        let mut rows: Vec<_> = self
            .by_serial
            .iter()
            .filter(|(_, record)| {
                !record.charge_samples_ms.is_empty()
                    || !record.play_samples_ms.is_empty()
                    || record.open.is_some()
            })
            .map(|(serial, record)| ControllerAnalytics {
                serial: serial.clone(),
                typical_charge: median_ms(&record.charge_samples_ms).map(Duration::from_millis),
                typical_play: median_ms(&record.play_samples_ms).map(Duration::from_millis),
                open: record.open.clone(),
            })
            .collect();
        rows.sort_by(|a, b| a.serial.cmp(&b.serial));
        rows
    }

    /// ETA for a live controller, if enough samples exist.
    pub fn eta_for(&self, controller: &ControllerStatus) -> Option<EtaHint> {
        match controller.state {
            PowerState::Discharging => {
                let typical = self.typical_play(&controller.serial)?;
                let left = typical
                    .saturating_mul(u32::from(controller.percent))
                    .checked_div(100)?;
                if left.is_zero() {
                    None
                } else {
                    Some(EtaHint::PlayLeft(left))
                }
            }
            PowerState::Charging => {
                let typical = self.typical_charge(&controller.serial)?;
                let remain_pct = 100u8.saturating_sub(controller.percent);
                let left = typical
                    .saturating_mul(u32::from(remain_pct))
                    .checked_div(100)?;
                if left.is_zero() {
                    None
                } else {
                    Some(EtaHint::ChargeToFull(left))
                }
            }
            _ => None,
        }
    }
}

/// Format a duration for UI: `~2h 15m`, `~40m`, `~1h`.
pub fn format_duration_short(duration: Duration) -> String {
    let total_mins = duration.as_secs() / 60;
    if total_mins < 1 {
        return "~<1m".to_string();
    }
    let hours = total_mins / 60;
    let mins = total_mins % 60;
    if hours == 0 {
        format!("~{mins}m")
    } else if mins == 0 {
        format!("~{hours}h")
    } else {
        format!("~{hours}h {mins}m")
    }
}

/// Compact ring label: `est. 8h`, `est. 40m`.
pub fn format_eta_ring(hint: EtaHint) -> String {
    let duration = match hint {
        EtaHint::PlayLeft(d) | EtaHint::ChargeToFull(d) => d,
    };
    let total_mins = duration.as_secs().div_ceil(60).max(1);
    if total_mins < 60 {
        format!("est. {total_mins}m")
    } else {
        let hours = total_mins / 60;
        format!("est. {hours}h")
    }
}

fn push_sample(ring: &mut Vec<u64>, value_ms: u64) {
    ring.push(value_ms);
    while ring.len() > SAMPLE_RING_CAP {
        ring.remove(0);
    }
}

fn push_waypoint(waypoints: &mut Vec<Waypoint>, point: Waypoint) {
    waypoints.push(point);
    while waypoints.len() > WAYPOINT_CAP {
        waypoints.remove(0);
    }
}

fn median_ms(samples: &[u64]) -> Option<u64> {
    if samples.is_empty() {
        return None;
    }
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 0 {
        Some((sorted[mid - 1] + sorted[mid]) / 2)
    } else {
        Some(sorted[mid])
    }
}

fn system_time_ms(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn store_path() -> PathBuf {
    prefs_dir().join("analytics.json")
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pad(serial: &str, percent: u8, state: PowerState) -> ControllerStatus {
        ControllerStatus {
            index: 1,
            product: "DualSense",
            connection: "USB",
            serial: serial.to_string(),
            percent,
            state,
        }
    }

    fn ms(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn observe_enabled(
        store: &mut AnalyticsStore,
        prev: &[ControllerStatus],
        next: &[ControllerStatus],
        at: SystemTime,
    ) {
        store.observe(prev, next, true, |_| false, at);
    }

    #[test]
    fn charge_from_empty_records_sample() {
        let mut store = AnalyticsStore::default();
        let discharging = vec![pad("a", 5, PowerState::Discharging)];
        let charging = vec![pad("a", 5, PowerState::Charging)];
        let complete = vec![pad("a", 100, PowerState::Complete)];

        observe_enabled(&mut store, &[], &discharging, ms(1_000));
        observe_enabled(&mut store, &discharging, &charging, ms(1_060));
        // Heartbeats during charge (~1 hour).
        for i in 1..=40 {
            observe_enabled(&mut store, &charging, &charging, ms(1_060 + i * 90));
        }
        let last = ms(1_060 + 40 * 90);
        observe_enabled(&mut store, &charging, &complete, last + Duration::from_secs(90));

        let typical = store.typical_charge("a").expect("charge sample");
        assert!(typical.as_secs() >= MIN_CHARGE_SECS);
        assert!(store.open_session("a").is_none());
    }

    #[test]
    fn short_top_up_charge_ignored() {
        let mut store = AnalyticsStore::default();
        let mid = vec![pad("a", 95, PowerState::Discharging)];
        let charging = vec![pad("a", 95, PowerState::Charging)];
        let complete = vec![pad("a", 100, PowerState::Complete)];

        observe_enabled(&mut store, &[], &mid, ms(1_000));
        observe_enabled(&mut store, &mid, &charging, ms(1_060));
        for i in 1..=20 {
            observe_enabled(&mut store, &charging, &charging, ms(1_060 + i * 90));
        }
        observe_enabled(
            &mut store,
            &charging,
            &complete,
            ms(1_060 + 21 * 90),
        );
        assert!(store.typical_charge("a").is_none());
    }

    #[test]
    fn play_from_full_to_mid_records_scaled() {
        let mut store = AnalyticsStore::default();
        let full = vec![pad("a", 100, PowerState::Complete)];
        let unplugged = vec![pad("a", 100, PowerState::Discharging)];
        let mid = vec![pad("a", 40, PowerState::Discharging)];
        let charging = vec![pad("a", 40, PowerState::Charging)];

        observe_enabled(&mut store, &[], &full, ms(1_000));
        observe_enabled(&mut store, &full, &unplugged, ms(1_060));
        // ~2 hours active drain.
        for i in 1..=80 {
            let pct = if i < 40 { 100 } else { 40 };
            let state = vec![pad("a", pct, PowerState::Discharging)];
            let prev_pct = if i == 1 {
                100
            } else if i <= 40 {
                100
            } else {
                40
            };
            let prev = vec![pad("a", prev_pct, PowerState::Discharging)];
            // First transition to 40% at i==40
            let current = if i == 40 { mid.clone() } else { state };
            let previous = if i == 1 {
                unplugged.clone()
            } else if i == 40 {
                vec![pad("a", 100, PowerState::Discharging)]
            } else {
                prev
            };
            observe_enabled(
                &mut store,
                &previous,
                &current,
                ms(1_060 + i * 90),
            );
        }
        let end = ms(1_060 + 80 * 90);
        observe_enabled(&mut store, &mid, &charging, end + Duration::from_secs(90));

        let typical = store.typical_play("a").expect("play sample");
        // 60% used over ~2h → scaled ~3.3h full drain.
        assert!(typical.as_secs() >= MIN_PLAY_SECS);
        assert!(store.open_session("a").is_some()); // charge began
    }

    #[test]
    fn short_100_to_90_ignored() {
        let mut store = AnalyticsStore::default();
        let full = vec![pad("a", 100, PowerState::Complete)];
        let unplugged = vec![pad("a", 100, PowerState::Discharging)];
        let ninety = vec![pad("a", 90, PowerState::Discharging)];
        let charging = vec![pad("a", 90, PowerState::Charging)];

        observe_enabled(&mut store, &[], &full, ms(1_000));
        observe_enabled(&mut store, &full, &unplugged, ms(1_060));
        for i in 1..=30 {
            observe_enabled(&mut store, &unplugged, &unplugged, ms(1_060 + i * 90));
        }
        observe_enabled(
            &mut store,
            &unplugged,
            &ninety,
            ms(1_060 + 31 * 90),
        );
        for i in 32..=50 {
            observe_enabled(&mut store, &ninety, &ninety, ms(1_060 + i * 90));
        }
        observe_enabled(
            &mut store,
            &ninety,
            &charging,
            ms(1_060 + 51 * 90),
        );
        assert!(store.typical_play("a").is_none());
    }

    #[test]
    fn multi_sitting_play_cycle_pauses() {
        let mut store = AnalyticsStore::default();
        let full = vec![pad("a", 100, PowerState::Complete)];
        let unplugged = vec![pad("a", 100, PowerState::Discharging)];
        let eighty = vec![pad("a", 80, PowerState::Discharging)];
        let sixty = vec![pad("a", 60, PowerState::Discharging)];
        let empty = vec![pad("a", 5, PowerState::Discharging)];

        observe_enabled(&mut store, &[], &full, ms(1_000));
        observe_enabled(&mut store, &full, &unplugged, ms(1_060));
        // Sitting 1.
        for i in 1..=20 {
            observe_enabled(&mut store, &unplugged, &unplugged, ms(1_060 + i * 90));
        }
        observe_enabled(&mut store, &unplugged, &eighty, ms(1_060 + 21 * 90));
        // Disconnect (pause).
        observe_enabled(&mut store, &eighty, &[], ms(1_060 + 22 * 90));
        assert_eq!(
            store.open_session("a").map(|o| o.kind),
            Some(SessionKind::PausedDischarge)
        );
        let waypoints_before = store.open_session("a").unwrap().waypoints.len();
        assert!(waypoints_before >= 2);

        // Gap overnight — should not count (large heartbeat gap on reconnect).
        let day2 = ms(1_060 + 22 * 90 + 20 * 60 * 60);
        observe_enabled(&mut store, &[], &eighty, day2);
        assert_eq!(
            store.open_session("a").map(|o| o.kind),
            Some(SessionKind::DischargingFromFull)
        );
        // Sitting 2.
        for i in 1..=20 {
            observe_enabled(&mut store, &eighty, &eighty, day2 + Duration::from_secs(i * 90));
        }
        let after_sit2 = day2 + Duration::from_secs(21 * 90);
        observe_enabled(&mut store, &eighty, &sixty, after_sit2);
        observe_enabled(&mut store, &sixty, &[], after_sit2 + Duration::from_secs(90));

        // Sitting 3 to empty.
        let day3 = after_sit2 + Duration::from_secs(90 + 20 * 60 * 60);
        observe_enabled(&mut store, &[], &sixty, day3);
        for i in 1..=25 {
            observe_enabled(&mut store, &sixty, &sixty, day3 + Duration::from_secs(i * 90));
        }
        let end = day3 + Duration::from_secs(26 * 90);
        observe_enabled(&mut store, &sixty, &empty, end);

        let typical = store.typical_play("a").expect("multi-sitting cycle");
        assert!(typical.as_secs() >= MIN_PLAY_SECS);
        assert!(store.open_session("a").is_none());
    }

    #[test]
    fn reconnect_higher_percent_aborts_paused() {
        let mut store = AnalyticsStore::default();
        let full = vec![pad("a", 100, PowerState::Complete)];
        let unplugged = vec![pad("a", 100, PowerState::Discharging)];
        let eighty = vec![pad("a", 80, PowerState::Discharging)];
        let ninety = vec![pad("a", 90, PowerState::Discharging)];

        observe_enabled(&mut store, &[], &full, ms(1_000));
        observe_enabled(&mut store, &full, &unplugged, ms(1_060));
        observe_enabled(&mut store, &unplugged, &eighty, ms(1_150));
        observe_enabled(&mut store, &eighty, &[], ms(1_240));
        observe_enabled(&mut store, &[], &ninety, ms(1_330));
        assert!(store.open_session("a").is_none());
    }

    #[test]
    fn waypoints_on_events_not_heartbeat_only() {
        let mut store = AnalyticsStore::default();
        let full = vec![pad("a", 100, PowerState::Complete)];
        let unplugged = vec![pad("a", 100, PowerState::Discharging)];

        observe_enabled(&mut store, &[], &full, ms(1_000));
        observe_enabled(&mut store, &full, &unplugged, ms(1_060));
        let after_start = store.open_session("a").unwrap().waypoints.len();
        assert_eq!(after_start, 1);
        for i in 1..=5 {
            observe_enabled(&mut store, &unplugged, &unplugged, ms(1_060 + i * 90));
        }
        assert_eq!(
            store.open_session("a").unwrap().waypoints.len(),
            after_start,
            "heartbeat must not add waypoints"
        );
        let ninety = vec![pad("a", 90, PowerState::Discharging)];
        observe_enabled(&mut store, &unplugged, &ninety, ms(1_060 + 6 * 90));
        assert_eq!(store.open_session("a").unwrap().waypoints.len(), after_start + 1);
    }

    #[test]
    fn ring_keeps_five_samples() {
        let mut ring = vec![1, 2, 3, 4, 5];
        push_sample(&mut ring, 6);
        assert_eq!(ring, vec![2, 3, 4, 5, 6]);
        assert_eq!(median_ms(&ring), Some(4));
    }

    #[test]
    fn prune_stale_and_lru() {
        let mut store = AnalyticsStore::default();
        let now = ms(STALE_SERIAL_DAYS * 24 * 60 * 60 + 10_000);
        let now_ms = system_time_ms(now);
        for i in 0..35 {
            let serial = format!("pad{i:02}");
            store.by_serial.insert(
                serial,
                SerialRecord {
                    last_seen_ms: now_ms.saturating_sub((i as u64 + 1) * 1_000),
                    ..SerialRecord::default()
                },
            );
        }
        // One ancient pad.
        store.by_serial.insert(
            "ancient".into(),
            SerialRecord {
                last_seen_ms: 1,
                ..SerialRecord::default()
            },
        );
        store.dirty = true;
        store.prune(now_ms, |s| s == "pad00");
        assert!(!store.by_serial.contains_key("ancient"));
        assert!(store.by_serial.len() <= SERIAL_CAP);
        assert!(store.by_serial.contains_key("pad00"));
    }

    #[test]
    fn disabled_observe_is_noop() {
        let mut store = AnalyticsStore::default();
        let full = vec![pad("a", 100, PowerState::Complete)];
        let unplugged = vec![pad("a", 100, PowerState::Discharging)];
        store.observe(&full, &unplugged, false, |_| false, ms(1_000));
        assert!(store.by_serial.is_empty());
    }

    #[test]
    fn format_duration_examples() {
        assert_eq!(format_duration_short(Duration::from_secs(40 * 60)), "~40m");
        assert_eq!(
            format_duration_short(Duration::from_secs(2 * 3600 + 15 * 60)),
            "~2h 15m"
        );
        assert_eq!(format_duration_short(Duration::from_secs(3600)), "~1h");
    }

    #[test]
    fn format_eta_ring_is_compact() {
        assert_eq!(
            format_eta_ring(EtaHint::PlayLeft(Duration::from_secs(8 * 3600))),
            "est. 8h"
        );
        assert_eq!(
            format_eta_ring(EtaHint::ChargeToFull(Duration::from_secs(40 * 60))),
            "est. 40m"
        );
        assert_eq!(
            format_eta_ring(EtaHint::PlayLeft(Duration::from_secs(2 * 3600 + 15 * 60))),
            "est. 2h"
        );
    }

    #[test]
    fn eta_scales_with_percent() {
        let mut store = AnalyticsStore::default();
        store.by_serial.insert(
            "a".into(),
            SerialRecord {
                last_seen_ms: 1,
                play_samples_ms: vec![10_000 * 1000], // 10_000s full drain
                charge_samples_ms: vec![3_600 * 1000],
                open: None,
            },
        );
        let discharging = pad("a", 50, PowerState::Discharging);
        match store.eta_for(&discharging) {
            Some(EtaHint::PlayLeft(d)) => assert_eq!(d.as_secs(), 5_000),
            other => panic!("unexpected {other:?}"),
        }
        let charging = pad("a", 75, PowerState::Charging);
        match store.eta_for(&charging) {
            Some(EtaHint::ChargeToFull(d)) => assert_eq!(d.as_secs(), 900),
            other => panic!("unexpected {other:?}"),
        }
    }
}
