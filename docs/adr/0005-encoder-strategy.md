# 0005 Media Foundation hardware encoder on the capture adapter

**Decision.** Encode with the Media Foundation H.264 MFT running on the same
adapter as the capture device, fed with NV12 textures from the D3D11 video
processor (BGRA->NV12 and scaling in one GPU pass). Fall back to the software
MFT through a system-memory path. Never pick a hardware encoder on another
adapter.

**Evidence.** Intel Quick Sync MFT and NVIDIA H.264 MFT were each selected
automatically on their adapters; output decodes in ffprobe (H.264 High,
every frame read). Convert+submit p50 about 0.8 ms; compose->encoded p50
13-15 ms, p95 24-30 ms on a mostly static desktop (small n, see
`docs/performance.md`).

**Low-delay settings.** CBR with a 1/4-second buffer, no B-frames,
low-latency mode, keyframes only on demand, CABAC.

**Zero-copy, precisely.** On the texture path pixels stay in video memory
from capture to encoder input; the compressed output is copied once into a
`Vec<u8>`. The system-memory path does one GPU read-back per frame and
reports itself as such (`FramePath::SystemMemory`).

**NVENC SDK.** Not used: the NVIDIA MFT already drives the NVENC hardware
without the SDK's redistribution terms. A direct NVENC backend might lower
latency further; not built, not claimed.
