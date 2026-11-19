//! End-to-end tests of the mobile core against an in-process fake host.
//!
//! The real host (`host/session-manager`) is Windows-only and captures the
//! screen. This fake speaks the same protocol with the cross-platform crates
//! only, so pairing, connecting, video delivery and input are tested on any
//! OS. It binds loopback, uses throwaway identities and writes no files. It
//! auto-approves pairing and answers just the messages these tests use.

use lanlink_auth::pairing::PairingHost;
use lanlink_auth::{Fingerprint, Identity, PairedDevice, PairingWindow, TrustStore};
use lanlink_mobile::*;
use lanlink_protocol::limits::PROTOCOL_VERSION;
use lanlink_protocol::video::{FrameHeader, FLAG_KEYFRAME};
use lanlink_protocol::{
    control, AuthState, Codec, ControlMsg, DisplayInfo, ErrorCode, InputEvent, InputKind,
    StreamKind, VideoConfig,
};
use lanlink_transport::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc as std_mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const PAIRING_CODE: &str = "12345678";
const SESSION_ID: u32 = 7;
const WAIT: Duration = Duration::from_secs(10);

/// SPS, PPS and an IDR slice, each with a 4-byte start code and no zero
/// bytes in the payloads (so no accidental start codes).
const SPS: [u8; 9] = [0, 0, 0, 1, 0x67, 0x42, 0xC0, 0x1E, 0xDA];
const PPS: [u8; 7] = [0, 0, 0, 1, 0x68, 0xCE, 0x3C];
const IDR: [u8; 9] = [0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21, 0xA0];

fn keyframe_au() -> Vec<u8> {
    [&SPS[..], &PPS[..], &IDR[..]].concat()
}

fn sps_pps() -> Vec<u8> {
    [&SPS[..], &PPS[..]].concat()
}

struct HostState {
    identity: Identity,
    trust: Mutex<TrustStore>,
    window: Mutex<Option<PairingWindow>>,
    inputs: std_mpsc::Sender<InputEvent>,
    /// When non-zero: after this many events on one input stream, stop
    /// reading it (STOP_SENDING), as a host would on a stream it drops.
    /// Reset to 0 once used.
    stop_input_stream_after: AtomicUsize,
}

struct FakeHost {
    // Dropping the runtime shuts the host's tasks down.
    _rt: tokio::runtime::Runtime,
    port: u16,
    fingerprint: Fingerprint,
    state: Arc<HostState>,
    inputs: std_mpsc::Receiver<InputEvent>,
}

impl FakeHost {
    fn start() -> FakeHost {
        let identity = Identity::generate("fake host").expect("identity");
        let fingerprint = identity.fingerprint();
        let (inputs_tx, inputs) = std_mpsc::channel();
        let state = Arc::new(HostState {
            identity,
            trust: Mutex::new(TrustStore::new()),
            window: Mutex::new(Some(
                PairingWindow::with_code(PAIRING_CODE, Instant::now()).expect("window"),
            )),
            inputs: inputs_tx,
            stop_input_stream_after: AtomicUsize::new(0),
        });
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("runtime");
        let endpoint = {
            let _guard = rt.enter();
            server_endpoint(&state.identity, "127.0.0.1:0".parse().unwrap()).expect("endpoint")
        };
        let port = endpoint.local_addr().expect("addr").port();
        let st = state.clone();
        rt.spawn(async move {
            while let Some(incoming) = endpoint.accept().await {
                let st = st.clone();
                tokio::spawn(async move {
                    if let Ok(conn) = incoming.await {
                        if let Err(e) = serve(conn, st).await {
                            eprintln!("fake host: connection ended: {e}");
                        }
                    }
                });
            }
        });
        FakeHost {
            _rt: rt,
            port,
            fingerprint,
            state,
            inputs,
        }
    }

    fn fp_hex(&self) -> String {
        self.fingerprint.to_hex()
    }

    /// Mark a phone as already paired, skipping the pairing exchange.
    fn trust(&self, phone: &Fingerprint) {
        self.state.trust.lock().unwrap().insert(PairedDevice {
            fingerprint: *phone,
            name: "test phone".into(),
            model: "test".into(),
            paired_at: 1,
            last_seen: 0,
        });
    }

