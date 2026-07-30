# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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

[0.1.1]: https://github.com/LankyMoose/dualsense-battery-indicators/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/LankyMoose/dualsense-battery-indicators/releases/tag/v0.1.0
