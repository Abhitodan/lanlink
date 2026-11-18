# Architecture decision records

Each record states the decision, the alternatives rejected and the evidence.
"Measured" means a number from this repository's tests or tools on the
development machine (i9-13900HX, Intel UHD + RTX 4060 Laptop, Windows 11
26200, 2560x1600 @ 240 Hz panel). Anything else is reasoning, not data.

| # | Decision | Status |
|---|---|---|
| 0001 | Rust for the host core and the shared protocol | Accepted |
| 0002 | Kotlin + Compose for the Android UI, Rust only for networking | Accepted |
| 0003 | QUIC (Quinn) instead of WebRTC | Accepted |
| 0004 | Windows Graphics Capture first, DXGI Desktop Duplication fallback | Accepted (measured) |
| 0005 | Media Foundation hardware encoder on the capture adapter | Accepted (measured) |
| 0006 | H.264 only in the first release | Accepted |
| 0007 | JNI boundary: handles, direct buffers, JSON for control events | Accepted |
| 0008 | Pairing with SPAKE2 bound to certificates and the TLS channel | Accepted |
| 0009 | Latency over quality: drop, never queue | Accepted |
| 0010 | Licensing: MIT OR Apache-2.0, permissive dependencies only | Accepted |
| 0011 | Android target SDK 35; local-network permission plan for API 37 | Accepted |

See the individual files in this directory.
