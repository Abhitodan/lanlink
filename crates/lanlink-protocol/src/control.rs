//! Control-channel messages (reliable, ordered, bidirectional stream).
//!
//! Wire form of one message: `u32 body_len | u8 tag | fields...` where
//! `body_len` counts the tag and fields. See `docs/protocol.md`.

use crate::error::{DecodeError, ErrorCode, NoticeCode};
use crate::limits::*;
use crate::wire::{Reader, Writer};

/// Video codecs the protocol can negotiate. Only H.264 is implemented by the
/// first release; the others are reserved so capability reports stay stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Codec {
    H264 = 1,
    Hevc = 2,
    Av1 = 3,
}

impl Codec {
    pub fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            1 => Ok(Codec::H264),
            2 => Ok(Codec::Hevc),
            3 => Ok(Codec::Av1),
            _ => Err(DecodeError::InvalidValue("codec")),
        }
    }
}

/// Quality intent selected by the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum QualityMode {
    Smooth = 0,
    Balanced = 1,
    Sharp = 2,
    Custom = 3,
}

impl QualityMode {
    pub fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(QualityMode::Smooth),
            1 => Ok(QualityMode::Balanced),
            2 => Ok(QualityMode::Sharp),
            3 => Ok(QualityMode::Custom),
            _ => Err(DecodeError::InvalidValue("quality")),
        }
    }
}

/// What the host knows about the connecting device after the TLS handshake.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum AuthState {
    /// Certificate is in the host's trust store; sessions may start.
    Paired = 0,
    /// Unknown certificate, but the host user has opened a pairing window.
    PairingAvailable = 1,
    /// Unknown certificate and pairing is closed. Nothing else is permitted.
    NotPaired = 2,
}

impl AuthState {
    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(AuthState::Paired),
            1 => Ok(AuthState::PairingAvailable),
            2 => Ok(AuthState::NotPaired),
            _ => Err(DecodeError::InvalidValue("auth_state")),
        }
    }
}

/// One decoder the client can offer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecCap {
    pub codec: Codec,
    pub hardware: bool,
    pub low_latency: bool,
    pub max_width: u16,
    pub max_height: u16,
    pub max_fps: u16,
}

/// One capturable display on the host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    /// Position in the Windows virtual desktop; may be negative.
    pub x: i32,
    pub y: i32,
    pub primary: bool,
    pub refresh_millihz: u32,
    pub scale_percent: u16,
    /// Clockwise rotation in degrees: 0, 90, 180 or 270.
    pub rotation: u16,
    pub hdr: bool,
}

/// `display_id` value meaning "the host's primary display".
pub const DISPLAY_PRIMARY: u32 = u32::MAX;

/// Stream parameters requested by the client. Zero means "host decides".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamParams {
    pub quality: QualityMode,
    pub codec: Codec,
    pub max_width: u16,
    pub max_height: u16,
    pub fps: u16,
    /// Target bitrate; 0 selects the quality mode's adaptive default.
    pub bitrate_kbps: u32,
    /// Hard ceiling the adaptive controller may never exceed; 0 = none.
    pub max_bitrate_kbps: u32,
}

/// Host-side stream telemetry, sent about once per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StreamStats {
    pub captured_fps_x10: u16,
    pub encoded_fps_x10: u16,
    pub bitrate_kbps: u32,
    pub target_bitrate_kbps: u32,
    pub encode_us_p50: u32,
    pub encode_us_p95: u32,
    /// Frames skipped before encoding because the encoder was busy.
    pub dropped_before_encode: u32,
    /// Encoded frames discarded because the network queue was full.
    pub dropped_before_send: u32,
    pub send_queue_depth: u8,
    pub rtt_us: u32,
    pub lost_packets: u64,
    pub congestion_window: u32,
}

