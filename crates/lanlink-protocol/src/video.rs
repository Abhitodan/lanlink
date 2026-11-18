//! Video stream records (host → client, one unidirectional stream per
//! session).
//!
//! The stream is a sequence of records, each starting with a one-byte tag:
//!
//! * `Config` announces the codec and coded size. It precedes the first frame
//!   and every change of resolution, display or codec.
//! * `Frame` carries exactly one complete access unit (H.264 Annex B). A
//!   keyframe always includes its parameter sets, so a decoder can start from
//!   any keyframe.
//!
//! Because the stream is reliable and ordered, a receiver never sees a
//! partial or reordered access unit. Loss shows up as delay, which the host
//! bounds by dropping queued frames and restarting from a keyframe.

use crate::control::Codec;
use crate::error::DecodeError;
use crate::limits::*;
use crate::wire::{Reader, Writer};

pub const TAG_CONFIG: u8 = 1;
pub const TAG_FRAME: u8 = 2;

/// Bytes following the tag for a `Config` record.
pub const CONFIG_BODY_LEN: usize = 23;
/// Bytes following the tag for a `Frame` header (payload follows).
pub const FRAME_HEADER_LEN: usize = 31;

/// The access unit can be decoded without any earlier frame.
pub const FLAG_KEYFRAME: u8 = 0x01;
/// One or more frames were dropped by the host immediately before this one.
/// Always set together with `FLAG_KEYFRAME`.
pub const FLAG_DISCONTINUITY: u8 = 0x02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoConfig {
    /// Increments on every reconfiguration; frames reference it.
    pub config_id: u16,
    pub codec: Codec,
    /// Coded picture size.
    pub width: u16,
    pub height: u16,
    pub fps: u16,
    pub display_id: u32,
    /// Size of the captured display in host pixels (before scaling).
    pub source_width: u32,
    pub source_height: u32,
    /// Clockwise rotation already applied by the host: always 0 today.
    pub rotation: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Increments by one per encoded frame, including frames the host later
    /// dropped. A gap therefore tells the receiver exactly how many were lost.
    pub seq: u32,
    pub config_id: u16,
    pub flags: u8,
    /// Host monotonic clock when the frame was captured.
    pub capture_us: u64,
    /// Capture → encoder output, measured on the host.
    pub encode_us: u32,
    /// Host monotonic clock when the frame was handed to the transport.
    pub send_us: u64,
    pub payload_len: u32,
}

impl FrameHeader {
    pub fn is_keyframe(&self) -> bool {
        self.flags & FLAG_KEYFRAME != 0
    }
}

impl VideoConfig {
    /// `tag | body`.
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(1 + CONFIG_BODY_LEN);
        w.u8(TAG_CONFIG);
        w.u16(self.config_id);
        w.u8(self.codec as u8);
        w.u16(self.width);
        w.u16(self.height);
        w.u16(self.fps);
        w.u32(self.display_id);
        w.u32(self.source_width);
        w.u32(self.source_height);
        w.u16(self.rotation);
        let out = w.into_bytes();
        debug_assert_eq!(out.len(), 1 + CONFIG_BODY_LEN);
        out
    }

    pub fn decode_body(body: &[u8; CONFIG_BODY_LEN]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        let c = VideoConfig {
            config_id: r.u16()?,
            codec: Codec::from_u8(r.u8()?)?,
            width: r.u16()?,
            height: r.u16()?,
            fps: r.u16()?,
            display_id: r.u32()?,
            source_width: r.u32()?,
            source_height: r.u32()?,
            rotation: r.u16()?,
        };
        r.finish()?;
        if c.width < 16 || c.height < 16 || c.width > MAX_VIDEO_DIM || c.height > MAX_VIDEO_DIM {
            return Err(DecodeError::InvalidValue("video size"));
        }
        if c.fps == 0 || c.fps > 240 {
            return Err(DecodeError::InvalidValue("fps"));
        }
        if !matches!(c.rotation, 0 | 90 | 180 | 270) {
            return Err(DecodeError::InvalidValue("rotation"));
        }
        Ok(c)
    }
}

