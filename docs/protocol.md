# LANLink wire protocol, version 1

Implementation: `crates/lanlink-protocol` (pure data, no I/O). All integers
are little-endian. Every decoder is strict: unknown tags, non-canonical
booleans (anything but 0/1), out-of-range values, oversize fields and
trailing bytes are errors, and nothing is allocated before its length has
been checked against a limit.

## Transport

* QUIC (RFC 9000) over UDP, default port **47990**, ALPN `lanlink/1`.
* TLS 1.3 only, **mutual** authentication with self-signed ECDSA P-256
  certificates. 0-RTT is disabled (early data is replayable).
* Identity = SHA-256 of the certificate DER ("fingerprint"). The client pins
  the host fingerprint; the host authorizes by the client fingerprint. TLS
  signature verification is never skipped.
* Idle timeout 6 s, keep-alive 1 s, initial RTT 30 ms.
* Discovery: DNS-SD `_lanlink._udp.local.`, TXT `v` (protocol), `id` (first
  16 hex of the host fingerprint), `n` (display name). Untrusted hints only.

## Streams

The first byte of every stream names its kind.

| Kind | Byte | Direction | Opened by | Content |
|---|---|---|---|---|
| Control | 1 | bidirectional | client, first | `ControlMsg` records |
| Input | 2 | client → host | client, once per session | `InputEvent` records |
| Video | 3 | host → client | host, once per session | video records |

Unreliable QUIC datagrams carry pointer positions (client → host) and cursor
positions (host → client).

## Control records

`u32 body_len | u8 tag | fields`, `1 ≤ body_len ≤ 307200`.
Strings are `u16 len | UTF-8`; byte strings `u32 len | bytes`.

| Tag | Message | Dir | Fields |
|---|---|---|---|
| 1 | Hello | C→H | u16 min, u16 max, str device_name(64), str model(64), str app_version(32) |
| 2 | HelloAck | H→C | u16 protocol, str host_name(64), str app_version(32), u8 auth (0 paired, 1 pairing available, 2 not paired) |
| 3 | PairRequest | C→H | bytes spake (≤64) |
| 4 | PairResponse | H→C | bytes spake (≤64) |
| 5 | PairConfirm | C→H | [32] mac |
| 6 | PairResult | H→C | bool accepted, [32] mac, u16 reason |
| 7 | Capabilities | C→H | u8 n(≤8) × {u8 codec, bool hw, bool low_latency, u16 max_w, u16 max_h, u16 max_fps}, u16 display_w, u16 display_h, u32 refresh_mHz |
| 8 | DisplayList | H→C | u8 n(≤16) × {u32 id, str name, u32 w, u32 h, i32 x, i32 y, bool primary, u32 refresh_mHz, u16 scale_%, u16 rotation, bool hdr} |
| 9 | SessionStart | C→H | u32 display_id (0xFFFFFFFF = primary), StreamParams, bool view_only |
| 10 | SessionStarted | H→C | u32 session_id, u32 display_id, bool view_only, str encoder(96), str capture(64) |
| 11 | SessionEnd | both | u16 reason |
| 12 | DisplaySelect | C→H | u32 display_id |
| 13 | SetStreamParams | C→H | StreamParams |
| 14 | KeyframeRequest | C→H | — |
| 15 | Ping | both | u32 id, u64 t_send_us |
| 16 | Pong | both | u32 id, u64 t_send_us, u64 t_recv_us, u64 t_reply_us |
| 17 | StreamStats | H→C | captured/encoded fps×10, bitrate, target, encode p50/p95, drops, queue, rtt, lost, cwnd |
| 18 | ClientStats | C→H | received/rendered fps×10, decode p50/p95, dropped, queue |
| 19 | CursorShape | H→C | u32 id, u16 w, u16 h (1..256), u16 hot_x < w, u16 hot_y < h, bytes RGBA (exactly w·h·4) |
| 20 | InputAck | H→C | u32 seq, u64 t_recv_us, u64 t_inject_us |
| 21 | Notice | H→C | u16 code, str message(256) |
| 22 | Error | both | u16 code, str message(256) |
| 23 | TextInput *(v2)* | C→H | bool active (phone keyboard open) |
| 24 | Caret *(v2)* | H→C | bool visible, u16 x, u16 y, u16 w, u16 h (0..65535 of the streamed display) |

`StreamParams` = u8 quality (0 smooth, 1 balanced, 2 sharp, 3 custom), u8
codec (1 = H.264), u16 max_w ≤ 8192, u16 max_h ≤ 8192, u16 fps ≤ 240, u32
bitrate_kbps (0 = auto), u32 max_bitrate_kbps (0 = none).

Unknown error/notice codes from a newer peer decode as "Internal"/"Unknown"
rather than failing, so codes can be added compatibly.

