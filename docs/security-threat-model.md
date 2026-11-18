# Security threat model

LANLink lets a phone see and control a Windows session. Compromise means
full control of the user's desktop, so the design assumes a hostile local
network.

## Assets

1. Control of the Windows session (input injection).
2. The screen contents.
3. The host identity key and the paired-device list (whoever can add a
   fingerprint can control the PC).
4. The phone identity key.

## Trust boundaries

| Boundary | Crossing | Control |
|---|---|---|
| Network → host process | UDP datagrams, QUIC handshakes | address filter, QUIC limits, TLS 1.3 mutual auth |
| Unauthenticated peer → host logic | Hello, pairing messages | strict bounded decoders; only pairing allowed before trust |
| Paired phone → Windows input | input records | session + sequence checks, view-only, rate limit, SendInput only in the user session |
| Host process → disk | identity, trust store | DPAPI (current user) |
| Phone app → disk | identity | AES-256-GCM key in Android Keystore; excluded from backup |
| Phone UI → native | JNI | handle registry, panic catching |
| iPhone app → disk | identity | iOS Keychain, `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (not synced) |
| iPhone UI → native | C ABI (`ios/native-bridge`) | handle registry, panic catching |

## Threats and mitigations

| Threat | Mitigation | Evidence |
|---|---|---|
| Someone on the Wi-Fi connects and controls the PC | Unknown certificates get `NotPaired` and are closed; pairing needs the PC user to open a window, the 8-digit code, and approval on the PC | e2e `unpaired_wrong_code_impostor_and_revoked_are_refused` |
| Guessing the pairing code | 10^8 codes, 3 attempts per 120 s window, counted when an exchange *starts* (atomic, so concurrent connections share them), window closes after failures, code single-use, an exchange cannot outlive its window | `pairing::tests::*`, e2e `concurrent_pairing_attempts_share_the_attempt_limit` |
| Machine-in-the-middle during pairing | SPAKE2 identities = both cert fingerprints; TLS exporter in key confirmation; 6-digit confirmation number compared by the user; QR pins the host fingerprint | `mitm_with_different_certificate_fails_even_with_right_code`, `different_tls_session_fails`, e2e identity-mismatch case |
| Impersonating a paired PC later | Client pins host fingerprint in the TLS handshake; manual pairing never accepts an unverified "already paired" reply; addresses answering with another identity are skipped | `wrong_pinned_fingerprint_is_rejected_in_the_handshake`, e2e `manual_pairing_refuses_an_unverified_already_paired_reply` |
| A web page or app opens a `lanlink://pair` link | The link only proposes pairing; the user confirms; addresses outside the local network are dropped | design |
| Spoofed names (text-direction overrides, control characters) | Names from the network are cleaned before display or storage | `display_name` tests |
| Connection flooding from the LAN | Slots reserved atomically at accept (8); unknown devices limited to 4; unpaired connections closed after 90 s | e2e |
| PC screen leaking on the phone | `FLAG_SECURE` while controlling (opt-out setting); keyboard asked not to learn typed text | design |
| Theft of the iPhone identity key | Stored in the iOS Keychain, readable only after the first unlock since boot and only on this device (`ThisDeviceOnly`: not in iCloud Keychain, not restored from a backup to another device). A transient Keychain error never deletes the identity | `IdentityStore` unit tests (Keychain attributes) |
| PC screen leaking from the iPhone (app switcher, screen recording) | A privacy cover hides the app in the app switcher. While iOS reports the screen is being recorded or mirrored, a "Screen recording is on" notice hides the video. iOS gives apps no way to block a user's own screenshots, so there is no equivalent of `FLAG_SECURE` (see residual risks) | Swift unit tests for the cover logic; **not yet checked on a real iPhone** |
| Impersonating a paired phone | TLS proves key possession; host authorizes by fingerprint (constant-time compare) | transport tests |
| Replay | TLS 1.3, no 0-RTT; per-session input sequence (gaps/duplicates close the connection); random session ids | `sequencer_rejects_duplicates_and_gaps` |
| Malformed or oversized messages | Every length checked before allocation; strict decoding; property tests over random and bit-flipped input | `tests/fuzz.rs` (8 properties × 2000 cases) |
| Input flooding | Bounded input queue (256) and token bucket (2000/s, burst 400); releases never rate-limited | `rate_limit_blocks_presses_but_never_releases` |
| Stuck keys after a crash or Wi-Fi drop | Held-state tracking; release on session end, disconnect and worker drop | e2e `pair_stream_control_and_release_on_disconnect` |
| Hidden remote access | No background service; console/tray host only while the user runs it; host shows who is controlling; emergency stop | e2e emergency-stop case |
| Revoked phone keeps access | Revoke removes trust and closes the live connection immediately | e2e revoke case |
| Exposure to the Internet | Handshakes refused from non-private addresses; no UPnP/port mapping | `netpolicy::tests` |
| Secure desktop / UAC / lock screen | Not captured or controlled; reported to the phone; no bypass | design (`interactive_desktop_active`) |
| Elevated windows (UIPI) | Windows drops injected input; LANLink detects and tells the user, never elevates | `foreground_is_higher_integrity` |
| Theft of host key/trust store | DPAPI-encrypted per Windows user; atomic writes | `storage::tests` |
| Sensitive data in logs | Message logs use names only (`ControlMsg::name`); no key material in `Debug` | `debug_output_hides_key` |

## Residual risks

* **The private-address filter is not a security boundary.** Guest Wi-Fi,
  VPNs and compromised LAN devices all have private addresses.
  Authentication is the boundary.
* A paired phone that is stolen and unlocked can control the PC until it is
  revoked on the PC. Session-approval mode (`--approve-sessions`) mitigates.
* `spake2` and `ring` are third-party crypto; `spake2` is pre-1.0 and
  unaudited.
* **iPhone screenshots cannot be blocked.** On iOS the user can take a
  screenshot of the remote screen at any time. The privacy cover and the
  screen-recording notice reduce accidental exposure; they do not stop a
  deliberate screenshot, and they have not been tested on a real iPhone.
  Android's `FLAG_SECURE` has no iOS counterpart.
* Screen contents are visible to whoever holds the paired phone; protected
  content stays black only where Windows enforces it.
* Denial of service from the LAN (UDP flood) is not mitigated beyond QUIC's
  own limits.
* The Windows firewall rule is the user's decision; the console host prints
  guidance but does not change firewall settings.