/// Client-side stream telemetry, sent about once per second.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientStats {
    pub received_fps_x10: u16,
    pub rendered_fps_x10: u16,
    pub decode_us_p50: u32,
    pub decode_us_p95: u32,
    pub frames_dropped: u32,
    pub decoder_queue_depth: u8,
}

/// Every message that can appear on the control stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlMsg {
    /// C→H, first message. Offers a protocol version range.
    Hello {
        protocol_min: u16,
        protocol_max: u16,
        device_name: String,
        device_model: String,
        app_version: String,
    },
    /// H→C. Selects the protocol version and reports authorization state.
    HelloAck {
        protocol: u16,
        host_name: String,
        app_version: String,
        auth: AuthState,
    },
    /// C→H. SPAKE2 message from the client.
    PairRequest { spake_msg: Vec<u8> },
    /// H→C. SPAKE2 message from the host.
    PairResponse { spake_msg: Vec<u8> },
    /// C→H. Key confirmation proving the client derived the shared key.
    PairConfirm { mac: [u8; 32] },
    /// H→C. Sent after the host user approves or rejects. `mac` proves the
    /// host derived the same key (all zero when `accepted` is false).
    PairResult {
        accepted: bool,
        mac: [u8; 32],
        reason: ErrorCode,
    },
    /// C→H. Decoder and display capabilities.
    Capabilities {
        codecs: Vec<CodecCap>,
        display_width: u16,
        display_height: u16,
        display_refresh_millihz: u32,
    },
    /// H→C. Sent after authentication and whenever the topology changes.
    DisplayList { displays: Vec<DisplayInfo> },
    /// C→H. Ask to start streaming and (unless `view_only`) control.
    SessionStart {
        display_id: u32,
        params: StreamParams,
        view_only: bool,
    },
    /// H→C. The session is live; video follows on a new unidirectional stream.
    SessionStarted {
        session_id: u32,
        display_id: u32,
        view_only: bool,
        encoder_name: String,
        capture_backend: String,
    },
    /// Either direction. Ends the session but may keep the connection.
    SessionEnd { reason: ErrorCode },
    /// C→H. Switch the captured display inside a running session.
    DisplaySelect { display_id: u32 },
    /// C→H. Change stream parameters without restarting the session.
    SetStreamParams { params: StreamParams },
    /// C→H. Decoder lost sync; send an IDR frame.
    KeyframeRequest,
    /// Either direction. `t_send_us` is the sender's monotonic clock.
    Ping { id: u32, t_send_us: u64 },
    /// Reply to `Ping`. `t_recv_us`/`t_reply_us` are the responder's clock,
    /// which lets the requester estimate offset with bounded uncertainty.
    Pong {
        id: u32,
        t_send_us: u64,
        t_recv_us: u64,
        t_reply_us: u64,
    },
    /// H→C.
    StreamStats(StreamStats),
    /// C→H.
    ClientStats(ClientStats),
    /// H→C. Pixel data for a cursor shape referenced by `CursorPos` datagrams.
    CursorShape {
        shape_id: u32,
        width: u16,
        height: u16,
        hot_x: u16,
        hot_y: u16,
        /// Straight-alpha RGBA, `width * height * 4` bytes.
        rgba: Vec<u8>,
    },
    /// H→C. A reliable input event was injected. Timestamps are host clock.
    InputAck {
        seq: u32,
        t_recv_us: u64,
        t_inject_us: u64,
    },
    /// H→C. Non-fatal condition.
    Notice { code: NoticeCode, message: String },
    /// Either direction. Fatal for the current operation.
    Error { code: ErrorCode, message: String },
    /// C→H, protocol 2+. The phone's keyboard opened (`true`) or closed.
    /// While active the host reports the text caret with [`ControlMsg::Caret`].
    TextInput { active: bool },
    /// H→C, protocol 2+. Where the PC's text caret is, normalised 0..65535
    /// to the streamed display like pointer positions (`x`,`y` = top-left,
    /// `w`,`h` = size). `visible` is false when no caret could be found or
    /// it is on another display.
    Caret {
        visible: bool,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    },
}

