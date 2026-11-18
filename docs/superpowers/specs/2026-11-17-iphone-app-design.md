# LANLink for iPhone: design

Status: draft for approval, 2026-11-17.

## Intent

The user wants LANLink on iPhone: "the same exact application ... all the UI and aspects". An iPhone user gets what an Android user gets today:
- the same screens;
- the same gestures, compact keyboard and caret following;
- the same security;
- connection to the same Windows PC app (`lanlink-host`), with no change on the PC.

## Goals

Parity with the Android app as of 2026-11-17 (see `docs/implementation-status.md`):

1. **Screens.** Onboarding, Home (paired PCs and PCs found on the network), PC detail, Pair, Remote, Settings, Diagnostics and Security, with the same copy, structure and palette.
2. **Pairing.**
   - QR scan in the app, and a manual address + 8-digit code.
   - `lanlink://pair` links show the "Pair with …?" confirmation and never pair on their own.
   - The 6-digit confirmation number, and the "waiting for approval on the PC" state.
   - The friendly error messages, including "unverified" (manual pairing never trusts an unverified "already paired" reply).
3. **Remote view.**
   - Hardware H.264 decode.
   - Letterboxed or fill view, landscape while controlling, immersive full screen.
   - Trackpad mode by default at 2× with the follow camera; direct-touch mode.
   - Zoom presets Fit / 2× / 1:1, pinch zoom.
   - Two-finger scroll, two-finger tap for right-click, press-and-drag.
   - Three-finger tap for the keyboard.
   - Click haptics and the cursor overlay drawn from host cursor shapes.
4. **Control rail.**
   - Right edge by default, left as a setting.
   - Auto-hides to a thin tab after 4 s and stays out while the keyboard or panel is open.
   - Buttons: keyboard, trackpad/touch, zoom, More.
   - The More panel: stream quality, switch display, performance overlay, Diagnostics, Disconnect, plus a shortcut row (Esc, Tab, Ctrl, Alt, Shift, Win, Del, Home, End, PgUp, PgDn, F1–F12, Ctrl+C/V/Z, Alt+Tab, Win+D).
5. **Keyboards.**
   - LANLink's compact keyboard is the default: same layout, layers, one-shot Shift, caps lock on double tap, key repeat, sticky Ctrl/Alt/Win, key-height setting.
   - The system keyboard is an option, with the shortcut bar above it.
   - The view slides so the PC's text caret (protocol v2 `Caret`) stays visible above whichever keyboard is open.
6. **Discovery.** PCs advertising `_lanlink._udp` appear under "On this network", with names cleaned (`DisplayText`).
7. **Session behavior.**
   - Automatic reconnection: addresses that answer with a different identity are skipped, not fatal.
   - Adaptive stream parameters and client stats sent to the PC.
   - Held keys released on disconnect.
   - View-only mode; keep the screen on during a session.
8. **Security parity.**
   - The identity key is stored so it never leaves the device and is excluded from backup.
   - Pairing URIs pointing outside the local network are rejected.
   - The PC screen is protected from capture as far as iOS allows (see "Platform differences").
   - The keyboard is asked not to learn typed text.
9. **Same tests.** Every Android core-logic unit test has a Swift twin with the same cases, so both apps are held to the same expected behavior.

## Non-goals

- App Store or TestFlight distribution. It needs an Apple Developer account, which the user does not have yet.
- An iPad-optimised layout. The app runs on iPad in the iPhone layout, but nothing is tuned for it.
- Features the Android app does not have: audio, clipboard, other codecs.
- Running a session in the background. iOS suspends the app, just as Android ends the session when the app leaves the screen.

## Anti-goals (failure even if it "works")

- Claiming the app works on an iPhone, or with a real PC, without evidence. CI on a simulator is the evidence available; nothing beyond it is claimed.
- A protocol fork, or any change that makes the PC treat iPhones differently.
- Weakening any Android security fix to make sharing easier.
- Regressing the Android app. The Android tests, the end-to-end tests and lint must still pass after the shared-core refactor.

## Constraints

