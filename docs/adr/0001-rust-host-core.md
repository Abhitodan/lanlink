# 0001 Rust for the host core and the shared protocol

**Decision.** The host (capture, encode, transport, input, sessions) and the
wire protocol are Rust. The protocol, pairing and transport crates are also
compiled into the Android app, so both ends run one implementation.

**Why.** Memory safety at the trust boundary (every byte from the phone is
parsed by Rust with bounded, strict decoders covered by property tests);
first-class Windows bindings (`windows-rs`); a mature QUIC stack (Quinn); no
runtime to ship. Sharing the crates removes client/host protocol mismatches:
the end-to-end tests drive the same client code the phone uses.

**Rejected.** C++ (no memory safety at the parser boundary); C#/.NET
(heavier runtime for a tray app, no raw-QUIC library as mature as Quinn);
Electron (excluded by the brief).

**Not claimed.** Rust does not make anything fast by itself. Latency claims
come only from measurements in `docs/performance.md`.
