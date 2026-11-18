# LANLink implementation ledger

Last updated: 2026-11-17. Only what has evidence behind it is marked done.
"Emulator" = Android Studio emulator (Pixel 9 Pro XL image, Android 16,
x86_64, software rendering), reaching the PC through `10.0.2.2`.
**No physical phone has been tested.**

## Environment

* Windows 11 Home 26200, i9-13900HX, Intel UHD + RTX 4060 Laptop (hybrid),
  2560×1600 @ 240 Hz, 150 %.
* Rust 1.100 MSVC; VS Build Tools 2022; SDK 26100; Android SDK 34–36,
  NDK 28.2, JDK 21, Gradle 8.12, cargo-ndk 4.1.2.
* No Android device attached to adb at any point.
* On 2026-11-15 the RTX 4060 was in a "GPU lost" state (`nvidia-smi`);
  by 2026-11-16 it had recovered. Both states were exercised.
* DXGI Desktop Duplication is unsupported on this laptop in every
  adapter/output combination (ADR 0004).
* Wi-Fi profile is *Public*; no firewall rule was created (needs the user).

## Automated checks (latest run)

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` | clean |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo test --workspace -- --test-threads=1` | 126 passed (119 unit/property + 7 end-to-end), 3 ignored (live input/caret; passed when run explicitly) — with an unlocked desktop |
| Android `testDebugUnitTest` | 34 passed |
| Android `lintDebug` | 0 errors, 37 warnings (style, dependency versions, target SDK) |
| Android `assembleDebug` / `assembleRelease` | built; release APK 10.3 MB (unsigned) |
| Android native library | built for arm64-v8a and x86_64 (≈4 MB each) |

Live and end-to-end Rust tests skip with a printed reason when the Windows
session is locked.

## Done, with evidence

| Feature | Evidence |
|---|---|
| Wire protocol (typed, bounded, versioned, test vectors) | 23 unit + 8 property tests (2000 cases each) |
| Identity, SPAKE2 pairing, trust store, QR link | 20 tests incl. wrong code, MITM certificate, different TLS session |
| QUIC transport with pinned mutual TLS | 5 loopback handshake tests incl. wrong fingerprint |
| Monitor enumeration, WGC capture, DD fallback, cursor shapes | 8 unit + 5 live tests; probe on real display |
| Hardware H.264 (Intel QSV, NVIDIA MFT), GPU NV12 conversion | `capture_encode`; ffprobe decodes every frame |
| SendInput injection, exact pixel mapping, Unicode text | live tests: 4/4 positions exact; "Héllo 😀" typed into a real edit control |
| Held input released on disconnect | e2e test (Ctrl released after the phone vanished) |
| Host session manager: pairing, sessions, single controller, takeover, approval, revoke, emergency stop | 4 e2e tests, 5/5 runs |
| Adaptive bitrate, stream planning, address policy | unit tests |
| DPAPI storage of host identity and trust list | unit tests (tamper rejected, encrypted at rest) |
| mDNS advertise and browse | self-discovery test on the real network stack |
| Console host with QR pairing | ran; QR rendered; identity persisted under `%LOCALAPPDATA%\LANLink` |
| Session lock detection (WTS) and lock handling | probe reports locked; host notifies the phone; **emulator: 0 input events injected while locked** |
| **Android app: pairing via deep link** | emulator: paired with confirmation code; host log `PAIRED` |
| **Android app: Windows screen displayed** | emulator screenshot shows the PC's (lock) screen, letterboxed, with cursor overlay |
| **Android app: touch input reaches the PC** | emulator taps → host recorded click at the centre and near the corner; swipe → hover moves |
| **Android app: pairing closed / identity mismatch messages** | emulator screenshots; mismatch rejected in TLS |
| Android app: automatic reconnection attempt | emulator log shows reconnect after host restart |
| **Real phone over Wi-Fi (2026-11-17)** | Pixel 10 Pro, Android 17 (app targets 35), same /24 Wi-Fi as the PC. Paired by QR, live video streamed; user reported it "looks good". Host stats over ~80 s: RTT 4–33 ms, phone received/rendered up to 24 fps (frames only on change), phone decode p50 8–15 ms, 0 client frames dropped, encode p50 ≈ 26 ms (NVIDIA MFT, host built `dev` + opt-level 2). Input on the phone not separately logged. |