- **No Mac here.** All iOS compiling, testing and packaging happens on GitHub Actions macOS runners. The code lives in a **public** GitHub repository created for this. The user chose GitHub's Macs, then a public repository for free builds.
- **Unsigned builds only.** CI builds for the iOS Simulator, plus an unsigned device build to prove the device target compiles. Installing on a real iPhone later needs a Mac with Xcode (free signing) or an Apple Developer account (TestFlight).
- **No Windows host on CI.** CI cannot connect to a real `lanlink-host`. Connection logic is tested against a fake host built from the cross-platform crates (`lanlink-protocol`, `lanlink-auth`, `lanlink-transport`).
- **Minimum iOS 16.** Needed for `NavigationStack`, `UIWindowScene.requestGeometryUpdate` and modern Network.framework. Swift 5.9+, built with Xcode 16 on `macos-15` runners.
- **Same licences policy.** Permissive dependencies only (ADR 0010). No new Swift package dependencies are planned.

## Approaches considered

| | Approach | Trade-off | Decision |
|---|---|---|---|
| A | Native SwiftUI app (UIKit for the remote view). The shared Rust core is reached through a thin C bridge. The JNI bridge's logic moves into one shared crate. Gesture, viewport and keyboard logic is ported to Swift with mirrored tests | that logic exists twice, kept in step by identical test cases | **chosen by the user** ("proceed" after A was recommended) |
| B | A, plus gesture, viewport and keyboard logic moved into Rust for both phones | one copy, but rewrites Android code the user just tuned on a real phone, with no visible gain there | rejected: risk to the app that already works |
| C | Compose Multiplatform shared UI | less native on iPhone (keyboard, gestures, video layer); restructures the Android build | rejected |

## Architecture

```
crates/lanlink-mobile      NEW  platform-neutral bridge core (Rust)
android/native-bridge      thin JNI over lanlink-mobile (behavior unchanged)
ios/native-bridge          NEW  thin C ABI over lanlink-mobile -> LanLinkCore.xcframework
ios/LANLink                NEW  SwiftUI app (project.yml for XcodeGen)
ios/LANLinkTests           NEW  XCTest: Swift twins of CoreLogicTest + bridge tests
ios/LANLinkUITests         NEW  XCUITest: walks the screens, attaches screenshots
.github/workflows/ios.yml  NEW  macOS CI
```

### 1. Shared bridge core: `crates/lanlink-mobile`

- **What moves.** Everything in `android/native-bridge/src/lib.rs` that is not JNI glue moves here:
  - the Tokio runtime;
  - the pairing-job registry, with abort on cancel;
  - the session registry keyed by opaque `i64` handles;
  - the event queue and its JSON encoding;
  - cursor-shape storage;
  - video state: held frame, parameter-set extraction, `sent_params`, decoder reset;
  - local-address filtering of pairing URIs;
  - input chunking for long text.
- **API.** A plain Rust API with `Result<T, String>` user-facing errors. Example: `pair_start(...) -> Result<i64, String>`, `next_video(handle, &mut [u8], &mut [i64; 8], timeout_ms) -> Result<i32, String>`. Return codes and `meta` layout stay exactly as today (`VIDEO_*`, `META_*`).
- **Tests.**
  - The existing bridge unit tests move with the code.
  - A new fake-host integration test covers pair, connect, session start, video records arriving, and input reaching the fake host. It builds on the cross-platform crates, so it runs on Windows and macOS CI.

### 2. Android bridge after the refactor

- **Shape.** `android/native-bridge` keeps its exact JNI signatures. Each function becomes: convert arguments, call `lanlink-mobile`, convert the result or throw `IOException`. `NativeBridge.kt` does not change.
- **Done when:**
  - Android unit tests and lint pass;
  - the Rust workspace tests pass, including the end-to-end tests;
  - the APK builds;
  - a manual phone check is listed as pending, not claimed.

### 3. iOS C bridge: `ios/native-bridge` (crate `lanlink-ios`, `staticlib`)

- **Functions.** One C function per JNI function, named `lanlink_<name>`.
- **Errors.** Every fallible function returns `int32_t` (0 = ok, negative = error) and takes a `char **error_out`. The error is a Rust-allocated UTF-8 string, freed with `lanlink_string_free`.
- **Data out.** Strings and byte arrays returned to Swift are Rust-allocated, with matching free functions.
- **Video.** `nextVideo` writes into a caller-provided buffer (`uint8_t *buf, size_t cap`), as the JNI version writes into a direct `ByteBuffer`.
- **Header.** A hand-written header, `include/lanlink.h`, with a CI step that fails if a Rust `extern "C"` function is missing from the header or vice versa. Using cbindgen instead is left to the builder.
- **Packaging.** Built for `aarch64-apple-ios`, `aarch64-apple-ios-sim` and `x86_64-apple-ios`, packaged with `xcodebuild -create-xcframework` into `LanLinkCore.xcframework` by `scripts/build-ios-core.sh`. Every entry point catches panics.