    fn trusts(&self, phone: &Fingerprint) -> bool {
        self.state.trust.lock().unwrap().contains(phone)
    }
}

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// One client connection, from Hello to close.
async fn serve(conn: quinn::Connection, st: Arc<HostState>) -> Result<(), String> {
    let client_fp = peer_fingerprint(&conn).map_err(err)?;
    let (mut send, mut recv) = conn.accept_bi().await.map_err(err)?;
    if read_stream_kind(&mut recv).await.map_err(err)? != StreamKind::Control {
        return Err("first stream was not control".into());
    }
    let Some(ControlMsg::Hello {
        protocol_min,
        protocol_max,
        ..
    }) = read_control(&mut recv).await.map_err(err)?
    else {
        return Err("expected Hello".into());
    };
    let protocol = control::negotiate_version(protocol_min, protocol_max).ok_or("version")?;
    assert!(protocol <= PROTOCOL_VERSION);

    let known = st.trust.lock().unwrap().contains(&client_fp);
    let pairing_open = st
        .window
        .lock()
        .unwrap()
        .as_ref()
        .is_some_and(|w| w.is_open(Instant::now()));
    let auth = if known {
        AuthState::Paired
    } else if pairing_open {
        AuthState::PairingAvailable
    } else {
        AuthState::NotPaired
    };
    write_control(
        &mut send,
        &ControlMsg::HelloAck {
            protocol,
            host_name: "Fake PC".into(),
            app_version: "0.0.0".into(),
            auth,
        },
    )
    .await
    .map_err(err)?;
    if auth == AuthState::NotPaired {
        return Err("not paired".into());
    }

    // Reliable input arrives on a stream the phone opens per session.
    let input_conn = conn.clone();
    let input_st = st.clone();
    tokio::spawn(async move {
        while let Ok(mut r) = input_conn.accept_uni().await {
            if !matches!(read_stream_kind(&mut r).await, Ok(StreamKind::Input)) {
                continue;
            }
            let mut on_this_stream = 0;
            while let Ok(Some(ev)) = read_input(&mut r).await {
                let _ = input_st.inputs.send(ev);
                on_this_stream += 1;
                let stop_after = input_st.stop_input_stream_after.load(Ordering::SeqCst);
                if stop_after != 0 && on_this_stream == stop_after {
                    input_st.stop_input_stream_after.store(0, Ordering::SeqCst);
                    let _ = r.stop(0u32.into());
                    break;
                }
            }
        }
    });

    let mut pairing = None;
    let mut video: Option<quinn::SendStream> = None;
    while let Some(msg) = read_control(&mut recv).await.map_err(err)? {
        match msg {
            ControlMsg::PairRequest { spake_msg } => {
                let attempt = st
                    .window
                    .lock()
                    .unwrap()
                    .as_mut()
                    .and_then(|w| w.begin_attempt(Instant::now()));
                let (_ticket, code) = attempt.ok_or("pairing window closed")?;
                let binding = channel_binding(&conn).map_err(err)?;
                let (keys, reply) = PairingHost::respond(
                    &code,
                    client_fp,
                    st.identity.fingerprint(),
                    &spake_msg,
                    &binding,
                )
                .map_err(err)?;
                pairing = Some(keys);
                write_control(&mut send, &ControlMsg::PairResponse { spake_msg: reply })
                    .await
                    .map_err(err)?;
            }
            ControlMsg::PairConfirm { mac } => {
                let keys = pairing.as_ref().ok_or("confirm without request")?;
                keys.verify_client(&mac).map_err(err)?;
                // Auto-approve: the real host asks the PC user here.
                st.trust.lock().unwrap().insert(PairedDevice {
                    fingerprint: client_fp,
                    name: "paired phone".into(),
                    model: "test".into(),
                    paired_at: 1,
                    last_seen: 0,
                });
                write_control(
                    &mut send,
                    &ControlMsg::PairResult {
                        accepted: true,
                        mac: keys.host_mac,
                        reason: ErrorCode::Normal,
                    },
                )
                .await
                .map_err(err)?;
            }
            ControlMsg::Capabilities { .. } => {
                write_control(
                    &mut send,
                    &ControlMsg::DisplayList {
                        displays: vec![DisplayInfo {
                            id: 1,
                            name: "Fake display".into(),
                            width: 1920,
                            height: 1080,
                            x: 0,
                            y: 0,
                            primary: true,
                            refresh_millihz: 60_000,
                            scale_percent: 100,
                            rotation: 0,
                            hdr: false,
                        }],
                    },
                )
                .await
                .map_err(err)?;
            }
            ControlMsg::SessionStart { .. } => {
                let mut v = conn.open_uni().await.map_err(err)?;
                write_stream_kind(&mut v, StreamKind::Video)
                    .await
                    .map_err(err)?;
                write_video_config(
                    &mut v,
                    &VideoConfig {
                        config_id: 1,
                        codec: Codec::H264,
                        width: 1280,
                        height: 720,
                        fps: 60,
                        display_id: 1,
                        source_width: 1920,
                        source_height: 1080,
                        rotation: 0,
                    },
                )
                .await
                .map_err(err)?;
                let au = keyframe_au();
                let now = monotonic_us();
                write_video_frame(
                    &mut v,
                    &FrameHeader {
                        seq: 1,
                        config_id: 1,
                        flags: FLAG_KEYFRAME,
                        capture_us: now,
                        encode_us: 100,
                        send_us: now + 100,
                        payload_len: au.len() as u32,
                    },
                    &au,
                )
                .await
                .map_err(err)?;
                // Keep the stream open: finishing it would end the video.
                video = Some(v);
                write_control(
                    &mut send,
                    &ControlMsg::SessionStarted {
                        session_id: SESSION_ID,
                        display_id: 1,
                        view_only: false,
                        encoder_name: "fake".into(),
                        capture_backend: "fake".into(),
                    },
                )
                .await
                .map_err(err)?;
            }
            // Pings, stats, keyframe requests and the rest need no answer.
            _ => {}
        }
    }
    drop(video);
    Ok(())
}

