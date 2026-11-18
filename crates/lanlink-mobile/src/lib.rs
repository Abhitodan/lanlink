//! Platform-neutral core of the mobile bridges.
//!
//! The Android JNI layer and the iOS C ABI are thin glue over this crate.
//!
//! Boundary rules:
//!
//! * Sessions live in a registry keyed by an opaque `i64` handle. Callers
//!   never hold a raw pointer, so a stale handle is an error, not a crash.
//! * Errors are user-presentable `String`s.
//! * Video is copied straight into a caller-provided buffer.
//! * Control-plane events (low rate) are small JSON strings.
//! * Blocking calls take a timeout and must be made from worker threads.

mod events;
mod input;
mod links;
mod pairing;
mod runtime;
mod session;
mod video;

pub use events::{cursor_pixels, poll_event};
pub use input::{
    release_all, send_button, send_key, send_pointer, send_pointer_rel, send_scroll, send_text,
};
pub use links::parse_pairing_uri;
pub use pairing::{generate_identity, identity_fingerprint, pair_cancel, pair_poll, pair_start};
pub use session::{
    bind_session, close, connect, end_session, select_display, send_capabilities,
    send_client_stats, set_stream_params, set_text_input, start_session,
};
pub use video::{
    decoder_reset, next_video, now_us, request_keyframe, META_CODEC_CONFIG, META_DISCONTINUITY,
    META_KEYFRAME, VIDEO_BUFFER_SMALL, VIDEO_CLOSED, VIDEO_CONFIG, VIDEO_TIMEOUT,
};
