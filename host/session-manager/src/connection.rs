//! One client connection: handshake, pairing, sessions.
//!
//! State machine (see `docs/protocol.md`):
//!
//! ```text
//! TLS up ─▶ Hello/HelloAck ─┬─ NotPaired ──────────────▶ close
//!                           ├─ PairingAvailable ─ pair ─▶ Paired
//!                           └─ Paired ─▶ Capabilities ─▶ SessionStart ⇄ SessionEnd
//! ```
//!
//! Only `PairRequest`/`PairConfirm` are accepted before the device is
//! paired; anything else closes the connection. Input is accepted only for
//! the current session, in sequence, and never in view-only sessions.

use crate::bitrate::{BitrateController, Sample};
use crate::host::{touch_trust, unix_now, ActiveSession, HostEvent, Shared};
use crate::input_worker::{InputMsg, InputWorker};
use crate::media::{self, MediaCmd, MediaEvent, MediaHandle, MediaParams, VideoOut};
use lanlink_auth::pairing::PairingKeys;
use lanlink_auth::{AttemptTicket, Fingerprint, PairedDevice, PairingHost};
use lanlink_protocol::input::{seq_newer, InputReject};
use lanlink_protocol::video::{FLAG_DISCONTINUITY, FLAG_KEYFRAME};
use lanlink_protocol::*;
use lanlink_transport::*;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);
const PAIR_CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(60);
const START_TIMEOUT: Duration = Duration::from_secs(15);
/// A delta frame older than this is dropped instead of sent; the stream
/// restarts from a fresh keyframe. Bounds the latency a stalled network can
/// build up.
const MAX_FRAME_AGE_US: u64 = 250_000;

struct InputGate {
    session_id: u32,
    view_only: bool,
    worker: Option<InputWorker>,
    sequencer: InputSequencer,
    last_pointer_seq: Option<u32>,
}

type Gate = Arc<Mutex<Option<InputGate>>>;

struct Session {
    id: u32,
    media: MediaHandle,
    media_rx: mpsc::Receiver<MediaEvent>,
    video_tx: mpsc::Sender<VideoOut>,
    writer_dropped: Arc<AtomicU64>,
    bitrate: Option<BitrateController>,
    last: (u64, u64, u64, u64, u64, u64),
    min_rtt_us: u32,
    last_lost: u64,
    last_sent: u64,
    client_stats: Option<ClientStats>,
    cursor_seq: u32,
    view_only: bool,
    /// The streamed display in virtual-desktop pixels (caret mapping).
    display: Arc<Mutex<input_windows::Rect>>,
    /// Runs while the phone's keyboard is open (protocol 2+).
    caret: Option<input_windows::caret::CaretWatcher>,
}

/// Connections from devices not in the trust store, at most (the host
/// allows `MAX_CONNECTIONS` in all).
const MAX_UNPAIRED_CONNECTIONS: usize = 4;

/// How often the caret is looked up while the phone's keyboard is open.
const CARET_POLL: Duration = Duration::from_millis(120);

pub(crate) async fn handle(shared: Arc<Shared>, conn: quinn::Connection) {
    let id = conn.stable_id();
    let addr = conn.remote_address();
    let fp = match peer_fingerprint(&conn) {
        Ok(f) => f,
        Err(_) => {
            conn.close(CLOSE_UNAUTHORIZED.into(), b"no certificate");
            return;
        }
    };
    // Unknown devices may use only part of the connection slots, so a
    // flood of strangers cannot lock a paired phone out.
    let trusted = shared
        .trust
        .lock()
        .map(|t| t.contains(&fp))
        .unwrap_or(false);
    let admitted = shared
        .connections
        .lock()
        .map(|mut c| {
            if !trusted {
                let strangers = {
                    let trust = shared.trust.lock();
                    c.values()
                        .filter(|(f, _)| !trust.as_ref().is_ok_and(|t| t.contains(f)))
                        .count()
                };
                if strangers >= MAX_UNPAIRED_CONNECTIONS {
                    return false;
                }
            }
            c.insert(id, (fp, conn.clone()));
            true
        })
        .unwrap_or(false);
    if !admitted {
        conn.close(CLOSE_UNAUTHORIZED.into(), b"busy");
        return;
    }
    tracing::info!(%addr, device = %fp.short(), "connection established");

    let gate: Gate = Arc::new(Mutex::new(None));
    let result = Conn {
        shared: shared.clone(),
        conn: conn.clone(),
        fp,
        gate: gate.clone(),
        device_name: String::new(),
        device_model: String::new(),
        paired: false,
        caps: Vec::new(),
        session: None,
        pairing: None,
        ticket: None,
        pair_done: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        protocol: 1,
    }
    .run()
    .await;
    if let Err(reason) = &result {
        tracing::info!(%addr, %reason, "connection ended");
    }

    if let Ok(mut c) = shared.connections.lock() {
        c.remove(&id);
    }
    if let Ok(mut a) = shared.active.lock() {
        if a.as_ref().is_some_and(|s| s.conn_id == id) {
            *a = None;
        }
    }
    // Release any held input before the task finishes.
    let g = gate.lock().ok().and_then(|mut g| g.take());
    if g.is_some() {
        let _ = tokio::task::spawn_blocking(move || drop(g)).await;
    }
}