### 4. The Swift app

Structure mirrors the Android packages, so the two can be diffed by eye.

- **`Core/`** (pure Swift, unit-tested):
  - Logic ports: `Viewport`, `TouchInterpreter`, `TrackpadCursor`, `CompactLayout` and `ShiftState`, `KeyMap` (Windows VK codes), `DisplayText`.
  - The ports keep identical constants: `HOLD_MS = 250`, `THIRD_FINGER_MS = 150`, `REVEAL_AT = 0.75`, `TYPING_ZOOM = 1.5`, repeat 400/55 ms, `DOUBLE_TAP_MS = 350`, follow-camera dead zone, precision curve.
  - `Stores`:
    - `Settings`: UserDefaults, same keys and defaults.
    - `HostRepository`: UserDefaults JSON, same schema.
    - `IdentityStore`: the identity blob in the Keychain with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, never synced to iCloud. That attribute is the equivalent of Android's Keystore wrapping plus backup exclusion.
  - `Discovery`: `NWBrowser` for `_lanlink._udp` with TXT records; failure and retry behave as on Android after fix E4.
  - `Bridge`: a Swift wrapper over the C API that throws `LANLinkError(message)` and frees every Rust allocation.
- **`Session/`:**
  - `RemoteSession`: the connect loop, reconnect, impostor-skip, event loop, stats, and stream planning (same quality presets).
  - `VideoDecoder`:
    - Display: an `AVSampleBufferDisplayLayer`.
    - Format: Annex-B access units become AVCC; SPS/PPS become a `CMVideoFormatDescription`.
    - Timing: each frame is marked display-immediately, which gives the lowest latency and no presentation clock.
    - Recovery: the layer is flushed and a keyframe requested on error or discontinuity, mirroring the Android decoder.
  - `RemoteView`:
    - A UIKit view hosting the display layer and a cursor overlay layer.
    - Raw `touchesBegan/Moved/Ended` feed `TouchInterpreter` exactly as Android's `MotionEvent`s do, so gestures behave identically. No `UIGestureRecognizer` re-interpretation.
    - Hardware keyboard through `pressesBegan`/`pressesEnded`.
    - The system keyboard through a hidden `UIKeyInput` view with autocorrection, spell checking and smart punctuation off.
- **`UI/`** (SwiftUI): one view per Android screen, with the same copy (strings copied verbatim) and the same layout order.
  - Components mirror `Components.kt`: `Page`, `Card`, `SectionLabel`, `PrimaryButton`, `SecondaryButton`, `StatusDot`, `MetricRow`, `Note`, `Toggle`.
  - The theme reuses `Theme.kt`'s tokens: light Ivory/Graphite/Blue/Sandstone, dark Night/Mist. It follows the system appearance as Android does.
  - Icons are ported from `Glyphs.kt` as SwiftUI `Shape`s, so they look the same.
  - The compact keyboard is a SwiftUI view with press-on-touch-down and repeat, the same as the Compose version.
  - The remote screen hosts `RemoteView` with `UIViewRepresentable` and overlays the rail, panel, shortcut bar, compact keyboard, stats and status overlays.

### 5. Platform differences (decided)

