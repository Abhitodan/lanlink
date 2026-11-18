//! End-to-end over loopback QUIC: real host (real screen capture and
//! hardware encoding), real client library. Only the input sink is replaced
//! by a recorder so the test does not move the developer's mouse; the real
//! `SendInput` path is covered by `input-windows/tests/live.rs`.
//!
//! Needs an interactive desktop. Run with `--test-threads=1` (each test
//! captures the screen).

#![cfg(windows)]

use input_windows::{Event, Sink};
use lanlink_auth::{Fingerprint, Identity, PairingUri, TrustStore};
use lanlink_client::{ClientError, ClientEvent, DeviceInfo, VideoItem};
use lanlink_protocol::*;
use session_manager::{HostCommand, HostConfig, HostEvent, HostHandle};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::time::timeout;

#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Event>>>);

impl Sink for Recorder {
    fn send(&mut self, events: &[Event]) -> usize {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .extend_from_slice(events);
        events.len()
    }
    fn scan_code(&self, vk: u16) -> u16 {
        vk
    }
}

fn device(name: &str) -> DeviceInfo {
    DeviceInfo {
        name: name.into(),
        model: "Test".into(),
        app_version: "0.1.0".into(),
    }
}

fn desktop_available() -> bool {
    capture_windows::init_process_dpi_awareness();
    capture_windows::init_thread();
    capture_windows::interactive_desktop_active()
}

struct TestHost {
    handle: HostHandle,
    addr: SocketAddr,
    recorder: Recorder,
}

async fn start_host(require_session_approval: bool) -> TestHost {
    let recorder = Recorder::default();
    let r = recorder.clone();
    let mut cfg = HostConfig::new("Test PC");
    cfg.bind = "127.0.0.1:0".parse().unwrap();
    cfg.require_session_approval = require_session_approval;
    cfg.input_sink = Arc::new(move || Box::new(r.clone()));
    cfg.advertise_addrs = vec!["127.0.0.1".parse().unwrap()];
    let identity = Identity::generate("test host").unwrap();
    let handle = session_manager::start(cfg, identity, TrustStore::new(), |_| {})
        .await
        .unwrap();
    let addr = handle.local_addr;
    TestHost {
        handle,
        addr,
        recorder,
    }
}

impl TestHost {
    /// Open pairing and return the QR link.
    async fn open_pairing(&mut self) -> PairingUri {
        self.handle.commands.send(HostCommand::OpenPairing).unwrap();
        loop {
            match timeout(Duration::from_secs(5), self.handle.events.recv())
                .await
                .unwrap()
            {
                Some(HostEvent::PairingWindowOpened { uri, .. }) => {
                    return PairingUri::parse(&uri).unwrap()
                }
                Some(_) => continue,
                None => panic!("host stopped"),
            }
        }
    }

    /// Answer approval prompts with `approve`, recording the code shown.
    fn auto_answer(
        mut self,
        approve: bool,
    ) -> (
        Arc<Mutex<Vec<String>>>,
        tokio::task::JoinHandle<HostHandle>,
        SocketAddr,
        Recorder,
    ) {
        let codes = Arc::new(Mutex::new(Vec::new()));
        let c = codes.clone();
        let addr = self.addr;
        let rec = self.recorder.clone();
        let task = tokio::spawn(async move {
            while let Some(ev) = self.handle.events.recv().await {
                match ev {
                    HostEvent::PairingApproval {
                        confirmation_code,
                        respond,
                        ..
                    } => {
                        c.lock().unwrap().push(confirmation_code);
                        let _ = respond.send(approve);
                    }
                    HostEvent::SessionApproval { respond, .. } => {
                        let _ = respond.send(approve);
                    }
                    _ => {}
                }
                if c.lock().unwrap().len() > 100 {
                    break;
                }
            }
            self.handle
        });
        (codes, task, addr, rec)
    }
}