mod tag {
    pub const HELLO: u8 = 1;
    pub const HELLO_ACK: u8 = 2;
    pub const PAIR_REQUEST: u8 = 3;
    pub const PAIR_RESPONSE: u8 = 4;
    pub const PAIR_CONFIRM: u8 = 5;
    pub const PAIR_RESULT: u8 = 6;
    pub const CAPABILITIES: u8 = 7;
    pub const DISPLAY_LIST: u8 = 8;
    pub const SESSION_START: u8 = 9;
    pub const SESSION_STARTED: u8 = 10;
    pub const SESSION_END: u8 = 11;
    pub const DISPLAY_SELECT: u8 = 12;
    pub const SET_STREAM_PARAMS: u8 = 13;
    pub const KEYFRAME_REQUEST: u8 = 14;
    pub const PING: u8 = 15;
    pub const PONG: u8 = 16;
    pub const STREAM_STATS: u8 = 17;
    pub const CLIENT_STATS: u8 = 18;
    pub const CURSOR_SHAPE: u8 = 19;
    pub const INPUT_ACK: u8 = 20;
    pub const NOTICE: u8 = 21;
    pub const ERROR: u8 = 22;
    pub const TEXT_INPUT: u8 = 23;
    pub const CARET: u8 = 24;
}

fn write_params(w: &mut Writer, p: &StreamParams) {
    w.u8(p.quality as u8);
    w.u8(p.codec as u8);
    w.u16(p.max_width);
    w.u16(p.max_height);
    w.u16(p.fps);
    w.u32(p.bitrate_kbps);
    w.u32(p.max_bitrate_kbps);
}

fn read_params(r: &mut Reader<'_>) -> Result<StreamParams, DecodeError> {
    let p = StreamParams {
        quality: QualityMode::from_u8(r.u8()?)?,
        codec: Codec::from_u8(r.u8()?)?,
        max_width: r.u16()?,
        max_height: r.u16()?,
        fps: r.u16()?,
        bitrate_kbps: r.u32()?,
        max_bitrate_kbps: r.u32()?,
    };
    if p.max_width > MAX_VIDEO_DIM || p.max_height > MAX_VIDEO_DIM {
        return Err(DecodeError::InvalidValue("max dimensions"));
    }
    if p.fps > 240 {
        return Err(DecodeError::InvalidValue("fps"));
    }
    Ok(p)
}

