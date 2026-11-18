# Performance

**Bottom line: no end-to-end (phone glass) latency has been measured yet.**
What exists are PC-side stage measurements with small samples on a mostly
static desktop, and loopback timings from the integration tests. They show
the PC pipeline is not the bottleneck; they do not show phone-side latency.

## Hardware and conditions

| Item | Value |
|---|---|
| CPU | Intel Core i9-13900HX, 31.7 GB RAM |
| GPUs | Intel UHD Graphics (iGPU) + NVIDIA RTX 4060 Laptop (driver 610.60), hybrid |
| Display | internal 2560×1600 @ 240 Hz, 150 % scale, SDR |
| OS | Windows 11 Home 26200 |
| Network | none (PC-only measurements and loopback QUIC) |

## Methodology

* `capture_encode` example (`scripts/benchmark.ps1`): captures the primary
  display with the production backend, encodes 1920×1200 @ 60 fps target,
  12 Mbit/s CBR, writes `.h264`, verified with `ffprobe`.
  * *compose → acquired*: Windows composition timestamp (QPC) to the frame
    being returned to us.
  * *convert + submit*: CPU time to issue the GPU colour conversion and hand
    the texture to the encoder.
  * *compose → encoded*: composition time to the encoder callback. One clock
    (QPC/monotonic), so the number is exact.
* Loopback integration test: host and client in one process, same monotonic
  clock, QUIC over 127.0.0.1. *capture → client received* is exact but
  excludes Wi-Fi, phone decoding and display.
* The phone's diagnostics screen will report *capture → decoded* using the
  Ping/Pong clock-offset estimate (uncertainty ±RTT/2). Not yet run.
* Physical input-to-photon latency requires a high-speed camera; not done.

## Results so far

| Run | Encoder | n | compose→acquired p50 / p95 | convert+submit p50 / p95 | compose→encoded p50 / p95 |
|---|---|---|---|---|---|
| 2026-11-15, NVIDIA GPU unavailable | Intel Quick Sync MFT | 42 | 3.2 / 6.3 ms | 0.8 / 1.6 ms | 14.5 / 24.0 ms |
| 2026-11-16, NVIDIA GPU available | NVIDIA H.264 MFT | 11 | 12.5 / 22.6 ms | 0.8 / 1.3 ms | 13.5 / 30.0 ms |

Notes: tiny samples, idle desktop (frames only when something changed),
p99 ≈ p95 at these sizes. The NVIDIA run's capture age is higher because the
capture device was on the discrete GPU while the panel is scanned out by the
integrated one (cross-adapter copy inside Windows); this needs a proper
comparison under continuous motion.

| Loopback integration measurement | Value |
|---|---|
| First keyframe, capture → client received | 38–121 ms over 6 runs (includes encoder start-up; not steady state) |
| Emergency stop on PC → phone connection closed | 0.18–0.24 ms in 5 of 6 runs; one 0.81 s outlier (test harness was draining queued events) |
| Static desktop, `next_frame` returns in 400 ms | 4–6 (blocking wait, no spinning) |

## Not yet measured (planned, in order)

1. Continuous-motion PC benchmark with the synthetic test pattern
   (`capture_encode --animate`): needs an unlocked session; the first attempt
   ran while the PC was locked and produced one frame, which is what led to
   the WTS lock detection fix.
2. Emulator end-to-end (decoder path on Android, loopback network).
3. Real phone over Wi-Fi: RTT, received/rendered fps, decode time, estimated
   capture→decoded latency, with p50/p95/p99 over ≥ 1000 frames, plus Wi-Fi
   topology (band, RSSI, AP).
4. Impaired network (latency, loss, jitter) using a reproducible shaper.
5. WGC vs DXGI DD on a single-GPU desktop.
6. Power and thermals on the phone during a 30-minute session.

## Optimisation history

| Change | Evidence |
|---|---|
| Graphics Capture instead of Desktop Duplication | DD unsupported on this hybrid laptop (ADR 0004) |
| GPU NV12 conversion + texture input to the encoder | no CPU pixel copies; convert+submit ≈ 0.8 ms |
| Drop-not-queue at every stage | design (ADR 0009); bounded queues verified by tests |
