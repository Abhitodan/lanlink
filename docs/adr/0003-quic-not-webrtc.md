# 0003 QUIC (Quinn) instead of WebRTC

**Decision.** One QUIC connection per phone: a reliable control stream, one
reliable input stream per session, one reliable video stream per session,
and unreliable datagrams for pointer and cursor positions.

**Why.** LAN only, no NAT traversal, so WebRTC's ICE/STUN/TURN machinery is
dead weight. QUIC gives TLS 1.3 with our own certificate verification
(fingerprint pinning), stream multiplexing without head-of-line blocking
between input and video, datagrams and congestion control, from a pure-Rust
library that builds for Windows and Android. WebRTC's RTP jitter buffer is
tuned for conferencing and adds buffering we would have to fight.

**MVP choice: reliable video stream.** Frames arrive whole and in order, so
the decoder never sees a corrupt frame. Latency under loss is bounded by the
host dropping queued frames and restarting from a keyframe (ADR 0009) rather
than by retransmitting stale ones indefinitely.

**Open.** Video over datagrams with FEC may beat this under real Wi-Fi loss.
It has not been built or measured; it is future work, not a claim.
