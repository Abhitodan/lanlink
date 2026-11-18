# LANLink Implementation & Verification Ledger

Last updated: 2026-11-18. Every capability listed as completed is backed by automated tests or recorded hardware runs.

## Verification Environment

* **Host PC:** Windows 11 Home (build 26200), Intel Core i9-13900HX, Intel UHD Graphics (iGPU) + NVIDIA RTX 4060 Laptop (dGPU, hybrid Optimus), `2560×1600 @ 240 Hz` display at `150%` scale.
* **Toolbar & Compilers:** Rust `1.85+` MSVC (`rust-toolchain.toml`), Visual Studio Build Tools 2022, Windows SDK `10.0.26100`, Android SDK 34–36 (`compileSdk = 35`, `targetSdk = 35`, `minSdk = 29`), NDK `28.2.13676358`, OpenJDK 21, Gradle `8.12`, `cargo-ndk 4.1.2`.
* **Physical Phone:** Google Pixel 10 Pro (Android 17, `arm64-v8a`) over local `/24` Wi-Fi.
* **Android Emulator:** Pixel 9 Pro XL system image (Android 16, `x86_64`) reaching the host over `10.0.2.2`.

## Automated Checks (Latest Run)

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | Clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | Clean (0 warnings) |
| `cargo test --workspace -- --test-threads=1` | **138 passed** (unit, property fuzzing, `lanlink-mobile` fake-host integration, and loopback QUIC end-to-end) |
| Android `:app:testDebugUnitTest` | **35 passed** (`ViewportTest`, `TouchInterpreterTest`, `TrackpadCursorTest`, `CompactKeyboardTest`, `KeyMapTest`, `DisplayTextTest`, `PairedHostTest`) |
| Android `:app:lintDebug` | **0 errors**, 0 source-code warnings |
| Android `:app:assembleDebug` / `:app:assembleRelease` | Built (`app-release.apk`, R8-minified, resource-shrunk, signed) |
| Android native bridge (`liblanlink.so`) | Built for `arm64-v8a` and `x86_64` (`--profile release-android`) |
| Windows host binaries (`--release`) | `lanlink-host.exe` (GUI + tray) and `lanlink-host-console.exe` (CLI) |

Live capture, input injection, and end-to-end tests require an unlocked interactive Windows desktop and skip cleanly with a diagnostic message on headless CI runners.

## Completed & Verified Features

| Subsystem / Feature | Verification Evidence |
|---|---|
| Binary wire protocol v1 & v2 (typed, length-bounded, strict decoding, test vectors) | 23 unit tests + 8 `proptest` properties (2,000 cases each) |
| Identity generation, SPAKE2 pairing, trust store, QR pairing URI | 20 unit tests including wrong code, MITM certificate mismatch, and cross-session replay |
| QUIC transport with pinned mutual TLS 1.3 (`ECDSA P-256`) | 5 loopback handshake tests including pinned-fingerprint rejection |
| Shared mobile core (`lanlink-mobile`) | 9 unit tests + 6 in-process fake-host QUIC integration tests |
| Monitor enumeration, WGC capture, DXGI DD fallback, cursor shapes | 8 unit + 5 live display tests; hardware adapter probe |
| Hardware H.264 encoding (Intel QSV & NVIDIA MFT), GPU `D3D11` NV12 conversion | `capture_encode` benchmark; output verified frame-by-frame with `ffprobe` |
| `SendInput` injection, per-monitor DPI coordinate mapping, Unicode text | Live tests: 4/4 corner/center positions exact; `"Héllo 😀"` injected into a Win32 edit control |
| Host caret tracking (Win32 GUI thread info + UI Automation on editable controls) | 3/3 live tests on Win32 edit controls; verified read-only documents (e.g. web pages) are ignored |
| Held input auto-release on disconnect or session end | End-to-end test (`Ctrl` key and mouse buttons released on client disconnect) |
| Host session manager: pairing window, single controller, takeover, approval, revoke, emergency stop | 7 end-to-end loopback QUIC tests |
| Adaptive bitrate, stream planning, private-LAN address policy | Unit test suite in `session-manager` |
| DPAPI encryption at rest for host identity and trust store | Unit tests verifying encryption at rest and tamper rejection |
| mDNS / DNS-SD advertisement and discovery (`_lanlink._udp.local.`) | Self-discovery integration test on the Windows network stack |
| Session lock / UAC detection (`WTS` session notifications + token integrity) | Emulator verification: 0 input events injected while Windows session locked |
| Windows GUI host (`lanlink-host.exe`): `egui` window, system tray, `WDA_EXCLUDEFROMCAPTURE` border + stop bar, hotkey | 5 unit tests; verified `WDA_EXCLUDEFROMCAPTURE` (`0x11`) excludes the indicator from captured frames |
| Android app: QR & deep-link pairing (`lanlink://pair`) with confirmation card | Verified on emulator and Pixel 10 Pro; host records `PAIRED` |
| Android app: hardware `MediaCodec` streaming, surface recovery, cursor & caret follow | Verified on Pixel 10 Pro over Wi-Fi (`RTT 4–33 ms`, decode `p50 8–15 ms`, `0` dropped client frames) |
| Android app: `TrackpadCursor`, velocity curve, 4-row compact PC keyboard, edge control rail | 35 JVM unit tests + live installation on Pixel 10 Pro |