struct Conn {
    shared: Arc<Shared>,
    conn: quinn::Connection,
    fp: Fingerprint,
    gate: Gate,
    device_name: String,
    device_model: String,
    paired: bool,
    caps: Vec<CodecCap>,
    session: Option<Session>,
    pairing: Option<PairingKeys>,
    /// The pairing attempt this connection used up (see `PairingWindow`).
    ticket: Option<AttemptTicket>,
    pair_done: Arc<std::sync::atomic::AtomicBool>,
    /// Protocol version agreed in Hello/HelloAck.
    protocol: u16,
}

async fn control_writer(mut send: quinn::SendStream, mut rx: mpsc::Receiver<ControlMsg>) {
    while let Some(msg) = rx.recv().await {
        if write_control(&mut send, &msg).await.is_err() {
            break;
        }
    }
    let _ = send.finish();
}

async fn control_reader(mut recv: quinn::RecvStream, tx: mpsc::Sender<Result<ControlMsg, String>>) {
    loop {
        let r = match read_control(&mut recv).await {
            Ok(Some(m)) => Ok(m),
            Ok(None) => Err("control stream closed".to_string()),
            Err(e) => Err(e.to_string()),
        };
        let stop = r.is_err();
        if tx.send(r).await.is_err() || stop {
            break;
        }
    }
}

