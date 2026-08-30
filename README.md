# DualSense Battery Indicators

System tray app that shows connected DualSense controller battery levels, colors each lightbar from battery level, and can identify a controller by flashing its light.

**Unofficial.** DualSense, PlayStation, and related marks are trademarks of Sony Interactive Entertainment Inc. This project is not affiliated with, endorsed by, or sponsored by Sony.

## Features

- Tray icon with a DualSense silhouette
- Tooltip shows how many controllers are connected
- Menu lists each controller in a submenu with battery %, **Identify** (flash lightbar), and an opt-in **Remember** toggle
- **Remember** a controller to keep it in the menu after disconnect with its last-known charge % (off by default)
- Desktop notifications when a pad **connects** (with battery %), hits **low battery** (≤5% discharging), or **finishes charging** (Configure → Settings → **Notifications**; on by default)
- **Start with Windows** autostart toggle in the Configure window’s **Settings** menu (Windows)
- Detects controllers connecting/disconnecting within a few seconds
- Lightbar color blends across a customizable **3-stop spectrum** (default **blue → purple → red**) as battery drops (updated about once a minute); edit via tray **Configure**
- At **≤5% while discharging**, the lightbar periodically pulses **orange**
- Single-instance (second launch exits quietly)
- Logs to a file (see Troubleshooting)

## Build

Requires Rust **1.85+** (edition 2024).

```bash
cargo build --release
```

Binary: `target/release/dualsense-battery-indicators` (`.exe` on Windows).

Release builds do **not** include the developer emulator (`dev-emulate` is off by default).

To rename the app later, change `package.name` in `Cargo.toml` and `DISPLAY_NAME` in `src/app_meta.rs` (runtime paths follow the package name).

## Run

```bash
cargo run --release
# or
./target/release/dualsense-battery-indicators
```

### Developer emulator (optional)

For testing notifications without real hardware, build with the `dev-emulate` feature and pass `--dev`:

```bash
cargo run --features dev-emulate -- --dev
```

That unlocks a **Developer** menu in the **Configure** window with emulated controller presets (low battery, charging, fully charged, etc.). Emulation is not compiled into normal release binaries.

### CLI

| Flag | Description |
|------|-------------|
| `-h` / `--help` | Print usage and exit |
| `-V` / `--version` | Print version and exit |
| `--install-autostart` | Windows: add a Startup entry for this exe |
| `--uninstall-autostart` | Windows: remove that Startup entry |
| `--list-controllers` | Print connected DualSense pads and exit |
| `--dev` | Enable Developer menu in Configure (only when built with `--features dev-emulate`) |

## Windows notes

- Release builds use the Windows subsystem (no console window for the tray app).
- The `.exe` and tray share the same DualSense silhouette icon (embedded at build time via `winres`).
- Log file: `%APPDATA%\dualsense-battery-indicators\app.log`
- Prefs file: `%APPDATA%\dualsense-battery-indicators\prefs.json` (notification toggles + lightbar spectrum)
- Remembered controllers: `%APPDATA%\dualsense-battery-indicators\controllers.json`
- Autostart writes `dualsense-battery-indicators.lnk` into the user Startup folder (also toggleable from **Configure → Settings**). Older `.cmd` entries are migrated automatically. Microsoft Store / MSIX builds use a Windows startup task instead.

## Platform support

| Platform | Status |
|----------|--------|
| Windows | Primary / tested |
| macOS | Expected to work (`hidapi` + `tray-icon`) |
| Linux | Expected to work; needs GTK for the tray (`tray-icon` gtk feature) and system `hidapi`/udev rules for DualSense access |

### Linux dependencies (typical)

- GTK 3 development libraries (for tray)
- `libhidapi` / pkg-config as required by the `hidapi` crate
- Permission to open the DualSense HID device (udev rule or group membership)
- A desktop notification daemon (e.g. the usual DE notification service) for toasts

### macOS

- Grant Input Monitoring / accessibility only if macOS prompts for HID access
- macOS may prompt once for notification permission
- Autostart is not automated; use Login Items manually if desired

