# Development

## Requirements (as audited on the development PC)

| Tool | Version used |
|---|---|
| Windows | 11, build 26200, x64 |
| Rust | 1.100 stable, MSVC toolchain (`rust-toolchain.toml`) |
| MSVC / Windows SDK | VS Build Tools 2022, SDK 10.0.26100 |
| Android SDK | platforms 34–36, build-tools 36 |
| Android NDK | 28.2.13676358 |
| JDK | 21 (Microsoft build) |
| Gradle | 8.12 (wrapper), AGP 8.10.1, Kotlin 2.2.10 |
| cargo-ndk | 4.1.2 |
| ffmpeg/ffprobe | optional, for checking `.h264` output |

## Commands

```powershell
# everything that runs on a PC (needs an unlocked desktop for live tests)
scripts\test-all.ps1

# host
cargo run --release -p lanlink-host-console            # product core in a console
cargo run --release -p capture-windows --example probe  # which capture backends work here
cargo run --release -p encoder --example capture_encode -- --seconds 5   # writes capture.h264

# Android
scripts\build-android.ps1            # native libs + debug APK
scripts\build-android.ps1 -Release   # release APK (unsigned unless keystore.properties exists)
```

`ANDROID_HOME` is used instead of a committed `local.properties`; the
scripts default it to `%LOCALAPPDATA%\Android\Sdk`.

## Running against a phone

1. On the PC: `cargo run --release -p lanlink-host-console`, then type `p`.
2. Allow inbound UDP 47990 (and 5353 for discovery) on **private** networks:
   review and run `scripts\firewall-allow.ps1` from an elevated PowerShell.
   If Windows lists your Wi-Fi as *Public*, switch it to *Private*.
3. Install the APK on the phone (`adb install android\app\build\outputs\apk\debug\app-debug.apk`),
   open LANLink, scan the QR code (or point the phone's camera app at it).
4. Compare the 6-digit number on both screens, type `y` on the PC.

## Emulator

The emulator reaches the PC's loopback at `10.0.2.2`. The test host makes
that easy and never injects input:

```powershell
cargo run --release -p lanlink-host-console --example test_host -- `
    --i-understand-this-auto-approves --advertise 10.0.2.2
adb shell am start -a android.intent.action.VIEW -d "<PAIRING_URI printed by the test host>"
```

## Debugging

* Host logs: `RUST_LOG=debug` (the console host defaults to `info`).
* Phone: `adb logcat -s LANLinkSession LANLinkDecoder`.
* Live statistics: the phone's performance overlay and Diagnostics screen;
  the host prints them at debug level.
* Capture problems on hybrid laptops: `examples/adapters.rs` prints every
  adapter/output combination and whether duplication works on it.

## Native build details

* Host crates are Windows-only (`#![cfg(windows)]`); the shared crates
  build everywhere.
* Android library: `cargo ndk -t arm64-v8a -t x86_64 -P 29 ... --profile
  release-android` → `android/app/src/main/jniLibs/<abi>/liblanlink.so`
  (≈4 MB each, symbols kept for crash reports; R8 + resource shrinking on
  the Kotlin side; release APK ≈10 MB).
* `ring` compiles C/assembly with the NDK clang that cargo-ndk configures.
