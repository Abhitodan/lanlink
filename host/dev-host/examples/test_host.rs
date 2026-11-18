//! Test host for device and emulator end-to-end runs.
//!
//! Same host core as the product, with two deliberate differences so it can
//! run unattended without touching the developer's machine:
//!
//! * pairing and session prompts are approved automatically, and
//! * input is **recorded and printed instead of injected** (the real
//!   `SendInput` path is covered by `input-windows/tests/live.rs`).
//!
//! It refuses to start unless `--i-understand-this-auto-approves` is given,
//! and uses a throw-away identity in a temporary directory.
//!
//! ```text
//! cargo run -p lanlink-host-console --example test_host -- \
//!     --i-understand-this-auto-approves --port 47990 --advertise 10.0.2.2 --uri-file out.txt
//! ```

use input_windows::{Event, Sink};
use lanlink_auth::{Identity, PairingUri, TrustStore};
use session_manager::{HostCommand, HostConfig, HostEvent};
use std::io::Write;
use std::sync::Arc;

struct PrintSink;

impl Sink for PrintSink {
    fn send(&mut self, events: &[Event]) -> usize {
        for e in events {
            println!("INPUT {e:?}");
        }
        let _ = std::io::stdout().flush();
        events.len()
    }
    fn scan_code(&self, vk: u16) -> u16 {
        vk
    }
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    if !std::env::args().any(|a| a == "--i-understand-this-auto-approves") {
        eprintln!("This test host approves every pairing automatically. Pass --i-understand-this-auto-approves.");
        std::process::exit(2);
    }
    tracing_subscriber::fmt()
        .with_env_filter("info,quinn=warn,mdns_sd=warn")
        .with_target(false)
        .init();
    capture_windows::init_process_dpi_awareness();
    capture_windows::init_thread();

    let port: u16 = arg("--port").and_then(|p| p.parse().ok()).unwrap_or(47990);
    let mut cfg = HostConfig::new("LANLink Test Host");
    cfg.bind = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    cfg.input_sink = Arc::new(|| Box::new(PrintSink));
    if let Some(a) = arg("--advertise") {
        cfg.advertise_addrs = vec![a.parse().expect("--advertise must be an IP")];
    }
    let identity = Identity::generate("test host").unwrap();
    let mut host = session_manager::start(cfg, identity, TrustStore::new(), |_| {})
        .await
        .expect("start host");
    host.commands.send(HostCommand::OpenPairing).unwrap();

    while let Some(ev) = host.events.recv().await {
        match ev {
            HostEvent::PairingWindowOpened { uri, .. } => {
                println!("PAIRING_URI {uri}");
                if let Some(path) = arg("--uri-file") {
                    std::fs::write(&path, &uri).expect("write uri file");
                }
                let _ = PairingUri::parse(&uri).expect("valid uri");
            }
            HostEvent::PairingApproval { device_name, confirmation_code, respond, .. } => {
                println!("PAIRING_APPROVAL device={device_name:?} code={confirmation_code}");
                let _ = respond.send(true);
            }
            HostEvent::SessionApproval { respond, .. } => {
                let _ = respond.send(true);
            }
            HostEvent::Paired { device_name, .. } => println!("PAIRED {device_name:?}"),
            HostEvent::SessionStarted { device_name, encoder, capture, width, height, fps, .. } => {
                println!("SESSION_STARTED {device_name:?} {width}x{height}@{fps} encoder={encoder:?} capture={capture:?}");
            }
            HostEvent::SessionEnded { device_name, reason } => println!("SESSION_ENDED {device_name:?} {reason}"),
            HostEvent::Stats { stream, client } => println!(
                "STATS cap={:.1} enc={:.1} kbps={} rtt_us={} enc_p50_us={} drop_enc={} drop_send={} client={:?}",
                stream.captured_fps_x10 as f32 / 10.0,
                stream.encoded_fps_x10 as f32 / 10.0,
                stream.bitrate_kbps,
                stream.rtt_us,
                stream.encode_us_p50,
                stream.dropped_before_encode,
                stream.dropped_before_send,
                client.map(|c| (c.received_fps_x10, c.rendered_fps_x10, c.decode_us_p50))
            ),
            HostEvent::Notice(n) => println!("NOTICE {n}"),
            HostEvent::Rejected { addr, reason } => println!("REJECTED {addr} {reason}"),
            _ => {}
        }
        let _ = std::io::stdout().flush();
    }
}