impl Conn {
    async fn run(mut self) -> Result<(), String> {
        let (send, mut recv) = timeout(HANDSHAKE_TIMEOUT, self.conn.accept_bi())
            .await
            .map_err(|_| "no control stream")?
            .map_err(|e| e.to_string())?;
        let kind = timeout(HANDSHAKE_TIMEOUT, read_stream_kind(&mut recv))
            .await
            .map_err(|_| "handshake timeout")?
            .map_err(|e| e.to_string())?;
        if kind != StreamKind::Control {
            self.conn
                .close(CLOSE_PROTOCOL.into(), b"expected control stream");
            return Err("first stream was not control".into());
        }
        let hello = timeout(HANDSHAKE_TIMEOUT, read_control(&mut recv))
            .await
            .map_err(|_| "handshake timeout")?
            .map_err(|e| e.to_string())?;
        let (out_tx, out_rx) = mpsc::channel::<ControlMsg>(64);
        tokio::spawn(control_writer(send, out_rx));

        let Some(ControlMsg::Hello {
            protocol_min,
            protocol_max,
            device_name,
            device_model,
            ..
        }) = hello
        else {
            self.conn.close(CLOSE_PROTOCOL.into(), b"expected Hello");
            return Err("first message was not Hello".into());
        };
        // Shown in approval prompts and stored in the trust store.
        self.device_name = wire::display_name(&device_name, 48);
        if self.device_name.is_empty() {
            self.device_name = "Unnamed phone".into();
        }
        self.device_model = wire::display_name(&device_model, 48);
        let Some(version) = control::negotiate_version(protocol_min, protocol_max) else {
            let _ = out_tx
                .send(ControlMsg::Error {
                    code: ErrorCode::ProtocolVersionUnsupported,
                    message:
                        "This LANLink version is not compatible with the PC. Update both apps."
                            .into(),
                })
                .await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            self.conn.close(CLOSE_PROTOCOL.into(), b"version");
            return Err("no common protocol version".into());
        };
        self.protocol = version;

        let known = self
            .shared
            .trust
            .lock()
            .map(|t| t.contains(&self.fp))
            .unwrap_or(false);
        let pairing_open = self
            .shared
            .pairing
            .lock()
            .map(|p| p.as_ref().is_some_and(|w| w.is_open(Instant::now())))
            .unwrap_or(false);
        let auth = if known {
            AuthState::Paired
        } else if pairing_open {
            AuthState::PairingAvailable
        } else {
            AuthState::NotPaired
        };
        let _ = out_tx
            .send(ControlMsg::HelloAck {
                protocol: version,
                host_name: self.shared.config.host_name.clone(),
                app_version: env!("CARGO_PKG_VERSION").into(),
                auth,
            })
            .await;
        match auth {
            AuthState::NotPaired => {
                let _ = out_tx
                    .send(ControlMsg::Error {
                        code: ErrorCode::NotPaired,
                        message: "This phone is not paired. Open pairing on the PC first.".into(),
                    })
                    .await;
                tokio::time::sleep(Duration::from_millis(200)).await;
                self.conn.close(CLOSE_UNAUTHORIZED.into(), b"not paired");
                return Err("unknown device and pairing closed".into());
            }
            AuthState::Paired => {
                self.paired = true;
                touch_trust(&self.shared, &self.fp);
            }
            AuthState::PairingAvailable => {
                // An unpaired connection may exist only to pair, promptly:
                // it must not hold a connection slot indefinitely.
                let conn = self.conn.clone();
                let done = self.pair_done.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(PAIR_CONFIRM_TIMEOUT + APPROVAL_TIMEOUT).await;
                    if !done.load(Ordering::SeqCst) {
                        conn.close(CLOSE_UNAUTHORIZED.into(), b"pairing timed out");
                    }
                });
            }
        }

        let (in_tx, mut in_rx) = mpsc::channel(16);
        tokio::spawn(control_reader(recv, in_tx));
        tokio::spawn(input_streams(self.conn.clone(), self.gate.clone()));
        tokio::spawn(datagrams(self.conn.clone(), self.gate.clone()));

        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let result = loop {
            tokio::select! {
                msg = in_rx.recv() => {
                    match msg {
                        Some(Ok(m)) => {
                            if let Err(e) = self.on_message(m, &out_tx).await {
                                break Err(e);
                            }
                        }
                        Some(Err(e)) => break Err(e),
                        None => break Err("control reader ended".into()),
                    }
                }
                ev = next_media_event(&mut self.session) => {
                    match ev {
                        Some(ev) => self.on_media_event(ev, &out_tx).await,
                        None => {
                            // Pipeline thread ended without a Fatal event.
                            self.end_session("stream ended").await;
                        }
                    }
                }
                _ = tick.tick(), if self.session.is_some() => self.on_tick(&out_tx).await,
                _ = self.conn.closed() => break Err("connection closed".into()),
            }
        };
        self.end_session("connection closed").await;
        if self.pairing.take().is_some() {
            // Started an exchange and left without confirming: a used guess.
            self.record_pair_failure();
        }
        result
    }

    async fn on_message(
        &mut self,
        msg: ControlMsg,
        out: &mpsc::Sender<ControlMsg>,
    ) -> Result<(), String> {
        if !self.paired {
            return match msg {
                ControlMsg::PairRequest { spake_msg } => {
                    self.on_pair_request(&spake_msg, out).await
                }
                ControlMsg::PairConfirm { mac } => self.on_pair_confirm(&mac, out).await,
                other => {
                    self.conn.close(CLOSE_UNAUTHORIZED.into(), b"not paired");
                    Err(format!("{} before pairing", other.name()))
                }
            };
        }
        match msg {
            ControlMsg::Capabilities { codecs, .. } => {
                self.caps = codecs;
                send_display_list(out).await;
            }
            ControlMsg::SessionStart {
                display_id,
                params,
                view_only,
            } => {
                self.start_session(display_id, params, view_only, out)
                    .await?
            }
            ControlMsg::SessionEnd { .. } => self.end_session("ended by the phone").await,
            ControlMsg::DisplaySelect { display_id } => {
                if let Some(s) = &self.session {
                    match resolve_monitor(display_id) {
                        Some(m) => s.media.send(MediaCmd::SelectDisplay(m)),
                        None => {
                            let _ = out
                                .send(ControlMsg::Error {
                                    code: ErrorCode::DisplayUnavailable,
                                    message: "That display is no longer connected.".into(),
                                })
                                .await;
                        }
                    }
                }
            }
            ControlMsg::SetStreamParams { params } => {
                if let Some(s) = &mut self.session {
                    s.bitrate = None; // re-created from the new plan
                    s.media.send(MediaCmd::SetParams(params));
                }
            }
            ControlMsg::KeyframeRequest => {
                if let Some(s) = &self.session {
                    s.media.send(MediaCmd::Keyframe);
                }
            }
            ControlMsg::Ping { id, t_send_us } => {
                let t_recv_us = monotonic_us();
                let _ = out.try_send(ControlMsg::Pong {
                    id,
                    t_send_us,
                    t_recv_us,
                    t_reply_us: monotonic_us(),
                });
            }
            ControlMsg::Pong { .. } => {}
            ControlMsg::ClientStats(cs) => {
                if let Some(s) = &mut self.session {
                    s.client_stats = Some(cs);
                }
            }
            ControlMsg::Error { code, message } => {
                // Debug formatting escapes newlines: no forged log lines.
                tracing::info!(?code, ?message, "client reported an error");
            }
            ControlMsg::TextInput { active } if self.protocol >= 2 => {
                self.set_text_input(active, out);
            }
            other => {
                self.conn
                    .close(CLOSE_PROTOCOL.into(), b"unexpected message");
                return Err(format!("unexpected {}", other.name()));
            }
        }
        Ok(())
    }

    async fn fail_pairing(
        &mut self,
        out: &mpsc::Sender<ControlMsg>,
        reason: ErrorCode,
        msg: &str,
    ) -> Result<(), String> {
        let _ = out
            .send(ControlMsg::PairResult {
                accepted: false,
                mac: [0; 32],
                reason,
            })
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.conn.close(CLOSE_UNAUTHORIZED.into(), msg.as_bytes());
        Err(msg.to_string())
    }

    async fn on_pair_request(
        &mut self,
        spake: &[u8],
        out: &mpsc::Sender<ControlMsg>,
    ) -> Result<(), String> {
        if self.pairing.is_some() || self.ticket.is_some() {
            return self
                .fail_pairing(out, ErrorCode::PairingFailed, "duplicate pair request")
                .await;
        }
        // Count the attempt now, atomically with the check: each started
        // exchange is one code guess, however many connections run at once.
        let attempt = self
            .shared
            .pairing
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().and_then(|w| w.begin_attempt(Instant::now())));
        let Some((ticket, code)) = attempt else {
            return self
                .fail_pairing(out, ErrorCode::PairingClosed, "pairing window closed")
                .await;
        };
        self.ticket = Some(ticket);
        let binding = channel_binding(&self.conn).map_err(|e| e.to_string())?;
        match PairingHost::respond(
            &code,
            self.fp,
            self.shared.identity.fingerprint(),
            spake,
            &binding,
        ) {
            Ok((keys, msg)) => {
                self.pairing = Some(keys);
                let _ = out.send(ControlMsg::PairResponse { spake_msg: msg }).await;
                Ok(())
            }
            Err(_) => {
                // No key was derived, so no code was tested: not a guess.
                if let Ok(mut p) = self.shared.pairing.lock() {
                    if let Some(w) = p.as_mut() {
                        w.refund(ticket);
                    }
                }
                self.ticket = None;
                self.fail_pairing(out, ErrorCode::PairingFailed, "malformed pairing message")
                    .await
            }
        }
    }

    /// A started attempt failed. Once every attempt of the window has been
    /// used and failed, close it.
    fn record_pair_failure(&self) {
        let Some(ticket) = self.ticket else { return };
        if let Ok(mut p) = self.shared.pairing.lock() {
            if let Some(w) = p.as_mut() {
                if w.issued(ticket) && w.exhausted() {
                    *p = None;
                    self.shared.emit(HostEvent::PairingWindowClosed);
                    self.shared.emit(HostEvent::Notice(
                        "Pairing closed after too many wrong codes.".into(),
                    ));
                }
            }
        }
    }

    async fn on_pair_confirm(
        &mut self,
        mac: &[u8; 32],
        out: &mpsc::Sender<ControlMsg>,
    ) -> Result<(), String> {
        let Some(keys) = self.pairing.take() else {
            return self
                .fail_pairing(out, ErrorCode::PairingFailed, "confirm without request")
                .await;
        };
        // The window this exchange was started in must still be the open one:
        // closing pairing on the PC, expiry, or another phone pairing first
        // all end exchanges in flight.
        let current = self.ticket.is_some_and(|t| {
            self.shared
                .pairing
                .lock()
                .map(|p| {
                    p.as_ref()
                        .is_some_and(|w| w.ticket_valid(t, Instant::now()))
                })
                .unwrap_or(false)
        });
        if !current {
            return self
                .fail_pairing(out, ErrorCode::PairingClosed, "pairing window closed")
                .await;
        }
        if keys.verify_client(mac).is_err() {
            self.record_pair_failure();
            return self
                .fail_pairing(out, ErrorCode::PairingFailed, "wrong pairing code")
                .await;
        }
        // The phone knows the code. Now the PC user decides.
        let (tx, rx) = oneshot::channel();
        self.shared.emit(HostEvent::PairingApproval {
            device_name: self.device_name.clone(),
            device_model: self.device_model.clone(),
            fingerprint: self.fp,
            confirmation_code: keys.confirmation_code.clone(),
            respond: tx,
        });
        let approved = self.await_approval(rx).await;
        if !approved {
            self.record_pair_failure();
            return self
                .fail_pairing(
                    out,
                    ErrorCode::PairingDenied,
                    "pairing not approved on the PC",
                )
                .await;
        }
        let now = unix_now();
        if let Ok(mut t) = self.shared.trust.lock() {
            t.insert(PairedDevice {
                fingerprint: self.fp,
                name: self.device_name.clone(),
                model: self.device_model.clone(),
                paired_at: now,
                last_seen: now,
            });
        }
        self.shared.persist_trust();
        if let Ok(mut p) = self.shared.pairing.lock() {
            // Single use. Leave a newer window the user opened meanwhile alone.
            if p.as_ref()
                .is_some_and(|w| self.ticket.is_some_and(|t| w.issued(t)))
            {
                *p = None;
            }
        }
        self.shared.emit(HostEvent::PairingWindowClosed);
        self.shared.emit(HostEvent::Paired {
            device_name: self.device_name.clone(),
            fingerprint: self.fp,
        });
        self.paired = true;
        self.pair_done.store(true, Ordering::SeqCst);
        let _ = out
            .send(ControlMsg::PairResult {
                accepted: true,
                mac: keys.host_mac,
                reason: ErrorCode::Normal,
            })
            .await;
        Ok(())
    }

    async fn start_session(
        &mut self,
        display_id: u32,
        params: StreamParams,
        view_only: bool,
        out: &mpsc::Sender<ControlMsg>,
    ) -> Result<(), String> {
        if self.session.is_some() {
            self.end_session("restarted by the phone").await;
        }
        let my_id = self.conn.stable_id();
        // One controlling phone at a time. The same phone reconnecting after
        // a network drop takes over its own stale session.
        enum Slot {
            Busy,
            Takeover(usize),
            Free,
        }
        let slot = {
            let mut a = self.shared.active.lock().map_err(|_| "lock")?;
            let slot = match a.as_ref() {
                Some(s) if s.conn_id != my_id && !s.fingerprint.ct_eq(&self.fp) => Slot::Busy,
                Some(s) if s.conn_id != my_id => Slot::Takeover(s.conn_id),
                _ => Slot::Free,
            };
            if !matches!(slot, Slot::Busy) {
                *a = Some(ActiveSession {
                    conn_id: my_id,
                    fingerprint: self.fp,
                });
            }
            slot
        };
        let takeover = match slot {
            Slot::Busy => {
                let _ = out
                    .send(ControlMsg::Error {
                        code: ErrorCode::HostBusy,
                        message: "Another phone is controlling this PC.".into(),
                    })
                    .await;
                return Ok(());
            }
            Slot::Takeover(old) => Some(old),
            Slot::Free => None,
        };
        if let Some(old) = takeover {
            if let Ok(conns) = self.shared.connections.lock() {
                if let Some((_, c)) = conns.get(&old) {
                    c.close(CLOSE_NORMAL.into(), b"replaced by a new connection");
                }
            }
        }

        if self
            .shared
            .session_approval
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            let (tx, rx) = oneshot::channel();
            self.shared.emit(HostEvent::SessionApproval {
                device_name: self.device_name.clone(),
                fingerprint: self.fp,
                view_only,
                respond: tx,
            });
            if !self.await_approval(rx).await {
                self.release_active();
                let _ = out
                    .send(ControlMsg::Error {
                        code: ErrorCode::SessionDenied,
                        message: "The session was not approved on the PC.".into(),
                    })
                    .await;
                return Ok(());
            }
        }

        let Some(monitor) = resolve_monitor(display_id) else {
            self.release_active();
            let _ = out
                .send(ControlMsg::Error {
                    code: ErrorCode::DisplayUnavailable,
                    message: "That display is not available.".into(),
                })
                .await;
            return Ok(());
        };

        let mut video = self.conn.open_uni().await.map_err(|e| e.to_string())?;
        write_stream_kind(&mut video, StreamKind::Video)
            .await
            .map_err(|e| e.to_string())?;
        let (video_tx, video_rx) = mpsc::channel(media::VIDEO_QUEUE);
        let (ev_tx, mut ev_rx) = mpsc::channel(64);
        let media = media::spawn(
            MediaParams {
                monitor: monitor.clone(),
                params,
                caps: self.caps.clone(),
                backend: self.shared.config.backend,
                encoder: self.shared.config.encoder,
            },
            video_tx.clone(),
            ev_tx,
        )
        .map_err(|e| e.to_string())?;

        // Wait until the pipeline is really producing (or failed), so the
        // phone gets either SessionStarted or a specific error.
        let started = loop {
            match timeout(START_TIMEOUT, ev_rx.recv()).await {
                Ok(Some(MediaEvent::Started {
                    encoder,
                    hardware,
                    capture,
                    plan,
                    display,
                    monitor,
                })) => break Ok((encoder, hardware, capture, plan, display, monitor)),
                Ok(Some(MediaEvent::Fatal(code, msg))) => break Err((code, msg)),
                Ok(Some(_)) => continue, // notices while starting
                Ok(None) => break Err((ErrorCode::Internal, "stream did not start".into())),
                Err(_) => {
                    break Err((
                        ErrorCode::CaptureFailed,
                        "screen capture did not start in time".into(),
                    ))
                }
            }
        };
        let (encoder_name, hardware, capture, plan, display, monitor) = match started {
            Ok(v) => v,
            Err((code, message)) => {
                drop(media);
                self.release_active();
                tracing::warn!(?code, %message, "session failed to start");
                let _ = out.send(ControlMsg::Error { code, message }).await;
                return Ok(());
            }
        };

        let session_id = loop {
            let v = rand_u32();
            if v != 0 {
                break v;
            }
        };
        let writer_dropped = Arc::new(AtomicU64::new(0));
        tokio::spawn(video_writer(
            video,
            video_rx,
            media.requester(),
            writer_dropped.clone(),
        ));

        if !view_only {
            let worker = InputWorker::spawn(
                self.shared.config.input_sink.clone(),
                display,
                input_windows::virtual_desktop(),
                out.clone(),
            )
            .map_err(|e| e.to_string())?;
            if let Ok(mut g) = self.gate.lock() {
                *g = Some(InputGate {
                    session_id,
                    view_only,
                    worker: Some(worker),
                    sequencer: InputSequencer::new(session_id),
                    last_pointer_seq: None,
                });
            }
        }

        let _ = out
            .send(ControlMsg::SessionStarted {
                session_id,
                display_id: monitor.id,
                view_only,
                encoder_name: format!(
                    "{encoder_name}{}",
                    if hardware { "" } else { " (software)" }
                ),
                capture_backend: capture.clone(),
            })
            .await;
        self.shared.emit(HostEvent::SessionStarted {
            device_name: self.device_name.clone(),
            fingerprint: self.fp,
            display: monitor.label.clone(),
            encoder: encoder_name,
            hardware,
            capture,
            width: plan.width,
            height: plan.height,
            fps: plan.fps,
            view_only,
        });
        tracing::info!(session_id, view_only, "session started");
        self.session = Some(Session {
            id: session_id,
            media,
            media_rx: ev_rx,
            video_tx,
            writer_dropped,
            bitrate: Some(BitrateController::new(
                plan.bitrate_bps,
                plan.min_bitrate_bps,
            )),
            last: (0, 0, 0, 0, 0, 0),
            min_rtt_us: u32::MAX,
            last_lost: 0,
            last_sent: 0,
            client_stats: None,
            cursor_seq: 0,
            view_only,
            display: Arc::new(Mutex::new(display)),
            caret: None,
        });
        Ok(())
    }

    /// Start or stop reporting the text caret to the phone.
    fn set_text_input(&mut self, active: bool, out: &mpsc::Sender<ControlMsg>) {
        let Some(s) = &mut self.session else { return };
        if !active || s.view_only {
            if s.caret.take().is_some() {
                tracing::debug!("caret tracking stopped");
            }
            return;
        }
        if s.caret.is_some() {
            return;
        }
        let display = s.display.clone();
        let out = out.clone();
        let hidden = ControlMsg::Caret {
            visible: false,
            x: 0,
            y: 0,
            w: 0,
            h: 0,
        };
        let watcher = input_windows::caret::CaretWatcher::spawn(CARET_POLL, move |caret| {
            let d = display.lock().map(|d| *d).ok();
            let msg = caret
                .zip(d)
                .and_then(|(c, d)| input_windows::caret::caret_on_display(c.rect, d))
                .map(|(x, y, w, h)| ControlMsg::Caret {
                    visible: true,
                    x,
                    y,
                    w,
                    h,
                })
                .unwrap_or_else(|| hidden.clone());
            // Superseded by the next one if the queue is momentarily full.
            let _ = out.try_send(msg);
        });
        match watcher {
            Ok(w) => {
                s.caret = Some(w);
                tracing::debug!("caret tracking started");
            }
            Err(e) => tracing::warn!(error = %e, "could not start caret tracking"),
        }
    }

    /// Wait for the PC user's answer. Gives up (and so withdraws the prompt,
    /// whose sender sees the receiver dropped) on timeout or as soon as the
    /// phone disconnects, so a vanished phone does not leave a question on
    /// screen or hold the session slot.
    async fn await_approval(&self, rx: oneshot::Receiver<bool>) -> bool {
        tokio::select! {
            r = timeout(APPROVAL_TIMEOUT, rx) => matches!(r, Ok(Ok(true))),
            _ = self.conn.closed() => false,
        }
    }

    fn release_active(&self) {
        let my_id = self.conn.stable_id();
        if let Ok(mut a) = self.shared.active.lock() {
            if a.as_ref().is_some_and(|s| s.conn_id == my_id) {
                *a = None;
            }
        }
    }

    async fn end_session(&mut self, reason: &str) {
        let g = self.gate.lock().ok().and_then(|mut g| g.take());
        let Some(session) = self.session.take() else {
            if g.is_some() {
                let _ = tokio::task::spawn_blocking(move || drop(g)).await;
            }
            return;
        };
        // Joining threads must not block the runtime.
        let _ = tokio::task::spawn_blocking(move || {
            drop(g); // releases held input
            drop(session); // stops the pipeline
        })
        .await;
        self.release_active();
        self.shared.emit(HostEvent::SessionEnded {
            device_name: self.device_name.clone(),
            reason: reason.to_string(),
        });
        tracing::info!(%reason, "session ended");
    }

    async fn on_media_event(&mut self, ev: MediaEvent, out: &mpsc::Sender<ControlMsg>) {
        let Some(s) = &mut self.session else { return };
        match ev {
            MediaEvent::CursorPos {
                x,
                y,
                visible,
                shape_id,
            } => {
                s.cursor_seq = s.cursor_seq.wrapping_add(1);
                let d = Datagram::CursorPos {
                    session_id: s.id,
                    seq: s.cursor_seq,
                    x,
                    y,
                    shape_id,
                    visible,
                };
                let _ = self.conn.send_datagram(d.encode().into());
            }
            MediaEvent::CursorShape { shape_id, shape } => {
                let _ = out
                    .send(ControlMsg::CursorShape {
                        shape_id,
                        width: shape.width,
                        height: shape.height,
                        hot_x: shape.hot_x,
                        hot_y: shape.hot_y,
                        rgba: shape.rgba,
                    })
                    .await;
            }
            MediaEvent::Notice(code, message) => {
                self.shared.emit(HostEvent::Notice(message.clone()));
                let _ = out.send(ControlMsg::Notice { code, message }).await;
            }
            MediaEvent::Started { plan, display, .. } => {
                // Reconfigured after a display switch, parameter change or
                // recovery.
                if let Ok(mut d) = s.display.lock() {
                    *d = display;
                }
                s.bitrate = Some(BitrateController::new(
                    plan.bitrate_bps,
                    plan.min_bitrate_bps,
                ));
                if let Ok(g) = self.gate.lock() {
                    if let Some(w) = g.as_ref().and_then(|g| g.worker.as_ref()) {
                        let _ = w.send(InputMsg::Geometry {
                            display,
                            desktop: input_windows::virtual_desktop(),
                        });
                    }
                }
            }
            MediaEvent::Fatal(code, message) => {
                let _ = out
                    .send(ControlMsg::Error {
                        code,
                        message: message.clone(),
                    })
                    .await;
                self.end_session(&message).await;
            }
        }
    }

    async fn on_tick(&mut self, out: &mpsc::Sender<ControlMsg>) {
        let Some(s) = &mut self.session else { return };
        let st = &s.media.stats;
        let captured = st.captured.load(Ordering::Relaxed);
        let encoded = st.encoded.load(Ordering::Relaxed);
        let bytes = st.bytes.load(Ordering::Relaxed);
        let dbe = st.dropped_before_encode.load(Ordering::Relaxed);
        let dbs = st.dropped_before_send.load(Ordering::Relaxed)
            + s.writer_dropped.load(Ordering::Relaxed);
        let (p50, p95) = st.take_encode_percentiles();
        let (lc, le, lb, ldbe, ldbs, _) = s.last;
        s.last = (captured, encoded, bytes, dbe, dbs, 0);
        let path = self.conn.stats().path;
        let rtt_us = path.rtt.as_micros().min(u32::MAX as u128) as u32;
        if rtt_us > 0 {
            s.min_rtt_us = s.min_rtt_us.min(rtt_us);
        }
        let stats = StreamStats {
            captured_fps_x10: ((captured - lc) * 10).min(u16::MAX as u64) as u16,
            encoded_fps_x10: ((encoded - le) * 10).min(u16::MAX as u64) as u16,
            bitrate_kbps: ((bytes - lb) * 8 / 1000) as u32,
            target_bitrate_kbps: st.target_bitrate.load(Ordering::Relaxed) / 1000,
            encode_us_p50: p50,
            encode_us_p95: p95,
            dropped_before_encode: (dbe - ldbe) as u32,
            dropped_before_send: (dbs - ldbs) as u32,
            send_queue_depth: (media::VIDEO_QUEUE - s.video_tx.capacity()) as u8,
            rtt_us,
            lost_packets: path.lost_packets,
            congestion_window: path.cwnd.min(u32::MAX as u64) as u32,
        };
        if let Some(ctl) = &mut s.bitrate {
            let sample = Sample {
                dropped_before_send: stats.dropped_before_send,
                rtt_us,
                min_rtt_us: if s.min_rtt_us == u32::MAX {
                    0
                } else {
                    s.min_rtt_us
                },
                lost_packets: path.lost_packets - s.last_lost,
                sent_packets: path.sent_packets - s.last_sent,
            };
            if let Some(bps) = ctl.update(sample) {
                s.media.send(MediaCmd::SetBitrate(bps));
                if bps < st.target_bitrate.load(Ordering::Relaxed) {
                    let _ = out.try_send(ControlMsg::Notice {
                        code: NoticeCode::BitrateReduced,
                        message: format!("Network busy: bitrate lowered to {} kbps", bps / 1000),
                    });
                }
            }
        }
        s.last_lost = path.lost_packets;
        s.last_sent = path.sent_packets;
        let _ = out.try_send(ControlMsg::StreamStats(stats));
        self.shared.emit(HostEvent::Stats {
            stream: stats,
            client: s.client_stats,
        });
    }
}

