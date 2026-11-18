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

## iPhone app

The iPhone app (`ios/`) is built and tested by CI on GitHub's `macos-15` runners (`.github/workflows/ios.yml`, workflow `ios`). On every pull request it:

1. adds the Rust targets `aarch64-apple-ios`, `aarch64-apple-ios-sim` and `x86_64-apple-ios` and runs `cargo test -p lanlink-mobile -p lanlink-ios`;
2. checks the C header (`python3 scripts/check-ios-header.py`);
3. builds `ios/build/LanLinkCore.xcframework` with `scripts/build-ios-core.sh` (deployment target iOS 16.0);
4. generates the Xcode project with `xcodegen` and runs the unit and UI tests on an iPhone simulator;
5. builds for `generic/platform=iOS` with `CODE_SIGNING_ALLOWED=NO`, which proves the app compiles for iPhone hardware without signing it;
6. uploads the `.xcresult` bundle (`ios-test-results`) and the UI-test screenshots (`ios-screenshots`) as artifacts.

Download the screenshots with `gh run download <run-id> -n ios-screenshots`.

### Local build on a Mac

Requires macOS with Xcode, Rust, `xcodegen` (`brew install xcodegen`) and the iOS targets:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cargo test -p lanlink-mobile -p lanlink-ios
IPHONEOS_DEPLOYMENT_TARGET=16.0 scripts/build-ios-core.sh   # -> ios/build/LanLinkCore.xcframework
(cd ios && xcodegen)                                       # -> ios/LANLink.xcodeproj
open ios/LANLink.xcodeproj
```

Command-line tests, as CI runs them:

```sh
xcodebuild test -project ios/LANLink.xcodeproj -scheme LANLink \
  -destination "platform=iOS Simulator,name=iPhone 16"
```

### Running on a real iPhone

The repository signs nothing. In Xcode, select the `LANLink` target, choose your own Apple development team under Signing & Capabilities, set a unique bundle identifier, and run on the connected iPhone. Allow the Local Network permission at the first prompt so the app can find and reach the PC. The phone and the PC must be on the same Wi-Fi, with the same firewall rules as for Android. The app has not been run on a real iPhone yet (see [implementation-status.md](implementation-status.md#iphone-app-ios-app-branch)).

## Debugging & Telemetry

- **Windows host log file:** `%LOCALAPPDATA%\LANLink\lanlink-host.log` (or set `RUST_LOG=debug` when running from a terminal).
- **Android logcat tags:** `adb logcat -s LANLinkSession LANLinkDecoder LANLinkKeyboard`.
- **In-app diagnostics:** Enable **Show performance overlay** in Settings or open **Performance diagnostics** from the session control panel to inspect live RTT, capture/encode/decode percentiles, bitrate, and frame counters.

## Native Build Details

- Windows host crates (`capture-windows`, `encoder`, `input-windows`, `session-manager`, `lanlink-host`, `lanlink-host-console`) are gated with `#![cfg(windows)]`.
- Shared crates (`lanlink-protocol`, `lanlink-auth`, `lanlink-transport`, `lanlink-discovery`, `lanlink-client`, `lanlink-mobile`) are cross-platform.
- iOS C-ABI library (`ios/native-bridge`, crate `lanlink-ios`): built by `scripts/build-ios-core.sh` with `--profile release-ios` for the three iOS targets and packaged as `LanLinkCore.xcframework` (device + simulator).
- Android JNI library (`android/native-bridge`): built via `cargo ndk -t arm64-v8a -t x86_64 -P 29 -o android/app/src/main/jniLibs build --profile release-android -p lanlink-android` (`opt-level = "s"`, `lto = "fat"`, `panic = "unwind"`).
