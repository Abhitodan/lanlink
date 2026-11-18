//! QUIC transport for LANLink.
//!
//! Responsibilities: build Quinn endpoints with pinned mutual TLS, expose the
//! peer's certificate fingerprint and the pairing channel binding, and read
//! and write the length-prefixed records defined in `lanlink-protocol` with
//! every size limit enforced *before* allocation.
//!
//! Everything above this crate deals in typed messages, never raw bytes.

mod tls;

pub use tls::{ServerTrust, SERVER_NAME};

use lanlink_auth::pairing::{BINDING_LEN, EXPORTER_LABEL};
use lanlink_auth::{Fingerprint, Identity};
use lanlink_protocol::limits::*;
use lanlink_protocol::video::{self, FrameHeader, VideoConfig};
use lanlink_protocol::{ControlMsg, DecodeError, InputEvent, StreamKind};
use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, RecvStream, SendStream};
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

/// Close the connection after this long without any packet from the peer.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(6);
/// Keep-alive interval; also bounds how fast a dead path is noticed.
pub const KEEP_ALIVE: Duration = Duration::from_secs(1);
/// QUIC application error code used for orderly closes.
pub const CLOSE_NORMAL: u32 = 0;
/// QUIC application error code used when the peer broke the protocol.
pub const CLOSE_PROTOCOL: u32 = 1;
/// QUIC application error code used when the peer is not authorized.
pub const CLOSE_UNAUTHORIZED: u32 = 2;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("TLS configuration failed: {0}")]
    Tls(String),
    #[error("could not bind UDP socket on {0}: {1}")]
    Bind(SocketAddr, std::io::Error),
    #[error("could not start connection: {0}")]
    Connect(#[from] quinn::ConnectError),
    #[error("connection failed: {0}")]
    Connection(#[from] quinn::ConnectionError),
    #[error("stream write failed: {0}")]
    Write(#[from] quinn::WriteError),
    #[error("stream read failed: {0}")]
    Read(String),
    #[error("stream closed by peer")]
    Closed,
    #[error("peer sent a malformed message: {0}")]
    Malformed(#[from] DecodeError),
    #[error("peer presented no usable certificate")]
    NoPeerCertificate,
    #[error("channel binding unavailable")]
    NoChannelBinding,
}

/// Transport parameters shared by both ends.
///
/// The numbers are bounds on memory and on how stale data can get, not
/// throughput tuning: receive windows cap what a peer can make us buffer,
/// and the send window caps how much encoded video can sit in Quinn (beyond
/// the session manager's own small frame queue) when the network stalls.
fn transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    t.max_idle_timeout(Some(
        IDLE_TIMEOUT.try_into().expect("idle timeout fits a varint"),
    ));
    t.keep_alive_interval(Some(KEEP_ALIVE));
    t.max_concurrent_bidi_streams(4u32.into());
    t.max_concurrent_uni_streams(16u32.into());
    t.stream_receive_window((2u32 * 1024 * 1024).into());
    t.receive_window((4u32 * 1024 * 1024).into());
    t.send_window(2 * 1024 * 1024);
    t.datagram_receive_buffer_size(Some(64 * 1024));
    t.datagram_send_buffer_size(16 * 1024);
    // A LAN path is far faster than the 333 ms default guess; a realistic
    // initial RTT makes loss recovery on the first packets prompt.
    t.initial_rtt(Duration::from_millis(30));
    t
}

/// Create the host endpoint.
pub fn server_endpoint(
    identity: &Identity,
    bind: SocketAddr,
) -> Result<quinn::Endpoint, TransportError> {
    let crypto = QuicServerConfig::try_from(tls::rustls_server(identity)?)
        .map_err(|e| TransportError::Tls(e.to_string()))?;
    let mut cfg = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    cfg.transport_config(Arc::new(transport_config()));
    // Unauthenticated peers can open at most this many half-open handshakes.
    cfg.max_incoming(32);
    quinn::Endpoint::server(cfg, bind).map_err(|e| TransportError::Bind(bind, e))
}

/// Create a client endpoint bound to an ephemeral port.
pub fn client_endpoint(bind: SocketAddr) -> Result<quinn::Endpoint, TransportError> {
    quinn::Endpoint::client(bind).map_err(|e| TransportError::Bind(bind, e))
}

/// Client configuration for one connection attempt.
pub fn client_config(
    identity: &Identity,
    trust: ServerTrust,
) -> Result<quinn::ClientConfig, TransportError> {
    let crypto = QuicClientConfig::try_from(tls::rustls_client(identity, trust)?)
        .map_err(|e| TransportError::Tls(e.to_string()))?;
    let mut cfg = quinn::ClientConfig::new(Arc::new(crypto));
    cfg.transport_config(Arc::new(transport_config()));
    Ok(cfg)
}

/// Fingerprint of the certificate the peer proved possession of.
pub fn peer_fingerprint(conn: &Connection) -> Result<Fingerprint, TransportError> {
    let identity = conn
        .peer_identity()
        .ok_or(TransportError::NoPeerCertificate)?;
    let certs = identity
        .downcast::<Vec<rustls::pki_types::CertificateDer<'static>>>()
        .map_err(|_| TransportError::NoPeerCertificate)?;
    let leaf = certs.first().ok_or(TransportError::NoPeerCertificate)?;
    Ok(Fingerprint::of_cert(leaf.as_ref()))
}

/// TLS exporter value unique to this connection, used to bind the pairing
/// exchange to the channel it runs on.
pub fn channel_binding(conn: &Connection) -> Result<[u8; BINDING_LEN], TransportError> {
    let mut out = [0u8; BINDING_LEN];
    conn.export_keying_material(&mut out, EXPORTER_LABEL, b"")
        .map_err(|_| TransportError::NoChannelBinding)?;
    Ok(out)
}

fn read_err(e: quinn::ReadExactError) -> TransportError {
    match e {
        quinn::ReadExactError::FinishedEarly(_) => TransportError::Closed,
        quinn::ReadExactError::ReadError(e) => TransportError::Read(e.to_string()),
    }
}

/// Write the stream-kind prefix. Must be the first write on a new stream.
pub async fn write_stream_kind(
    send: &mut SendStream,
    kind: StreamKind,
) -> Result<(), TransportError> {
    send.write_all(&[kind as u8]).await?;
    Ok(())
}

/// Read the stream-kind prefix from a stream the peer opened.
pub async fn read_stream_kind(recv: &mut RecvStream) -> Result<StreamKind, TransportError> {
    let mut b = [0u8; 1];
    recv.read_exact(&mut b).await.map_err(read_err)?;
    Ok(StreamKind::from_u8(b[0])?)
}

pub async fn write_control(send: &mut SendStream, msg: &ControlMsg) -> Result<(), TransportError> {
    send.write_all(&msg.encode_framed()).await?;
    Ok(())
}

/// Read one control message. Returns `Ok(None)` on a clean end of stream at
/// a message boundary.
pub async fn read_control(recv: &mut RecvStream) -> Result<Option<ControlMsg>, TransportError> {
    let mut len = [0u8; 4];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(read_err(e)),
    }
    let len = u32::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_CONTROL_MSG {
        return Err(DecodeError::FrameTooLarge(len).into());
    }
    let mut body = vec![0u8; len];
    recv.read_exact(&mut body).await.map_err(read_err)?;
    Ok(Some(ControlMsg::decode_body(&body)?))
}

pub async fn write_input(send: &mut SendStream, ev: &InputEvent) -> Result<(), TransportError> {
    send.write_all(&ev.encode_framed()).await?;
    Ok(())
}

/// Read one reliable input record; `Ok(None)` on clean end of stream.
pub async fn read_input(recv: &mut RecvStream) -> Result<Option<InputEvent>, TransportError> {
    let mut len = [0u8; 2];
    match recv.read_exact(&mut len).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(read_err(e)),
    }
    let len = u16::from_le_bytes(len) as usize;
    if len == 0 || len > MAX_INPUT_MSG {
        return Err(DecodeError::FrameTooLarge(len).into());
    }
    let mut body = [0u8; MAX_INPUT_MSG];
    recv.read_exact(&mut body[..len]).await.map_err(read_err)?;
    Ok(Some(InputEvent::decode_body(&body[..len])?))
}

