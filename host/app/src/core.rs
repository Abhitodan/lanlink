//! Runs the LANLink host core (session manager, discovery) on a Tokio
//! runtime in its own thread and forwards everything it reports to the UI.

use crate::AppMsg;
use session_manager::storage::HostStore;
use session_manager::{HostConfig, HostHandle};
use std::sync::mpsc::Sender;
use std::sync::Arc;

pub struct StartConfig {
    pub name: String,
    pub port: u16,
    pub ask_each_session: bool,
}

/// What the UI keeps once the host is up.
pub struct Running {
    pub handle: Arc<HostHandle>,
    pub name: String,
    pub fingerprint: String,
    pub addresses: Vec<std::net::IpAddr>,
    pub port: u16,
    pub discovery: Option<String>,
}

fn send(tx: &Sender<AppMsg>, ctx: &eframe::egui::Context, msg: AppMsg) {
    let _ = tx.send(msg);
    ctx.request_repaint();
}

pub fn spawn(cfg: StartConfig, tx: Sender<AppMsg>, ctx: eframe::egui::Context) {
    let spawned = std::thread::Builder::new()
        .name("lanlink-core".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_multi_thread()
                .worker_threads(4)
                .thread_name("lanlink-rt")
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    send(
                        &tx,
                        &ctx,
                        AppMsg::Failed(format!("Cannot start the network runtime: {e}")),
                    );
                    return;
                }
            };
            rt.block_on(run(cfg, tx, ctx));
        });
    if let Err(e) = spawned {
        tracing::error!(error = %e, "could not start the host thread");
    }
}

async fn run(cfg: StartConfig, tx: Sender<AppMsg>, ctx: eframe::egui::Context) {
    capture_windows::init_thread();
    let store = match HostStore::open(HostStore::default_dir()) {
        Ok(s) => s,
        Err(e) => {
            send(
                &tx,
                &ctx,
                AppMsg::Failed(format!("Cannot open LANLink storage: {e}")),
            );
            return;
        }
    };
    let identity = match store.load_or_create_identity(&cfg.name) {
        Ok(i) => i,
        Err(e) => {
            send(
                &tx,
                &ctx,
                AppMsg::Failed(format!("Cannot load this PC's identity: {e}")),
            );
            return;
        }
    };
    let trust = store.load_trust().unwrap_or_else(|e| {
        tracing::error!(error = %e, "paired-device list unreadable; starting with none");
        Default::default()
    });

    let mut config = HostConfig::new(cfg.name.clone());
    config.bind = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    config.require_session_approval = cfg.ask_each_session;
    let store_dir = store.dir().to_path_buf();
    let persist = move |t: &lanlink_auth::TrustStore| {
        if let Ok(s) = HostStore::open(&store_dir) {
            if let Err(e) = s.save_trust(t) {
                tracing::error!(error = %e, "could not save paired devices");
            }
        }
    };
    let mut handle = match session_manager::start(config, identity, trust, persist).await {
        Ok(h) => h,
        Err(e) => {
            send(
                &tx,
                &ctx,
                AppMsg::Failed(format!(
                    "Cannot listen on UDP port {}: {e}. Is another LANLink host (for example the console host) already running?",
                    cfg.port
                )),
            );
            return;
        }
    };
    // The UI owns the handle (commands, paired devices); this task owns the
    // event stream.
    let (_dummy_tx, dummy_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut events = std::mem::replace(&mut handle.events, dummy_rx);
    let fp_hex = handle.fingerprint.to_hex();
    let port = handle.local_addr.port();
    let (advert, discovery) = match lanlink_discovery::Advertiser::start(&cfg.name, &fp_hex, port) {
        Ok(a) => (Some(a), None),
        Err(e) => (
            None,
            Some(format!(
                "Discovery is unavailable ({e}); phones can still connect by address."
            )),
        ),
    };
    let running = Running {
        fingerprint: handle.fingerprint.short(),
        handle: Arc::new(handle),
        name: cfg.name,
        addresses: lanlink_discovery::local_addresses(),
        port,
        discovery,
    };
    send(&tx, &ctx, AppMsg::Ready(Box::new(running)));
    while let Some(ev) = events.recv().await {
        send(&tx, &ctx, AppMsg::Host(ev));
    }
    drop(advert);
    send(&tx, &ctx, AppMsg::Stopped);
}