async fn next_media_event(session: &mut Option<Session>) -> Option<MediaEvent> {
    match session {
        Some(s) => s.media_rx.recv().await,
        None => std::future::pending().await,
    }
}

fn resolve_monitor(display_id: u32) -> Option<capture_windows::MonitorInfo> {
    let list = capture_windows::enumerate_monitors().ok()?;
    if display_id == DISPLAY_PRIMARY {
        list.into_iter().find(|m| m.primary)
    } else {
        list.into_iter().find(|m| m.id == display_id)
    }
}

async fn send_display_list(out: &mpsc::Sender<ControlMsg>) {
    let displays = capture_windows::enumerate_monitors()
        .unwrap_or_default()
        .into_iter()
        .map(|m| DisplayInfo {
            id: m.id,
            name: m.label,
            width: m.width,
            height: m.height,
            x: m.x,
            y: m.y,
            primary: m.primary,
            refresh_millihz: m.refresh_millihz,
            scale_percent: m.scale_percent,
            rotation: m.rotation,
            hdr: m.hdr,
        })
        .collect();
    let _ = out.send(ControlMsg::DisplayList { displays }).await;
}

fn rand_u32() -> u32 {
    use std::hash::{BuildHasher, Hasher};
    // Session ids only need to be unpredictable enough not to collide with
    // a previous session on the same connection; authentication is done by
    // TLS. RandomState is seeded from the OS RNG.
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u64(monotonic_us());
    h.finish() as u32
}