/// One record from the video stream.
#[derive(Debug)]
pub enum VideoRecord {
    Config(VideoConfig),
    /// The access unit is in the buffer passed to [`read_video_record`].
    Frame(FrameHeader),
}

pub async fn write_video_config(
    send: &mut SendStream,
    cfg: &VideoConfig,
) -> Result<(), TransportError> {
    send.write_all(&cfg.encode()).await?;
    Ok(())
}

pub async fn write_video_frame(
    send: &mut SendStream,
    header: &FrameHeader,
    payload: &[u8],
) -> Result<(), TransportError> {
    debug_assert_eq!(header.payload_len as usize, payload.len());
    send.write_all(&header.encode()).await?;
    send.write_all(payload).await?;
    Ok(())
}

/// Read one video record. For a frame, `payload` is resized to hold exactly
/// the access unit; the buffer is reused across calls so the steady state
/// allocates nothing. `Ok(None)` means the host ended the stream.
pub async fn read_video_record(
    recv: &mut RecvStream,
    payload: &mut Vec<u8>,
) -> Result<Option<VideoRecord>, TransportError> {
    let mut tag = [0u8; 1];
    match recv.read_exact(&mut tag).await {
        Ok(()) => {}
        Err(quinn::ReadExactError::FinishedEarly(0)) => return Ok(None),
        Err(e) => return Err(read_err(e)),
    }
    match tag[0] {
        video::TAG_CONFIG => {
            let mut body = [0u8; video::CONFIG_BODY_LEN];
            recv.read_exact(&mut body).await.map_err(read_err)?;
            Ok(Some(VideoRecord::Config(VideoConfig::decode_body(&body)?)))
        }
        video::TAG_FRAME => {
            let mut body = [0u8; video::FRAME_HEADER_LEN];
            recv.read_exact(&mut body).await.map_err(read_err)?;
            // `decode_body` enforces MAX_VIDEO_FRAME before we size the buffer.
            let header = FrameHeader::decode_body(&body)?;
            payload.resize(header.payload_len as usize, 0);
            recv.read_exact(payload).await.map_err(read_err)?;
            Ok(Some(VideoRecord::Frame(header)))
        }
        other => Err(DecodeError::UnknownTag(other).into()),
    }
}

/// Microseconds on a process-wide monotonic clock. Only differences between
/// values from the *same* process are meaningful.
pub fn monotonic_us() -> u64 {
    use std::sync::OnceLock;
    use std::time::Instant;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}
