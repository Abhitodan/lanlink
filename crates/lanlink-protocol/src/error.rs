//! Decode failures and the protocol-level error codes carried on the wire.

use thiserror::Error;

/// Why a received byte sequence was rejected. Every variant is a hard error:
/// the receiver must not act on any part of the message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum DecodeError {
    #[error("message truncated")]
    Truncated,
    #[error("field `{0}` exceeds its size limit")]
    TooLarge(&'static str),
    #[error("field `{0}` is not valid UTF-8")]
    InvalidUtf8(&'static str),
    #[error("unknown message tag {0:#04x}")]
    UnknownTag(u8),
    #[error("invalid value for `{0}`")]
    InvalidValue(&'static str),
    #[error("unexpected trailing bytes")]
    TrailingBytes,
    #[error("frame length {0} exceeds limit")]
    FrameTooLarge(usize),
}

/// Error codes sent in `ControlMsg::Error` and `SessionEnd`. Numeric values
/// are part of the wire contract; never renumber, only append.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    /// Orderly close requested by the peer's user.
    Normal = 0,
    ProtocolVersionUnsupported = 1,
    MalformedMessage = 2,
    UnexpectedMessage = 3,
    NotPaired = 4,
    PairingClosed = 5,
    PairingFailed = 6,
    PairingDenied = 7,
    PairingRateLimited = 8,
    SessionDenied = 9,
    DeviceRevoked = 10,
    HostBusy = 11,
    DisplayUnavailable = 12,
    CaptureFailed = 13,
    EncoderFailed = 14,
    CodecUnsupported = 15,
    HostStopped = 16,
    Timeout = 17,
    InputRateLimited = 18,
    HostLocked = 19,
    Internal = 255,
}

impl ErrorCode {
    pub fn from_u16(v: u16) -> Self {
        use ErrorCode::*;
        match v {
            0 => Normal,
            1 => ProtocolVersionUnsupported,
            2 => MalformedMessage,
            3 => UnexpectedMessage,
            4 => NotPaired,
            5 => PairingClosed,
            6 => PairingFailed,
            7 => PairingDenied,
            8 => PairingRateLimited,
            9 => SessionDenied,
            10 => DeviceRevoked,
            11 => HostBusy,
            12 => DisplayUnavailable,
            13 => CaptureFailed,
            14 => EncoderFailed,
            15 => CodecUnsupported,
            16 => HostStopped,
            17 => Timeout,
            18 => InputRateLimited,
            19 => HostLocked,
            // Codes from a newer peer degrade to a generic failure instead of
            // being a decode error, so new codes stay backward compatible.
            _ => Internal,
        }
    }

    pub fn as_u16(self) -> u16 {
        self as u16
    }
}

/// Non-fatal conditions the host reports while a session stays up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum NoticeCode {
    /// Capture is being re-established (mode change, device reset, hotplug).
    CaptureRecovering = 1,
    /// The interactive desktop is not capturable (lock screen / secure desktop).
    DesktopUnavailable = 2,
    /// Capture resumed after one of the conditions above.
    CaptureResumed = 3,
    /// The foreground window runs at a higher integrity level; Windows drops
    /// injected input for it (UIPI). LANLink does not work around this.
    InputBlockedByIntegrity = 4,
    /// The display is in HDR mode; the stream is the system's SDR rendition.
    HdrToneMapped = 5,
    /// Encoder fell back from the hardware path.
    EncoderFallback = 6,
    /// Bitrate lowered because the network could not sustain the target.
    BitrateReduced = 7,
    Unknown = 0xffff,
}

impl NoticeCode {
    pub fn from_u16(v: u16) -> Self {
        use NoticeCode::*;
        match v {
            1 => CaptureRecovering,
            2 => DesktopUnavailable,
            3 => CaptureResumed,
            4 => InputBlockedByIntegrity,
            5 => HdrToneMapped,
            6 => EncoderFallback,
            7 => BitrateReduced,
            _ => Unknown,
        }
    }

    pub fn as_u16(self) -> u16 {
        self as u16
    }
}
