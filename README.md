# PS5 Battery Display

System tray app that shows connected DualSense (PS5) controller battery levels, colors each lightbar from battery level, and can identify a controller by flashing its light.

## Features

- Tray icon with a DualSense silhouette
- Tooltip shows how many controllers are connected
- Menu lists each controller (battery % + status); click one to **identify** (white flash ×5)
- Lightbar hue slides **blue → purple → red** as battery drops (updated about once a minute)
- At **≤5% while discharging**, the lightbar periodically pulses **orange**
- Single-instance (second launch exits quietly)
- Logs to a file (see below)

## Build

```bash
cargo build --release
```

Binary: `target/release/ps5-battery-display` (`.exe` on Windows).

## Run

```bash
cargo run --release
# or
./target/release/ps5-battery-display
```

### CLI

| Flag | Description |
|------|-------------|
| `--version` / `-V` | Print version and exit |
| `--install-autostart` | Windows: add a Startup entry for this exe |
| `--uninstall-autostart` | Windows: remove that Startup entry |

## Windows notes

- Release builds use the Windows subsystem (no console window for the tray app).
- Log file: `%APPDATA%\ps5-battery-display\app.log`
- Autostart writes `ps5-battery-display.cmd` into the user Startup folder.

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

### macOS

- Grant Input Monitoring / accessibility only if macOS prompts for HID access
- Autostart is not automated; use Login Items manually if desired

## Battery accuracy

DualSense firmware reports battery in **11 coarse steps** (0–10). Percentages use the Linux mid-point mapping (e.g. step 0 → 5%, step 9 → 95%, step 10/full → 100%).

## License

MIT
