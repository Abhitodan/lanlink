# Changelog

All notable changes to LANLink are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-11-17

### Added
- **Windows Host (`lanlink-host.exe` & `lanlink-host-console.exe`):**
  - Native Windows GUI application (`egui` + system tray + activity log + device management) alongside the headless/terminal host.
  - Non-intrusive on-screen control border and stop bar excluded from screen capture (`WDA_EXCLUDEFROMCAPTURE`) plus global emergency-stop hotkey (`Ctrl+Alt+Shift+Esc`).
  - Zero-copy GPU capture pipeline using Windows Graphics Capture (`WGC`) with automatic fallback to `DXGI Desktop Duplication`.
  - GPU `BGRA` → `NV12` conversion (`ID3D11VideoProcessor`) feeding asynchronous Media Foundation hardware H.264 encoders (Intel Quick Sync, NVIDIA NVENC, AMD AMF, with software fallback).
  - Multi-monitor enumeration, live display switching, and per-monitor DPI-aware `SendInput` coordinate mapping across negative virtual-desktop origins.
  - Protocol v2 live text-caret tracking via Win32 GUI thread info and Windows UI Automation (`IUIAutomationTextPattern` / `ValuePattern` on editable controls).
  - Session lock and UAC secure-desktop detection via WTS session notifications and token integrity checks.
  - Per-user `DPAPI` encryption at rest for the host identity key and trusted-device store.
- **Shared Rust Core (`crates/lanlink-*`):**
  - Strict, length-bounded binary wire protocol (`lanlink-protocol` v1 & v2) validated with unit tests, canonical test vectors, and `proptest` fuzzing (16,000 cases per run).
  - Mutual TLS 1.3 over QUIC (`quinn` + `rustls` + `ring`) with `ECDSA P-256` certificate fingerprint pinning and 0-RTT disabled.
  - Channel-bound `SPAKE2` pairing (`EXPORTER-lanlink-pair-v1`) with atomic 3-attempt window budgets and 6-digit user confirmation codes.
  - mDNS / DNS-SD (`_lanlink._udp.local.`) LAN advertisement and discovery.
  - Platform-neutral mobile client bridge (`lanlink-mobile`) with handle-isolated session registry, Annex-B parameter-set extraction, and fake-host integration test suite.
- **Android App (`com.lanlink.app`):**
  - Jetpack Compose UI (`Onboarding`, `Home`, `Pair`, `Host Details`, `Remote Session`, `Settings`, `Performance Diagnostics`, `Security`).
  - Hardware-accelerated low-latency H.264 `MediaCodec` decoding direct to `SurfaceView` with automatic surface/decoder recovery and keyframe resynchronization.
  - Phone-integrated `TrackpadCursor` with precision slow-motion zone, velocity acceleration, dead-zone follow camera (`60%` central region), and caret-follow panning when typing.
  - Built-in 4-row compact PC keyboard (`Esc`, `Tab`, `Ctrl`, `Alt`, `Win`, arrow keys, key-repeat, one-shot/locked Shift) plus system IME support with `IME_FLAG_NO_PERSONALIZED_LEARNING`.
  - Auto-hiding left/right edge control rail, `Fit` / `2×` / `1:1` zoom presets, and three-finger tap keyboard toggle.
  - `FLAG_SECURE` screen protection by default during remote sessions and `Android Keystore` `AES-256-GCM` identity key wrapping.
  - QR code scanning (`lanlink://pair`) with link-confirmation protection and private-LAN address filtering.
