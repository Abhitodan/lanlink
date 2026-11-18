# LANLink for iPhone Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A native iPhone LANLink app with the Android app's screens, gestures, keyboards and security, on the same Rust core, built and tested on GitHub's macOS runners.

**Architecture:**
- The JNI bridge's logic moves into a shared crate, `crates/lanlink-mobile`. Android keeps a thin JNI layer over it; iOS gets a thin C ABI (`ios/native-bridge`) packaged as `LanLinkCore.xcframework`.
- The SwiftUI app (UIKit for the remote view) mirrors the Android packages one-for-one. Its pure logic is a translation of the Kotlin core, held to the same test cases.

**Tech Stack:**
- Rust 1.85+ (workspace).
- Swift 5.9+, SwiftUI, UIKit, AVFoundation (`AVSampleBufferDisplayLayer`, `AVCaptureMetadataOutput`), Network.framework (`NWBrowser`), Security.framework (Keychain).
- XCTest / XCUITest, XcodeGen, GitHub Actions `macos-15` with Xcode 16.

**Spec:** `docs/superpowers/specs/2026-11-17-iphone-app-design.md`

## Global Constraints

- iOS deployment target **16.0**; iPhone layout only (`TARGETED_DEVICE_FAMILY = 1`).
- Bundle id `io.lanlink.app` (builder's call per spec); display name `LANLink`.
- The protocol is unchanged: no new message types, no change to the PC app.
- No new third-party dependencies (Swift or Rust) unless permissively licensed (ADR 0010). None are planned.
- **Copy:** UI strings are copied verbatim from the Android screens. The only exceptions are the platform rewordings in the spec ("Android Keystore" → "iOS Keychain", and the screenshot setting text in spec §5).
- **Constants:** keep the Kotlin names and values: `HOLD_MS = 250`, `THIRD_FINGER_MS = 150`, `TAP_MS = 250`, `LONG_PRESS_MS = 420`, `DOUBLE_TAP_MS = 280` (touch) and `350` (shift), `REVEAL_AT = 0.75`, `TYPING_ZOOM = 1.5`, `MAX_ZOOM = 6`, key repeat 400 ms delay / 55 ms interval, rail auto-hide 4 s, key height 28–48 dp (default 36).
- **Storage:** Settings keys and defaults identical to `android/.../core/Stores.kt`. Identity in the Keychain with `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly`, `kSecAttrSynchronizable = false`.
- The iOS staticlib builds with a profile that has `panic = "unwind"`. Every C entry point catches panics.
- **Evidence:** nothing is claimed beyond what CI shows. The status doc lists "real iPhone", "real PC from iPhone" and "device latency" as not verified.
- **Repository:** public, under the user's signed-in `gh` account.
  - The first push follows a clean scan for personal data and secrets.
  - Each task's iOS work after the baseline lands through a pull request on branch `ios-app`; CI results come from the PR checks.

## Review Focus

1. **Local-network permission denied, or Wi-Fi off.** Discovery and connect must show the "allow Local Network in Settings" message within the connect deadline, not spin forever. (Test in Task 8.)
2. **Video that starts mid-stream or arrives malformed.** This covers frames before any SPS/PPS, a discontinuity, and a truncated NAL. The decoder waits for a keyframe with parameter sets, requests one, and never crashes. (Tests in Task 10.)
3. **App sent to background mid-session.** The privacy cover is up before the snapshot, held input is released (`releaseAll`), and the session reconnects on return. (Test in Task 11.)
4. **Rotation or keyboard show/hide while zoomed.** The viewport is re-clamped and never shows empty space beyond the picture. (Swift twins of `panCannotExposeEmptySpace` and the keyboard-region clamp in Task 6; a RemoteView geometry test in Task 11.)
5. **Misuse across the C boundary.** Stale or zero handles, NULL pointers, non-UTF-8 strings and panics return errors; every Rust allocation handed to Swift is freed exactly once. (Tests in Task 4 and Task 5.)

---

### Task 1: Baseline commit and public repository

**Files:**
- Modify: `.gitignore` (add `ios/build/`, `ios/*.xcodeproj/`, `*.xcframework/`, `DerivedData/`)
- No source changes.

**Interfaces:** Produces the `main` branch on GitHub that every later task branches from.

- [ ] **Step 1: Scan for personal data and secrets**

Run:
```bash
git ls-files --others --exclude-standard | xargs grep -I -n -i -E -f "$PERSONAL_PATTERNS" || echo CLEAN   # pattern file kept outside the repo: owner name, email, home LAN address, private-key headers
```
Also run `git ls-files --others --exclude-standard | grep -i -E "\.(jks|keystore|log|h264)$|local\.properties|keystore\.properties"`.
Expected: `CLEAN` and no files listed. Anything found is fixed before Step 2.

- [ ] **Step 2: Commit the tree as-is**

```bash
git add -A
git update-index --chmod=+x android/gradlew
git commit -m "Baseline: LANLink Windows host, Android app, shared Rust core"
```

- [ ] **Step 3: Create the public repository and push**

Run `gh repo create lanlink --public --source . --remote origin --push --description "Control a Windows PC from your phone over your own Wi-Fi"`.
Expected: the repository URL is printed, and `git log origin/main -1` matches the local commit.

- [ ] **Step 4: Confirm the existing CI**

Read the run with `gh run list --limit 1` after it finishes (no polling loop: check when the push notification or the next task begins). Expected: `rust` and `android` jobs green. If a job fails because of the runner environment (not code), fix the workflow in a follow-up commit on `main`.

---

### Task 2: Shared bridge core `crates/lanlink-mobile`

**Files:**
- Create: `crates/lanlink-mobile/Cargo.toml`, `src/lib.rs`, `src/runtime.rs`, `src/pairing.rs`, `src/session.rs`, `src/video.rs`, `src/input.rs`, `src/events.rs` (moved from `android/native-bridge/src/json.rs` + `event_json`), `src/links.rs` (URI parse + local-address filter)
- Modify: `Cargo.toml` (workspace member), `android/native-bridge/src/lib.rs` (becomes JNI glue only), `android/native-bridge/Cargo.toml`
- Test: unit tests move with the code (`parameter_sets`, JSON encoding)

**Interfaces:**
- Produces (module paths under `lanlink_mobile::`):
  - `generate_identity(label: &str) -> Result<Vec<u8>, String>`, `identity_fingerprint(id: &[u8]) -> Result<String, String>`, `parse_pairing_uri(uri: &str) -> Result<String /*JSON*/, String>`
  - `pair_start(identity: &[u8], name: &str, model: &str, host: &str, port: i32, fp_hex: &str, code: &str) -> Result<i64, String>`, `pair_poll(job: i64) -> Result<String /*JSON*/, String>`, `pair_cancel(job: i64)`
  - `connect(identity, name, model, host, port, fp_hex) -> Result<i64, String>`, `close(h: i64)`
  - `send_capabilities(h, max_w, max_h, max_fps, hardware, low_latency, display_w, display_h, refresh_millihz) -> Result<(), String>`
  - `start_session(h, display_id: i64, quality, max_w, max_h, fps, kbps, max_kbps, view_only) -> Result<(), String>`, `set_stream_params(...)`, `end_session(h)`, `select_display(h, id: i64)`, `bind_session(h, session_id: i64)`, `set_text_input(h, active) -> Result<bool, String>`
  - `send_client_stats(h, received_fps_x10, rendered_fps_x10, decode_p50_us, decode_p95_us, dropped, queue) -> Result<(), String>`
  - `poll_event(h, timeout_ms: i32) -> Result<Option<String>, String>`, `cursor_pixels(h, shape_id: i64) -> Result<Option<Vec<u8>>, String>`
  - `next_video(h, buf: &mut [u8], meta: &mut [i64; 8], timeout_ms: i32) -> Result<i32, String>`, `decoder_reset(h)`, `request_keyframe(h)`, `now_us() -> i64`
  - `send_pointer(h, x, y)`, `send_pointer_rel(h, dx, dy)`, `send_button(h, button, down, x, y) -> Result<bool, String>`, `send_scroll(h, dx, dy) -> Result<bool, String>`, `send_key(h, vk, down) -> Result<bool, String>`, `send_text(h, text: &str) -> Result<bool, String>`, `release_all(h)`
  - constants `VIDEO_TIMEOUT = -1`, `VIDEO_CONFIG = -2`, `VIDEO_CLOSED = -3`, `VIDEO_BUFFER_SMALL = -4`, `META_KEYFRAME = 1`, `META_CODEC_CONFIG = 2`, `META_DISCONTINUITY = 4`
- Event JSON and `meta` layouts are exactly today's (see the doc comments in `android/native-bridge/src/lib.rs`).

- [ ] **Step 1: Move code, not behavior.** Cut each non-JNI function and type from `android/native-bridge/src/lib.rs` into the module above, keeping bodies. Replace raw-pointer video copy with `buf[..n].copy_from_slice(..)`. `meta` widens to 8 entries; unused entries are 0.
- [ ] **Step 2: Rewrite each JNI function as glue.** Convert arguments, call `lanlink_mobile::…`, and map `Err(msg)` to `IOException(msg)` (existing `guard`). For `nextVideo`, wrap the direct buffer with `std::slice::from_raw_parts_mut(ptr, cap)` and copy back the first 5 or 7 `meta` values as today.
- [ ] **Step 3: Run** `cargo test -p lanlink-mobile -p lanlink-android && cargo clippy --workspace --all-targets -- -D warnings`. Expected: all pass, 0 warnings.
- [ ] **Step 4: Run the full checks** with `scripts\test-all.ps1` (end-to-end included, unlocked desktop), plus the native library and `assembleDebug` via `scripts\build-android.ps1`. Expected: everything passes, the APK builds, and the Android unit-test count is unchanged (34).
- [ ] **Step 5: Commit** `refactor: move bridge logic into lanlink-mobile (Android behaviour unchanged)` on branch `ios-app`; open the PR `iPhone app` and bind it (`ccd_pr`).

---

### Task 3: Fake-host integration test

**Files:**
- Create: `crates/lanlink-mobile/tests/fake_host.rs`

**Interfaces:**
- Consumes: the Task 2 API.
- The fake host: `lanlink_transport::server_endpoint` + `lanlink_protocol` messages + `lanlink_auth::{PairingHost, PairingWindow, TrustStore}`, on `127.0.0.1:0`, in-process, cross-platform. It answers Hello, pairs (auto-approves), answers Capabilities with one display, starts a session, writes one `VideoConfig` and one keyframe (`00 00 00 01 67 … 68 … 65 …`), and records input.

- [ ] **Step 1: Write the tests:**
  - `pairs_then_connects_with_the_pinned_identity`: `pair_start` → poll until `"state":"done"`; the fingerprint equals the fake host's; `connect` returns a handle.
  - `session_delivers_config_then_parameter_sets_then_frame`: `next_video` returns `VIDEO_CONFIG`, then a buffer with `meta[0] == META_CODEC_CONFIG` holding only SPS+PPS, then the keyframe with `META_KEYFRAME`.
  - `input_reaches_the_host_in_order`: `send_key(0x41,true)`, `send_key(0x41,false)` and `send_text("hé")` arrive in that order with consecutive sequence numbers.
  - `buffer_too_small_reports_needed_size`: `cap = 4` → `VIDEO_BUFFER_SMALL` and `meta[0]` = the needed length.
  - `stale_handle_is_an_error`: after `close(h)`, `send_key(h, …)` returns `Err("connection closed")`.
- [ ] **Step 2: Run** `cargo test -p lanlink-mobile --test fake_host`. Expected: FAIL first only where the fake host is incomplete; iterate until PASS.
- [ ] **Step 3: Add** `cargo test -p lanlink-mobile` to the `rust` job in `.github/workflows/ci.yml`.
- [ ] **Step 4: Commit and push** to `ios-app`. Expected: PR checks green.

---

### Task 4: iOS C ABI crate and xcframework

**Files:**
- Create: `ios/native-bridge/Cargo.toml` (crate `lanlink-ios`, `crate-type = ["staticlib", "rlib"]`), `ios/native-bridge/src/lib.rs`, `ios/native-bridge/include/lanlink.h`, `ios/native-bridge/include/module.modulemap` (`module LanLinkCore { header "lanlink.h" export * }`)
- Create: `scripts/build-ios-core.sh`, `scripts/check-ios-header.py`
- Modify: `Cargo.toml` (member; profile `[profile.release-ios] inherits = "release" panic = "unwind" strip = "debuginfo"`)
- Test: `ios/native-bridge/tests/abi.rs`

**Interfaces:**
- Produces C functions (all `int32_t` status: `0` ok, `-1` error; the error goes to `char **error_out`, which may be NULL):
  - `lanlink_generate_identity(const char *label, uint8_t **out, size_t *out_len, char **err)`, `lanlink_identity_fingerprint(const uint8_t *id, size_t len, char **out, char **err)`, `lanlink_parse_pairing_uri(const char *uri, char **out_json, char **err)`
  - `lanlink_pair_start(const uint8_t *id, size_t len, const char *name, const char *model, const char *host, int32_t port, const char *fp_hex, const char *code, int64_t *out_job, char **err)`, `lanlink_pair_poll(int64_t job, char **out_json, char **err)`, `lanlink_pair_cancel(int64_t job)`
  - `lanlink_connect(…same identity/name/model/host/port/fp… , int64_t *out_handle, char **err)`, `lanlink_close(int64_t h)`
  - one function per remaining Task 2 function, with the same argument order. Booleans are `bool`. A result that is `bool` comes back through `bool *out`; `Option<String>` comes back as `char **out` (NULL = none); `cursor_pixels` returns `uint8_t **out, size_t *out_len`.
  - `int32_t lanlink_next_video(int64_t h, uint8_t *buf, size_t cap, int64_t meta[8], int32_t timeout_ms, int32_t *out_code, char **err)`
  - `void lanlink_string_free(char *)`, `void lanlink_bytes_free(uint8_t *, size_t)`, `int64_t lanlink_now_us(void)`
- `scripts/build-ios-core.sh` builds the targets `aarch64-apple-ios`, `aarch64-apple-ios-sim` and `x86_64-apple-ios` with `--profile release-ios`. It `lipo`s the two simulator libraries and creates `ios/build/LanLinkCore.xcframework` with the headers.

- [ ] **Step 1: Write the ABI tests** in `tests/abi.rs`, calling the `extern "C"` functions from Rust:
  - `null_and_bad_utf8_are_errors_not_crashes`: a NULL `label` → `-1` with a non-NULL `err`; invalid UTF-8 → `-1`.
  - `errors_and_outputs_are_freeable`: generate an identity, take its fingerprint, then free both; run under `cargo test` (Miri is not required).
  - `stale_handle_returns_error`: `lanlink_send_key(12345, 0x41, true, &mut ok, &mut err)` → `-1`, and `err` is "connection closed".
  - `panics_become_errors`: a test-only entry `lanlink__test_panic(char **err)`, compiled under `#[cfg(test)]`, returns `-1` with "internal error in LANLink native code".
- [ ] **Step 2: Run** `cargo test -p lanlink-ios`. Expected: PASS on Windows (the ABI is platform-neutral).
- [ ] **Step 3: Write `scripts/check-ios-header.py`.** It collects `pub extern "C" fn lanlink_*` names from `src/lib.rs` and the declared names in `lanlink.h`, and exits 1 on any difference. Run it. Expected: exit 0.
- [ ] **Step 4: Commit and push** to `ios-app`. The xcframework is built in Task 5's CI.

---

### Task 5: iOS project skeleton, theme, components and CI

**Files:**
- Create: `ios/project.yml`; `ios/LANLink/App/LANLinkApp.swift`, `ios/LANLink/App/Info.plist`
- Create: `ios/LANLink/Core/Bridge.swift`
- Create: `ios/LANLink/UI/Theme/Theme.swift`, `ios/LANLink/UI/Theme/Glyphs.swift`, `ios/LANLink/UI/Components.swift`
- Create: `ios/LANLinkTests/BridgeTests.swift`
- Create: `.github/workflows/ios.yml`

**Interfaces:**
- Produces:
  - `enum Bridge` with static throwing Swift functions named like the Kotlin `NativeBridge` (`generateIdentity(label:) -> Data`, `identityFingerprint(_:) -> String`, `parsePairingUri(_:) -> String`, `pairStart(...) -> Int64`, … `nextVideo(handle:buffer:meta:timeoutMs:) -> Int32`).
  - `struct LANLinkError: Error, LocalizedError { let message: String }`.
  - Constants mirror `NativeBridge.kt`.
- Produces:
  - `Theme` colour tokens with the exact hex values of `Theme.kt` (Ivory `#FAF8F3`, IvoryDeep `#F1EDE4`, Line `#E2DDD2`, Graphite `#1F2328`, GraphiteSoft `#5B616B`, Blue `#2F4A6D`, BlueSoft `#DCE4EE`, Sandstone `#D9B98C`, SandstoneSoft `#F3E7D5`, Danger `#A23B33`, Ok `#3E7A55`, Night `#15181C`, NightRaised `#1E2227`, NightLine `#2D3239`, Mist `#E9E6DF`, MistSoft `#A9AEB6`, BlueNight `#9DB6D6`), light and dark schemes as in `Theme.kt`;
  - `Glyph` shapes ported path-for-path from `Glyphs.kt`;
  - SwiftUI views `Page`, `Card`, `SectionLabel`, `PrimaryButton`, `SecondaryButton`, `StatusDot`, `MetricRow`, `Note`, `ToggleRow`, matching `Components.kt`.
- `Info.plist` keys:
  - `NSLocalNetworkUsageDescription` = "LANLink connects to your PC over your local network."
  - `NSBonjourServices` = `["_lanlink._udp"]`
  - `NSCameraUsageDescription` = "LANLink uses the camera to scan the pairing QR code shown on your PC."
  - `CFBundleURLTypes` with the scheme `lanlink`
  - `UISupportedInterfaceOrientations`: portrait and both landscapes.

- [ ] **Step 1: Write `BridgeTests`:**
  - `testIdentityRoundTrip`: `generateIdentity` gives non-empty `Data`, and its fingerprint is 64 hex chars.
  - `testParsePairingUriRejectsPublicAddress`: `lanlink://pair?v=1&a=8.8.8.8&p=47990&f=<64 hex>&c=12345678` throws a `LANLinkError` whose message contains "outside your local network".
  - `testStaleHandleThrows`: `sendKey(handle: 999_999, vk: 0x41, down: true)` throws.
- [ ] **Step 2: Write `ios.yml`** (`macos-15`, on push and pull_request):
  1. checkout;
  2. `rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios`;
  3. `cargo test -p lanlink-mobile -p lanlink-ios`;
  4. `python3 scripts/check-ios-header.py`;
  5. `scripts/build-ios-core.sh`;
  6. `brew install xcodegen && (cd ios && xcodegen)`;
  7. `xcodebuild test -project ios/LANLink.xcodeproj -scheme LANLink -destination 'platform=iOS Simulator,name=iPhone 16' -resultBundlePath build/Tests.xcresult`;
  8. `xcodebuild build -project ios/LANLink.xcodeproj -scheme LANLink -destination 'generic/platform=iOS' CODE_SIGNING_ALLOWED=NO`;
  9. upload `build/Tests.xcresult` and the screenshots folder as artifacts (`if: always()`).

  Cache `~/.cargo/registry`, `target/` and `ios/build`, keyed on `Cargo.lock` plus a hash of `ios/native-bridge/**`.
- [ ] **Step 3: Push** to `ios-app`. Expected: the PR's `ios` check is green, with 3 tests passed. If the simulator name is missing on the runner, pick one listed by `xcrun simctl list devices available` in the workflow.

---

### Task 6: Core logic in Swift (34 test twins)

**Files:**
- Create: `ios/LANLink/Core/Viewport.swift`, `TouchInterpreter.swift`, `TrackpadCursor.swift`, `CompactKeyboard.swift` (`KeyAction`, `KeySpec`, `CompactLayout`, `ShiftState`), `KeyMap.swift`, `DisplayText.swift`
- Test: `ios/LANLinkTests/CoreLogicTests.swift`

**Interfaces:**
- Produces Swift translations with the same type, member and parameter names as the Kotlin files of the same name in `android/app/src/main/java/com/lanlink/app/core/`:
  - `Pair<Float,Float>` becomes a tuple `(Float, Float)`;
  - Kotlin `object` becomes a Swift `enum` with static members;
  - Kotlin `interface TouchActions` becomes a Swift `protocol TouchActions`, with a default `threeFingerTap()` in an extension.
- Time stays `Int64` milliseconds passed in by the caller (no clocks inside), as in Kotlin.

- [ ] **Step 1: Translate the 34 tests** of `android/app/src/test/java/com/lanlink/app/core/CoreLogicTest.kt` into XCTest methods, `test` + the same name. Same inputs, same expected values, and the same tolerances (`accuracy:`):
  - `ViewportTest` (11);
  - `TouchInterpreterTest` (13, including the `Rec` recorder);
  - `TrackpadCursorTest` (4);
  - `CompactKeyboardTest` (3);
  - `KeyMapTest` (2);
  - `DisplayTextTest` (1).
- [ ] **Step 2: Push.** Expected: the 34 new tests fail to compile (types missing).
- [ ] **Step 3: Translate the six Kotlin files** line for line. `KeyMap.vkFor(keyCode:)` maps iOS `UIKeyboardHIDUsage` values instead of Android key codes. The test twins use the HID usages for the same keys (A, 1, Return, Tab, arrows, Escape, F1, modifiers). `DisplayText` must handle the supplementary ranges from the Rust `display_name` (tags U+E0000–E007F, U+110BD, U+1D173–1D17A).
- [ ] **Step 4: Push.** Expected: `ios` check green, 37 tests passed.

---

### Task 7: Stores

**Files:**
- Create: `ios/LANLink/Core/Stores.swift` (`Settings`, `PairedHost`, `HostRepository`, `IdentityStore`, `Quality`, `PointerMode` reuse from Task 6)
- Test: `ios/LANLinkTests/StoresTests.swift`

**Interfaces:**
- Produces:
  - `final class Settings` with properties named as in `Stores.kt`. It is backed by an injected `UserDefaults` (tests use a suite name), with the same keys, defaults and clamps (`keyHeight` 28…48, `startZoom` 1…4, `deviceName` max 48). `allowScreenshots` is the stored key `"allowShots"`.
  - `struct PairedHost: Codable, Equatable`, with JSON keys `fp,name,addrs,port,pairedAt,last` and `shortId`.
  - `final class HostRepository` with `all()`, `get(_:)`, `put(_:)`, `remove(_:)`, `clear()`, `touch(fp:address:)`; `touch` keeps the 6 most recent addresses.
  - `final class IdentityStore` with `get(deviceLabel:) throws -> Data`, `fingerprint(deviceLabel:) throws -> String`, `clear()`. Keychain service `"io.lanlink.identity"`, account `"identity"`, with the attributes from the Global Constraints.
- Default device name: `UIDevice.current.name` if it is not the generic "iPhone", else the model name ("iPhone"). Run it through `DisplayText.name`.

- [ ] **Step 1: Write the tests:**
  - `testSettingsDefaultsMatchAndroid`: `compactKeyboard == true`, `keyHeight == 36`, `pointerMode == .trackpad`, `startZoom == 2`, `allowScreenshots == false`, `landscape == true`, `keepScreenOn == true`.
  - `testKeyHeightIsClamped`: setting 100 reads back 48; setting 1 reads back 28.
  - `testHostRepositoryRoundTripAndTouchOrder`: put two hosts; `touch` moves the address to the front; `all()` is sorted by `lastConnected` descending.
  - `testIdentityIsStableAndClearable`: two `get` calls return equal data; after `clear()`, a new identity differs (Keychain works in the simulator test host).
- [ ] **Step 2: Push.** Expected: they fail to compile. **Step 3:** Implement. **Step 4: Push.** Expected: green.

---

### Task 8: Discovery

**Files:**
- Create: `ios/LANLink/Core/Discovery.swift`
- Test: `ios/LANLinkTests/DiscoveryTests.swift`

**Interfaces:**
- Produces:
  - `@MainActor final class Discovery: ObservableObject`, with `@Published hosts: [FoundHost]` and `@Published status: DiscoveryStatus` (`.idle`, `.searching`, `.failed(String)`, `.localNetworkDenied`).
  - `func start()` and `func stop()`.
  - `struct FoundHost { serviceName, name, shortId, address, port, protocol }`.
- `NWBrowser(for: .bonjourWithTXTRecord(type: "_lanlink._udp", domain: nil), using: .udp)`. Each result is resolved with an `NWConnection` to read the endpoint address. Names go through `DisplayText.name(...)`, with "PC" if empty. A failed start leaves the browser nil, so `start()` retries (parity with Android fix E4). A `.waiting` or `.failed` state with a `kDNSServiceErr_PolicyDenied` / `NoAuth` error maps to `.localNetworkDenied`.
- Pure helper for tests: `static func parseTXT(_ txt: [String: String], serviceName: String) -> (name: String, shortId: String, protocol: Int)`.

- [ ] **Step 1: Write the tests:**
  - `testTXTParsingCleansNames`: `["n": "\u{202E}enohP", "id": "abcd", "v": "2"]` → name `"enohP"`, shortId `"abcd"`, protocol 2.
  - `testEmptyNameFallsBackToPC`: `["n": "\u{200B}"]` → `"PC"`.
  - `testDeniedErrorMapsToLocalNetworkDenied`: `Discovery.status(for: NWError.dns(DNSServiceErrorType(kDNSServiceErr_PolicyDenied)))` → `.localNetworkDenied` (Review Focus 1).
- [ ] **Steps 2–4:** push (fails to compile) → implement → push (green).

---

### Task 9: App model, navigation and all non-remote screens

**Files:**
- Create: `ios/LANLink/UI/AppModel.swift` (Swift twin of `MainViewModel.kt`: `Screen`, `PairState` including `.offer`, the pairing loop and friendly errors)
- Create: `ios/LANLink/UI/Screens/OnboardingScreen.swift`, `HomeScreen.swift`, `HostScreen.swift`, `PairScreen.swift`, `QRScanner.swift`, `SettingsScreen.swift`, `DiagnosticsScreen.swift`, `SecurityScreen.swift`, `AppRoot.swift`
- Modify: `ios/LANLink/App/LANLinkApp.swift` (`onOpenURL` → `model.offerPairFromLink`, handled once)
- Test: `ios/LANLinkTests/AppModelTests.swift`, `ios/LANLinkUITests/ScreensUITests.swift`

**Interfaces:**
- Consumes: `Bridge`, `Settings`, `HostRepository`, `IdentityStore`, `Discovery`.
- Produces:
  - `@MainActor final class AppModel: ObservableObject`, with `@Published screen: Screen`, `@Published pair: PairState`, `@Published hosts: [PairedHost]`.
  - Methods named as in `MainViewModel.kt`: `go(_:)`, `replace(_:)`, `back() -> Bool`, `finishOnboarding(next:)`, `pairFromUri(_:)`, `offerPairFromLink(_:)`, `acceptOffer()`, `pairManual(address:port:code:)`, `cancelPairing()`, `forget(_:)`, `clearCredentials()`, `connect(_:address:)`, `endSession()`.
  - `static func friendlyPairError(_ e: String) -> String`, with the same branches and copy as Kotlin, including "unverified".
- Screens follow the Android composables element by element (same order, same strings). Native `Toggle`, `Slider`, `Menu`, `ProgressView` and `.alert` are tinted with `Theme` (spec "Look of the controls").
- `QRScanner`: `UIViewControllerRepresentable` over `AVCaptureSession` + `AVCaptureMetadataOutput` (`.qr`), returning the first payload. Camera permission is denied → a `Note` with the Settings hint.
- Launch argument for UI tests: `-uiTestSeed` seeds `HostRepository` with the fictional "Studio PC" (fingerprint `5a3c9e17b2d84f60a1c7e9b35d2f8046c1e7a93b5d28f4061ac3e7b95d1f2a84`, `192.168.1.20`) and marks onboarding done. This exists only for UI tests.

- [ ] **Step 1: Write the `AppModelTests`:**
  - `testLinkOnlyOffersAndDoesNotPersistOnboarding`: `offerPairFromLink(valid)` → `pair` is `.offer` with name "Studio PC"; `settings.onboarded` is still false; `acceptOffer()` sets it true.
  - `testPublicAddressLinkFails`: `offerPairFromLink` with `a=8.8.8.8` → `.failed` containing "outside your local network".
  - `testFriendlyErrors`: `"the pairing code is wrong"` → the Kotlin "The code was not accepted…" string; `"…(unverified)"` → the "Scan the QR code on the PC instead." string.
  - `testBackFromRemoteEndsSession`.
- [ ] **Step 2: Write `ScreensUITests`.** Launch with `-uiTestSeed`, then:
  - capture Home (asserts "Studio PC" and "Pair a PC");
  - open the URL `lanlink://pair?v=1&a=192.168.1.20&p=47990&f=9d41…05c&c=48213907&n=Studio%20PC` with `XCUIApplication().open(URL)` and capture Offer (asserts "Pair with Studio PC?");
  - Cancel; open Settings and capture, scrolling to Privacy;
  - capture Diagnostics and Security;
  - launch without the seed and capture Onboarding.

  Each screenshot is an `XCTAttachment` with `lifetime = .keepAlways`, named `01-home` … `07-onboarding`.
- [ ] **Steps 3–5:** push (fails) → implement → push. Expected: the `ios` check is green, and the `.xcresult` artifact holds 7 screenshots. Download the artifact (`gh run download`), look at each screenshot next to the Android screenshot in `docs/media/`, and fix mismatches in layout, copy or colours before moving on.

---

### Task 10: Session and video decoding

**Files:**
- Create: `ios/LANLink/Session/RemoteSession.swift`, `ios/LANLink/Session/VideoDecoder.swift`, `ios/LANLink/Session/AnnexB.swift`
- Test: `ios/LANLinkTests/AnnexBTests.swift`, `ios/LANLinkTests/VideoDecoderTests.swift`

**Interfaces:**
- Consumes: `Bridge` and the Task 7 stores.
- Produces:
  - `final class RemoteSession: ObservableObject`, the twin of `RemoteSession.kt`:
    - `start()`, `stop()`; `@Published state: SessionState` (phase, message, displays, stats, sticky modifiers, view-only);
    - input methods `pointer(x:y:)`, `pointerBy(dx:dy:)`, `button(_:down:x:y:)`, `scroll(dx:dy:)`, `key(_:down:)`, `tapKey(_:)`, `text(_:)`, `toggleSticky(_:)`, `setTextInput(_:)`, `releaseAll()`;
    - a `CursorSink` protocol with `onCursorPosition`, `onCursorShape`, `onCaret(visible:x:y:w:h:)`, `onVideoGeometry`;
    - the connect loop with the impostor skip and the "not paired" stop (fix E3 parity); the same quality presets and stream planning as Kotlin.
  - `enum AnnexB`:
    - `static func nalUnits(_ data: Data) -> [Data]` handles 3- and 4-byte start codes;
    - `static func avcc(_ nals: [Data]) -> Data` writes 4-byte big-endian lengths;
    - `static func parameterSets(_ nals: [Data]) -> (sps: Data, pps: Data)?`.
  - `final class VideoDecoder`:
    - owns `AVSampleBufferDisplayLayer`; `func handle(config:)`, `func handle(frame: Data, keyframe: Bool, discontinuity: Bool) -> DecodeResult` (`.displayed`, `.waitingForKeyframe`, `.failed`), `func reset()`;
    - builds the `CMVideoFormatDescription` with `CMVideoFormatDescriptionCreateFromH264ParameterSets` and marks each sample `kCMSampleAttachmentKey_DisplayImmediately`;
    - on `layer.status == .failed` or a discontinuity: `flush()` and `requestKeyframe`.
  - A video pump thread calls `Bridge.nextVideo` into a reused buffer, grows it on `VIDEO_BUFFER_SMALL`, and handles `VIDEO_CONFIG` / `META_CODEC_CONFIG` as `VideoDecoder.kt` does.

- [ ] **Step 1: Write the tests:**
  - `testSplitsThreeAndFourByteStartCodes`: `[0,0,0,1,0x67,1,2, 0,0,1,0x68,3, 0,0,0,1,0x65,9,9]` → three NALs `[0x67,1,2]`, `[0x68,3]`, `[0x65,9,9]` (same bytes as the Rust `extracts_parameter_sets` test).
  - `testAvccLengthPrefixes`: `[[0x65,9,9]]` → `[0,0,0,3,0x65,9,9]`.
  - `testTruncatedInputYieldsNoGarbage`: `[0,0,1]` → `[]`; `[]` → `[]`.
  - `testDeltaFrameBeforeParameterSetsWaits`: a fresh `VideoDecoder` given a non-keyframe → `.waitingForKeyframe`, and its keyframe-request callback has been called once (Review Focus 2).
  - `testKeyframeWithParameterSetsCreatesFormat`: a real 64×64 H.264 keyframe fixture (`ios/LANLinkTests/Fixtures/key64.h264`, generated once with `ffmpeg -f lavfi -i testsrc=s=64x64:d=0.04 -c:v libx264 -profile:v baseline -bsf:v h264_mp4toannexb key64.h264`) → `.displayed`.
- [ ] **Steps 2–4:** push (fails) → implement → push (green).

---

### Task 11: Remote view and remote screen

**Files:**
- Create: `ios/LANLink/Session/RemoteView.swift` (UIKit), `ios/LANLink/Session/CursorLayer.swift`, `ios/LANLink/Session/KeyInputView.swift` (`UIKeyInput` + `UITextInputTraits`)
- Create: `ios/LANLink/UI/Screens/RemoteScreen.swift` (rail, panel, shortcut bar, stats/status overlays), `ios/LANLink/UI/Screens/CompactKeyboardView.swift`
- Create: `ios/LANLink/UI/PrivacyCover.swift`
- Test: `ios/LANLinkTests/RemoteViewTests.swift`, an addition to `ScreensUITests` (`08-remote-keyboard` using `-uiTestRemoteDemo`, which shows `RemoteScreen` with a still test frame and no network)

**Interfaces:**
- Consumes: `RemoteSession`, `VideoDecoder`, `TouchInterpreter`, `Viewport`, `TrackpadCursor`, `CompactLayout`, `ShiftState`, `Settings`.
- `RemoteView` (twin of `RemoteView.kt`):
  - touches: `touchesBegan/Moved/Ended/Cancelled` → `TouchInterpreter.onDown/onSecondDown/onThirdDown/onMove/onUp…` with millisecond times from `event.timestamp`;
  - timers: a `CADisplayLink`-free `DispatchSourceTimer` for `onTimer`;
  - follow camera and `revealTypingSpot` from `Viewport`; `setKeyboardInset(_ px: CGFloat)`;
  - `onVideoGeometry` re-sends `setTextInput(true)` while the keyboard is open;
  - hardware keys: `pressesBegan`/`pressesEnded` → `KeyMap.vkFor`;
  - haptics: `UIImpactFeedbackGenerator(.rigid)` on click and drag start, `.light` on key press (the twins of Android's `KEYBOARD_TAP` and click feedback);
  - the cursor overlay draws host cursor shapes from `Bridge.cursorPixels`, as `CursorView` does on Android.
- `KeyInputView`: `autocorrectionType = .no`, `spellCheckingType = .no`, `smartQuotesType = .no`, `smartDashesType = .no`, `smartInsertDeleteType = .no`, `autocapitalizationType = .none`, `inlinePredictionType = .no` (iOS 17+, guarded).
- `RemoteScreen` (twin of `RemoteScreen.kt`):
  - the control rail (left/right per `controlsLeft`, auto-hide 4 s, stays visible while the keyboard or panel is open) and `ControlPanel` with the same chips and actions;
  - `ShortcutBar` only while the system keyboard is up and the compact one is not;
  - the compact keyboard at the bottom, centred, max width 640 pt;
  - the covered inset = the compact height, or the system keyboard plus the bar height;
  - landscape via `requestGeometryUpdate`, `persistentSystemOverlays(.hidden)`, `statusBarHidden`, `isIdleTimerDisabled = settings.keepScreenOn`.
- `PrivacyCover`:
  - While `RemoteScreen` is shown and `!settings.allowScreenshots`, a full-screen Night cover with the logo appears on `scenePhase != .active`; it is applied in `sceneWillResignActive`, before the snapshot.
  - When `UIScreen.main.isCaptured` changes to true, the video layer hides behind a "Screen recording is on — the PC screen is hidden" notice.
  - On background: `session.releaseAll()` and `stop()`; on foreground: reconnect (Review Focus 3).

- [ ] **Step 1: Write the tests:**
  - `testTrackpadDragMovesCursorByExactPixels`: feed synthetic touches to a `RemoteView` with a recording session, 2560×1600 source, 1.0 sensitivity, no acceleration. Expected: same deltas as Kotlin `movesByExactPcPixelsFromTheReportedPosition`.
  - `testKeyboardInsetKeepsCaretVisible`: caret near the bottom, `setKeyboardInset(400)` → after reveal, the caret bottom is at 75% of the visible height (±2 pt).
  - `testRotationReclampsViewport` (Review Focus 4): zoom 2×, change bounds from 844×390 to 390×844 → `Viewport.clamp` leaves no empty edge.
  - `testBackgroundReleasesInputAndCovers` (Review Focus 3): post `UIScene.willDeactivateNotification` → the recorder saw `releaseAll`, and the cover view is in the hierarchy.
  - `testCompactKeyboardRepeatsBackspace`: holding ⌫ for 600 ms produces 1 + ⌊(600−400)/55⌋ + 1 = 5 taps (using an injected clock).
- [ ] **Steps 2–4:** push (fails) → implement → push. Expected: green, plus the `08-remote-keyboard` screenshot, compared with `docs/media/compact-keyboard.png`.

---

### Task 12: Docs, status and wrap-up

**Files:**
- Modify: `README.md` (iPhone section: build instructions through CI, screenshots from the UI-test artifact saved to `docs/media/ios-*.png`), `docs/implementation-status.md` (an iPhone section with evidence and the not-verified list), `docs/security-threat-model.md` (rows for Keychain, the privacy cover and the iOS screenshot limitation), `docs/development.md` (iOS build via CI; local build on a Mac with `scripts/build-ios-core.sh` + `xcodegen`)

- [ ] **Step 1:** Save the 8 UI-test screenshots to `docs/media/ios-01-home.png` … and reference them in the README.
- [ ] **Step 2:** Write the status entries with exact test counts from the last green run (Rust, Swift unit, UI tests), plus the "not verified" list from the spec.
- [ ] **Step 3:** Push. Expected: all checks green on the PR (`rust`, `android`, `ios`). Merging stays the user's decision.