async fn video_writer(
    mut send: quinn::SendStream,
    mut rx: mpsc::Receiver<VideoOut>,
    keyframes: std::sync::mpsc::Sender<MediaCmd>,
    dropped: Arc<AtomicU64>,
) {
    let mut awaiting_key = false;
    while let Some(item) = rx.recv().await {
        let r = match item {
            VideoOut::Config(cfg) => {
                awaiting_key = false;
                write_video_config(&mut send, &cfg).await
            }
            VideoOut::Frame(f) => {
                let age = monotonic_us().saturating_sub(f.capture_us);
                if !f.keyframe && (awaiting_key || age > MAX_FRAME_AGE_US) {
                    if !awaiting_key {
                        awaiting_key = true;
                        let _ = keyframes.send(MediaCmd::Keyframe);
                    }
                    dropped.fetch_add(1, Ordering::Relaxed);
                    continue;
                }
                let mut flags = 0;
                if f.keyframe {
                    flags |= FLAG_KEYFRAME;
                    if f.discontinuity || awaiting_key {
                        flags |= FLAG_DISCONTINUITY;
                    }
                    awaiting_key = false;
                }
                let header = FrameHeader {
                    seq: f.seq,
                    config_id: f.config_id,
                    flags,
                    capture_us: f.capture_us,
                    encode_us: f.encode_us,
                    send_us: monotonic_us(),
                    payload_len: f.data.len() as u32,
                };
                write_video_frame(&mut send, &header, &f.data).await
            }
        };
        if r.is_err() {
            break;
        }
    }
    let _ = send.finish();
}

