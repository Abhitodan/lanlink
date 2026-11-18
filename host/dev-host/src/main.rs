//! `lanlink-host-console`: run the LANLink host in a terminal.
//!
//! The tray application is the product UI; this console host exposes the
//! same core for development, troubleshooting and headless testing.
//!
//! Commands (type and press Enter):
//!   p        open pairing (shows a QR code and an 8-digit code)
//!   s        emergency stop: end the active session now
//!   l        list paired phones
//!   r <n>    revoke paired phone number n
//!   y / n    answer the pending approval prompt
//!   q        quit
//!
//! Options:
//!   --port <n>            UDP port (default 47990)
//!   --approve-sessions    ask before every session, not only at pairing
//!   --name <text>         name shown on phones (default: computer name)

use lanlink_auth::Fingerprint;
use qrcode::render::unicode::Dense1x2;
use qrcode::QrCode;
use session_manager::storage::HostStore;
use session_manager::{HostCommand, HostConfig, HostEvent};
use std::io::BufRead;
use tokio::sync::{mpsc, oneshot};

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .map(|n| {
            let lower = n.to_lowercase();
            let mut c = lower.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => n,
            }
        })
        .unwrap_or_else(|_| "Windows PC".into())
}

fn print_qr(uri: &str) {
    match QrCode::new(uri.as_bytes()) {
        Ok(code) => {
            let image = code
                .render::<Dense1x2>()
                .dark_color(Dense1x2::Light)
                .light_color(Dense1x2::Dark)
                .quiet_zone(true)
                .build();
            println!("{image}");
        }
        Err(e) => println!("(could not draw QR code: {e})"),
    }
}

