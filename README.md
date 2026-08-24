# DualSense Battery Indicators

System tray app that shows connected DualSense controller battery levels, colors each lightbar from battery level, and can identify a controller by flashing its light.

**Unofficial.** DualSense, PlayStation, and related marks are trademarks of Sony Interactive Entertainment Inc. This project is not affiliated with, endorsed by, or sponsored by Sony.

## Features

- Tray icon with a DualSense silhouette
- Tooltip shows how many controllers are connected
- Menu lists each controller (battery % + status); click one to **identify** (white flash ×5)
- Desktop notifications when a pad **connects** (with battery %), hits **low battery** (≤5% discharging), or **finishes charging** (native **Configure → Settings** menu; on by default)
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
- Autostart writes `dualsense-battery-indicators.lnk` into the user Startup folder (also toggleable from **Configure → Settings**). Older `.cmd` entries are migrated automatically.

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
git tag v0.1.9
git push origin v0.1.9
```

The release workflow attaches `dualsense-battery-indicators.exe` to the GitHub Release for that tag. You can also run the **Release** workflow manually (`workflow_dispatch`).

## License

MIT — see [LICENSE](LICENSE).