/// Reliable input streams (one per session, opened by the client).
async fn input_streams(conn: quinn::Connection, gate: Gate) {
    while let Ok(mut recv) = conn.accept_uni().await {
        match timeout(HANDSHAKE_TIMEOUT, read_stream_kind(&mut recv)).await {
            Ok(Ok(StreamKind::Input)) => {}
            _ => {
                conn.close(CLOSE_PROTOCOL.into(), b"unexpected stream");
                return;
            }
        }
        let conn = conn.clone();
        let gate = gate.clone();
        tokio::spawn(async move {
            loop {
                let ev = match read_input(&mut recv).await {
                    Ok(Some(ev)) => ev,
                    Ok(None) => return,
                    Err(e) => {
                        tracing::warn!(error = %e, "malformed input stream");
                        conn.close(CLOSE_PROTOCOL.into(), b"malformed input");
                        return;
                    }
                };
                let t_recv_us = monotonic_us();
                let Ok(mut g) = gate.lock() else { return };
                let Some(g) = g.as_mut() else { continue };
                if g.view_only {
                    continue;
                }
                match g.sequencer.accept(ev.session_id, ev.seq) {
                    Ok(()) => {}
                    Err(InputReject::WrongSession) => continue, // stale stream
                    Err(e) => {
                        tracing::warn!(?e, "input sequence violation");
                        conn.close(CLOSE_PROTOCOL.into(), b"input sequence");
                        return;
                    }
                }
                if let Some(w) = &g.worker {
                    let release = matches!(
                        ev.kind,
                        InputKind::Key { down: false, .. }
                            | InputKind::Button { down: false, .. }
                            | InputKind::ReleaseAll
                    );
                    if !w.send(InputMsg::Reliable {
                        seq: ev.seq,
                        kind: ev.kind,
                        t_recv_us,
                    }) {
                        tracing::warn!("input queue full; client is flooding");
                        if release {
                            // A lost release would leave a key or button held
                            // on the PC. End the connection instead: dropping
                            // the input worker releases everything it holds.
                            conn.close(CLOSE_PROTOCOL.into(), b"input flood");
                            return;
                        }
                    }
                }
            }
        });
    }
}