| Windows host app `lanlink-host` (egui window, tray, indicator, hotkey) — 2026-11-17 | runs on this PC (port 47991 for the check, beside the console host); window renders Home/Live/Activity (screenshot); stays up > 30 s; indicator border + bar visible with `WDA_EXCLUDEFROMCAPTURE` confirmed (affinity 0x11, absent from a screen grab); 5 unit tests |
| Protocol v2 `TextInput` / `Caret` (2026-11-17) | test vectors + property tests; e2e: phone-side client opens the keyboard on a real host session → host sends a caret report, connection stays healthy; v1 peers negotiate 1 and never see the new tags |
| Host caret finder (Win32 caret, then UI Automation; editable controls only) | live test 3/3 on a real Win32 edit control: both paths land inside the control; probe on Chrome: a web page (read-only document) is correctly **not** reported (first version wrongly reported the whole page; fixed) |
| Live "ask before every session" switch | `HostCommand::SetSessionApproval`; compiles, clippy clean; not exercised with a phone |

## Partially done / not verified

| Item | State |
|---|---|
| Host app: pairing page, approval dialog, Phones page, Settings, tray menu, Stop bar click, emergency hotkey | implemented; not clicked through (the test tooling here cannot drive a non-allowlisted window); the hotkey registered (no "taken" warning) |
| Compact LANLink keyboard (default; phone keyboard selectable in Settings) | 3 unit tests (layout, shift/caps lock); installed; **screenshot on the Pixel 10 Pro**: centred, ~67 % of the width and ~39 % of the height in landscape (system keyboard was ~60 %), control rail beside it. Typing through it not yet confirmed by the user |
| Keyboard over the picture + caret follow, right-edge control rail (2026-11-17) | built; 4 new unit tests (reveal pan, keyboard-region clamp); APK installed on the Pixel 10 Pro; **not yet tried by the user**. Caret follow needs a v2 host: the console host still running from earlier is v1, so the phone falls back to following the pointer until the PC side is restarted on the new build. Caret in Chrome/Electron text boxes not yet verified (needs focus in such a box) |
| Host app on UDP 47990 with a phone | not yet: the console host held the port during this work, and Windows Firewall will ask once for the new exe |
| Phone control redesign (2026-11-17): landscape while controlling, trackpad by default starting at 2×, follow camera (middle-60 % dead zone), precision curve, zoom presets Fit / 2× / 1:1, floating control button with an always-visible keyboard button, three-finger tap for keyboard, click haptics | installed on the Pixel 10 Pro. First try by the user: **trackpad wrong, typing impossible** (root causes below). Fixed build installed; **not yet re-tried by the user**. The magnifier was removed: on this phone `android.widget.Magnifier` cannot read the SurfaceView ("failed to copy content from the view Surface", logged on every touch event) |

## Earlier partial items

| Item | State |
|---|---|
| Continuous live video on Android | only 2 frames observed: the PC was **locked** during the emulator run, so Windows composed no new frames. Needs a run with the PC unlocked. |
| Soft keyboard and IME composition | implemented (`KeyboardInputView`); not exercised on a device |
| Trackpad mode, pinch zoom, two-finger scroll/right-click | gesture logic unit-tested; not exercised on a device (adb cannot multi-touch) |
| Display switching, rotation, decoder recovery | implemented; not exercised (single display; no rotation test yet) |
| Diagnostics screen values | implemented; latency estimate not yet observed |
| Hardware keyboard routing | implemented; not tested |
| mDNS discovery on Android (NsdManager) | implemented; emulator NAT does not forward mDNS, so not tested |

## Not started

* Installer and code signing for `lanlink-host` (the app, tray, indicator
  and start-at-login switch now exist — see above).
* NVENC SDK backend, HEVC/AV1, video over datagrams/FEC.
* Clipboard sync, audio.
* Android 17 `ACCESS_LOCAL_NETWORK` flow (target SDK is 35).
* Impaired-network test rig; long-duration soak tests; battery measurement.
* `cargo audit` / licence report (tools not installed); release signing.
* Full real-phone test pass: first live session done 2026-11-17 (above);
  IME, gestures, rotation, latency percentiles still to do.

## Bugs found and fixed by testing