## Engineering Bugs Found & Resolved During Verification

| Issue | Root Cause | Resolution |
|---|---|---|
| Second WGC capture session in a process faulted | COM apartment uninitialized on worker thread | Pin process-wide COM multi-threaded apartment (`COINIT_MULTITHREADED`) |
| `DXGI Desktop Duplication` failed on hybrid Optimus laptop | dGPU cannot duplicate panel scanned out by iGPU | Prioritize `Windows.Graphics.Capture` (`WGC`); retain `DXGI DD` as fallback ([ADR 0004](adr/0004-capture-backend.md)) |
| Stale frame captured on locked Windows session | WGC frame pool held pre-lock surface | Query WTS session-lock state (`interactive_desktop_active()`) and drop non-release input while locked |
| Win32 indicator bar crashed on initial `WM_PAINT` | Hidden `1×1` window painted with empty label buffer | Guard `WM_PAINT` to draw only when visible with non-empty text |
| UI Automation caret probe matched entire browser document | Read-only `Document` elements expose `TextPattern` | Restrict UIA fallback to editable controls (`Edit` or writable `ValuePattern`) |
| Relative trackpad moves drifted from phone cursor | Windows pointer ballistics (`Enhance pointer precision`) and UDP datagram loss compounded relative deltas | Phone integrates motion into `TrackpadCursor` (`0..65535` normalized coordinates), sends idempotent absolute positions, and attaches click coordinates on the reliable input stream |
| Early input right after `bind_session` could be dropped | Client `input_writer` waited for `changed()` notification before opening the input stream | Open the session's QUIC input stream on demand inside the `rx.recv()` arm; covered by `input_sent_immediately_after_bind_is_not_lost` |
| Returning to `RemoteScreen` after surface recreation waited for `VIDEO_CONFIG` | Host sends `VideoConfig` once at session start; a recreated `VideoDecoder` started unconfigured | Cache active stream geometry in `RemoteSession` and initialize recreated `VideoDecoder` instances immediately while requesting an in-band keyframe + SPS/PPS |
| Backgrounding Android app during a drag left input held | `surfaceDestroyed` stopped the decoder without resetting `TouchInterpreter` or releasing sticky modifiers | Reset gesture state and invoke `releaseSticky` + `releaseAll` when the surface is destroyed |

## Failure Scenarios Matrix

| # | Scenario | Detection → Behaviour | Verification |
|---|---|---|---|
| 1 | Laptop enters sleep | QUIC idle timeout (`6 s`) → phone retries for `60 s`, then reports unreachable | Unit + timer logic |
| 2 | Laptop locked / UAC prompt | WTS lock flag → notice sent to phone, non-release input dropped | Verified on emulator |
| 3 | Android app backgrounded | `surfaceDestroyed` → held input released, decoder stopped; decoder recreated + keyframe requested on return | Implemented & unit-checked |
| 4 | Wi-Fi drop mid-session | QUIC idle timeout → host releases all held keys/buttons; phone reconnects | Verified by e2e test |
| 5 | GPU adapter reset | Capture `DeviceLost` / `AccessLost` → rebuild pipeline on working adapter | Exercised during dGPU reset |
| 6 | Hardware encoder failure | Encoder `failed()` → rebuild pipeline; fall back to software H.264 MFT | Implemented |
| 7 | Display resolution / DPI change | Capture size change → rebuild encoder + send updated `VideoConfig` | Implemented |
| 8 | Secondary monitor unplugged | Selected display missing → fall back to primary monitor + send notice | Implemented |
| 9 | Pairing declined on PC | `PairResult(accepted = false)` → clear error message on phone | Verified by e2e + unit tests |
| 10 | Host identity mismatch (MITM / reinstall) | Pinned TLS 1.3 verifier rejects certificate in handshake | Verified by e2e + emulator |
| 11 | Client disconnects while holding key/drag | Input worker drop releases all held keys and mouse buttons via `SendInput` | Verified by e2e test |

