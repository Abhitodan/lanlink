//! The host: accepts connections, owns shared state, and talks to the UI.

use crate::input_worker::SinkFactory;
use crate::netpolicy::is_local_address;
use capture_windows::BackendPreference;
use encoder::EncoderPreference;
use lanlink_auth::{Fingerprint, Identity, PairingUri, PairingWindow, TrustStore};
use lanlink_protocol::{ClientStats, StreamStats};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;
use tokio::sync::{mpsc, oneshot};

#[derive(Debug, Error)]
pub enum HostError {
    #[error("{0}")]
    Transport(#[from] lanlink_transport::TransportError),
}

pub struct HostConfig {
    pub bind: SocketAddr,
    pub host_name: String,
    /// Ask the PC user before every session, not only at pairing.
    pub require_session_approval: bool,
    /// Accept handshakes from non-private addresses. Off by default.
    pub allow_non_local: bool,
    pub backend: BackendPreference,
    pub encoder: EncoderPreference,
    /// Creates the input sink for each session. `SendInputSink` in the real
    /// app; tests substitute a recorder.
    pub input_sink: SinkFactory,
    /// Addresses advertised in the pairing QR code. Empty = detect.
    pub advertise_addrs: Vec<IpAddr>,
}

impl HostConfig {
    pub fn new(host_name: impl Into<String>) -> Self {
        HostConfig {
            bind: SocketAddr::from(([0, 0, 0, 0], lanlink_protocol::limits::DEFAULT_PORT)),
            host_name: host_name.into(),
            require_session_approval: false,
            allow_non_local: false,
            backend: BackendPreference::Auto,
            encoder: EncoderPreference::Auto,
            input_sink: Arc::new(|| Box::new(input_windows::SendInputSink)),
            advertise_addrs: Vec::new(),
        }
    }
}

/// Notifications for the host UI.
pub enum HostEvent {
    Listening {
        addr: SocketAddr,
        fingerprint: Fingerprint,
    },
    PairingWindowOpened {
        uri: String,
        code: String,
        expires_in: Duration,
    },
    PairingWindowClosed,
    /// The phone proved it knows the code. Show `confirmation_code` and ask
    /// the user to compare it with the phone and approve.
    PairingApproval {
        device_name: String,
        device_model: String,
        fingerprint: Fingerprint,
        confirmation_code: String,
        respond: oneshot::Sender<bool>,
    },
    Paired {
        device_name: String,
        fingerprint: Fingerprint,
    },
    SessionApproval {
        device_name: String,
        fingerprint: Fingerprint,
        view_only: bool,
        respond: oneshot::Sender<bool>,
    },
    SessionStarted {
        device_name: String,
        fingerprint: Fingerprint,
        display: String,
        encoder: String,
        hardware: bool,
        capture: String,
        width: u32,
        height: u32,
        fps: u32,
        view_only: bool,
    },
    SessionEnded {
        device_name: String,
        reason: String,
    },
    Stats {
        stream: StreamStats,
        client: Option<ClientStats>,
    },
    Notice(String),
    Rejected {
        addr: SocketAddr,
        reason: String,
    },
}

pub enum HostCommand {
    OpenPairing,
    ClosePairing,
    /// Emergency stop: end the active session immediately.
    StopSession,
    Revoke(Fingerprint),
    /// Change "ask before every session" without restarting the host.
    SetSessionApproval(bool),
    Shutdown,
}

pub(crate) struct ActiveSession {
    pub conn_id: usize,
    pub fingerprint: Fingerprint,
}

pub(crate) struct Shared {
    pub config: HostConfig,
    pub identity: Identity,
    pub trust: Mutex<TrustStore>,
    persist: Box<dyn Fn(&TrustStore) + Send + Sync>,
    pub pairing: Mutex<Option<PairingWindow>>,
    pub events: mpsc::UnboundedSender<HostEvent>,
    pub active: Mutex<Option<ActiveSession>>,
    pub connections: Mutex<HashMap<usize, (Fingerprint, quinn::Connection)>>,
    pub port: u16,
    /// Live value of "ask before every session" (starts from the config).
    pub session_approval: AtomicBool,
    /// Connections in progress or established (see [`ConnectionSlot`]).
    slots: AtomicUsize,
}

/// A reserved connection slot, held from the moment a handshake is accepted
/// until its task ends. Counting only established connections would let a
/// burst of simultaneous handshakes all pass the limit.
struct ConnectionSlot(Arc<Shared>);

impl ConnectionSlot {
    fn reserve(shared: &Arc<Shared>) -> Option<Self> {
        let mut n = shared.slots.load(Ordering::Acquire);
        loop {
            if n >= MAX_CONNECTIONS {
                return None;
            }
            match shared
                .slots
                .compare_exchange_weak(n, n + 1, Ordering::AcqRel, Ordering::Acquire)
            {
                Ok(_) => return Some(ConnectionSlot(shared.clone())),
                Err(actual) => n = actual,
            }
        }
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.slots.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Shared {
    pub fn emit(&self, e: HostEvent) {
        let _ = self.events.send(e);
    }

    pub fn persist_trust(&self) {
        if let Ok(t) = self.trust.lock() {
            (self.persist)(&t);
        }
    }

    pub fn close_connections_of(&self, fp: &Fingerprint, reason: &str) {
        if let Ok(conns) = self.connections.lock() {
            for (f, c) in conns.values() {
                if f.ct_eq(fp) {
                    c.close(
                        lanlink_transport::CLOSE_UNAUTHORIZED.into(),
                        reason.as_bytes(),
                    );
                }
            }
        }
    }
}

pub struct HostHandle {
    pub commands: mpsc::UnboundedSender<HostCommand>,
    pub events: mpsc::UnboundedReceiver<HostEvent>,
    pub local_addr: SocketAddr,
    pub fingerprint: Fingerprint,
    shared: Arc<Shared>,
}

impl HostHandle {
    pub fn paired_devices(&self) -> Vec<lanlink_auth::PairedDevice> {
        self.shared
            .trust
            .lock()
            .map(|t| t.devices().to_vec())
            .unwrap_or_default()
    }
}

/// Maximum simultaneous connections (authenticated or not).
const MAX_CONNECTIONS: usize = 8;

/// Start listening. Must be called inside a Tokio runtime.
pub async fn start(
    config: HostConfig,
    identity: Identity,
    trust: TrustStore,
    persist: impl Fn(&TrustStore) + Send + Sync + 'static,
) -> Result<HostHandle, HostError> {
    let endpoint = lanlink_transport::server_endpoint(&identity, config.bind)?;
    let local_addr = endpoint
        .local_addr()
        .map_err(|e| lanlink_transport::TransportError::Bind(config.bind, e))?;
    let (ev_tx, ev_rx) = mpsc::unbounded_channel();
    let (cmd_tx, cmd_rx) = mpsc::unbounded_channel();
    let config_session_approval = config.require_session_approval;
    let fingerprint = identity.fingerprint();
    let shared = Arc::new(Shared {
        config,
        identity,
        trust: Mutex::new(trust),
        persist: Box::new(persist),
        pairing: Mutex::new(None),
        events: ev_tx,
        active: Mutex::new(None),
        connections: Mutex::new(HashMap::new()),
        port: local_addr.port(),
        session_approval: AtomicBool::new(config_session_approval),
        slots: AtomicUsize::new(0),
    });
    shared.emit(HostEvent::Listening {
        addr: local_addr,
        fingerprint,
    });
    tracing::info!(%local_addr, fingerprint = %fingerprint.short(), "LANLink host listening");

    tokio::spawn(accept_loop(shared.clone(), endpoint.clone()));
    tokio::spawn(command_loop(shared.clone(), endpoint, cmd_rx));

    Ok(HostHandle {
        commands: cmd_tx,
        events: ev_rx,
        local_addr,
        fingerprint,
        shared,
    })
}

async fn accept_loop(shared: Arc<Shared>, endpoint: quinn::Endpoint) {
    while let Some(incoming) = endpoint.accept().await {
        let addr = incoming.remote_address();
        if !shared.config.allow_non_local && !is_local_address(addr.ip()) {
            tracing::warn!(%addr, "refusing connection from a non-local address");
            shared.emit(HostEvent::Rejected {
                addr,
                reason: "not a local-network address".into(),
            });
            incoming.refuse();
            continue;
        }
        let Some(slot) = ConnectionSlot::reserve(&shared) else {
            tracing::warn!(%addr, "connection limit reached; refusing");
            incoming.refuse();
            continue;
        };
        let shared = shared.clone();
        tokio::spawn(async move {
            let _slot = slot;
            match incoming.await {
                Ok(conn) => crate::connection::handle(shared, conn).await,
                Err(e) => tracing::info!(%addr, error = %e, "handshake failed"),
            }
        });
    }
}

fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn unix_now() -> u64 {
    now_unix()
}

async fn command_loop(
    shared: Arc<Shared>,
    endpoint: quinn::Endpoint,
    mut rx: mpsc::UnboundedReceiver<HostCommand>,
) {
    while let Some(cmd) = rx.recv().await {
        match cmd {
            HostCommand::OpenPairing => match PairingWindow::open(Instant::now()) {
                Ok(window) => {
                    let code = window.code().to_string();
                    let expires = window.expires();
                    if let Ok(mut p) = shared.pairing.lock() {
                        *p = Some(window);
                    }
                    let mut addrs = shared.config.advertise_addrs.clone();
                    if addrs.is_empty() {
                        addrs = lanlink_discovery::local_addresses();
                    }
                    let uri = PairingUri {
                        addresses: addrs,
                        port: shared.port,
                        fingerprint: shared.identity.fingerprint(),
                        code: code.clone(),
                        name: shared.config.host_name.clone(),
                    }
                    .encode();
                    shared.emit(HostEvent::PairingWindowOpened {
                        uri,
                        code,
                        expires_in: expires.saturating_duration_since(Instant::now()),
                    });
                    let s = shared.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep_until(expires.into()).await;
                        let closed = s
                            .pairing
                            .lock()
                            .map(|mut p| {
                                let expired = p.as_ref().is_some_and(|w| w.expires() == expires);
                                if expired {
                                    *p = None;
                                }
                                expired
                            })
                            .unwrap_or(false);
                        if closed {
                            s.emit(HostEvent::PairingWindowClosed);
                        }
                    });
                }
                Err(e) => shared.emit(HostEvent::Notice(format!("Could not open pairing: {e}"))),
            },
            HostCommand::ClosePairing => {
                if let Ok(mut p) = shared.pairing.lock() {
                    *p = None;
                }
                shared.emit(HostEvent::PairingWindowClosed);
            }
            HostCommand::StopSession => {
                let target = shared
                    .active
                    .lock()
                    .ok()
                    .and_then(|a| a.as_ref().map(|a| a.conn_id));
                if let Some(id) = target {
                    if let Ok(conns) = shared.connections.lock() {
                        if let Some((_, c)) = conns.get(&id) {
                            c.close(
                                lanlink_transport::CLOSE_NORMAL.into(),
                                b"session stopped on the PC",
                            );
                        }
                    }
                }
            }
            HostCommand::Revoke(fp) => {
                let removed = shared
                    .trust
                    .lock()
                    .map(|mut t| t.revoke(&fp))
                    .unwrap_or(false);
                if removed {
                    shared.persist_trust();
                }
                shared.close_connections_of(&fp, "device revoked");
            }
            HostCommand::SetSessionApproval(on) => {
                shared.session_approval.store(on, Ordering::Relaxed);
            }
            HostCommand::Shutdown => {
                endpoint.close(
                    lanlink_transport::CLOSE_NORMAL.into(),
                    b"host shutting down",
                );
                break;
            }
        }
    }
}

pub(crate) fn touch_trust(shared: &Shared, fp: &Fingerprint) {
    if let Ok(mut t) = shared.trust.lock() {
        t.touch(fp, now_unix());
    }
    shared.persist_trust();
}