## Battery accuracy

DualSense firmware reports battery in **11 coarse steps** (0–10). Percentages use the Linux mid-point mapping (e.g. step 0 → 5%, step 9 → 95%, step 10/full → 100%).

## Troubleshooting

- **Log file**
  - Windows: `%APPDATA%\dualsense-battery-indicators\app.log`
  - Unix: `$XDG_STATE_HOME/dualsense-battery-indicators/app.log` or `~/.local/state/dualsense-battery-indicators/app.log`
- **Second launch does nothing** — only one instance is allowed; the second process exits after logging.
- **Exe icon looks stale in Explorer** — rebuild release, then refresh the folder or restart Explorer (Windows caches icons).
- **Controller not listed** — wait a few seconds after power-on (presence is scanned every 3s); check the log if open/read fails.
- **Tray slow to show disconnect** — fixed in 0.1.2 (faster liveness probes). Bluetooth pads can linger in Windows HID briefly after power-off.
- **Same controller listed twice (USB + Bluetooth)** — fixed in 0.1.3 (MAC-based identity; USB preferred).
- **Lightbar stuck off or default blue** — fixed in 0.1.6 (separate `LIGHT_OUT` claim, then RGB). If colors stop updating while **Steam is running**, update to 0.1.7 (skips the claim when Steam is open). Test with `--set-lightbar 255 100 0`. If it still fails, check the log for HID write errors.

## Releases

See [CHANGELOG.md](CHANGELOG.md) for release notes.

CI builds on Windows. To publish a binary:

```bash
git tag v0.1.10
git push origin v0.1.10
```

The release workflow attaches `dualsense-battery-indicators.exe` and `dualsense-battery-indicators.msix` to the GitHub Release for that tag. You can also run the **Release** workflow manually (`workflow_dispatch`).

Until SignPath is connected, the `.exe` is unsigned. After that, GitHub Releases ship an Authenticode-signed `.exe` (publisher **SignPath Foundation**). The `.msix` is for [Microsoft Store](msix/README.md) submission; Partner Center re-signs it.

## Code signing policy

Free code signing provided by [SignPath.io](https://signpath.io/), certificate by [SignPath Foundation](https://signpath.org/).

- **Authors:** [LankyMoose](https://github.com/LankyMoose) (repository owner)
- **Reviewers:** [LankyMoose](https://github.com/LankyMoose)
- **Approvers:** [LankyMoose](https://github.com/LankyMoose) (each SignPath signing request is approved in the SignPath dashboard)

GitHub and SignPath accounts used for this project must have multi-factor authentication enabled.

### One-time SignPath setup

1. Apply for the [SignPath Foundation open-source program](https://signpath.org/) with this repository: https://github.com/LankyMoose/dualsense-battery-indicators (MIT, public, already shipping GitHub Releases).
2. After approval, in the SignPath dashboard create a project with slug `dualsense-battery-indicators` and a signing policy slug `release-signing`. Artifact configuration: Windows `.exe`, product name **DualSense Battery Indicators**, product version matching the release.
3. Install the SignPath GitHub App on this repository.
4. In repo **Settings → Secrets and variables → Actions**:
   - Secret `SIGNPATH_API_TOKEN` (Submitter role on that policy)
   - Variable `SIGNPATH_ORGANIZATION_ID`
5. The next `v*` tag waits up to 60 minutes for you to **approve the signing request** in SignPath, then attaches the signed exe.

Until the token is set, releases still publish; they just stay unsigned.

## Privacy policy

This program will not transfer any information to other networked systems unless specifically requested by the user or the person installing or operating it.

Battery status, notification preferences, lightbar colors, and remembered controllers stay on the local machine (`prefs.json`, `controllers.json`, and `app.log` under the app data directory). Desktop toasts are shown by the OS. There is no telemetry, account, or network API.

## Microsoft Store

See [msix/README.md](msix/README.md) for Partner Center identity, `runFullTrust` justification, screenshots, and how to build `dualsense-battery-indicators.msix`.

## License

MIT — see [LICENSE](LICENSE).