## Security Review (2026-11-17)

A multi-stage security audit was conducted across the protocol, pairing state machine, QUIC connection limiter, Windows input injector, and Android client, followed by an independent verification pass. All 16 findings (`H1–H2`, `M1–M5`, `L1–L4`, `E1–E5`) were remediated and covered by regression tests:

| ID | Severity | Finding | Remediation | Verification |
|---|---|---|---|---|
| H1 | High | Pairing guess counter incremented only on failed confirmation, allowing concurrent connections to exceed the 3-guess window limit | Reserve attempts atomically when an exchange starts (`PairingWindow::begin_attempt`); bind confirmation tickets to the active window | 3 unit tests + e2e `concurrent_pairing_attempts_share_the_attempt_limit` |
| H2 | High | Manual pairing (without QR fingerprint pin) accepted an unverified `"already paired"` response | Reject unpinned `Paired` responses during manual pairing (`ClientError::UnverifiedHost`) and require QR verification | e2e `manual_pairing_refuses_an_unverified_already_paired_reply` |
| M1 | Medium | Connection limit counted only completed handshakes; unpaired connections could linger | Atomic slot reservation at `accept` (8 total, max 4 unknown peers) + strict unpaired handshake deadline | e2e concurrency suite |
| M2 | Medium | External `lanlink://pair` deep links initiated pairing immediately | Deep links now display an explicit `PairState.Offer` confirmation card, strip non-private IP addresses, and consume the intent once | Unit + build verification |
| M3 | Medium | Network-supplied device/host names permitted Unicode bidirectional overrides and control characters | Sanitize all display names via `wire::display_name` (Rust) and `DisplayText.name` (Kotlin) | Rust & Kotlin unit tests |
| M4 | Medium | System IME could learn sensitive text typed into the remote PC | Set `IME_FLAG_NO_PERSONALIZED_LEARNING` on `KeyboardInputView` (and use built-in compact keyboard by default) | Verified in `RemoteView.kt` |
| M5 | Medium | Remote PC screen could appear in phone screenshots, recordings, and Recents switcher | Enable `WindowManager.LayoutParams.FLAG_SECURE` while on `Screen.Remote` by default (configurable in Privacy settings) | Verified in `MainActivity.kt` |
| L1 | Low | Pasted text longer than 400 chars was dropped by the token-bucket burst limit | Raise burst to a full `1024`-byte text record and pace presses/text instead of dropping; releases never wait | Unit tests `long_text_is_paced_not_dropped`, `presses_are_paced_to_the_rate_and_releases_never_wait` |
| L2 | Low | Unbounded `deleteSurroundingText` count from a misbehaving IME | Clamp per-request backspace/delete count to `MAX_IME_DELETE = 256` | Verified in `RemoteView.kt` |
| L3 | Low | Cancelling pairing on the phone left the native task running | Abort native pairing task on `pairCancel` | Verified in `lanlink-mobile` |
| L4 | Low | Unescaped client error strings in host log output | Escape control characters in structured tracing fields | Verified by `clippy` |
| E1 | Low–Med | Disconnecting during session approval left the PC prompt open for 60 s | Abort approval wait immediately when client connection closes | e2e test suite |
| E2 | Low–Med | Full input queue could drop a key/button release event | Close the connection on release queue overflow so worker teardown releases all held inputs | e2e test suite |
| E3 | Low–Med | LAN impostor advertising a paired PC's short mDNS ID could abort reconnection | Skip addresses that fail TLS identity pinning (`impostors` set) and continue trying remaining candidate addresses | Verified in `RemoteSession.kt` |
| E4 | Low | Failed `NsdManager` discovery start left listener reference non-null | Clear listener and callback state on failure so subsequent `start()` calls retry cleanly | Verified in `Discovery.kt` |
| E5 | Low | Malformed JSON control event could crash the client event thread | Catch `JSONException` per event and continue polling | Verified in `RemoteSession.kt` |