impl ControlMsg {
    /// Encode the message body (`tag | fields`), without the length prefix.
    pub fn encode_body(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(64);
        match self {
            ControlMsg::Hello {
                protocol_min,
                protocol_max,
                device_name,
                device_model,
                app_version,
            } => {
                w.u8(tag::HELLO);
                w.u16(*protocol_min);
                w.u16(*protocol_max);
                w.str(device_name, MAX_NAME);
                w.str(device_model, MAX_MODEL);
                w.str(app_version, MAX_VERSION);
            }
            ControlMsg::HelloAck {
                protocol,
                host_name,
                app_version,
                auth,
            } => {
                w.u8(tag::HELLO_ACK);
                w.u16(*protocol);
                w.str(host_name, MAX_NAME);
                w.str(app_version, MAX_VERSION);
                w.u8(*auth as u8);
            }
            ControlMsg::PairRequest { spake_msg } => {
                w.u8(tag::PAIR_REQUEST);
                w.bytes(spake_msg);
            }
            ControlMsg::PairResponse { spake_msg } => {
                w.u8(tag::PAIR_RESPONSE);
                w.bytes(spake_msg);
            }
            ControlMsg::PairConfirm { mac } => {
                w.u8(tag::PAIR_CONFIRM);
                w.raw(mac);
            }
            ControlMsg::PairResult {
                accepted,
                mac,
                reason,
            } => {
                w.u8(tag::PAIR_RESULT);
                w.bool(*accepted);
                w.raw(mac);
                w.u16(reason.as_u16());
            }
            ControlMsg::Capabilities {
                codecs,
                display_width,
                display_height,
                display_refresh_millihz,
            } => {
                w.u8(tag::CAPABILITIES);
                let n = codecs.len().min(MAX_CODEC_CAPS);
                w.u8(n as u8);
                for c in &codecs[..n] {
                    w.u8(c.codec as u8);
                    w.bool(c.hardware);
                    w.bool(c.low_latency);
                    w.u16(c.max_width);
                    w.u16(c.max_height);
                    w.u16(c.max_fps);
                }
                w.u16(*display_width);
                w.u16(*display_height);
                w.u32(*display_refresh_millihz);
            }
            ControlMsg::DisplayList { displays } => {
                w.u8(tag::DISPLAY_LIST);
                let n = displays.len().min(MAX_DISPLAYS);
                w.u8(n as u8);
                for d in &displays[..n] {
                    w.u32(d.id);
                    w.str(&d.name, MAX_NAME);
                    w.u32(d.width);
                    w.u32(d.height);
                    w.i32(d.x);
                    w.i32(d.y);
                    w.bool(d.primary);
                    w.u32(d.refresh_millihz);
                    w.u16(d.scale_percent);
                    w.u16(d.rotation);
                    w.bool(d.hdr);
                }
            }
            ControlMsg::SessionStart {
                display_id,
                params,
                view_only,
            } => {
                w.u8(tag::SESSION_START);
                w.u32(*display_id);
                write_params(&mut w, params);
                w.bool(*view_only);
            }
            ControlMsg::SessionStarted {
                session_id,
                display_id,
                view_only,
                encoder_name,
                capture_backend,
            } => {
                w.u8(tag::SESSION_STARTED);
                w.u32(*session_id);
                w.u32(*display_id);
                w.bool(*view_only);
                w.str(encoder_name, MAX_ENCODER_NAME);
                w.str(capture_backend, MAX_NAME);
            }
            ControlMsg::SessionEnd { reason } => {
                w.u8(tag::SESSION_END);
                w.u16(reason.as_u16());
            }
            ControlMsg::DisplaySelect { display_id } => {
                w.u8(tag::DISPLAY_SELECT);
                w.u32(*display_id);
            }
            ControlMsg::SetStreamParams { params } => {
                w.u8(tag::SET_STREAM_PARAMS);
                write_params(&mut w, params);
            }
            ControlMsg::KeyframeRequest => w.u8(tag::KEYFRAME_REQUEST),
            ControlMsg::Ping { id, t_send_us } => {
                w.u8(tag::PING);
                w.u32(*id);
                w.u64(*t_send_us);
            }
            ControlMsg::Pong {
                id,
                t_send_us,
                t_recv_us,
                t_reply_us,
            } => {
                w.u8(tag::PONG);
                w.u32(*id);
                w.u64(*t_send_us);
                w.u64(*t_recv_us);
                w.u64(*t_reply_us);
            }
            ControlMsg::StreamStats(s) => {
                w.u8(tag::STREAM_STATS);
                w.u16(s.captured_fps_x10);
                w.u16(s.encoded_fps_x10);
                w.u32(s.bitrate_kbps);
                w.u32(s.target_bitrate_kbps);
                w.u32(s.encode_us_p50);
                w.u32(s.encode_us_p95);
                w.u32(s.dropped_before_encode);
                w.u32(s.dropped_before_send);
                w.u8(s.send_queue_depth);
                w.u32(s.rtt_us);
                w.u64(s.lost_packets);
                w.u32(s.congestion_window);
            }
            ControlMsg::ClientStats(s) => {
                w.u8(tag::CLIENT_STATS);
                w.u16(s.received_fps_x10);
                w.u16(s.rendered_fps_x10);
                w.u32(s.decode_us_p50);
                w.u32(s.decode_us_p95);
                w.u32(s.frames_dropped);
                w.u8(s.decoder_queue_depth);
            }
            ControlMsg::CursorShape {
                shape_id,
                width,
                height,
                hot_x,
                hot_y,
                rgba,
            } => {
                w.u8(tag::CURSOR_SHAPE);
                w.u32(*shape_id);
                w.u16(*width);
                w.u16(*height);
                w.u16(*hot_x);
                w.u16(*hot_y);
                w.bytes(rgba);
            }
            ControlMsg::InputAck {
                seq,
                t_recv_us,
                t_inject_us,
            } => {
                w.u8(tag::INPUT_ACK);
                w.u32(*seq);
                w.u64(*t_recv_us);
                w.u64(*t_inject_us);
            }
            ControlMsg::Notice { code, message } => {
                w.u8(tag::NOTICE);
                w.u16(code.as_u16());
                w.str(message, MAX_MESSAGE_TEXT);
            }
            ControlMsg::Error { code, message } => {
                w.u8(tag::ERROR);
                w.u16(code.as_u16());
                w.str(message, MAX_MESSAGE_TEXT);
            }
            ControlMsg::TextInput { active } => {
                w.u8(tag::TEXT_INPUT);
                w.bool(*active);
            }
            ControlMsg::Caret {
                visible,
                x,
                y,
                w: cw,
                h,
            } => {
                w.u8(tag::CARET);
                w.bool(*visible);
                w.u16(*x);
                w.u16(*y);
                w.u16(*cw);
                w.u16(*h);
            }
        }
        w.into_bytes()
    }

