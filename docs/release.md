# Release Guide

## Production Artifacts

Run the automated release packaging script from PowerShell:

```powershell
scripts\package-release.ps1
```

This builds, tests, and packages all release artifacts into `dist\`:

| Artifact | Path in `dist\` | Contents / Details |
|---|---|---|
| Windows x64 Portable Archive | `dist\lanlink-v0.1.0-windows-x64.zip` | `lanlink-host.exe` (GUI + system tray), `lanlink-host-console.exe` (CLI host), `firewall-allow.ps1`, `README.md`, `LICENSE-MIT`, `LICENSE-APACHE` |
| Android Production APK | `dist\lanlink-v0.1.0-android.apk` | Signed, R8-minified, resource-shrunk universal APK (`arm64-v8a` + `armeabi-v7a` + `x86_64`, API 29+) |
| SHA-256 Checksums | `dist\SHA256SUMS.txt` | Cryptographic SHA-256 digests for all published files |

Individual build scripts:

| Artifact | Command | Output |
|---|---|---|
| Windows host binaries | `scripts\build-host.ps1` | `target\release\lanlink-host.exe`, `target\release\lanlink-host-console.exe` |
| Android release APK | `scripts\build-android.ps1 -Release` | `android\app\build\outputs\apk\release\app-release.apk` |

## Code Signing

- **Android:** Place your upload keystore **outside** the repository and configure `android\keystore.properties` with `storeFile`, `storePassword`, `keyAlias`, and `keyPassword` (`android\keystore.properties` and `*.jks` / `*.keystore` are git-ignored). When `keystore.properties` is absent, Gradle falls back to the local signing key so local release builds remain directly installable for testing.
- **Windows:** Sign `lanlink-host.exe` and `lanlink-host-console.exe` with `signtool.exe` using an Authenticode certificate so Windows SmartScreen and firewall prompts identify the publisher.

## Pre-Release Verification Checklist

1. `scripts\test-all.ps1` passes completely (`cargo fmt --check`, `cargo clippy -D warnings`, all Rust unit/property/integration tests with an unlocked desktop, Android unit tests, and Android lint with 0 errors).
2. Real-phone verification recorded in [docs/implementation-status.md](implementation-status.md).
3. Dependency and license check: review `Cargo.lock` and `android/app/build.gradle.kts` against [ADR 0010](adr/0010-licensing.md).
4. Verify packaged artifacts: publish `SHA256SUMS.txt` alongside the archive and APK on GitHub Releases.

## Protocol Compatibility & Rollback

The wire protocol is explicitly versioned (ALPN `lanlink/1` + version range in `Hello`, currently negotiating v1–v2). If a peer does not support v2 extensions (`TextInput` / `Caret`), the session transparently negotiates v1. Incompatible major generations fail cleanly with an explicit update message.

To roll back a release, reinstall the previous APK and Windows binaries; on-disk identity and trust files (`LLID` v1 blob and `lanlink-trust v1`) maintain forward and backward compatibility across v0.1.x.