async fn pair_ok(host: &mut TestHost, phone: &Identity) -> Fingerprint {
    let uri = host.open_pairing().await;
    let addr = SocketAddr::new(uri.addresses[0], uri.port);
    let fp = uri.fingerprint;
    // Approve from a concurrent task.
    let mut events = std::mem::replace(
        &mut host.handle.events,
        tokio::sync::mpsc::unbounded_channel().1,
    );
    let host_code = Arc::new(Mutex::new(String::new()));
    let hc = host_code.clone();
    let approver = tokio::spawn(async move {
        while let Some(ev) = events.recv().await {
            if let HostEvent::PairingApproval {
                confirmation_code,
                respond,
                ..
            } = ev
            {
                *hc.lock().unwrap() = confirmation_code;
                let _ = respond.send(true);
                break;
            }
        }
        events
    });
    let mut phone_code = String::new();
    let paired = lanlink_client::pair(phone, &device("Phone A"), addr, Some(fp), &uri.code, |c| {
        phone_code = c.to_string()
    })
    .await
    .unwrap();
    host.handle.events = approver.await.unwrap();
    assert_eq!(paired.host_fingerprint, fp);
    assert_eq!(
        phone_code,
        *host_code.lock().unwrap(),
        "both screens show the same code"
    );
    assert_eq!(phone_code.len(), 6);
    fp
}

fn params() -> StreamParams {
    StreamParams {
        quality: QualityMode::Balanced,
        codec: Codec::H264,
        max_width: 1280,
        max_height: 0,
        fps: 30,
        bitrate_kbps: 0,
        max_bitrate_kbps: 0,
    }
}

fn caps() -> Vec<CodecCap> {
    vec![CodecCap {
        codec: Codec::H264,
        hardware: true,
        low_latency: true,
        max_width: 1920,
        max_height: 1080,
        max_fps: 60,
    }]
}

