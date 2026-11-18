//! LANLink wire protocol.
//!
//! This crate is pure data: it has no I/O, no async and no platform code, so
//! the Windows host, the Android client and the fuzz/property tests all share
//! one implementation of every encoder and decoder.
//!
//! Design rules (see `docs/protocol.md` for the full specification):
//!
//! * Every length is bounded by a constant in [`limits`] and checked before
//!   any allocation.
//! * Decoders are strict: unknown tags, non-canonical booleans, out-of-range
//!   values and trailing bytes are all errors.
//! * Nothing here panics on untrusted input.

pub mod control;
pub mod error;
pub mod input;
pub mod limits;
pub mod video;
pub mod wire;

pub use control::{
    AuthState, ClientStats, Codec, CodecCap, ControlMsg, DisplayInfo, QualityMode, StreamParams,
    StreamStats, DISPLAY_PRIMARY,
};
pub use error::{DecodeError, ErrorCode, NoticeCode};
pub use input::{Datagram, InputEvent, InputKind, InputSequencer, MouseButton};
pub use video::{FrameGate, FrameHeader, GateDecision, VideoConfig};

/// First byte written on every QUIC stream, identifying its purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StreamKind {
    /// Bidirectional, opened by the client. Carries [`ControlMsg`].
    Control = 1,
    /// Unidirectional, client → host. Carries [`InputEvent`].
    Input = 2,
    /// Unidirectional, host → client. Carries video records.
    Video = 3,
}

impl StreamKind {
    pub fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            1 => Ok(StreamKind::Control),
            2 => Ok(StreamKind::Input),
            3 => Ok(StreamKind::Video),
            other => Err(DecodeError::UnknownTag(other)),
        }
    }
}

/// Map a normalized coordinate (`0..=65535`) onto `0..extent` pixels.
///
/// Uses the pixel-center convention so `0` maps to the first pixel and
/// `65535` to the last, with no off-by-one at either edge.
pub fn denormalize(v: u16, extent: u32) -> i32 {
    if extent <= 1 {
        return 0;
    }
    let max = (extent - 1) as u64;
    ((v as u64 * max + 32767) / 65535) as i32
}

/// Inverse of [`denormalize`]: map pixel `px` in `0..extent` to `0..=65535`.
pub fn normalize(px: i64, extent: u32) -> u16 {
    if extent <= 1 {
        return 0;
    }
    let max = (extent - 1) as i64;
    let px = px.clamp(0, max);
    ((px * 65535 + max / 2) / max) as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_hits_both_edges_and_roundtrips() {
        for extent in [2u32, 3, 1080, 1600, 2560, 3840] {
            assert_eq!(denormalize(0, extent), 0);
            assert_eq!(denormalize(65535, extent), extent as i32 - 1);
            for px in [0i64, 1, (extent / 2) as i64, extent as i64 - 1] {
                assert_eq!(denormalize(normalize(px, extent), extent), px as i32);
            }
        }
        assert_eq!(denormalize(40000, 0), 0);
        assert_eq!(normalize(-5, 100), 0);
        assert_eq!(normalize(500, 100), 65535);
    }
}
