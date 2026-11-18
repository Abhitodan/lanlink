# Architecture

## Components

```text
                      Windows host                                         Android client
┌──────────────────────────────────────────────────────┐        ┌────────────────────────────────────────┐
│ lanlink-host (GUI, tray, WDA_EXCLUDEFROMCAPTURE bar) │        │ Compose UI (screens, control rail)     │
│ lanlink-host-console (CLI host)                      │        │   └─ MainViewModel (nav, pairing)      │
│   └─ session-manager                                 │        │        └─ RemoteSession (state machine,│
│        ├─ host.rs        accept, policy, commands    │        │             reconnect, input, stats)   │
│        ├─ connection.rs  handshake, pairing, session │        │             ├─ VideoDecoder (MediaCodec│
│        ├─ media.rs       capture→encode thread       │ QUIC   │             │   async → SurfaceView)  │
│        │    ├─ capture-windows (WGC / DXGI DD)       │◀──────▶│             └─ RemoteView (touch, IME, │
│        │    └─ encoder (D3D11 VP + MF H.264 MFT)     │ TLS1.3 │                  cursor, caret follow) │
│        ├─ input_worker.rs ─ input-windows (SendInput,│ mutual │ JNI: liblanlink.so (native-bridge)     │
│        │                     Win32 + UIA caret)      │        │   └─ lanlink-mobile → lanlink-client   │
│        ├─ bitrate.rs, quality.rs, netpolicy.rs       │        │ NsdManager discovery, Android Keystore │
│        └─ storage.rs (DPAPI)                         │        └────────────────────────────────────────┘
│ lanlink-discovery (mDNS advertise)                   │
└──────────────────────────────────────────────────────┘
    shared crates: lanlink-protocol · lanlink-auth · lanlink-transport · lanlink-client · lanlink-mobile
```

| Crate / module | Responsibility | Interface |
|---|---|---|
| `lanlink-protocol` | Message types, bounded codecs, `FrameGate`, `InputSequencer` | pure functions |
| `lanlink-auth` | Identity, SPAKE2 pairing, trust store, QR pairing URI | pure functions |
| `lanlink-transport` | Quinn endpoints, pinned mutual TLS 1.3, framed stream I/O | async fns |
| `lanlink-discovery` | DNS-SD advertise/browse, local interface addresses | blocking |
| `lanlink-client` | Pair, connect, session events, video receiver, input pacing | async + channels |
| `lanlink-mobile` | Handle registry, JSON event serialization, Annex-B parameter sets | sync C/Rust API |
| `capture-windows` | Monitors, `Capture` trait (`WGC`, `DXGI DD`), cursor shapes | thread-bound |
| `encoder` | `VideoEncoder::submit(texture) → callback(EncodedFrame)` | thread-bound |
| `input-windows` | `Injector<S: Sink>`: coordinate mapping, held state, rate limit, caret | sync |
| `session-manager` | Host: `start() → HostHandle {commands, events}` | async + channels |
| `lanlink-host` | Windows desktop GUI (`egui`), system tray, screen border overlay, hotkey | desktop app |
| `lanlink-host-console` | Terminal host and diagnostic CLI | console app |
| `native-bridge` | Panic-safe JNI surface for Android | `NativeBridge.kt` |

## Threads

**Host.** Tokio runtime (4 workers) for networking. Per session: one media
thread (capture + encoder submit; COM objects are thread-bound), the encoder's
event thread (async MFT), one input thread (`SendInput`), and while the phone
keyboard is open on protocol v2, a caret-polling loop (`120 ms` interval).
Joining OS threads is done in `spawn_blocking`, never on a Tokio worker. In the
GUI app (`lanlink-host`), an independent Win32 thread drives the
`WDA_EXCLUDEFROMCAPTURE` border indicator and stop bar.

**Client.** Native Tokio runtime (2 workers). Kotlin: connector thread,
event thread (`pollEvent`), decoder feeder thread, `MediaCodec` callback
`HandlerThread`, main thread for UI and non-blocking input calls.

## Data paths

**Media:** WGC frame (GPU `BGRA` texture) → `D3D11` video processor → pooled
`NV12` texture → Media Foundation H.264 MFT (async) → callback → bounded
channel (3) → QUIC video stream → `lanlink-client` reader → `FrameGate` →
bounded channel (4) → `nextVideo` writes directly into a `MediaCodec` direct
`ByteBuffer` → hardware decoder → `SurfaceView`.

**Input:** Touch → `TouchInterpreter` → `RemoteView` maps through `Viewport`
and `TrackpadCursor` → JNI → QUIC datagram (pointer moves) or reliable input
stream (clicks, scroll, keys, Unicode text) → host validates session ID,
strictly incrementing sequence number, and view-only mode → bounded queue (256)
→ input thread → `Injector` → `SendInput`.

## Session lifecycle

1. **Connect:** Mutual TLS 1.3 with pinned host fingerprint → `Hello` / `HelloAck(Paired)`.
2. **Negotiate:** `Capabilities` → `DisplayList`.
3. **Start:** `SessionStart` → host claims the single control slot (the same
   paired phone reconnecting may take over its own stale session) → optional
   interactive approval on the PC → media pipeline starts → first `Config` +
   keyframe → `SessionStarted`.
4. **Stream:** Continuous video on screen change; 1 Hz `StreamStats` /
   `ClientStats`; Ping/Pong RTT & clock-offset estimation; adaptive bitrate.
5. **End:** Phone `SessionEnd`, PC emergency stop, device revocation, or
   connection loss → held input released immediately, media pipeline stopped,
   session slot freed.
6. **Recovery:** On unexpected network loss, the phone automatically retries
   across known host addresses for up to 60 seconds.

## Failure handling

| Condition | Detection | Response |
|---|---|---|
| Lock screen / UAC | `interactive_desktop_active()` polled every 500 ms; capture errors | Notice to phone; non-release input dropped; capture resumes when unlocked |
| Mode change, display unplugged | Capture `AccessLost`, size change | Rebuild pipeline; fall back to primary display |
| GPU lost / encoder error | `DeviceLost`, encoder `failed()` | Rebuild on a working adapter; software H.264 fallback |
| Decoder error / surface recreated | `MediaCodec.Callback.onError`, `SurfaceHolder` callbacks | Release held input on surface destroy; recreate decoder with stored geometry and request keyframe on return |
| Wi-Fi drop | QUIC idle timeout (6 s) | Host releases all held keys/buttons; phone reconnects for up to 60 s |
| Elevated foreground window (UIPI) | Token integrity comparison | Notice sent to phone |