| Android behavior | iPhone equivalent |
|---|---|
| `FLAG_SECURE`: blocks screenshots, recordings and the Recents preview | iOS cannot block screenshots. Instead, the PC image is covered whenever the app leaves the foreground, so the app-switcher snapshot is clean, and blanked with a notice while `UIScreen.isCaptured` (recording or mirroring). The setting is renamed "Allow the PC screen in recordings and the app switcher" (default off). The Settings note says plainly that iOS cannot block screenshots |
| `IME_FLAG_NO_PERSONALIZED_LEARNING` | iOS has no such flag: autocorrection, spell checking, smart quotes and dashes, and predictive text are turned off on the input view. The compact keyboard (default) uses no system keyboard at all |
| Back gesture / BackHandler | navigation back buttons; on the remote screen, a Disconnect in the panel and the hide-keyboard key |
| Local network is implicit | `NSLocalNetworkUsageDescription` plus `NSBonjourServices = [_lanlink._udp]`. The first connection or browse triggers the system prompt; a denied permission shows a message pointing to Settings → Privacy → Local Network |
| Camera QR scan through the ZXing activity | `AVCaptureSession` + `AVCaptureMetadataOutput` (QR), with `NSCameraUsageDescription`. In-app scans pair directly, as on Android |
| `lanlink://` intent filter | `CFBundleURLTypes` scheme `lanlink`; `onOpenURL` → the same Offer confirmation; handled once |
| Keep screen on | `UIApplication.isIdleTimerDisabled` while a session is shown |
| Landscape while controlling, immersive | `requestGeometryUpdate(.iOS(interfaceOrientations: .landscape))` plus `supportedInterfaceOrientations`; status bar and home indicator hidden (`persistentSystemOverlays(.hidden)`) |
| Haptics (`KEYBOARD_TAP`, click) | `UIImpactFeedbackGenerator(.light)` and `.rigid` |
| Multicast lock | not needed: Bonjour through `NWBrowser` needs no multicast entitlement |

### 6. Build and CI (`.github/workflows/ios.yml`, `macos-15`)

1. `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`.
2. `cargo test -p lanlink-mobile -p lanlink-ios`, plus the cross-platform crates' tests.
3. `scripts/build-ios-core.sh` builds the xcframework.
4. `brew install xcodegen`, then `xcodegen` generates `LANLink.xcodeproj` from `ios/project.yml`. The project file is generated, not committed.
5. `xcodebuild test`, iPhone 16 simulator: unit tests and UI tests.
6. `xcodebuild build` for `generic/platform=iOS` with signing disabled, which proves the device build compiles.
7. Upload the UI-test screenshots and test results (`.xcresult`) as artifacts. These are the visual evidence of each screen.

The existing `ci.yml` (Windows and Android) stays, and also runs the `lanlink-mobile` tests.

## What counts as done

- **CI green on macOS:** Rust bridge tests (including the fake-host test), Swift unit tests (every Android core test case has a twin), UI tests, and a device build that compiles.
- **Screenshots** from the UI tests for Onboarding, Home, Pair, Offer, Settings, Diagnostics and Security, checked by eye against the Android screens.
- **Android unchanged and verified:** all existing checks pass after the refactor.
- **Recorded as not verified:** a real iPhone, a real PC connection from an iPhone, decode latency on a device.

## Calls made after the builder check (the author's, open to correction)

- **Reference version.** The Swift ports follow the Android code as committed in the repository's first commit (made before the shared-core refactor). Later Android changes reach iOS through the mirrored test cases: change the Kotlin test, then the Swift twin, then the code.
- **Baseline first.** The current tree is committed as-is before any refactor, so "Android unchanged" can be shown by a diff and by the test suite.
- **What goes in the repository.** The whole monorepo, including `docs/media` (personal data already removed) and the film source in `brag-output/work/film`. The rendered `brag-output/*.mp4|gif` stay git-ignored.
- **Look of the controls.**
  - Rebuilt to match Android: the distinctive pieces (cards, buttons, chips, status dots, the control rail and panel, the compact keyboard, overlays, icons, palette, spacing and copy).
  - Native iOS, tinted with the same colours: standard interaction controls (switches, sliders, alerts, menus, progress spinners). iPhone users expect these, and Material copies would feel foreign.
  - The screenshot check compares layout, copy, colours and the distinctive pieces, not the pixels of those native controls.
- **Panic handling.** The iOS staticlib uses a profile with `panic = "unwind"`, so `catch_unwind` at the C boundary works. The Android release profile is checked for the same.
- **Copy that names Android.** For example "Android Keystore" becomes "iOS Keychain".

## CI budget and repository

The user chose a **public** repository ("Public (free builds)"), which gets unlimited free minutes on standard GitHub runners, macOS included. It is created with the user's signed-in `gh` account. CI runs on every push and pull request. Even so, the iOS workflow caches the Rust targets and the xcframework to keep runs short.

Before the first push:
- the tree is scanned again for personal data and secrets (names, addresses, keys, `keystore.properties`, logs);
- the push waits for that scan to come back clean.

## Left to the builder

- File splits inside each folder; test naming.
- Whether to generate the C header with cbindgen or keep it hand-written behind the CI consistency check.
- The fake host's exact shape, as long as it uses the real protocol, auth and transport crates.
- UI-test navigation details and which simulator model to use, as long as it is an iPhone.