**Test vector** (Ping, id 7, t = 0x0102030405060708):
`0d 00 00 00 0f 07 00 00 00 08 07 06 05 04 03 02 01`.

**Test vectors (v2 bodies)**: TextInput active → `17 01`; Caret visible,
x 0x0102, y 0x0304, w 5, h 0x0600 → `18 01 02 01 04 03 05 00 00 06`.

## State machine

```
TLS up → client sends Hello
host   → HelloAck(auth)
  auth = NotPaired        → host sends Error(NotPaired), closes
  auth = PairingAvailable → PairRequest → PairResponse → PairConfirm
                            → (PC user approves) → PairResult(accepted)
                            → connection continues as Paired
  auth = Paired           → Capabilities → DisplayList
                            → SessionStart → SessionStarted (+ video stream)
                            → … → SessionEnd → (new SessionStart allowed)
```

Before pairing completes the host accepts only PairRequest/PairConfirm; any
other message closes the connection. Only one phone controls a PC at a time
(`Error HostBusy`); the same phone reconnecting replaces its own stale
session.

## Input

Reliable record: `u16 body_len | u8 tag | u32 session_id | u32 seq | fields`,
`body_len ≤ 2048`. `seq` starts at 1 and must increase by exactly 1 per
session; a duplicate or gap closes the connection (the stream is reliable,
so either means forgery or a bug). Records for another session are ignored.

| Tag | Kind | Fields |
|---|---|---|
| 1 | Button | u8 button (0 L, 1 R, 2 M, 3 X1, 4 X2), bool down, bool has_pos, u16 x, u16 y |
| 2 | Scroll | i16 dx, i16 dy (1/120 notch; +dy up, +dx right) |
| 3 | Key | u16 vk (1..=254, Windows virtual key), bool down |
| 4 | Text | str (≤1024 bytes, no NUL) — Unicode, layout-independent |
| 5 | ReleaseAll | — |
| 6 | MoveAbs | u16 x, u16 y |
| 7 | MoveRel | i16 dx, i16 dy (host pixels) |

Coordinates are normalized to the captured display: 0 = left/top edge,
65535 = right/bottom edge (pixel-centre convention). The client resolves
letterboxing, zoom and pan; the host resolves DPI and virtual-desktop
offsets (negative origins included).

Datagrams (first byte = tag): `0x10 PointerAbs {u32 session, u32 seq, u16 x,
u16 y}`, `0x11 PointerRel {.., i16 dx, i16 dy}`, `0x20 CursorPos {u32
session, u32 seq, u16 x, u16 y, u32 shape_id, bool visible}`. Receivers drop
datagrams whose wrapping `seq` is not newer than the last seen.

The host releases every held button and key when a session ends, the
connection closes, or the input worker stops for any reason.

## Video

Records on the video stream, each starting with a tag byte:

* `1 Config`: u16 config_id, u8 codec, u16 width, u16 height, u16 fps, u32
  display_id, u32 source_w, u32 source_h, u16 rotation (23 bytes).
* `2 Frame`: u32 seq, u16 config_id, u8 flags (0x01 keyframe, 0x02
  discontinuity — only with keyframe), u64 capture_us, u32 encode_us, u64
  send_us, u32 payload_len (1..8 MiB), then the H.264 Annex B access unit.

A Config precedes the first frame and every reconfiguration. `seq` counts
every encoded frame including ones the host dropped, so gaps are visible.
Keyframes always carry SPS and PPS. Times are the host's monotonic clock in
microseconds; they are only comparable with host times, or with client times
through the Ping/Pong offset estimate (uncertainty ±RTT/2).

### Recovery rules

* Receiver: decode a delta frame only if the previous `seq` was decoded and
  the config matches (`FrameGate`); otherwise skip and send
  `KeyframeRequest` once per gap.
* Host: on any drop, force the next frame to be a keyframe and flag it as a
  discontinuity; never send a delta frame older than 250 ms.
* Waiting for retransmission vs. requesting a keyframe: the video stream is
  reliable, so missing data is retransmitted by QUIC; the host only discards
  data it has not yet handed to QUIC. A keyframe is requested whenever the
  receiver observes a gap or resets its decoder.

## Versioning

The major generation is in the ALPN (`lanlink/1`), so incompatible
generations fail in the handshake. Within a generation, `Hello` offers a
range and the host picks the highest common version. New messages get new
tags; existing layouts never change. Unknown tags are rejected, so a peer
must only send messages the negotiated version defines.

| Version | Adds |
|---|---|
| 1 | everything above except tags 23–24 |
| 2 | `TextInput` / `Caret`: while the phone keyboard is open the host polls the text caret (Win32 caret, then UI Automation; editable controls only) about every 120 ms and reports changes |
