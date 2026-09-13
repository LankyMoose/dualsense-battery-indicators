# Packaging notes (SDSC Utils)

## Sideload vs Store

- [`AppxManifest.xml`](AppxManifest.xml) uses a dummy `Publisher="CN=SDSCUtils-Sideload"`.
- After you associate the app with Partner Center, Visual Studio / the Store packaging tools rewrite **Identity Name** and **Publisher** to the Store-assigned values. Do not invent a production CN here.
- `pack-msix.ps1` stamps `Identity Version` from `Cargo.toml` (`1.3.0` → `1.3.0.0`) so each tagged upload is a newer package for Store auto-update.

## Local pack

```powershell
cargo build --release
./packaging/pack-msix.ps1 -ExePath target/release/sdsc-utils.exe -OutDir target/msix
```

Requires the Windows SDK (`makeappx.exe`). Logos are generated into `packaging/Assets/` at pack time (`cargo run --bin gen_msix_logos`).

## Store submission

See [STORE.md](STORE.md) for the Partner Center checklist (account, listing copy, first upload).

## Autostart

`desktop:StartupTask` TaskId `SdscUtilsStartup` must stay in sync with [`src/autostart.rs`](../src/autostart.rs). Manifest default is `Enabled="false"` (user opts in via Settings).