// ---- test helpers (blocking: the mobile API is for worker threads) ----

const MODEL: &str = "Test Phone";

fn new_phone() -> (Vec<u8>, Fingerprint) {
    let id = generate_identity("test phone").expect("identity");
    let fp = Fingerprint::from_hex(&identity_fingerprint(&id).expect("fp")).expect("hex");
    (id, fp)
}

fn connect_to(host: &FakeHost, id: &[u8]) -> i64 {
    connect(
        id,
        "Test phone",
        MODEL,
        "127.0.0.1",
        host.port as i32,
        &host.fp_hex(),
    )
    .expect("connect")
}

/// Poll control events until one whose JSON contains `needle` arrives.
fn wait_event(h: i64, needle: &str) -> String {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        if let Some(ev) = poll_event(h, 200).expect("poll_event") {
            if ev.contains(needle) {
                return ev;
            }
        }
    }
    panic!("no event containing {needle} within {WAIT:?}");
}

fn start_streaming(h: i64) {
    send_capabilities(h, 1920, 1080, 60, true, true, 1170, 2532, 60_000).expect("capabilities");
    wait_event(h, "\"type\":\"displays\"");
    start_session(h, 0, 1, 1920, 1080, 60, 8000, 16000, false).expect("start_session");
    wait_event(h, "\"type\":\"sessionStarted\"");
}

/// Call `next_video` until it returns something other than a timeout.
fn next(h: i64, buf: &mut [u8], meta: &mut [i64; 8]) -> i32 {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let r = next_video(h, buf, meta, 500).expect("next_video");
        if r != VIDEO_TIMEOUT {
            return r;
        }
    }
    panic!("no video within {WAIT:?}");
}

#[test]
fn pairs_then_connects_with_the_pinned_identity() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();

    let job = pair_start(
        &id,
        "Test phone",
        MODEL,
        "127.0.0.1",
        host.port as i32,
        &host.fp_hex(),
        PAIRING_CODE,
    )
    .expect("pair_start");

    let deadline = Instant::now() + WAIT;
    let done = loop {
        let state = pair_poll(job).expect("pair_poll");
        if state.contains("\"state\":\"done\"") {
            break state;
        }
        assert!(
            !state.contains("\"state\":\"failed\""),
            "pairing failed: {state}"
        );
        assert!(Instant::now() < deadline, "pairing timed out: {state}");
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        done.contains(&format!("\"fingerprint\":\"{}\"", host.fp_hex())),
        "fingerprint is not the host's: {done}"
    );
    assert!(done.contains("\"hostName\":\"Fake PC\""), "{done}");
    assert!(host.trusts(&phone_fp), "host did not record the phone");

    let h = connect_to(&host, &id);
    assert!(h > 0);
    close(h);
}

#[test]
fn session_delivers_config_then_parameter_sets_then_frame() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();
    host.trust(&phone_fp);
    let h = connect_to(&host, &id);
    start_streaming(h);

    let mut buf = vec![0u8; 4096];
    let mut meta = [0i64; 8];

    assert_eq!(next(h, &mut buf, &mut meta), VIDEO_CONFIG);
    // width, height, fps, configId, displayId, sourceWidth, sourceHeight
    assert_eq!(meta[..7], [1280, 720, 60, 1, 1, 1920, 1080]);

    let n = next(h, &mut buf, &mut meta);
    assert!(n > 0, "expected codec config, got {n}");
    assert_eq!(meta[0], META_CODEC_CONFIG);
    assert_eq!(&buf[..n as usize], &sps_pps()[..]);

    let n = next(h, &mut buf, &mut meta);
    assert!(n > 0, "expected the keyframe, got {n}");
    assert_eq!(meta[0], META_KEYFRAME);
    assert_eq!(&buf[..n as usize], &keyframe_au()[..]);
    close(h);
}

