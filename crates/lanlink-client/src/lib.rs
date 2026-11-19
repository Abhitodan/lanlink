//! LANLink client: pairing, authenticated connection, session control,
//! video reception and input sending.
//!
//! Used by the Android app (through `native-bridge`) and by the desktop
//! integration tests, so both exercise the same code.
//!
//! Received video passes through a [`FrameGate`] before it is handed to the
//! decoder: a delta frame is only delivered if its reference chain is
//! intact; otherwise it is discarded and a keyframe is requested. The video
//! hand-off queue is small and never grows: if the decoder falls behind,
//! frames are discarded the same way rather than queued.

use lanlink_auth::pairing::PairingClient;
use lanlink_auth::{Fingerprint, Identity};
use lanlink_protocol::limits::{PROTOCOL_VERSION, PROTOCOL_VERSION_MIN};
use lanlink_protocol::*;
use lanlink_transport::*;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;

pub use lanlink_auth;
pub use lanlink_protocol;

const STEP_TIMEOUT: Duration = Duration::from_secs(8);
const APPROVAL_WAIT: Duration = Duration::from_secs(95);
/// Decoder hand-off queue. Small: each slot is a frame of latency.
pub const VIDEO_QUEUE: usize = 4;
const PING_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Debug, Error)]
pub enum ClientError {
    #[error("could not reach the PC: {0}")]
    Connect(String),
    #[error("the PC did not answer in time")]
    Timeout,
    #[error("connection lost: {0}")]
    Lost(String),
    #[error("the PC is a different computer than the one paired (identity mismatch)")]
    IdentityMismatch,
    #[error("this phone is not paired with the PC")]
    NotPaired,
    #[error("pairing is not open on the PC")]
    PairingClosed,
    #[error("the pairing code is wrong")]
    WrongCode,
    #[error("pairing was declined on the PC")]
    PairingDenied,
    /// Manual pairing reached a device that claims to know this phone
    /// already. Without the QR code nothing proves it is the user's PC.
    #[error("the PC says this phone is already paired, but its identity cannot be checked without the QR code (unverified)")]
    UnverifiedHost,
    #[error("incompatible LANLink version")]
    Version,
    #[error("{0:?}: {1}")]
    Host(ErrorCode, String),
    #[error("unexpected reply from the PC: {0}")]
    Protocol(String),
    #[error("{0}")]
    Auth(#[from] lanlink_auth::AuthError),
}

impl From<TransportError> for ClientError {
    fn from(e: TransportError) -> Self {
        match e {
            TransportError::Connection(quinn::ConnectionError::TimedOut) => ClientError::Timeout,
            TransportError::Connection(quinn::ConnectionError::TransportError(t))
                if t.to_string().to_lowercase().contains("certificate") =>
            {
                ClientError::IdentityMismatch
            }
            TransportError::Connection(c) => ClientError::Lost(c.to_string()),
            TransportError::Malformed(m) => ClientError::Protocol(m.to_string()),
            other => ClientError::Lost(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct DeviceInfo {
    pub name: String,
    pub model: String,
    pub app_version: String,
}

/// Result of a successful pairing.
#[derive(Debug, Clone)]
pub struct Paired {
    pub host_name: String,
    pub host_fingerprint: Fingerprint,
    /// Shown on the phone while the PC user approves; must match the PC.
    pub confirmation_code: String,
}

fn bind_for(addr: &SocketAddr) -> SocketAddr {
    if addr.is_ipv6() {
        "[::]:0".parse().expect("valid")
    } else {
        "0.0.0.0:0".parse().expect("valid")
    }
}

async fn step<T>(
    f: impl std::future::Future<Output = Result<T, TransportError>>,
) -> Result<T, ClientError> {
    timeout(STEP_TIMEOUT, f)
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(Into::into)
}

async fn handshake(
    identity: &Identity,
    device: &DeviceInfo,
    addr: SocketAddr,
    trust: ServerTrust,
) -> Result<
    (
        quinn::Endpoint,
        quinn::Connection,
        quinn::SendStream,
        quinn::RecvStream,
        String,
        AuthState,
        u16,
    ),
    ClientError,
> {
    let endpoint =
        client_endpoint(bind_for(&addr)).map_err(|e| ClientError::Connect(e.to_string()))?;
    let cfg = client_config(identity, trust).map_err(|e| ClientError::Connect(e.to_string()))?;
    let connecting = endpoint
        .connect_with(cfg, addr, SERVER_NAME)
        .map_err(|e| ClientError::Connect(e.to_string()))?;
    let conn = timeout(STEP_TIMEOUT, connecting)
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(|e| ClientError::from(TransportError::Connection(e)))?;
    let (mut send, mut recv) =
        step(async { conn.open_bi().await.map_err(TransportError::Connection) }).await?;
    step(write_stream_kind(&mut send, StreamKind::Control)).await?;
    step(write_control(
        &mut send,
        &ControlMsg::Hello {
            protocol_min: PROTOCOL_VERSION_MIN,
            protocol_max: PROTOCOL_VERSION,
            device_name: device.name.clone(),
            device_model: device.model.clone(),
            app_version: device.app_version.clone(),
        },
    ))
    .await?;
    match step(read_control(&mut recv)).await? {
        Some(ControlMsg::HelloAck {
            protocol,
            host_name,
            auth,
            ..
        }) => Ok((
            endpoint,
            conn,
            send,
            recv,
            // Shown in the app and saved with the paired PC.
            lanlink_protocol::wire::display_name(&host_name, 48),
            auth,
            protocol,
        )),
        Some(ControlMsg::Error {
            code: ErrorCode::ProtocolVersionUnsupported,
            ..
        }) => Err(ClientError::Version),
        Some(ControlMsg::Error { code, message }) => Err(ClientError::Host(code, message)),
        Some(other) => Err(ClientError::Protocol(other.name().into())),
        None => Err(ClientError::Lost("closed during handshake".into())),
    }
}

/// Pair with a host. `host_fp` comes from the QR code (pinned) or is
/// `None` for manual pairing, in which case the certificate seen on this
/// connection is authenticated by the pairing exchange itself.
/// `on_confirmation_code` is called as soon as the code to compare is known.
pub async fn pair(
    identity: &Identity,
    device: &DeviceInfo,
    addr: SocketAddr,
    host_fp: Option<Fingerprint>,
    code: &str,
    on_confirmation_code: impl FnOnce(&str),
) -> Result<Paired, ClientError> {
    let code = lanlink_auth::pairing::normalize_code(code)?;
    let trust = host_fp
        .map(ServerTrust::Pinned)
        .unwrap_or(ServerTrust::Unverified);
    let (_endpoint, conn, mut send, mut recv, host_name, auth, _protocol) =
        handshake(identity, device, addr, trust).await?;
    let seen_fp = peer_fingerprint(&conn).map_err(ClientError::from)?;
    match auth {
        // "Already paired" skips the code exchange, so it proves nothing
        // about the PC. Only trust it when TLS already checked the PC's
        // pinned identity (QR code); otherwise any device answering at that
        // address could get itself saved as the user's PC.
        AuthState::Paired if host_fp.is_none() => {
            conn.close(CLOSE_NORMAL.into(), b"unverified");
            return Err(ClientError::UnverifiedHost);
        }
        AuthState::Paired => {
            conn.close(CLOSE_NORMAL.into(), b"already paired");
            return Ok(Paired {
                host_name,
                host_fingerprint: seen_fp,
                confirmation_code: String::new(),
            });
        }
        AuthState::NotPaired => return Err(ClientError::PairingClosed),
        AuthState::PairingAvailable => {}
    }
    let binding = channel_binding(&conn).map_err(ClientError::from)?;
    let (client, msg) = PairingClient::start(&code, identity.fingerprint(), seen_fp)?;
    step(write_control(
        &mut send,
        &ControlMsg::PairRequest { spake_msg: msg },
    ))
    .await?;
    let host_msg = match step(read_control(&mut recv)).await? {
        Some(ControlMsg::PairResponse { spake_msg }) => spake_msg,
        Some(ControlMsg::PairResult { reason, .. }) => return Err(pair_error(reason)),
        Some(other) => return Err(ClientError::Protocol(other.name().into())),
        None => return Err(ClientError::Lost("closed during pairing".into())),
    };
    let keys = client.finish(&host_msg, &binding)?;
    step(write_control(
        &mut send,
        &ControlMsg::PairConfirm {
            mac: keys.client_mac,
        },
    ))
    .await?;
    on_confirmation_code(&keys.confirmation_code);
    let result = timeout(APPROVAL_WAIT, read_control(&mut recv))
        .await
        .map_err(|_| ClientError::Timeout)?
        .map_err(ClientError::from)?;
    match result {
        Some(ControlMsg::PairResult {
            accepted: true,
            mac,
            ..
        }) => {
            // The host proves it derived the same key: only the computer
            // that showed the code can produce this.
            keys.verify_host(&mac)
                .map_err(|_| ClientError::IdentityMismatch)?;
            conn.close(CLOSE_NORMAL.into(), b"paired");
            Ok(Paired {
                host_name,
                host_fingerprint: seen_fp,
                confirmation_code: keys.confirmation_code,
            })
        }
        Some(ControlMsg::PairResult { reason, .. }) => Err(pair_error(reason)),
        Some(other) => Err(ClientError::Protocol(other.name().into())),
        None => Err(ClientError::Lost("closed during pairing".into())),
    }
}

fn pair_error(reason: ErrorCode) -> ClientError {
    match reason {
        ErrorCode::PairingFailed => ClientError::WrongCode,
        ErrorCode::PairingDenied => ClientError::PairingDenied,
        ErrorCode::PairingClosed => ClientError::PairingClosed,
        other => ClientError::Host(other, "pairing failed".into()),
    }
}

/// Control-plane events delivered to the app.
#[derive(Debug, Clone)]
pub enum ClientEvent {
    DisplayList(Vec<DisplayInfo>),
    SessionStarted {
        session_id: u32,
        display_id: u32,
        view_only: bool,
        encoder: String,
        capture: String,
    },
    SessionEnded(ErrorCode),
    Stats(StreamStats),
    CursorShape {
        shape_id: u32,
        width: u16,
        height: u16,
        hot_x: u16,
        hot_y: u16,
        rgba: Vec<u8>,
    },
    CursorPos {
        x: u16,
        y: u16,
        visible: bool,
        shape_id: u32,
    },
    /// The PC's text caret, normalised to the streamed display.
    Caret {
        visible: bool,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
    },
    InputAck {
        seq: u32,
    },
    /// Round trip measured with Ping/Pong; `offset_us` estimates
    /// host_clock − client_clock with uncertainty ±`rtt_us`/2.
    Rtt {
        rtt_us: u32,
        offset_us: i64,
    },
    Notice(NoticeCode, String),
    Error(ErrorCode, String),
    Closed(String),
}

/// One item for the decoder.
#[derive(Debug)]
pub enum VideoItem {
    Config(VideoConfig),
    Frame {
        header: FrameHeader,
        data: Vec<u8>,
        recv_us: u64,
    },
}

#[derive(Default)]
pub struct VideoCounters {
    pub received: AtomicU64,
    pub delivered: AtomicU64,
    pub skipped: AtomicU64,
    pub bytes: AtomicU64,
}

/// An authenticated connection to a paired host.
pub struct Connection {
    _endpoint: quinn::Endpoint,
    conn: quinn::Connection,
    pub host_name: String,
    /// Protocol version agreed with the host.
    pub protocol: u16,
    control: mpsc::Sender<ControlMsg>,
    pub events: mpsc::Receiver<ClientEvent>,
    pub video: mpsc::Receiver<VideoItem>,
    input: mpsc::Sender<InputKind>,
    session: watch::Sender<u32>,
    session_id: Arc<AtomicU32>,
    pointer_seq: AtomicU32,
    pub counters: Arc<VideoCounters>,
}

/// Connect to a paired host whose certificate fingerprint is `host_fp`.
pub async fn connect(
    identity: &Identity,
    device: &DeviceInfo,
    addr: SocketAddr,
    host_fp: Fingerprint,
) -> Result<Connection, ClientError> {
    let (endpoint, conn, send, recv, host_name, auth, protocol) =
        handshake(identity, device, addr, ServerTrust::Pinned(host_fp)).await?;
    if auth != AuthState::Paired {
        conn.close(CLOSE_NORMAL.into(), b"not paired");
        return Err(ClientError::NotPaired);
    }

    let (control_tx, control_rx) = mpsc::channel::<ControlMsg>(64);
    let (event_tx, event_rx) = mpsc::channel::<ClientEvent>(256);
    let (video_tx, video_rx) = mpsc::channel::<VideoItem>(VIDEO_QUEUE);
    let (input_tx, input_rx) = mpsc::channel::<InputKind>(256);
    let (session_tx, session_rx) = watch::channel(0u32);
    let session_id = Arc::new(AtomicU32::new(0));
    let counters = Arc::new(VideoCounters::default());

    tokio::spawn(control_writer(send, control_rx));
    tokio::spawn(control_reader(
        recv,
        event_tx.clone(),
        session_id.clone(),
        control_tx.clone(),
    ));
    tokio::spawn(video_reader(
        conn.clone(),
        video_tx,
        control_tx.clone(),
        counters.clone(),
    ));
    tokio::spawn(datagram_reader(
        conn.clone(),
        event_tx.clone(),
        session_id.clone(),
    ));
    tokio::spawn(input_writer(conn.clone(), input_rx, session_rx));
    tokio::spawn(pinger(control_tx.clone()));
    let ev = event_tx;
    let c = conn.clone();
    tokio::spawn(async move {
        let reason = c.closed().await;
        let _ = ev.send(ClientEvent::Closed(reason.to_string())).await;
    });

    Ok(Connection {
        _endpoint: endpoint,
        conn,
        host_name,
        protocol,
        control: control_tx,
        events: event_rx,
        video: video_rx,
        input: input_tx,
        session: session_tx,
        session_id,
        pointer_seq: AtomicU32::new(0),
        counters,
    })
}

impl Connection {
    pub async fn send(&self, msg: ControlMsg) -> Result<(), ClientError> {
        self.control
            .send(msg)
            .await
            .map_err(|_| ClientError::Lost("control channel closed".into()))
    }

    /// Tell the host the phone's keyboard opened or closed, so it reports
    /// the text caret. Hosts older than protocol 2 do not know the message
    /// and would treat it as a protocol error, so nothing is sent to them.
    /// Returns whether it was sent.
    pub fn set_text_input(&self, active: bool) -> bool {
        self.protocol >= 2 && self.try_send(ControlMsg::TextInput { active })
    }

    /// Non-blocking variant for UI threads.
    pub fn try_send(&self, msg: ControlMsg) -> bool {
        self.control.try_send(msg).is_ok()
    }

    /// Call when a `SessionStarted` event arrives, so input is tagged with
    /// the right session and a fresh input stream is opened. Input sent
    /// while no session is bound (id 0) is dropped.
    pub fn bind_session(&self, session_id: u32) {
        self.session_id.store(session_id, Ordering::SeqCst);
        self.pointer_seq.store(0, Ordering::SeqCst);
        let _ = self.session.send(session_id);
    }

    pub fn session_id(&self) -> u32 {
        self.session_id.load(Ordering::SeqCst)
    }

    /// Reliable input (buttons, keys, text, scroll). Never blocks; returns
    /// `false` if the queue is full. Input sent while no session is bound is
    /// dropped.
    pub fn send_input(&self, kind: InputKind) -> bool {
        self.input.try_send(kind).is_ok()
    }

    /// Latest pointer position, as an unreliable datagram (falls back to a
    /// reliable move if the path does not support datagrams).
    pub fn send_pointer(&self, x: u16, y: u16) {
        let session_id = self.session_id();
        if session_id == 0 {
            return;
        }
        let seq = self
            .pointer_seq
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let d = Datagram::PointerAbs {
            session_id,
            seq,
            x,
            y,
        };
        if self.conn.max_datagram_size().is_some() {
            let _ = self.conn.send_datagram(d.encode().into());
        } else {
            let _ = self.input.try_send(InputKind::MoveAbs { x, y });
        }
    }

    pub fn send_pointer_rel(&self, dx: i16, dy: i16) {
        let session_id = self.session_id();
        if session_id == 0 {
            return;
        }
        let seq = self
            .pointer_seq
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);
        let d = Datagram::PointerRel {
            session_id,
            seq,
            dx,
            dy,
        };
        if self.conn.max_datagram_size().is_some() {
            let _ = self.conn.send_datagram(d.encode().into());
        } else {
            let _ = self.input.try_send(InputKind::MoveRel { dx, dy });
        }
    }

    pub fn rtt(&self) -> Duration {
        self.conn.rtt()
    }

    pub fn close(&self) {
        self.conn.close(CLOSE_NORMAL.into(), b"closed by phone");
    }

    pub fn is_closed(&self) -> bool {
        self.conn.close_reason().is_some()
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.conn.close(CLOSE_NORMAL.into(), b"closed by phone");
    }
}

async fn control_writer(mut send: quinn::SendStream, mut rx: mpsc::Receiver<ControlMsg>) {
    while let Some(m) = rx.recv().await {
        if write_control(&mut send, &m).await.is_err() {
            break;
        }
    }
    let _ = send.finish();
}

async fn control_reader(
    mut recv: quinn::RecvStream,
    tx: mpsc::Sender<ClientEvent>,
    session_id: Arc<AtomicU32>,
    control: mpsc::Sender<ControlMsg>,
) {
    loop {
        let msg = match read_control(&mut recv).await {
            Ok(Some(m)) => m,
            Ok(None) => break,
            Err(e) => {
                let _ = tx.send(ClientEvent::Closed(e.to_string())).await;
                break;
            }
        };
        let ev = match msg {
            ControlMsg::DisplayList { displays } => ClientEvent::DisplayList(displays),
            ControlMsg::SessionStarted {
                session_id: id,
                display_id,
                view_only,
                encoder_name,
                capture_backend,
            } => {
                session_id.store(id, Ordering::SeqCst);
                ClientEvent::SessionStarted {
                    session_id: id,
                    display_id,
                    view_only,
                    encoder: encoder_name,
                    capture: capture_backend,
                }
            }
            ControlMsg::SessionEnd { reason } => ClientEvent::SessionEnded(reason),
            ControlMsg::StreamStats(s) => ClientEvent::Stats(s),
            ControlMsg::CursorShape {
                shape_id,
                width,
                height,
                hot_x,
                hot_y,
                rgba,
            } => ClientEvent::CursorShape {
                shape_id,
                width,
                height,
                hot_x,
                hot_y,
                rgba,
            },
            ControlMsg::InputAck { seq, .. } => ClientEvent::InputAck { seq },
            ControlMsg::Pong {
                t_send_us,
                t_recv_us,
                t_reply_us,
                ..
            } => {
                let now = monotonic_us();
                let rtt = now.saturating_sub(t_send_us);
                let offset =
                    ((t_recv_us as i64 - t_send_us as i64) + (t_reply_us as i64 - now as i64)) / 2;
                ClientEvent::Rtt {
                    rtt_us: rtt.min(u32::MAX as u64) as u32,
                    offset_us: offset,
                }
            }
            ControlMsg::Ping { id, t_send_us } => {
                let t = monotonic_us();
                let _ = control.try_send(ControlMsg::Pong {
                    id,
                    t_send_us,
                    t_recv_us: t,
                    t_reply_us: monotonic_us(),
                });
                continue;
            }
            ControlMsg::Notice { code, message } => ClientEvent::Notice(code, message),
            ControlMsg::Error { code, message } => ClientEvent::Error(code, message),
            ControlMsg::Caret {
                visible,
                x,
                y,
                w,
                h,
            } => ClientEvent::Caret {
                visible,
                x,
                y,
                w,
                h,
            },
            _ => continue,
        };
        // Cursor positions and stats are superseded by later ones; the rest
        // must arrive, so wait for room.
        if tx.send(ev).await.is_err() {
            break;
        }
    }
}

async fn pinger(control: mpsc::Sender<ControlMsg>) {
    let mut id = 0u32;
    let mut tick = tokio::time::interval(PING_INTERVAL);
    loop {
        tick.tick().await;
        id = id.wrapping_add(1);
        if control
            .send(ControlMsg::Ping {
                id,
                t_send_us: monotonic_us(),
            })
            .await
            .is_err()
        {
            break;
        }
    }
}

async fn video_reader(
    conn: quinn::Connection,
    tx: mpsc::Sender<VideoItem>,
    control: mpsc::Sender<ControlMsg>,
    counters: Arc<VideoCounters>,
) {
    // One video stream per session; accept them as they come.
    while let Ok(mut recv) = conn.accept_uni().await {
        match timeout(STEP_TIMEOUT, read_stream_kind(&mut recv)).await {
            Ok(Ok(StreamKind::Video)) => {}
            _ => continue,
        }
        let mut gate = FrameGate::new();
        let mut buf = Vec::with_capacity(256 * 1024);
        loop {
            let rec = match read_video_record(&mut recv, &mut buf).await {
                Ok(Some(r)) => r,
                Ok(None) => break,
                Err(e) => {
                    tracing::warn!(error = %e, "video stream error");
                    break;
                }
            };
            match rec {
                VideoRecord::Config(c) => {
                    gate.on_config(c.config_id);
                    if tx.send(VideoItem::Config(c)).await.is_err() {
                        return;
                    }
                }
                VideoRecord::Frame(h) => {
                    counters.received.fetch_add(1, Ordering::Relaxed);
                    counters
                        .bytes
                        .fetch_add(buf.len() as u64, Ordering::Relaxed);
                    match gate.on_frame(&h) {
                        GateDecision::Decode => {
                            let item = VideoItem::Frame {
                                header: h,
                                data: std::mem::take(&mut buf),
                                recv_us: monotonic_us(),
                            };
                            match tx.try_send(item) {
                                Ok(()) => {
                                    counters.delivered.fetch_add(1, Ordering::Relaxed);
                                }
                                Err(mpsc::error::TrySendError::Full(VideoItem::Frame {
                                    data,
                                    ..
                                })) => {
                                    // Decoder is behind: the next delta would
                                    // lack this reference, so resync.
                                    buf = data;
                                    counters.skipped.fetch_add(1, Ordering::Relaxed);
                                    gate.require_keyframe();
                                    let _ = control.try_send(ControlMsg::KeyframeRequest);
                                }
                                Err(_) => return,
                            }
                        }
                        GateDecision::Skip { request_keyframe } => {
                            counters.skipped.fetch_add(1, Ordering::Relaxed);
                            if request_keyframe {
                                let _ = control.try_send(ControlMsg::KeyframeRequest);
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn datagram_reader(
    conn: quinn::Connection,
    tx: mpsc::Sender<ClientEvent>,
    session_id: Arc<AtomicU32>,
) {
    let mut last: Option<u32> = None;
    while let Ok(b) = conn.read_datagram().await {
        if let Ok(Datagram::CursorPos {
            session_id: sid,
            seq,
            x,
            y,
            shape_id,
            visible,
        }) = Datagram::decode(&b)
        {
            if sid != session_id.load(Ordering::SeqCst) {
                continue;
            }
            if last.is_some_and(|l| !input::seq_newer(seq, l)) {
                continue;
            }
            last = Some(seq);
            // Superseded by the next position if the app is busy.
            let _ = tx.try_send(ClientEvent::CursorPos {
                x,
                y,
                visible,
                shape_id,
            });
        }
    }
}

/// Writes reliable input for the bound session.
///
/// The host keeps one input sequencer per session (not per stream) and
/// closes the connection with "input sequence" on anything but the next
/// number. So `seq` belongs to the session: it continues when the stream is
/// reopened after a write error and restarts only when the session changes.
/// A seq is used up only by a successful write; an event whose write failed
/// was not read by the host (it had stopped the stream, or the connection
/// is gone), so the next event takes its number.
async fn input_writer(
    conn: quinn::Connection,
    mut rx: mpsc::Receiver<InputKind>,
    mut session: watch::Receiver<u32>,
) {
    let mut stream: Option<(u32, quinn::SendStream)> = None; // (session, stream)
    let mut seq = InputSeq::default();
    loop {
        tokio::select! {
            changed = session.changed() => {
                if changed.is_err() { break; }
                let id = *session.borrow();
                // The stream for a new session is opened by the first input,
                // so input sent right after `bind_session` is never lost.
                if stream.as_ref().is_some_and(|(sid, _)| *sid != id) {
                    if let Some((_, mut s)) = stream.take() { let _ = s.finish(); }
                }
            }
            kind = rx.recv() => {
                let Some(kind) = kind else { break };
                let id = *session.borrow();
                // No session: there is nothing to send this to.
                if id == 0 { continue; }
                if stream.as_ref().is_some_and(|(sid, _)| *sid != id) {
                    if let Some((_, mut s)) = stream.take() { let _ = s.finish(); }
                }
                if stream.is_none() {
                    match conn.open_uni().await {
                        Ok(mut s) => {
                            if write_stream_kind(&mut s, StreamKind::Input).await.is_err() {
                                continue;
                            }
                            stream = Some((id, s));
                        }
                        Err(_) => break,
                    }
                }
                let Some((sid, s)) = stream.as_mut() else { continue };
                let ev = InputEvent { session_id: *sid, seq: seq.next(*sid), kind };
                if write_input(s, &ev).await.is_ok() {
                    seq.sent(ev.session_id, ev.seq);
                } else {
                    stream = None;
                }
            }
        }
    }
}

/// The reliable-input sequence of the bound session, kept across input
/// stream reopens.
#[derive(Debug, Default)]
struct InputSeq {
    session_id: u32,
    last_sent: u32,
}

impl InputSeq {
    /// The number for the next event of `session_id`: 1 for a new session,
    /// otherwise one after the last successful write.
    fn next(&self, session_id: u32) -> u32 {
        if session_id == self.session_id {
            self.last_sent.wrapping_add(1)
        } else {
            1
        }
    }

    /// The event `seq` of `session_id` was written.
    fn sent(&mut self, session_id: u32, seq: u32) {
        self.session_id = session_id;
        self.last_sent = seq;
    }
}

/// Capabilities message for a client with the given decoder.
pub fn capabilities(caps: Vec<CodecCap>, display: (u16, u16), refresh_millihz: u32) -> ControlMsg {
    ControlMsg::Capabilities {
        codecs: caps,
        display_width: display.0,
        display_height: display.1,
        display_refresh_millihz: refresh_millihz,
    }
}

impl std::fmt::Debug for Connection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connection")
            .field("host_name", &self.host_name)
            .field("remote", &self.conn.remote_address())
            .field("session_id", &self.session_id())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::InputSeq;

    #[test]
    fn input_seq_is_per_session_and_used_only_by_successful_writes() {
        let mut s = InputSeq::default();
        assert_eq!(s.next(7), 1);
        s.sent(7, 1);
        assert_eq!(s.next(7), 2);
        // The write of 2 failed and the stream is reopened: 2 is still next.
        assert_eq!(s.next(7), 2);
        s.sent(7, 2);
        assert_eq!(s.next(7), 3);
        // A new session restarts at 1, as the host's new sequencer expects.
        assert_eq!(s.next(8), 1);
        s.sent(8, 1);
        assert_eq!(s.next(8), 2);
    }

    #[test]
    fn input_seq_wraps() {
        let mut s = InputSeq::default();
        s.sent(7, u32::MAX);
        assert_eq!(s.next(7), 0);
    }
}
