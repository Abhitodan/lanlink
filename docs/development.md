# Development Guide

## Toolchain Requirements

| Tool | Tested Version |
|---|---|
| Windows | Windows 11 x64 (build 26200) |
| Rust | `1.85+` stable, MSVC toolchain (`rust-toolchain.toml`) |
| MSVC / Windows SDK | Visual Studio Build Tools 2022, Windows 11 SDK `10.0.26100` |
| Android SDK | Platforms 34–36, Build-Tools 36 (`compileSdk = 35`, `targetSdk = 35`, `minSdk = 29`) |
| Android NDK | `28.2.13676358` |
| JDK | OpenJDK 21 (Temurin / Microsoft build; JDK 17+ supported) |
| Gradle / Kotlin | Gradle `8.12` (wrapper), AGP `8.10.1`, Kotlin `2.2.10` |
| `cargo-ndk` | `4.1.2` (`cargo install cargo-ndk`) |
| `ffmpeg` / `ffprobe` | Optional, for inspecting raw `.h264` capture dumps |

## Common Commands

```powershell
# Run all automated checks (format, clippy -D warnings, Rust tests, Android unit tests + lint)
scripts\test-all.ps1

# Build Windows host binaries (target\release\lanlink-host.exe & lanlink-host-console.exe)
scripts\build-host.ps1

# Build Android native libraries (arm64-v8a + x86_64) and APK
scripts\build-android.ps1            # Debug APK
scripts\build-android.ps1 -Release   # Release APK (R8-minified, resource-shrunk)

# Build complete release bundle in dist\ with SHA256SUMS.txt
scripts\package-release.ps1

# Hardware capture & encode diagnostic probes
cargo run --release -p capture-windows --example probe
cargo run --release -p capture-windows --example adapters
cargo run --release -p encoder --example capture_encode -- --seconds 5
```

`ANDROID_HOME` is read from the environment and defaults to `%LOCALAPPDATA%\Android\Sdk` in the PowerShell build scripts (no committed `local.properties` required).

## Running with a Physical Phone

1. Start the Windows host: run `target\release\lanlink-host.exe` (GUI) or `cargo run --release -p lanlink-host-console` (CLI, press `p` to open pairing).
2. Allow inbound `UDP 47990` (session transport) and `UDP 5353` (mDNS discovery) on **Private** networks:
   ```powershell
   # Run once in an elevated PowerShell prompt
   scripts\firewall-allow.ps1
   ```
   Ensure Windows lists your home/office Wi-Fi profile as *Private* rather than *Public*.
3. Install the APK on your phone:
   ```powershell
   adb install -r android\app\build\outputs\apk\release\app-release.apk
   ```
4. Open **LANLink** on the phone, scan the QR code displayed on the PC, verify the 6-digit confirmation number on both screens, and approve on the PC.

## Running with the Android Emulator

The Android Emulator reaches the host PC's loopback interface at `10.0.2.2`. A dedicated `test_host` example is included for emulator verification (it never injects input into your desktop):

```powershell
cargo run --release -p lanlink-host-console --example test_host -- `
    --i-understand-this-auto-approves --advertise 10.0.2.2
adb shell am start -a android.intent.action.VIEW -d "<PAIRING_URI printed by test_host>"
```

## Debugging & Telemetry

- **Windows host log file:** `%LOCALAPPDATA%\LANLink\lanlink-host.log` (or set `RUST_LOG=debug` when running from a terminal).
- **Android logcat tags:** `adb logcat -s LANLinkSession LANLinkDecoder LANLinkKeyboard`.
- **In-app diagnostics:** Enable **Show performance overlay** in Settings or open **Performance diagnostics** from the session control panel to inspect live RTT, capture/encode/decode percentiles, bitrate, and frame counters.

## Native Build Details

- Windows host crates (`capture-windows`, `encoder`, `input-windows`, `session-manager`, `lanlink-host`, `lanlink-host-console`) are gated with `#![cfg(windows)]`.
- Shared crates (`lanlink-protocol`, `lanlink-auth`, `lanlink-transport`, `lanlink-discovery`, `lanlink-client`, `lanlink-mobile`) are cross-platform.
- Android JNI library (`android/native-bridge`): built via `cargo ndk -t arm64-v8a -t x86_64 -P 29 -o android/app/src/main/jniLibs build --profile release-android -p lanlink-android` (`opt-level = "s"`, `lto = "fat"`, `panic = "unwind"`).
