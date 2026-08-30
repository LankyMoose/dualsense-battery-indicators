# Microsoft Store

This package is a full-trust Win32 tray app (`runFullTrust`). Microsoft Store is the only distribution path that skips browser/SmartScreen download warnings; GitHub Releases still use the signed `.exe`.

**Unofficial.** DualSense / PlayStation marks belong to Sony. Certification may ask you to change the listing title or add a stronger disclaimer.

## One-time Partner Center setup

1. Register as an individual or company at [Partner Center](https://partner.microsoft.com/dashboard) (paid developer account).
2. Create a new app and reserve a name.
3. Open **Product management → Product identity** and copy:
   - **Package/Identity name** → `msix/pack.ps1 -IdentityName ...` or repo variable `MSIX_IDENTITY_NAME`
   - **Publisher** (the `CN=...` value) → `-Publisher` / `MSIX_PUBLISHER`
4. Fill listing fields: description, screenshots of the running tray/Configure UI (1920×1080 or 1366×768), support URL (`https://github.com/LankyMoose/dualsense-battery-indicators/issues`), privacy URL (`https://github.com/LankyMoose/dualsense-battery-indicators#privacy-policy`).
5. Age rating questionnaire (this app is a utility; no user-generated content, no account).
6. On first submit, Partner Center will ask why you need **`runFullTrust`**. Suggested text:

   > This is a packaged classic Win32 desktop app (system tray, HID access to DualSense controllers, desktop notifications). Tray icons and raw HID require full trust; it does not run in an AppContainer.

## Build a package

Windows SDK (`makeappx.exe`) required.

```powershell
cargo build --release
pwsh msix/pack.ps1 -Exe target/release/dualsense-battery-indicators.exe
```

Store submission (identity from Partner Center):

```powershell
pwsh msix/pack.ps1 -Exe target/release/dualsense-battery-indicators.exe `
  -IdentityName "YourPartnerCenter.IdentityName" `
  -Publisher "CN=XXXXXXXX-XXXX-XXXX-XXXX-XXXXXXXXXXXX"
```

Upload `target/msix/dualsense-battery-indicators.msix` on the submission **Packages** page. Partner Center re-signs it. You do not need a paid Authenticode cert for the Store package.

## Sideload (optional)

Unsigned MSIX will not install locally. For local testing, sign with a self-signed cert whose subject matches `-Publisher`, then `Add-AppxPackage`.

## Notes

- **Start with Windows** in Configure uses the MSIX startup task, not a Startup-folder shortcut.
- Store and GitHub `.exe` installs are separate; settings files may not be shared.
- GitHub Actions attaches an MSIX to each tagged release using `CN=LankyMoose` unless you set `MSIX_IDENTITY_NAME` / `MSIX_PUBLISHER` repo variables.