    /// Encode with the `u32` length prefix, ready to write to the stream.
    pub fn encode_framed(&self) -> Vec<u8> {
        let body = self.encode_body();
        debug_assert!(body.len() <= MAX_CONTROL_MSG);
        let mut out = Vec::with_capacity(body.len() + 4);
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out
    }

    /// Decode one message body. Rejects unknown tags, out-of-range values,
    /// oversized fields and trailing bytes.
    pub fn decode_body(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() > MAX_CONTROL_MSG {
            return Err(DecodeError::FrameTooLarge(body.len()));
        }
        let mut r = Reader::new(body);
        let t = r.u8()?;
        let msg = match t {
            tag::HELLO => {
                let protocol_min = r.u16()?;
                let protocol_max = r.u16()?;
                if protocol_min > protocol_max {
                    return Err(DecodeError::InvalidValue("protocol range"));
                }
                ControlMsg::Hello {
                    protocol_min,
                    protocol_max,
                    device_name: r.str(MAX_NAME, "device_name")?,
                    device_model: r.str(MAX_MODEL, "device_model")?,
                    app_version: r.str(MAX_VERSION, "app_version")?,
                }
            }
            tag::HELLO_ACK => ControlMsg::HelloAck {
                protocol: r.u16()?,
                host_name: r.str(MAX_NAME, "host_name")?,
                app_version: r.str(MAX_VERSION, "app_version")?,
                auth: AuthState::from_u8(r.u8()?)?,
            },
            tag::PAIR_REQUEST => ControlMsg::PairRequest {
                spake_msg: r.bytes(MAX_SPAKE_MSG, "spake_msg")?,
            },
            tag::PAIR_RESPONSE => ControlMsg::PairResponse {
                spake_msg: r.bytes(MAX_SPAKE_MSG, "spake_msg")?,
            },
            tag::PAIR_CONFIRM => ControlMsg::PairConfirm { mac: r.array()? },
            tag::PAIR_RESULT => ControlMsg::PairResult {
                accepted: r.bool()?,
                mac: r.array()?,
                reason: ErrorCode::from_u16(r.u16()?),
            },
            tag::CAPABILITIES => {
                let n = r.u8()? as usize;
                if n > MAX_CODEC_CAPS {
                    return Err(DecodeError::TooLarge("codecs"));
                }
                let mut codecs = Vec::with_capacity(n);
                for _ in 0..n {
                    codecs.push(CodecCap {
                        codec: Codec::from_u8(r.u8()?)?,
                        hardware: r.bool()?,
                        low_latency: r.bool()?,
                        max_width: r.u16()?,
                        max_height: r.u16()?,
                        max_fps: r.u16()?,
                    });
                }
                ControlMsg::Capabilities {
                    codecs,
                    display_width: r.u16()?,
                    display_height: r.u16()?,
                    display_refresh_millihz: r.u32()?,
                }
            }
            tag::DISPLAY_LIST => {
                let n = r.u8()? as usize;
                if n > MAX_DISPLAYS {
                    return Err(DecodeError::TooLarge("displays"));
                }
                let mut displays = Vec::with_capacity(n);
                for _ in 0..n {
                    let d = DisplayInfo {
                        id: r.u32()?,
                        name: r.str(MAX_NAME, "display name")?,
                        width: r.u32()?,
                        height: r.u32()?,
                        x: r.i32()?,
                        y: r.i32()?,
                        primary: r.bool()?,
                        refresh_millihz: r.u32()?,
                        scale_percent: r.u16()?,
                        rotation: r.u16()?,
                        hdr: r.bool()?,
                    };
                    if !matches!(d.rotation, 0 | 90 | 180 | 270) {
                        return Err(DecodeError::InvalidValue("rotation"));
                    }
                    displays.push(d);
                }
                ControlMsg::DisplayList { displays }
            }
            tag::SESSION_START => ControlMsg::SessionStart {
                display_id: r.u32()?,
                params: read_params(&mut r)?,
                view_only: r.bool()?,
            },
            tag::SESSION_STARTED => ControlMsg::SessionStarted {
                session_id: r.u32()?,
                display_id: r.u32()?,
                view_only: r.bool()?,
                encoder_name: r.str(MAX_ENCODER_NAME, "encoder_name")?,
                capture_backend: r.str(MAX_NAME, "capture_backend")?,
            },
            tag::SESSION_END => ControlMsg::SessionEnd {
                reason: ErrorCode::from_u16(r.u16()?),
            },
            tag::DISPLAY_SELECT => ControlMsg::DisplaySelect {
                display_id: r.u32()?,
            },
            tag::SET_STREAM_PARAMS => ControlMsg::SetStreamParams {
                params: read_params(&mut r)?,
            },
            tag::KEYFRAME_REQUEST => ControlMsg::KeyframeRequest,
            tag::PING => ControlMsg::Ping {
                id: r.u32()?,
                t_send_us: r.u64()?,
            },
            tag::PONG => ControlMsg::Pong {
                id: r.u32()?,
                t_send_us: r.u64()?,
                t_recv_us: r.u64()?,
                t_reply_us: r.u64()?,
            },
            tag::STREAM_STATS => ControlMsg::StreamStats(StreamStats {
                captured_fps_x10: r.u16()?,
                encoded_fps_x10: r.u16()?,
                bitrate_kbps: r.u32()?,
                target_bitrate_kbps: r.u32()?,
                encode_us_p50: r.u32()?,
                encode_us_p95: r.u32()?,
                dropped_before_encode: r.u32()?,
                dropped_before_send: r.u32()?,
                send_queue_depth: r.u8()?,
                rtt_us: r.u32()?,
                lost_packets: r.u64()?,
                congestion_window: r.u32()?,
            }),
            tag::CLIENT_STATS => ControlMsg::ClientStats(ClientStats {
                received_fps_x10: r.u16()?,
                rendered_fps_x10: r.u16()?,
                decode_us_p50: r.u32()?,
                decode_us_p95: r.u32()?,
                frames_dropped: r.u32()?,
                decoder_queue_depth: r.u8()?,
            }),
            tag::CURSOR_SHAPE => {
                let shape_id = r.u32()?;
                let width = r.u16()?;
                let height = r.u16()?;
                let hot_x = r.u16()?;
                let hot_y = r.u16()?;
                if width == 0 || height == 0 || width > MAX_CURSOR_DIM || height > MAX_CURSOR_DIM {
                    return Err(DecodeError::InvalidValue("cursor size"));
                }
                if hot_x >= width || hot_y >= height {
                    return Err(DecodeError::InvalidValue("cursor hotspot"));
                }
                let expect = width as usize * height as usize * 4;
                let rgba = r.bytes(expect, "cursor rgba")?;
                if rgba.len() != expect {
                    return Err(DecodeError::InvalidValue("cursor rgba length"));
                }
                ControlMsg::CursorShape {
                    shape_id,
                    width,
                    height,
                    hot_x,
                    hot_y,
                    rgba,
                }
            }
            tag::INPUT_ACK => ControlMsg::InputAck {
                seq: r.u32()?,
                t_recv_us: r.u64()?,
                t_inject_us: r.u64()?,
            },
            tag::NOTICE => ControlMsg::Notice {
                code: NoticeCode::from_u16(r.u16()?),
                message: r.str(MAX_MESSAGE_TEXT, "message")?,
            },
            tag::ERROR => ControlMsg::Error {
                code: ErrorCode::from_u16(r.u16()?),
                message: r.str(MAX_MESSAGE_TEXT, "message")?,
            },
            tag::TEXT_INPUT => ControlMsg::TextInput { active: r.bool()? },
            tag::CARET => ControlMsg::Caret {
                visible: r.bool()?,
                x: r.u16()?,
                y: r.u16()?,
                w: r.u16()?,
                h: r.u16()?,
            },
            other => return Err(DecodeError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(msg)
    }

    /// Short name for logs. Never logs payloads (they can contain key
    /// confirmation material or device names).
    pub fn name(&self) -> &'static str {
        match self {
            ControlMsg::Hello { .. } => "Hello",
            ControlMsg::HelloAck { .. } => "HelloAck",
            ControlMsg::PairRequest { .. } => "PairRequest",
            ControlMsg::PairResponse { .. } => "PairResponse",
            ControlMsg::PairConfirm { .. } => "PairConfirm",
            ControlMsg::PairResult { .. } => "PairResult",
            ControlMsg::Capabilities { .. } => "Capabilities",
            ControlMsg::DisplayList { .. } => "DisplayList",
            ControlMsg::SessionStart { .. } => "SessionStart",
            ControlMsg::SessionStarted { .. } => "SessionStarted",
            ControlMsg::SessionEnd { .. } => "SessionEnd",
            ControlMsg::DisplaySelect { .. } => "DisplaySelect",
            ControlMsg::SetStreamParams { .. } => "SetStreamParams",
            ControlMsg::KeyframeRequest => "KeyframeRequest",
            ControlMsg::Ping { .. } => "Ping",
            ControlMsg::Pong { .. } => "Pong",
            ControlMsg::StreamStats(_) => "StreamStats",
            ControlMsg::ClientStats(_) => "ClientStats",
            ControlMsg::CursorShape { .. } => "CursorShape",
            ControlMsg::InputAck { .. } => "InputAck",
            ControlMsg::Notice { .. } => "Notice",
            ControlMsg::Error { .. } => "Error",
            ControlMsg::TextInput { .. } => "TextInput",
            ControlMsg::Caret { .. } => "Caret",
        }
    }
}

/// Pick the protocol version for a client's offered range, if any overlaps.
pub fn negotiate_version(client_min: u16, client_max: u16) -> Option<u16> {
    let hi = client_max.min(PROTOCOL_VERSION);
    let lo = client_min.max(PROTOCOL_VERSION_MIN);
    (lo <= hi).then_some(hi)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_negotiation() {
        assert_eq!(negotiate_version(1, 1), Some(1));
        assert_eq!(negotiate_version(1, 9), Some(PROTOCOL_VERSION));
        // A version-1 phone still connects to a version-2 PC, and vice versa
        // (a version-2 phone offering 1..=2 to a version-1 PC picks 1).
        assert_eq!(negotiate_version(1, 2), Some(2));
        assert_eq!(negotiate_version(PROTOCOL_VERSION + 1, 9), None);
        assert_eq!(negotiate_version(0, 0), None);
    }

    /// Fixed test vector; also documented in docs/protocol.md.
    #[test]
    fn ping_test_vector() {
        let m = ControlMsg::Ping {
            id: 7,
            t_send_us: 0x0102_0304_0506_0708,
        };
        assert_eq!(
            m.encode_framed(),
            [
                13, 0, 0, 0,  // body length
                15, // tag
                7, 0, 0, 0, // id
                8, 7, 6, 5, 4, 3, 2, 1, // t_send_us
            ]
        );
    }

    #[test]
    fn hello_test_vector() {
        let m = ControlMsg::Hello {
            protocol_min: 1,
            protocol_max: 1,
            device_name: "Pix".into(),
            device_model: "".into(),
            app_version: "0.1".into(),
        };
        assert_eq!(
            m.encode_body(),
            [1, 1, 0, 1, 0, 3, 0, b'P', b'i', b'x', 0, 0, 3, 0, b'0', b'.', b'1']
        );
    }

    /// Fixed test vectors for the protocol-2 messages (docs/protocol.md).
    #[test]
    fn text_input_and_caret_test_vectors() {
        let t = ControlMsg::TextInput { active: true };
        assert_eq!(t.encode_body(), [23, 1]);
        assert_eq!(ControlMsg::decode_body(&[23, 1]), Ok(t));
        let c = ControlMsg::Caret {
            visible: true,
            x: 0x0102,
            y: 0x0304,
            w: 5,
            h: 0x0600,
        };
        let body = [24, 1, 2, 1, 4, 3, 5, 0, 0, 6];
        assert_eq!(c.encode_body(), body);
        assert_eq!(ControlMsg::decode_body(&body), Ok(c));
        // Booleans must be 0 or 1.
        assert!(ControlMsg::decode_body(&[23, 2]).is_err());
    }

    #[test]
    fn rejects_trailing_bytes() {
        let mut b = ControlMsg::KeyframeRequest.encode_body();
        b.push(0);
        assert_eq!(ControlMsg::decode_body(&b), Err(DecodeError::TrailingBytes));
    }

    #[test]
    fn rejects_unknown_tag_and_empty() {
        assert_eq!(
            ControlMsg::decode_body(&[0xee]),
            Err(DecodeError::UnknownTag(0xee))
        );
        assert_eq!(ControlMsg::decode_body(&[]), Err(DecodeError::Truncated));
    }

    #[test]
    fn cursor_shape_must_match_dimensions() {
        let good = ControlMsg::CursorShape {
            shape_id: 1,
            width: 2,
            height: 2,
            hot_x: 0,
            hot_y: 1,
            rgba: vec![0; 16],
        };
        assert_eq!(ControlMsg::decode_body(&good.encode_body()), Ok(good));

        let short = ControlMsg::CursorShape {
            shape_id: 1,
            width: 2,
            height: 2,
            hot_x: 0,
            hot_y: 0,
            rgba: vec![0; 12],
        };
        assert!(ControlMsg::decode_body(&short.encode_body()).is_err());

        let hotspot_outside = ControlMsg::CursorShape {
            shape_id: 1,
            width: 2,
            height: 2,
            hot_x: 2,
            hot_y: 0,
            rgba: vec![0; 16],
        };
        assert!(ControlMsg::decode_body(&hotspot_outside.encode_body()).is_err());
    }

    #[test]
    fn hello_rejects_inverted_range() {
        let m = ControlMsg::Hello {
            protocol_min: 3,
            protocol_max: 1,
            device_name: String::new(),
            device_model: String::new(),
            app_version: String::new(),
        };
        assert!(ControlMsg::decode_body(&m.encode_body()).is_err());
    }
}