enum Pending {
    Pairing(oneshot::Sender<bool>),
    Session(oneshot::Sender<bool>),
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,quinn=warn,mdns_sd=warn".into()),
        )
        .with_target(false)
        .init();

    capture_windows::init_process_dpi_awareness();
    capture_windows::init_thread();

    let name = arg("--name").unwrap_or_else(computer_name);
    let port: u16 = arg("--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(lanlink_protocol::limits::DEFAULT_PORT);

    let store = match HostStore::open(HostStore::default_dir()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("Cannot open LANLink storage: {e}");
            std::process::exit(1);
        }
    };
    let identity = match store.load_or_create_identity(&name) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("Cannot load the host identity: {e}");
            std::process::exit(1);
        }
    };
    let trust = store.load_trust().unwrap_or_else(|e| {
        eprintln!("Paired-device list unreadable ({e}); starting with none.");
        Default::default()
    });

    let mut cfg = HostConfig::new(name.clone());
    cfg.bind = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    cfg.require_session_approval = std::env::args().any(|a| a == "--approve-sessions");

    let store_dir = store.dir().to_path_buf();
    let persist = move |t: &lanlink_auth::TrustStore| {
        if let Ok(s) = HostStore::open(&store_dir) {
            if let Err(e) = s.save_trust(t) {
                tracing::error!(error = %e, "could not save paired devices");
            }
        }
    };
    let mut host = match session_manager::start(cfg, identity, trust, persist).await {
        Ok(h) => h,
        Err(e) => {
            eprintln!("Cannot start: {e}");
            eprintln!("Is another LANLink host already using UDP port {port}?");
            std::process::exit(1);
        }
    };
    let fp_hex = host.fingerprint.to_hex();
    let _advert = match lanlink_discovery::Advertiser::start(&name, &fp_hex, host.local_addr.port())
    {
        Ok(a) => Some(a),
        Err(e) => {
            eprintln!("Discovery unavailable ({e}); phones can still connect by address.");
            None
        }
    };

    println!();
    println!(
        "LANLink host \"{name}\"  identity {}",
        host.fingerprint.short()
    );
    println!(
        "Listening on UDP {}  addresses: {:?}",
        host.local_addr.port(),
        lanlink_discovery::local_addresses()
    );
    println!("Windows Firewall must allow inbound UDP {} for phones to connect (see docs/development.md).", host.local_addr.port());
    println!(
        "Commands: p = pair a phone, s = stop session, l = list phones, r <n> = revoke, q = quit"
    );
    println!();

    let (line_tx, mut lines) = mpsc::unbounded_channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if line_tx.send(line).is_err() {
                break;
            }
        }
    });

    let mut pending: Option<Pending> = None;
    loop {
        tokio::select! {
            line = lines.recv() => {
                let Some(line) = line else { break };
                let line = line.trim().to_string();
                match line.as_str() {
                    "y" | "n" => {
                        let yes = line == "y";
                        match pending.take() {
                            Some(Pending::Pairing(tx)) | Some(Pending::Session(tx)) => {
                                let _ = tx.send(yes);
                                println!("{}", if yes { "Approved." } else { "Declined." });
                            }
                            None => println!("Nothing is waiting for approval."),
                        }
                    }
                    "p" => { let _ = host.commands.send(HostCommand::OpenPairing); }
                    "s" => { let _ = host.commands.send(HostCommand::StopSession); println!("Stopping the session."); }
                    "l" => {
                        let devices = host.paired_devices();
                        if devices.is_empty() { println!("No paired phones."); }
                        for (i, d) in devices.iter().enumerate() {
                            println!("  {}. {} ({})  id {}", i + 1, d.name, d.model, d.fingerprint.short());
                        }
                    }
                    "q" => { let _ = host.commands.send(HostCommand::Shutdown); break; }
                    other if other.starts_with("r ") => {
                        let devices = host.paired_devices();
                        match other[2..].trim().parse::<usize>().ok().and_then(|n| devices.get(n.wrapping_sub(1))) {
                            Some(d) => {
                                let fp: Fingerprint = d.fingerprint;
                                let _ = host.commands.send(HostCommand::Revoke(fp));
                                println!("Revoked {}.", d.name);
                            }
                            None => println!("No such phone; use l to list."),
                        }
                    }
                    "" => {}
                    _ => println!("Unknown command. p, s, l, r <n>, y, n, q"),
                }
            }
            ev = host.events.recv() => {
                let Some(ev) = ev else { break };
                match ev {
                    HostEvent::PairingWindowOpened { uri, code, expires_in } => {
                        println!("Scan this with LANLink on the phone, or enter the code {} {}:", &code[..4], &code[4..]);
                        print_qr(&uri);
                        println!("Pairing stays open for {} s. Link: {uri}", expires_in.as_secs());
                    }
                    HostEvent::PairingWindowClosed => println!("Pairing closed."),
                    HostEvent::PairingApproval { device_name, device_model, fingerprint, confirmation_code, respond } => {
                        println!();
                        println!(">>> \"{device_name}\" ({device_model}, id {}) wants to pair.", fingerprint.short());
                        println!(">>> The phone shows the code {confirmation_code}. Approve only if it matches. [y/n]");
                        pending = Some(Pending::Pairing(respond));
                    }
                    HostEvent::Paired { device_name, .. } => println!("Paired with {device_name}."),
                    HostEvent::SessionApproval { device_name, view_only, respond, .. } => {
                        println!(">>> \"{device_name}\" wants to {} this PC. [y/n]", if view_only { "view" } else { "control" });
                        pending = Some(Pending::Session(respond));
                    }
                    HostEvent::SessionStarted { device_name, display, encoder, hardware, capture, width, height, fps, view_only, .. } => {
                        println!("*** {device_name} is {} this PC ({display}). Stream {width}x{height} @ {fps} fps, {encoder}{}, {capture}. Type s to stop.",
                            if view_only { "viewing" } else { "CONTROLLING" },
                            if hardware { "" } else { " (software)" });
                    }
                    HostEvent::SessionEnded { device_name, reason } => println!("Session with {device_name} ended: {reason}."),
                    HostEvent::Stats { stream, client } => {
                        tracing::debug!(?stream, ?client, "stats");
                    }
                    HostEvent::Notice(n) => println!("Note: {n}"),
                    HostEvent::Rejected { addr, reason } => tracing::warn!(%addr, %reason, "connection refused"),
                    HostEvent::Listening { .. } => {}
                }
            }
        }
    }
}
