# 0004 Windows Graphics Capture first, Desktop Duplication fallback

**Decision.** `capture_windows::open(.., Auto)` tries Windows Graphics
Capture (WGC), then DXGI Desktop Duplication (DD).

**Evidence (development laptop, hybrid Intel UHD + RTX 4060).**

* DXGI lists the internal panel under the RTX 4060.
* DD failed with `0x887A0004` (unsupported) for every device/output
  combination tried (`examples/adapters.rs`), both while the RTX 4060 was in a
  "GPU lost" state and after it recovered.
* WGC worked in both states: through the Intel adapter while the NVIDIA GPU
  was lost, through the NVIDIA adapter after it recovered
  (`examples/probe.rs`; `tests/live.rs` 5/5 in repeated runs).
* Compose-to-acquire (WGC, mostly idle desktop): p50 5.2 ms / p95 9.5 ms on
  Intel (n=18); p50 6.8 ms on NVIDIA (n=6). Small samples.

**Why WGC first.** It works with a D3D11 device on any adapter, which a
hybrid laptop needs, and delivers frames only when the screen changes.

**Costs.** WGC has no pointer channel (handled by `cursor_gdi.rs`) and no
dirty rectangles (reported as 1.0). DD is kept because on single-GPU desktops
it reports dirty regions and pointer shapes natively.

**Bug found by the live tests.** Per-thread COM initialisation tore down the
apartment between captures and crashed the next WGC session (access
violation, 3/3 runs). Fixed by pinning the MTA for the process.