| Bug | Found by | Fix |
|---|---|---|
| WGC crashed (access violation) on the second capture in a process | live capture tests, 3/3 runs | pin the COM MTA for the process |
| Desktop Duplication tried only the listed adapter | probe on hybrid laptop | WGC first; DD fallback |
| Lock screen reported as "available" | animated benchmark got one stale frame | WTS session-lock flag |
| Input accepted while locked (incl. pointer moves inside releases) | emulator run | drop non-release input; strip release positions while locked |
| Host app crashed ~15 s after start (0xC000041D in user32) | first launches; bisected to the indicator thread | the hidden 1×1 bar got a first WM_PAINT and drew text from an empty buffer; paint now only when visible with text |
| Mutex guard held across `.await` | compiler | restructured active-session claim |
| Caret finder reported a whole Chrome web page as the "text field" | probe on this PC (focused element was a read-only Document with TextPattern) | only editable elements (Edit control, or writable Value pattern); whole-field fallback only for Edit controls |
| Trackpad pointer drifted from the one drawn on the phone; clicks landed elsewhere | user on the Pixel 10 Pro; code + PC settings: relative moves went to Windows as `MOUSEEVENTF_MOVE`, which applies the PC's pointer speed and "Enhance pointer precision" (on here: `MouseSpeed=1`) on top of the phone's curve, and every lost datagram (host showed 200–300 lost packets) lost motion | phone owns the position (`TrackpadCursor`) and sends absolute moves (loss-tolerant, unaffected by PC mouse settings); clicks carry the position on the reliable stream; final position re-sent on finger lift; 4 unit tests |
| Could not type | user; phone log had **no** show-keyboard request from LANLink during the session; keyboard was two taps deep in the panel; `SHOW_IMPLICIT` may be ignored; three-finger tap failed when fingers landed unevenly (first two read as pinch) | keyboard button always on screen; show after focus settles (`post`), flags 0 plus `WindowInsetsController.show(ime)`; toggling follows the real keyboard state; third finger within 150 ms still counts (2 unit tests); `LANLinkKeyboard` log lines for diagnosis |

## Failure scenarios (spec §24)

| # | Scenario | Detection → behaviour | Tested |
|---|---|---|---|
| 1 | Laptop asleep | QUIC idle timeout (6 s) → phone reconnects 60 s, then "cannot reach" | no |
| 2 | Laptop locked | WTS lock flag → notice to phone, input dropped | **emulator** |
| 3 | Android to background | surface destroyed → decoder stops; stream continues; keyframe on return | no |
| 4 | Wi-Fi drops | idle timeout → host releases input; phone reconnects | host side by e2e (disconnect) |
| 5–6 | PC or phone changes network | connection lost → reconnect over known addresses | no |
| 7 | GPU path switch | capture `DeviceLost`/`AccessLost` → rebuild on a working adapter | adapter fallback observed on GPU-lost day |
| 8 | GPU encoding fails | encoder `failed()` → rebuild; software fallback | no (fault not injected) |
| 9 | Phone cannot decode codec | decoder error → restart; capabilities limit size | no |
| 10 | Resolution change | content size change → AccessLost → rebuild + new Config | no |
| 11 | Monitor hot-plug | display gone → fall back to primary + notice | no |
| 12 | Scale change | coordinates are physical pixels (per-monitor DPI aware) | no |
| 13 | Pairing denied | PairResult denied → message | e2e (session denied); pairing-denied path unit-level |
| 14 | Identity mismatch | TLS pin fails → message | **e2e + emulator** |
| 15 | Local-network permission revoked | n/a at target 35 (ADR 0011) | no |
| 16 | Firewall blocks | connect timeout → message naming the firewall | no |
| 17 | Decoder stops producing | — (watchdog not implemented) | no |
| 18 | Client gone while dragging | held button released | e2e (key); button by unit test |
| 19 | PC reboots | identity persists; phone reconnects when host runs | no |
| 20 | Host crashes | phone reconnects; Windows releases nothing held by a dead process — see note | no |

Note (20): if the host process itself dies while a key is held, LANLink cannot
send the release; Windows does not auto-release injected keys. Not yet
mitigated.

## Security review, 2026-11-17

Method: the Claude-BugHunter workflow (map the attack surface, hunt by bug
class, validate before reporting), applied to source review rather than a
web target. The hunt used its race-condition, brute-force/rate-limit and
Android-app checklists. A second reviewer (a separate read-only agent)
challenged every finding and found five more (E1–E5).

