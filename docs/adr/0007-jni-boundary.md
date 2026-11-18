# 0007 JNI boundary

* Sessions are opaque `long` handles in a registry; Kotlin never holds a raw
  pointer, so use-after-close is an exception, not a crash.
* Every entry point catches panics and maps errors to `IOException`.
* `nextVideo` copies each access unit straight into the MediaCodec input
  buffer (a direct `ByteBuffer`): the one copy decoding needs, no per-frame
  `byte[]`.
* Control events (tens per second at most) cross as small JSON strings.
  Never used on the media path.
* Blocking calls take timeouts and run on worker threads. One process-wide
  tokio runtime with two workers.
* Built with `panic = "unwind"` for Android so `catch_unwind` works.
