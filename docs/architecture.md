# Architecture

## Components

```
                      Windows host                                         Android client
┌──────────────────────────────────────────────────────┐        ┌────────────────────────────────────────┐
│ lanlink-host-console (console UI)                    │        │ Compose UI (screens, toolbar)          │
│   └─ session-manager                                 │        │   └─ MainViewModel (nav, pairing)      │
│        ├─ host.rs        accept, policy, commands    │        │        └─ RemoteSession (state machine,│
│        ├─ connection.rs  handshake, pairing, session │        │             reconnect, input, stats)   │
│        ├─ media.rs       capture→encode thread       │ QUIC   │             ├─ VideoDecoder (MediaCodec│
│        │    ├─ capture-windows (WGC / DXGI DD)       │◀──────▶│             │   async → SurfaceView)  │
│        │    └─ encoder (D3D11 VP + MF H.264 MFT)     │ TLS1.3 │             └─ RemoteView (touch, IME, │
│        ├─ input_worker.rs ─ input-windows (SendInput)│ mutual │                  cursor overlay)       │
│        ├─ bitrate.rs, quality.rs, netpolicy.rs       │        │ JNI: liblanlink.so (native-bridge)     │
│        └─ storage.rs (DPAPI)                         │        │   └─ lanlink-client                    │
│ lanlink-discovery (mDNS advertise)                   │        │ NsdManager discovery, Keystore         │
└──────────────────────────────────────────────────────┘        └────────────────────────────────────────┘
        shared crates: lanlink-protocol · lanlink-auth · lanlink-transport · lanlink-client
```

| Crate / module | Responsibility | Interface |
|---|---|---|
| `lanlink-protocol` | Message types, bounded codecs, frame gate, input sequencer | pure functions |
| `lanlink-auth` | Identity, SPAKE2 pairing, trust store, QR link | pure functions |
| `lanlink-transport` | Quinn endpoints, pinned mutual TLS, framed stream I/O | async fns |
| `lanlink-discovery` | DNS-SD advertise/browse, local addresses | blocking |
| `lanlink-client` | Pair, connect, session events, video, input | async + channels |
| `capture-windows` | Monitors, `Capture` trait (WGC, DD), cursor shapes | thread-bound |
| `encoder` | `VideoEncoder::submit(texture) → callback(EncodedFrame)` | thread-bound |
| `input-windows` | `Injector<S: Sink>`: mapping, held state, rate limit | sync |
| `session-manager` | Host: `start() → HostHandle {commands, events}` | async + channels |
| `native-bridge` | JNI surface for the app | `NativeBridge.kt` |

## Threads

**Host.** Tokio runtime (4 workers) for networking. Per session: one media
thread (capture + encoder submit; COM objects are thread-bound), the encoder's
event thread (async MFT), one input thread (SendInput). Joining threads is
done in `spawn_blocking`, never on a runtime worker.

**Client.** Native tokio runtime (2 workers). Kotlin: connector thread,
event thread (`pollEvent`), decoder feeder thread, MediaCodec callback
HandlerThread, main thread for UI and input calls (non-blocking).

## Data paths

Media: WGC frame (GPU, BGRA) → video processor → pooled NV12 texture → MF
H.264 MFT (async) → callback → bounded channel (3) → QUIC video stream →
`lanlink-client` reader → `FrameGate` → bounded channel (4) → `nextVideo`
writes into a MediaCodec input buffer → decoder → SurfaceView.

Input: touch → `TouchInterpreter` → `RemoteView` maps through `Viewport`
→ JNI → datagram (pointer) or input stream (transitions) → host validates
(session, sequence, view-only) → bounded queue (256) → input thread →
`Injector` → `SendInput`.

## Session lifecycle

1. Connect: TLS with pinned host fingerprint → Hello/HelloAck(Paired).
2. Capabilities → DisplayList.
3. SessionStart → host claims the single control slot (same phone may take
   over its stale session) → optional approval on the PC → media pipeline
   starts → first Config/keyframe → SessionStarted.
4. Streaming; once a second StreamStats/ClientStats; adaptive bitrate.
5. End: phone SessionEnd, PC emergency stop, revoke, or connection loss →
   input released, pipeline stopped, slot freed.
6. Phone side: on unexpected loss, reconnect for up to 60 s and resume.

## Failure handling (summary; details in `implementation-status.md`)

| Condition | Detection | Response |
|---|---|---|
| Lock screen / UAC | `interactive_desktop_active()` polled every 500 ms; capture errors | Notice to phone; capture resumes when unlocked |
| Mode change, display unplugged | capture `AccessLost`, size change | rebuild pipeline; fall back to primary display |
| GPU lost / encoder error | `DeviceLost`, encoder `failed()` | rebuild on whichever adapter works |
| Decoder error / surface recreated | MediaCodec `onError`, surface callbacks | recreate decoder, request keyframe |
| Wi-Fi drop | QUIC idle timeout (6 s) | phone reconnects for 60 s; host releases input |
| Elevated foreground window | token integrity comparison | notice to phone |