| # | Severity | Finding | Fix | Evidence |
|---|---|---|---|---|
| H1 | High | Pairing guesses were counted only when a confirmation failed, so concurrent connections each got a guess while the window looked open (about 8–11 per window instead of 3). Closing pairing on the PC did not stop an exchange in flight | an attempt is reserved atomically when the exchange starts (`PairingWindow::begin_attempt`); the confirmation must present a ticket for the same, still-open window; malformed SPAKE2 messages give their attempt back | 3 unit tests; e2e `concurrent_pairing_attempts_share_the_attempt_limit` |
| H2 | High | Manual pairing (no QR, so the PC's identity is not pinned) accepted a device's "already paired" reply with no code check, and the app saved that device as a trusted PC | refused unless the identity was pinned (`ClientError::UnverifiedHost`), with a message to scan the QR code | e2e `manual_pairing_refuses_an_unverified_already_paired_reply` |
| M1 | Medium | The connection limit counted only finished handshakes (a burst got past it); unpaired connections could stay idle forever during a pairing window | atomic slot reservation at accept; at most 4 of 8 slots for unknown devices; unpaired deadline starts at HelloAck | e2e (the concurrent-pairing test runs into the cap) |
| M2 | Medium | Any web page could open `lanlink://pair` and make the phone pair at once with addresses and an identity it chose | links now only propose pairing (a confirmation card); QR addresses outside the local network are dropped; the link is consumed once | builds; not tried on the phone |
| M3 | Medium | Device and PC names allowed text-direction overrides and control characters (a name could read as something else in the approval dialog) | `wire::display_name` on the host and in the client; `DisplayText` for mDNS names on the phone | unit tests (Rust and Kotlin) |
| M4 | Medium | The phone keyboard could learn everything typed into the PC | `IME_FLAG_NO_PERSONALIZED_LEARNING` (the default compact keyboard uses no IME at all) | builds |
| M5 | Medium | The PC screen appeared in screenshots, screen recordings and the recent-apps preview | `FLAG_SECURE` while controlling; setting "Allow screenshots of the PC screen" (off by default) | builds; not tried on the phone |
| L1 | Low (functional) | Text over 400 characters was always refused by the rate limiter, and the phone was told it arrived | burst raised to one full text message (1024); text and presses are paced to the rate (presses wait at most 50 ms) instead of dropped, so a keystroke right after a paste still lands; releases never wait | unit tests `long_text_is_paced_not_dropped`, `presses_are_paced_to_the_rate_and_releases_never_wait` |
| L2 | Low | An IME could make the phone send an unbounded number of Backspace taps | clamped to 256 per request | builds |
| L3 | Low | Cancelling pairing left the exchange running (PC prompt stayed up) | the native task is aborted | builds |
| L4 | Low | A client error message could forge host log lines | logged with escaping | clippy |
| E1 | Low–Med | A phone that disconnected left its approval prompt on the PC for 60 s and held the session slot | approval wait ends when the connection closes; the PC drops prompts nobody waits for | e2e suite still passes |
| E2 | Low–Med | A key/button release could be dropped when the input queue was full, leaving it held on the PC | the connection is closed instead; the worker's drop releases everything | e2e suite still passes |
| E3 | Low–Med | A LAN device advertising the PC's public id could make reconnecting abort with "not the PC you paired with" | addresses that answer with another identity are skipped, the rest are still tried | builds |
| E4 | Low | Discovery stayed stuck after a failed start or registration | listener/callback cleared so the next attempt retries | builds |
| E5 | Low | A malformed event could crash the session thread | caught and ignored | builds |

Review loop: three rounds with the second reviewer. Round 2 confirmed 14
fixes and found a regression (keystrokes refused right after a paste) and an
onboarding gap for links; both were fixed in round 3.

Accepted residual risks (documented, not fixed): during a burst of
handshakes from strangers, all 8 slots can be busy for the few milliseconds
before the unknown-device cap applies; a phone reconnecting with the same
identity replaces its own older connection before session approval; other
native-bridge JSON paths trust the app's own library.

Not changed, by decision: only one pairing exchange in flight per window
(would hand a LAN attacker an easy way to block pairing; the attempt
reservation already bounds guesses). Not yet done: asking the PC whether
the focused field is a password (UIA `IsPassword`) to switch the phone
keyboard to password mode.

Incident during this work: a failed in-place edit truncated
`CoreLogicTest.kt`. It was rebuilt by replaying every recorded edit from
the session transcript (33 tests, the same count as before), and all
tests pass. Edit scripts now write to a temporary file and rename it.

## GitHub package, 2026-11-17

README rewritten for GitHub, with a launch GIF/MP4, use-case, pipeline and pairing diagrams, and real screenshots. Added LICENSE-MIT, LICENSE-APACHE, SECURITY.md, CONTRIBUTING.md, issue/PR templates and a CI workflow. `scripts/build-host.ps1` now builds the PC app too.

- **Launch film** (`docs/media/launch.mp4`, 25 s, 1920×1080, 750 frames): built the brag way (plan in `brag-output/brag-plan.md`) with HyperFrames. The check passed with no errors. It is silent: no music source here.
- **Screenshots:** phone screens from the Android emulator with fictional data. The PC app is cropped to leave out the real computer name and address. The keyboard is cropped from a real phone screenshot, with the desktop behind the keys removed.
- **Test fixtures:** personal values (a name and a LAN address) replaced with neutral ones; tests pass.
- **CI workflow: not yet run on GitHub.** No repository has been created or pushed.
