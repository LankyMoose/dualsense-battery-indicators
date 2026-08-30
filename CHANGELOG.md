# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.10] - 2026-08-30

### Added

- Opt-in **Remember** toggle per controller in the tray menu. Remembered pads stay listed after disconnect with their last-known battery % (`controllers.json` beside `prefs.json`). Each controller is a submenu with **Identify** and **Remember**.

### Changed

- Notification toggles in Configure → Settings now sit under a **Notifications** submenu (**On connect**, **When low**, **When charged**). **Start with Windows** remains a top-level Settings item.

## [0.1.9] - 2026-08-25

### Fixed

- Windows autostart no longer flashes a Command Prompt window at login. Startup now uses a `.lnk` shortcut instead of a `.cmd` script; an existing `.cmd` entry is migrated automatically on launch.

## [0.1.8] - 2026-08-25

### Added

- Desktop notification when a DualSense connects, including current battery percent. Toggle via Configure → Settings → **Notify on connect** (on by default; stored in `prefs.json`).

## [0.1.7] - 2026-08-25

### Fixed

- Lightbar color and identify work again while Steam is running: skip the Bluetooth `LIGHT_OUT` claim when `steam.exe` is present (Steam Input already initialized the bar). The 0.1.6 claim sequence is still used when Steam is not running.

## [0.1.6] - 2026-08-05

### Fixed

- Lightbar color and identify now work on Bluetooth without Steam: claim control with a dedicated `LIGHT_OUT` setup report, then set RGB in a **second** report (Linux `hid-playstation` sequence). Combining setup+RGB in one packet left the bar dark or stuck on the default blue.

### Added

- `--set-lightbar R G B` CLI flag to push a color to all connected pads (useful for debugging).

## [0.1.5] - 2026-08-03

### Added

- Tray **Configure** opens a settings window with a native menu bar (**Settings** for notifications/autostart, **Developer** presets when `--dev`) and a three-stop lightbar spectrum editor (full / mid / empty). Spectrum saved in `prefs.json`.

### Changed

- Default lightbar spectrum stops are now `#4141FB` → `#6605BE` → `#BE0000` (full / mid / empty).
- Notification and autostart toggles live in Configure → **Settings** instead of the tray menu.

## [0.1.4] - 2026-08-02

### Added

- Desktop notifications when a controller enters low battery (≤5% discharging) or finishes charging, with tray toggles persisted in `prefs.json`.
- Optional `dev-emulate` Cargo feature and `--dev` flag for emulated controller presets (not included in release builds).

## [0.1.3] - 2026-08-02

### Fixed

- A DualSense connected over both USB and Bluetooth no longer appears twice in the tray. Identity is resolved via the controller MAC (pairing-info feature report) when Windows leaves the USB HID serial empty, and the USB path is preferred.

### Added

- `--list-controllers` CLI flag to print connected pads (connection, identity, battery) and exit.

## [0.1.2] - 2026-07-30

### Fixed

- Tray no longer freezes or thrash-refreshes when a DualSense is visible to HID but cannot be read (powered off, sleeping, or exclusively held).
- Disconnect detection is much faster: clear immediately when HID drops the pad, and re-probe connected pads every few seconds so a Bluetooth power-off is reflected promptly.
- Battery HID reads fail faster so a dead pad cannot block polling for tens of seconds.

## [0.1.1] - 2026-07-30

### Fixed

- Lightbar color and identify now work when Steam is not running, by sending DualSense lightbar setup flags (`valid_flag2` / `LIGHT_OUT`) with every RGB output report.

## [0.1.0] - 2026-07-30

### Added

- System tray app for DualSense / DualSense Edge battery status (USB and Bluetooth).
- Battery-driven lightbar colors (blue → purple → red) and identify flash from the tray menu.
- Low-battery orange pulse while discharging at ≤5%.
- Connect/disconnect detection via periodic presence scan.
- Windows autostart (tray toggle and CLI flags).
- File logging, single-instance guard, `--help` / `--version`.
- Embedded DualSense silhouette for the tray and `.exe` icon.
- Windows CI and tagged release workflow.

[0.1.10]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.9...v0.1.10
[0.1.9]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.8...v0.1.9
[0.1.8]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.7...v0.1.8
[0.1.7]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.6...v0.1.7
[0.1.6]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.5...v0.1.6
[0.1.5]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.4...v0.1.5
[0.1.4]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.3...v0.1.4
[0.1.3]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.2...v0.1.3
[0.1.2]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.1...v0.1.2
[0.1.1]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/LankyMoose/dualsense-battery-indicators/releases/tag/v0.1.0