## iPhone app (`ios-app` branch)

Status: **in progress, not merged.** The app builds and passes its automated tests in CI. It has **not been run on a real iPhone** and has not controlled a real PC from an iPhone. Nothing in this section is a device result.

### Verified in CI (GitHub runners, commit `3df172a`)

| Check | Result |
|---|---|
| `cargo test -p lanlink-mobile -p lanlink-ios` (workflow `ios`, run 44398845842, `macos-15`) | **16 passed** (the C-ABI crate and the shared mobile core) |
| `cargo test --workspace` (workflow `CI`, run 44398845819, Windows runner) | **132 passed**, 3 ignored (excludes the integration-test crate and skips one capture test, see `ci.yml`) |
| Xcode `LANLinkTests` (Swift unit tests, iPhone simulator, run 44398845842) | **70 passed**, 0 failures |
| Xcode `LANLinkUITests` (simulator, same run) | **2 passed**: `testScreens` (walks the screens and saves the screenshots shown in the README) and `testRemoteKeyboard` |
| Device build (`generic/platform=iOS`, `CODE_SIGNING_ALLOWED=NO`) | Built, unsigned |
| C header check (`scripts/check-ios-header.py`) | Passed |
| Android `:app:testDebugUnitTest` and `:app:lintDebug` (workflow `CI`) | Passed |

The Swift unit tests include twins of the Android core-logic tests (viewport, touch, trackpad cursor, compact keyboard, key map, display text, paired host), plus tests for the stores (including the Keychain attributes), discovery, the Annex-B parser and video decoder, the remote session, the app-switcher privacy cover and key repeat. The "Screen recording is on" notice has no test. The decode path is tested with a small H.264 fixture on the simulator.

### Changes to shared and Android code made for the iPhone work

* **Input sent right after a session binds is no longer dropped.** The client opens the input stream on demand instead of waiting for a change notification (`input_sent_immediately_after_bind_is_not_lost`). This is in the shared client crate, so it applies to Android and iOS.
* **The stream is requested again after a reconnect.** Before this, a reconnect could stay on "Starting the stream…". Android (`RemoteSession.kt`) and iOS now request the stream once per connection. Covered on iOS by a unit test (`RemoteSessionTests`, `SessionRequestGate`). On Android the same change is made, and the existing unit tests and the debug build pass, but no Android test exercises this path and it is not verified on a device.

### Differences from the Android app

| Area | Android | iPhone |
|---|---|---|
| Identity protection | Android Keystore `AES-256-GCM`, excluded from backup | iOS Keychain, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (not synced) |
| Screen privacy | `FLAG_SECURE` blocks screenshots, recording and Recents | iOS cannot block a user's own screenshots. The app shows a privacy cover in the app switcher and a "Screen recording is on" notice that hides the video |
| Network permission | None before Android 17 | Local Network permission prompt |
| Text input | IME composition (Chinese, Japanese, Korean) supported | No composing text for Chinese, Japanese or Korean keyboards |
| Mouse | Hover and scroll wheel from a connected mouse | No hover or scroll wheel |
| Hardware keys | Auto-repeat | No hardware-key auto-repeat |

### Not verified

None of the following has been tested on a real iPhone:

* A real iPhone, and a real PC controlled from an iPhone.
* Device latency (no iPhone latency numbers exist).
* Real Bonjour discovery, the Local Network prompt, and what happens when it is denied.
* The camera QR scanner.
* A pairing link that cold-launches the app.
* The app-switcher cover and the screen-recording notice.
* Reconnect after the app was in the background.
* Hardware keyboards.
* Gesture feel.
* Decoding a real H.264 stream from a PC (only the small test fixture is decoded).
* The alert tint on iOS 16.
* Teardown while a pairing job is running.

## Roadmap / Not Yet Implemented

* Authenticode code-signing certificate and MSIX/WiX installer for `lanlink-host.exe`.
* HEVC / AV1 codecs and unreliable QUIC datagram video mode with forward error correction (FEC).
* Remote audio streaming and opt-in clipboard synchronization.
* Android 17 (API 37) `ACCESS_LOCAL_NETWORK` runtime permission migration ([ADR 0011](adr/0011-android-local-network-permission.md)).
* iPhone app: run it on a real iPhone and complete the checks under [Not verified](#not-verified) before calling it supported.
* High-speed camera physical input-to-photon latency measurement.