impl FrameHeader {
    /// `tag | header`; the caller writes `payload_len` payload bytes after it.
    pub fn encode(&self) -> [u8; 1 + FRAME_HEADER_LEN] {
        let mut w = Writer::with_capacity(1 + FRAME_HEADER_LEN);
        w.u8(TAG_FRAME);
        w.u32(self.seq);
        w.u16(self.config_id);
        w.u8(self.flags);
        w.u64(self.capture_us);
        w.u32(self.encode_us);
        w.u64(self.send_us);
        w.u32(self.payload_len);
        let v = w.into_bytes();
        let mut out = [0u8; 1 + FRAME_HEADER_LEN];
        out.copy_from_slice(&v);
        out
    }

    pub fn decode_body(body: &[u8; FRAME_HEADER_LEN]) -> Result<Self, DecodeError> {
        let mut r = Reader::new(body);
        let h = FrameHeader {
            seq: r.u32()?,
            config_id: r.u16()?,
            flags: r.u8()?,
            capture_us: r.u64()?,
            encode_us: r.u32()?,
            send_us: r.u64()?,
            payload_len: r.u32()?,
        };
        r.finish()?;
        if h.flags & !(FLAG_KEYFRAME | FLAG_DISCONTINUITY) != 0 {
            return Err(DecodeError::InvalidValue("frame flags"));
        }
        if h.flags & FLAG_DISCONTINUITY != 0 && h.flags & FLAG_KEYFRAME == 0 {
            return Err(DecodeError::InvalidValue("discontinuity without keyframe"));
        }
        if h.payload_len == 0 {
            return Err(DecodeError::InvalidValue("empty frame"));
        }
        if h.payload_len as usize > MAX_VIDEO_FRAME {
            return Err(DecodeError::FrameTooLarge(h.payload_len as usize));
        }
        Ok(h)
    }
}

/// Receiver-side continuity tracking.
///
/// Decides whether a frame may be handed to the decoder. After any gap the
/// tracker refuses delta frames until the next keyframe, because feeding a
/// delta frame whose reference is missing corrupts the picture.
#[derive(Debug, Default)]
pub struct FrameGate {
    last_seq: Option<u32>,
    config_id: Option<u16>,
    waiting_for_keyframe: bool,
    pub frames_skipped: u64,
}

/// What the receiver should do with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GateDecision {
    /// Feed it to the decoder.
    Decode,
    /// Discard it and (if `request_keyframe`) ask the host for an IDR.
    Skip { request_keyframe: bool },
}

impl FrameGate {
    pub fn new() -> Self {
        Self {
            waiting_for_keyframe: true,
            ..Self::default()
        }
    }

    /// Call when a `Config` record arrives.
    pub fn on_config(&mut self, config_id: u16) {
        self.config_id = Some(config_id);
        self.waiting_for_keyframe = true;
    }

    /// Call after a decoder reset/flush: the next frame must be a keyframe.
    pub fn require_keyframe(&mut self) {
        self.waiting_for_keyframe = true;
    }

