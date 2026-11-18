# Release

**Status: no release has been made.** The steps below are the procedure; the
items marked ☐ have not been done yet.

## Artifacts

| Artifact | Command | Notes |
|---|---|---|
| Windows host | `scripts\build-host.ps1` | `lanlink-host-console.exe`; no installer yet (☐) |
| Android APK | `scripts\build-android.ps1 -Release` | unsigned unless `android\keystore.properties` exists |

## Signing (☐)

* Android: create an upload keystore **outside** the repository and point
  `android\keystore.properties` at it (`storeFile`, `storePassword`,
  `keyAlias`, `keyPassword`). The file is git-ignored. Never commit keys.
* Windows: sign `lanlink-host-console.exe` (and a future installer) with an
  Authenticode certificate so SmartScreen and firewall prompts show the
  publisher.

## Checks before tagging

1. `scripts\test-all.ps1` passes (fmt, clippy `-D warnings`, all Rust tests
   with an unlocked desktop, Android unit tests, Android lint).
2. Real-phone test pass recorded in `implementation-status.md` (☐ — no
   phone has been tested yet).
3. Dependency audit: `cargo audit` and `cargo deny check licenses` (☐ tools
   not installed on the development PC); review `Cargo.lock` changes.
4. Licence report: `cargo about` or `cargo deny list` for Rust, Gradle
   `dependencies` for Android (☐).
5. Verify artifacts: SHA-256 checksums published next to downloads;
   `apksigner verify --print-certs` on the APK.

## Rollback

The protocol is versioned (ALPN `lanlink/1` + version range in Hello), so an
older phone app and a newer host negotiate or fail with a clear "update both
apps" error. To roll back, reinstall the previous APK and executable; the
identity and paired-device files are forward and backward compatible within
format version 1 (`LLID` v1 blob, `lanlink-trust v1` text).
