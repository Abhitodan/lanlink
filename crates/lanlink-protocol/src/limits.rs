//! Hard size limits. A receiver rejects anything larger before allocating.

/// Wire protocol version implemented by this crate.
pub const PROTOCOL_VERSION: u16 = 2;

/// Oldest protocol version this crate can still speak.
pub const PROTOCOL_VERSION_MIN: u16 = 1;

/// ALPN identifier. The major protocol generation is part of the ALPN so an
/// incompatible future generation fails cleanly in the TLS handshake.
pub const ALPN: &[u8] = b"lanlink/1";

/// Default UDP port the host listens on.
pub const DEFAULT_PORT: u16 = 47990;

/// DNS-SD service type advertised by hosts.
pub const MDNS_SERVICE: &str = "_lanlink._udp.local.";

/// Largest control message body (dominated by a 256x256 RGBA cursor shape).
pub const MAX_CONTROL_MSG: usize = 300 * 1024;

/// Largest reliable input record body.
pub const MAX_INPUT_MSG: usize = 2048;

/// Largest encoded access unit accepted by a receiver.
pub const MAX_VIDEO_FRAME: usize = 8 * 1024 * 1024;

/// Largest datagram payload either side sends (fits the QUIC minimum MTU).
pub const MAX_DATAGRAM: usize = 1100;

pub const MAX_NAME: usize = 64;
pub const MAX_MODEL: usize = 64;
pub const MAX_VERSION: usize = 32;
pub const MAX_MESSAGE_TEXT: usize = 256;
pub const MAX_ENCODER_NAME: usize = 96;
pub const MAX_TEXT_INPUT: usize = 1024;
pub const MAX_DISPLAYS: usize = 16;
pub const MAX_CODEC_CAPS: usize = 8;
pub const MAX_CURSOR_DIM: u16 = 256;
pub const MAX_SPAKE_MSG: usize = 64;

/// Largest video dimension the protocol will describe.
pub const MAX_VIDEO_DIM: u16 = 8192;