    pub fn on_frame(&mut self, h: &FrameHeader) -> GateDecision {
        let gap = match self.last_seq {
            Some(last) => h.seq != last.wrapping_add(1),
            None => false,
        };
        self.last_seq = Some(h.seq);

        if self.config_id != Some(h.config_id) {
            // A frame for a configuration we were never told about.
            self.frames_skipped += 1;
            self.waiting_for_keyframe = true;
            return GateDecision::Skip {
                request_keyframe: false,
            };
        }
        if gap {
            self.waiting_for_keyframe = true;
        }
        if h.is_keyframe() {
            self.waiting_for_keyframe = false;
            return GateDecision::Decode;
        }
        if self.waiting_for_keyframe {
            self.frames_skipped += 1;
            // Ask once per gap; the host answers with a keyframe, which
            // clears the flag above.
            return GateDecision::Skip {
                request_keyframe: gap,
            };
        }
        GateDecision::Decode
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(seq: u32, key: bool) -> FrameHeader {
        FrameHeader {
            seq,
            config_id: 1,
            flags: if key { FLAG_KEYFRAME } else { 0 },
            capture_us: 0,
            encode_us: 0,
            send_us: 0,
            payload_len: 10,
        }
    }

    #[test]
    fn header_lengths_match_constants() {
        let c = VideoConfig {
            config_id: 1,
            codec: Codec::H264,
            width: 1920,
            height: 1080,
            fps: 60,
            display_id: 0,
            source_width: 2560,
            source_height: 1600,
            rotation: 0,
        };
        let e = c.encode();
        assert_eq!(e.len(), 1 + CONFIG_BODY_LEN);
        let body: [u8; CONFIG_BODY_LEN] = e[1..].try_into().unwrap();
        assert_eq!(VideoConfig::decode_body(&body), Ok(c));

        let h = hdr(5, true);
        let e = h.encode();
        let body: [u8; FRAME_HEADER_LEN] = e[1..].try_into().unwrap();
        assert_eq!(FrameHeader::decode_body(&body), Ok(h));
    }

    #[test]
    fn rejects_oversized_and_empty_frames() {
        let mut h = hdr(1, true);
        h.payload_len = (MAX_VIDEO_FRAME + 1) as u32;
        let body: [u8; FRAME_HEADER_LEN] = h.encode()[1..].try_into().unwrap();
        assert!(matches!(
            FrameHeader::decode_body(&body),
            Err(DecodeError::FrameTooLarge(_))
        ));
        h.payload_len = 0;
        let body: [u8; FRAME_HEADER_LEN] = h.encode()[1..].try_into().unwrap();
        assert!(FrameHeader::decode_body(&body).is_err());
    }

    #[test]
    fn rejects_unknown_flags() {
        let mut h = hdr(1, true);
        h.flags = 0x80;
        let body: [u8; FRAME_HEADER_LEN] = h.encode()[1..].try_into().unwrap();
        assert!(FrameHeader::decode_body(&body).is_err());
    }

    #[test]
    fn gate_waits_for_first_keyframe() {
        let mut g = FrameGate::new();
        g.on_config(1);
        assert_eq!(
            g.on_frame(&hdr(1, false)),
            GateDecision::Skip {
                request_keyframe: false
            }
        );
        assert_eq!(g.on_frame(&hdr(2, true)), GateDecision::Decode);
        assert_eq!(g.on_frame(&hdr(3, false)), GateDecision::Decode);
    }

    #[test]
    fn gate_never_decodes_delta_after_gap() {
        let mut g = FrameGate::new();
        g.on_config(1);
        assert_eq!(g.on_frame(&hdr(1, true)), GateDecision::Decode);
        assert_eq!(g.on_frame(&hdr(2, false)), GateDecision::Decode);
        // Frame 3 lost.
        assert_eq!(
            g.on_frame(&hdr(4, false)),
            GateDecision::Skip {
                request_keyframe: true
            }
        );
        // Still waiting; no repeated request for a contiguous frame.
        assert_eq!(
            g.on_frame(&hdr(5, false)),
            GateDecision::Skip {
                request_keyframe: false
            }
        );
        assert_eq!(g.on_frame(&hdr(6, true)), GateDecision::Decode);
        assert_eq!(g.on_frame(&hdr(7, false)), GateDecision::Decode);
        assert_eq!(g.frames_skipped, 2);
    }

    #[test]
    fn gate_requires_matching_config() {
        let mut g = FrameGate::new();
        g.on_config(2);
        assert!(matches!(
            g.on_frame(&hdr(1, true)),
            GateDecision::Skip { .. }
        ));
    }
}
