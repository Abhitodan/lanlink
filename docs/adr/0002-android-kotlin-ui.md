# 0002 Kotlin + Compose UI, Rust only for networking

**Decision.** UI, decoding (MediaCodec), rendering (SurfaceView), touch and
IME are Kotlin. Rust (`liblanlink.so`) does QUIC, TLS, pairing and protocol
framing.

**Why.** MediaCodec, SurfaceView, NsdManager, the Keystore and the IME are
Java APIs; wrapping them from Rust adds risk without benefit. QUIC with
custom mutual TLS has no good JVM implementation (Cronet exposes HTTP/3, not
raw QUIC streams and datagrams), so Rust is materially beneficial there.

**Rendering.** Video never touches Compose: MediaCodec renders to a
SurfaceView. The cursor overlay is a plain View that redraws only itself.
