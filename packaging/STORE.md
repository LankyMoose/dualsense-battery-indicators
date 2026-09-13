# Partner Center checklist (out-of-band)

These steps cannot be done from the repo. Do them in your browser / Partner Center.

## Account

1. Open [storedeveloper.microsoft.com](https://storedeveloper.microsoft.com) and create a **free Individual** developer account (government ID + selfie).
2. Reserve the app name **SDSC Utils** (exact Store title).

## Listing copy

- **Title:** SDSC Utils
- **Short description:** System tray battery and lightbar utilities for DualSense controllers.
- **Compatibility (body):** Works with DualSense wireless controllers and DualSense Edge wireless controllers.
- **Disclaimer:** Unofficial. DualSense, DualSense Edge, PlayStation, and related marks are trademarks of Sony Interactive Entertainment Inc. This app is not affiliated with, endorsed by, or sponsored by Sony.
- **Privacy policy URL:** the Privacy policy section of the GitHub README (`https://github.com/LankyMoose/sdsc-utils#privacy-policy` after the repo rename).
- **Screenshots:** capture locally from a running build (Store requires them).
- **Age rating:** complete the questionnaire (expect Everyone / suitable for all ages for this utility).

## Capabilities justification

When asked about `runFullTrust`: tray icon + raw DualSense HID (`hidapi`) for battery, lightbar, and identify.

## First upload

1. Download `sdsc-utils.msix` from the Release workflow artifact (or pack locally — see [README.md](README.md)).
2. Confirm the package identity matches Partner Center (`LankyMoose.SDSCUtils` / publisher CN in [`AppxManifest.xml`](AppxManifest.xml)).
3. Submit for certification.

Do **not** expand SDSC as “Sony DualSense Controller Utilities” anywhere in the listing.
