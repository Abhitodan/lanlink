# Performance & Benchmarks

## Summary

- **PC capture & hardware encode:** On a `2560×1600 @ 240 Hz` hybrid laptop display, GPU colour conversion (`BGRA` → `NV12`) and encoder submission take **`0.8 ms` p50** (`1.3–1.6 ms` p95) with zero CPU pixel copies. Total time from Windows desktop composition to encoded H.264 access unit is **`13.5–14.5 ms` p50** (`24.0–30.0 ms` p95).
- **Real phone over Wi-Fi (Pixel 10 Pro):** Over local `/24` Wi-Fi, measured QUIC round-trip time is **`4–33 ms`**, Android hardware `MediaCodec` decode time is **`8–15 ms` p50**, and **`0` client frames were dropped** across an 80-second live session.

## Test Hardware & Environment

| Component | Specification |
|---|---|
| Host CPU / RAM | Intel Core i9-13900HX, 31.7 GB RAM |
| Host GPUs | Intel UHD Graphics (iGPU) + NVIDIA RTX 4060 Laptop (driver 610.60), hybrid Optimus |
| Host Display | Internal `2560×1600 @ 240 Hz`, 150% DPI scale, SDR |
| Host OS | Windows 11 Home (build 26200) |
| Phone | Google Pixel 10 Pro, Android 17 (API 35 target), hardware H.264 `MediaCodec` |
| Network | Local `/24` Wi-Fi LAN (plus loopback `127.0.0.1` for deterministic integration benchmarks) |

## Methodology

- **PC stage benchmark (`scripts/benchmark.ps1` / `capture_encode` example):** Captures the primary display using the production backend, converts on the GPU, encodes `1920×1200 @ 60 fps` target at `12 Mbit/s` CBR, writes `.h264`, and validates every frame with `ffprobe`.
  - *compose → acquired*: Windows composition timestamp (`QPC`) to frame acquisition.
  - *convert + submit*: CPU time to dispatch GPU `NV12` conversion and hand the `ID3D11Texture2D` to the Media Foundation MFT.
  - *compose → encoded*: Composition timestamp to encoder output callback on a single monotonic clock (`QPC`).
- **Loopback integration benchmark:** Host and client in one process on `127.0.0.1` sharing a single monotonic clock.
- **Live Wi-Fi telemetry:** Measured via the protocol's 1 Hz `StreamStats` / `ClientStats` and `Ping` / `Pong` RTT probes during an interactive session on a Pixel 10 Pro.

## Measured Results

### 1. PC Capture & Hardware Encoding Stages

| Run | Hardware Encoder | Samples | compose → acquired (p50 / p95) | convert + submit (p50 / p95) | compose → encoded (p50 / p95) |
|---|---|---:|---|---|---|
| Intel iGPU path | Intel Quick Sync H.264 MFT | 42 | `3.2 / 6.3 ms` | `0.8 / 1.6 ms` | `14.5 / 24.0 ms` |
| Discrete GPU path | NVIDIA H.264 MFT | 11 | `12.5 / 22.6 ms` | `0.8 / 1.3 ms` | `13.5 / 30.0 ms` |

*Note:* On a hybrid laptop where the internal panel is scanned out by the integrated GPU, capturing on the discrete GPU incurs an OS cross-adapter copy in *compose → acquired*.

### 2. Loopback QUIC End-to-End Timings

| Metric | Measured Value |
|---|---|
| First keyframe (capture → client received, cold encoder start) | `38–121 ms` across 6 runs |
| Emergency stop on PC → client connection terminated | `0.18–0.24 ms` (5 of 6 runs) |
| Static desktop idle wakeups (`next_frame` over 400 ms) | `4–6` frames (event-driven wait, zero busy-spinning) |

### 3. Real Phone Over Wi-Fi (Pixel 10 Pro)

| Metric | Measured Value |
|---|---|
| Network round-trip time (QUIC Ping/Pong) | `4–33 ms` |
| Phone hardware H.264 decode time (`MediaCodec` queue → output) | `8–15 ms` p50 |
| Host encode time during live session | `≈ 26 ms` p50 |
| Rendered frame rate (on-change desktop updates) | up to `24 fps` |
| Client frames dropped | `0` |

## Pipeline Optimizations

| Optimization | Impact |
|---|---|
| Windows Graphics Capture (`WGC`) primary backend | Works reliably across hybrid iGPU + dGPU laptops where `DXGI Desktop Duplication` returns `DXGI_ERROR_UNSUPPORTED` ([ADR 0004](adr/0004-capture-backend.md)) |
| GPU `ID3D11VideoProcessor` `BGRA` → `NV12` + texture-backed MFT input | Eliminates CPU readback and stride conversion; `convert + submit` stays around `0.8 ms` ([ADR 0005](adr/0005-encoder-strategy.md)) |
| Direct `ByteBuffer` JNI write into `MediaCodec` input buffers | Eliminates intermediate Java `byte[]` allocations on the video path ([ADR 0007](adr/0007-jni-boundary.md)) |
| Drop-never-queue (`VIDEO_QUEUE = 3`, `FrameGate`) | Prevents latency accumulation under network jitter; forces an immediate self-contained IDR keyframe on any drop ([ADR 0009](adr/0009-latency-vs-quality.md)) |