async fn next_event(
    c: &mut lanlink_client::Connection,
    what: &str,
    pred: impl Fn(&ClientEvent) -> bool,
) -> ClientEvent {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match timeout(left, c.events.recv()).await {
            Ok(Some(ev)) if pred(&ev) => return ev,
            Ok(Some(ClientEvent::Error(code, msg))) => {
                panic!("host error while waiting for {what}: {code:?} {msg}")
            }
            Ok(Some(_)) => continue,
            Ok(None) => panic!("connection ended while waiting for {what}"),
            Err(_) => panic!("timed out waiting for {what}"),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn pair_stream_control_and_release_on_disconnect() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(false).await;
    let phone = Identity::generate("phone").unwrap();
    let host_fp = pair_ok(&mut host, &phone).await;
    assert_eq!(host.handle.paired_devices().len(), 1);

    let mut conn = lanlink_client::connect(&phone, &device("Phone A"), host.addr, host_fp)
        .await
        .unwrap();
    conn.send(lanlink_client::capabilities(caps(), (2400, 1080), 120_000))
        .await
        .unwrap();
    let ClientEvent::DisplayList(displays) = next_event(&mut conn, "display list", |e| {
        matches!(e, ClientEvent::DisplayList(_))
    })
    .await
    else {
        unreachable!()
    };
    assert!(displays.iter().any(|d| d.primary));

    conn.send(ControlMsg::SessionStart {
        display_id: DISPLAY_PRIMARY,
        params: params(),
        view_only: false,
    })
    .await
    .unwrap();
    let ClientEvent::SessionStarted {
        session_id,
        encoder,
        capture,
        ..
    } = next_event(&mut conn, "session start", |e| {
        matches!(e, ClientEvent::SessionStarted { .. })
    })
    .await
    else {
        unreachable!()
    };
    println!("session {session_id:#x}: encoder '{encoder}', capture '{capture}'");
    conn.bind_session(session_id);

    // Video: a config, then a self-contained keyframe at the configured size.
    let first = timeout(Duration::from_secs(10), conn.video.recv())
        .await
        .unwrap()
        .unwrap();
    let VideoItem::Config(cfg) = first else {
        panic!("expected config first")
    };
    assert_eq!(cfg.width, 1280);
    assert_eq!(cfg.codec, Codec::H264);
    let VideoItem::Frame {
        header,
        data,
        recv_us,
    } = timeout(Duration::from_secs(10), conn.video.recv())
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("expected a frame")
    };
    assert!(header.is_keyframe());
    assert_eq!(header.config_id, cfg.config_id);
    let info = encoder::annexb::inspect(&data);
    assert!(info.is_self_contained_keyframe(), "{info:?}");
    // Same process, same monotonic clock: this is a true capture→receive time.
    let latency_ms = recv_us.saturating_sub(header.capture_us) as f64 / 1000.0;
    println!(
        "first keyframe: {} bytes, capture->client {latency_ms:.1} ms (loopback)",
        data.len()
    );

    // Input: click at the centre, hold Ctrl, type text, move the pointer.
    assert!(conn.send_input(InputKind::Button {
        button: MouseButton::Left,
        down: true,
        pos: Some((32768, 32768))
    }));
    assert!(conn.send_input(InputKind::Button {
        button: MouseButton::Left,
        down: false,
        pos: None
    }));
    assert!(conn.send_input(InputKind::Key {
        vk: 0x11,
        down: true
    }));
    assert!(conn.send_input(InputKind::Text {
        text: "hi 😀".into()
    }));
    conn.send_pointer(1000, 2000);
    // Wait for the acks of all four reliable events.
    let mut acked = 0;
    while acked < 4 {
        if let ClientEvent::InputAck { .. } = next_event(&mut conn, "input ack", |e| {
            matches!(e, ClientEvent::InputAck { .. })
        })
        .await
        {
            acked += 1;
        }
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    {
        let ev = host.recorder.0.lock().unwrap();
        // Guarantees: the click's own position is injected immediately
        // before the press, and reliable events keep their order. Pointer
        // datagrams are unordered relative to the reliable stream (by
        // design), so they may appear anywhere.
        let down = ev
            .iter()
            .position(|e| {
                *e == Event::Button {
                    button: MouseButton::Left,
                    down: true,
                }
            })
            .expect("button press injected");
        assert!(down >= 1 && matches!(ev[down - 1], Event::MoveAbs { .. }));
        let reliable: Vec<&Event> = ev
            .iter()
            .filter(|e| !matches!(e, Event::MoveAbs { .. }))
            .collect();
        assert_eq!(
            *reliable[0],
            Event::Button {
                button: MouseButton::Left,
                down: true
            }
        );
        assert_eq!(
            *reliable[1],
            Event::Button {
                button: MouseButton::Left,
                down: false
            }
        );
        assert!(
            matches!(
                reliable[2],
                Event::Key {
                    vk: 0xA2,
                    down: true,
                    ..
                }
            ),
            "Ctrl maps to left Ctrl: {:?}",
            reliable[2]
        );
        let moves = ev
            .iter()
            .filter(|e| matches!(e, Event::MoveAbs { .. }))
            .count();
        assert_eq!(moves, 2, "click position + pointer datagram");
        let units: Vec<u16> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Unicode { unit, down: true } => Some(*unit),
                _ => None,
            })
            .collect();
        assert_eq!(units, [0x68, 0x69, 0x20, 0xD83D, 0xDE00]);
    }

    // The phone vanishes while Ctrl is held: the host must release it.
    drop(conn);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let released = host.recorder.0.lock().unwrap().iter().any(|e| {
            matches!(
                e,
                Event::Key {
                    vk: 0xA2,
                    down: false,
                    ..
                }
            )
        });
        if released {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Ctrl was not released after disconnect"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    println!("held Ctrl released after disconnect");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unpaired_wrong_code_impostor_and_revoked_are_refused() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(false).await;
    let stranger = Identity::generate("stranger").unwrap();

    // 1. Unknown device, pairing closed.
    let r = lanlink_client::connect(
        &stranger,
        &device("Stranger"),
        host.addr,
        host.handle.fingerprint,
    )
    .await;
    assert!(
        matches!(
            r,
            Err(ClientError::NotPaired) | Err(ClientError::Host(ErrorCode::NotPaired, _))
        ),
        "{r:?}"
    );

    // 2. Wrong code.
    let uri = host.open_pairing().await;
    let r = lanlink_client::pair(
        &stranger,
        &device("Stranger"),
        host.addr,
        Some(uri.fingerprint),
        "00000000",
        |_| {},
    )
    .await;
    let wrong_code_rejected = matches!(r, Err(ClientError::WrongCode));
    assert!(wrong_code_rejected || uri.code == "00000000", "{r:?}");

    // 3. Expected host fingerprint does not match (e.g. a different PC or
    //    a machine-in-the-middle answering at that address).
    let other = Identity::generate("other").unwrap();
    let r = lanlink_client::pair(
        &stranger,
        &device("Stranger"),
        host.addr,
        Some(other.fingerprint()),
        &uri.code,
        |_| {},
    )
    .await;
    assert!(
        matches!(
            r,
            Err(ClientError::IdentityMismatch) | Err(ClientError::Lost(_))
        ),
        "{r:?}"
    );

    // 4. Pair properly, then revoke: the next connection is refused.
    let phone = Identity::generate("phone").unwrap();
    let fp = pair_ok(&mut host, &phone).await;
    let conn = lanlink_client::connect(&phone, &device("Phone"), host.addr, fp)
        .await
        .unwrap();
    host.handle
        .commands
        .send(HostCommand::Revoke(phone.fingerprint()))
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !conn.is_closed() {
        assert!(
            Instant::now() < deadline,
            "revoked device was not disconnected"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let r = lanlink_client::connect(&phone, &device("Phone"), host.addr, fp).await;
    assert!(
        matches!(
            r,
            Err(ClientError::NotPaired) | Err(ClientError::Host(ErrorCode::NotPaired, _))
        ),
        "{r:?}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_phone_is_busy_and_emergency_stop_ends_session() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(false).await;
    let a = Identity::generate("a").unwrap();
    let b = Identity::generate("b").unwrap();
    let fp = pair_ok(&mut host, &a).await;
    pair_ok(&mut host, &b).await;

    let mut ca = lanlink_client::connect(&a, &device("A"), host.addr, fp)
        .await
        .unwrap();
    ca.send(ControlMsg::SessionStart {
        display_id: DISPLAY_PRIMARY,
        params: params(),
        view_only: false,
    })
    .await
    .unwrap();
    next_event(&mut ca, "session A", |e| {
        matches!(e, ClientEvent::SessionStarted { .. })
    })
    .await;

    let mut cb = lanlink_client::connect(&b, &device("B"), host.addr, fp)
        .await
        .unwrap();
    cb.send(ControlMsg::SessionStart {
        display_id: DISPLAY_PRIMARY,
        params: params(),
        view_only: true,
    })
    .await
    .unwrap();
    let busy = loop {
        match timeout(Duration::from_secs(10), cb.events.recv())
            .await
            .unwrap()
        {
            Some(ClientEvent::Error(code, _)) => break code,
            Some(ClientEvent::SessionStarted { .. }) => {
                panic!("second phone must not get a session")
            }
            Some(_) => continue,
            None => panic!("closed"),
        }
    };
    assert_eq!(busy, ErrorCode::HostBusy);

    // Emergency stop on the PC.
    let t = Instant::now();
    host.handle.commands.send(HostCommand::StopSession).unwrap();
    loop {
        match timeout(Duration::from_secs(5), ca.events.recv())
            .await
            .unwrap()
        {
            Some(ClientEvent::Closed(_)) | None => break,
            Some(_) => continue,
        }
    }
    println!(
        "emergency stop took effect on the phone after {:?}",
        t.elapsed()
    );
    assert!(t.elapsed() < Duration::from_secs(2));
    assert!(ca.is_closed());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn session_approval_denied_is_reported() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(true).await;
    let phone = Identity::generate("phone").unwrap();
    let fp = pair_ok(&mut host, &phone).await;
    let (_codes, _task, addr, _rec) = host.auto_answer(false);
    let mut c = lanlink_client::connect(&phone, &device("P"), addr, fp)
        .await
        .unwrap();
    c.send(ControlMsg::SessionStart {
        display_id: DISPLAY_PRIMARY,
        params: params(),
        view_only: false,
    })
    .await
    .unwrap();
    let code = loop {
        match timeout(Duration::from_secs(10), c.events.recv())
            .await
            .unwrap()
        {
            Some(ClientEvent::Error(code, _)) => break code,
            Some(ClientEvent::SessionStarted { .. }) => panic!("denied session must not start"),
            Some(_) => continue,
            None => panic!("closed"),
        }
    };
    assert_eq!(code, ErrorCode::SessionDenied);
}

/// Protocol 2: with the phone's keyboard open the host reports the text
/// caret (visible or not, depending on what has focus on this PC), and an
/// old-style message flow is unaffected.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn keyboard_open_makes_the_host_report_the_caret() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(false).await;
    let phone = Identity::generate("phone").unwrap();
    let fp = pair_ok(&mut host, &phone).await;
    let mut c = lanlink_client::connect(&phone, &device("P"), host.addr, fp)
        .await
        .unwrap();
    assert_eq!(c.protocol, 2, "both sides speak protocol 2");
    c.send(ControlMsg::SessionStart {
        display_id: DISPLAY_PRIMARY,
        params: params(),
        view_only: false,
    })
    .await
    .unwrap();
    next_event(&mut c, "session start", |e| {
        matches!(e, ClientEvent::SessionStarted { .. })
    })
    .await;
    assert!(c.set_text_input(true));
    let ev = next_event(&mut c, "caret report", |e| {
        matches!(e, ClientEvent::Caret { .. })
    })
    .await;
    println!("caret report: {ev:?}");
    assert!(c.set_text_input(false));
    // The connection stays healthy after the round trip.
    c.send(ControlMsg::KeyframeRequest).await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!c.is_closed());
}

/// Many connections starting a pairing exchange at once still get only
/// `MAX_ATTEMPTS` code guesses per window between them.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_pairing_attempts_share_the_attempt_limit() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(false).await;
    let uri = host.open_pairing().await;
    let wrong = if uri.code == "00000000" {
        "11111111"
    } else {
        "00000000"
    };
    let attempts = (0..6).map(|i| {
        let id = Identity::generate(&format!("guesser {i}")).unwrap();
        let addr = host.addr;
        let fp = uri.fingerprint;
        tokio::spawn(async move {
            lanlink_client::pair(&id, &device("Guesser"), addr, Some(fp), wrong, |_| {}).await
        })
    });
    let mut guesses = 0;
    for a in attempts.collect::<Vec<_>>() {
        match a.await.unwrap() {
            Err(ClientError::WrongCode) => guesses += 1,
            // Refused before any guess: window spent, or turned away by
            // the cap on connections from unknown devices.
            Err(ClientError::PairingClosed) | Err(ClientError::NotPaired) => {}
            Err(ClientError::Host(ErrorCode::NotPaired, _)) => {}
            Err(ClientError::Lost(_)) | Err(ClientError::Connect(_)) => {}
            other => panic!("unexpected {other:?}"),
        }
    }
    assert!(
        (1..=lanlink_auth::pairing::MAX_ATTEMPTS as usize).contains(&guesses),
        "{guesses} code guesses were evaluated"
    );
    // The window is spent: even the right code no longer pairs. (Wait for
    // the refused connections to close first, or the cap on unknown devices
    // turns this one away before it can ask.)
    tokio::time::sleep(Duration::from_millis(800)).await;
    let phone = Identity::generate("late phone").unwrap();
    let r = lanlink_client::pair(
        &phone,
        &device("Phone"),
        host.addr,
        Some(uri.fingerprint),
        &uri.code,
        |_| {},
    )
    .await;
    assert!(
        matches!(
            r,
            Err(ClientError::PairingClosed)
                | Err(ClientError::NotPaired)
                | Err(ClientError::Host(ErrorCode::NotPaired, _))
        ),
        "{r:?}"
    );
}

/// Manual pairing (no QR, so no pinned identity) must not accept a device's
/// claim that the phone is "already paired": it skips the code check.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn manual_pairing_refuses_an_unverified_already_paired_reply() {
    if !desktop_available() {
        eprintln!("SKIP: no interactive desktop");
        return;
    }
    let mut host = start_host(false).await;
    let phone = Identity::generate("phone").unwrap();
    let fp = pair_ok(&mut host, &phone).await;

    let r = lanlink_client::pair(
        &phone,
        &device("Phone"),
        host.addr,
        None,
        "12345678",
        |_| {},
    )
    .await;
    assert!(matches!(r, Err(ClientError::UnverifiedHost)), "{r:?}");

    // With the PC's identity pinned (QR code) the shortcut is fine.
    let r = lanlink_client::pair(
        &phone,
        &device("Phone"),
        host.addr,
        Some(fp),
        "12345678",
        |_| {},
    )
    .await;
    assert_eq!(r.unwrap().host_fingerprint, fp);
}