#[test]
fn input_reaches_the_host_in_order() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();
    host.trust(&phone_fp);
    let h = connect_to(&host, &id);
    start_streaming(h);

    bind_session(h, SESSION_ID as i64).expect("bind_session");

    assert!(send_key(h, 0x41, true).expect("key down"));
    assert!(send_key(h, 0x41, false).expect("key up"));
    assert!(send_text(h, "h\u{e9}").expect("text"));

    let got: Vec<InputEvent> = (0..3)
        .map(|_| host.inputs.recv_timeout(WAIT).expect("input event"))
        .collect();
    let kinds: Vec<_> = got.iter().map(|e| e.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            InputKind::Key {
                vk: 0x41,
                down: true
            },
            InputKind::Key {
                vk: 0x41,
                down: false
            },
            InputKind::Text {
                text: "h\u{e9}".into()
            },
        ]
    );
    let seqs: Vec<u32> = got.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
    assert!(got.iter().all(|e| e.session_id == SESSION_ID));
    close(h);
}

#[test]
fn input_sent_immediately_after_bind_is_not_lost() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();
    host.trust(&phone_fp);
    let h = connect_to(&host, &id);
    start_streaming(h);

    bind_session(h, SESSION_ID as i64).expect("bind_session");

    assert!(send_key(h, 0x41, true).expect("key down"));
    assert!(send_key(h, 0x41, false).expect("key up"));
    assert!(send_text(h, "h\u{e9}").expect("text"));

    let got: Vec<InputEvent> = (0..3)
        .map(|_| host.inputs.recv_timeout(WAIT).expect("input event"))
        .collect();
    let kinds: Vec<_> = got.iter().map(|e| e.kind.clone()).collect();
    assert_eq!(
        kinds,
        [
            InputKind::Key {
                vk: 0x41,
                down: true
            },
            InputKind::Key {
                vk: 0x41,
                down: false
            },
            InputKind::Text {
                text: "h\u{e9}".into()
            },
        ]
    );
    let seqs: Vec<u32> = got.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, [1, 2, 3]);
    assert!(got.iter().all(|e| e.session_id == SESSION_ID));
    close(h);
}

/// The host keeps one input sequencer per session, not per stream, and
/// closes the connection on anything but the next number. When the client
/// reopens its input stream for the same session after a write error, the
/// sequence must continue, not restart at 1.
#[test]
fn input_sequence_continues_after_the_stream_is_reopened() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();
    host.trust(&phone_fp);
    let h = connect_to(&host, &id);
    start_streaming(h);

    bind_session(h, SESSION_ID as i64).expect("bind_session");
    host.state
        .stop_input_stream_after
        .store(1, Ordering::SeqCst);

    assert!(send_key(h, 0x41, true).expect("key down"));
    let first = host.inputs.recv_timeout(WAIT).expect("first input event");
    // Let the host's STOP_SENDING reach the client, so the next write on the
    // old stream fails and the client drops that stream.
    std::thread::sleep(Duration::from_millis(500));
    // Written to the stopped stream: fails and is dropped, as before.
    assert!(send_key(h, 0x41, false).expect("key up"));
    std::thread::sleep(Duration::from_millis(200));
    // Opens a new input stream for the same session.
    assert!(send_text(h, "x").expect("text"));
    let second = host
        .inputs
        .recv_timeout(WAIT)
        .expect("input on the new stream");

    assert_eq!(
        first.kind,
        InputKind::Key {
            vk: 0x41,
            down: true
        }
    );
    assert_eq!(second.kind, InputKind::Text { text: "x".into() });
    assert_eq!((first.seq, second.seq), (1, 2));
    assert!([&first, &second].iter().all(|e| e.session_id == SESSION_ID));
    close(h);
}

#[test]
fn buffer_too_small_reports_needed_size() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();
    host.trust(&phone_fp);
    let h = connect_to(&host, &id);
    start_streaming(h);

    let mut big = vec![0u8; 4096];
    let mut meta = [0i64; 8];
    assert_eq!(next(h, &mut big, &mut meta), VIDEO_CONFIG);

    let mut small = [0u8; 4];
    assert_eq!(next(h, &mut small, &mut meta), VIDEO_BUFFER_SMALL);
    assert_eq!(meta[0], sps_pps().len() as i64);

    // The frame was kept: a big enough buffer now gets the parameter sets.
    let n = next(h, &mut big, &mut meta);
    assert_eq!(n as usize, sps_pps().len());
    assert_eq!(meta[0], META_CODEC_CONFIG);
    close(h);
}

#[test]
fn stale_handle_is_an_error() {
    let host = FakeHost::start();
    let (id, phone_fp) = new_phone();
    host.trust(&phone_fp);
    let h = connect_to(&host, &id);
    close(h);
    assert_eq!(
        send_key(h, 0x41, true),
        Err("connection closed".to_string())
    );
}