/// Unreliable pointer positions.
async fn datagrams(conn: quinn::Connection, gate: Gate) {
    while let Ok(b) = conn.read_datagram().await {
        let Ok(d) = Datagram::decode(&b) else {
            continue;
        };
        let Ok(mut g) = gate.lock() else { return };
        let Some(g) = g.as_mut() else { continue };
        let (session_id, seq, msg) = match d {
            Datagram::PointerAbs {
                session_id,
                seq,
                x,
                y,
            } => (session_id, seq, InputMsg::PointerAbs(x, y)),
            Datagram::PointerRel {
                session_id,
                seq,
                dx,
                dy,
            } => (session_id, seq, InputMsg::PointerRel(dx, dy)),
            Datagram::CursorPos { .. } => continue,
        };
        if g.view_only || session_id != g.session_id {
            continue;
        }
        // Absolute positions supersede older ones; relative motion must
        // not be dropped just for arriving out of order, only duplicates.
        let newer = g.last_pointer_seq.is_none_or(|last| seq_newer(seq, last));
        if matches!(msg, InputMsg::PointerAbs(..)) && !newer {
            continue;
        }
        if newer {
            g.last_pointer_seq = Some(seq);
        }
        if let Some(w) = &g.worker {
            let _ = w.send(msg);
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        tracing::debug!(session = self.id, "session resources released");
    }
}
