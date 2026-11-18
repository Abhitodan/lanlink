//! JNI bridge between the Android app and `lanlink-client`.
//!
//! Boundary rules:
//!
//! * Every entry point catches panics; Rust errors become Java exceptions
//!   (`java.io.IOException` with a user-presentable message).
//! * Sessions live in a registry keyed by an opaque `long` handle. Kotlin
//!   never holds a raw pointer, so a stale handle is an error, not a crash.
//! * Video never crosses as a Java `byte[]`: `nextVideo` writes the access
//!   unit straight into the `MediaCodec` input buffer (a direct
//!   `ByteBuffer`), which is the one copy decoding needs anyway.
//! * Control-plane events (low rate) cross as small JSON strings.
//! * Blocking calls take a timeout and are made from Kotlin worker threads,
//!   never from the main thread.

use jni::objects::{JByteBuffer, JClass, JLongArray, JString};
use jni::sys::{jboolean, jbyteArray, jint, jlong, jstring, JNI_FALSE, JNI_TRUE};
use jni::JNIEnv;
use lanlink_client::lanlink_auth::{Fingerprint, Identity, PairingUri};
use lanlink_client::lanlink_protocol::*;
use lanlink_client::{ClientEvent, Connection, DeviceInfo, VideoItem};
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tokio::sync::mpsc;

mod json;
use json::Json;

fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("lanlink-net")
            .enable_all()
            .build()
            .expect("tokio runtime")
    })
}

// ---------------------------------------------------------------- errors

fn throw(env: &mut JNIEnv, msg: &str) {
    let _ = env.throw_new("java/io/IOException", msg);
}

/// Run `f`, converting panics and errors into a Java exception and
/// returning `default`.
fn guard<'l, T>(
    env: &mut JNIEnv<'l>,
    default: T,
    f: impl FnOnce(&mut JNIEnv<'l>) -> Result<T, String>,
) -> T {
    match std::panic::catch_unwind(AssertUnwindSafe(|| f(env))) {
        Ok(Ok(v)) => v,
        Ok(Err(msg)) => {
            throw(env, &msg);
            default
        }
        Err(_) => {
            throw(env, "internal error in LANLink native code");
            default
        }
    }
}

fn jstr(env: &mut JNIEnv, s: &JString) -> Result<String, String> {
    if s.is_null() {
        return Ok(String::new());
    }
    env.get_string(s).map(Into::into).map_err(|e| e.to_string())
}

fn bytes_in(env: &mut JNIEnv, a: &jni::objects::JByteArray) -> Result<Vec<u8>, String> {
    env.convert_byte_array(a).map_err(|e| e.to_string())
}

fn new_jstring(env: &mut JNIEnv, s: &str) -> jstring {
    env.new_string(s)
        .map(|s| s.into_raw())
        .unwrap_or(std::ptr::null_mut())
}

// ---------------------------------------------------------------- identity

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_generateIdentity<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    label: JString<'l>,
) -> jbyteArray {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let label = jstr(env, &label)?;
        let id = Identity::generate(&label).map_err(|e| e.to_string())?;
        env.byte_array_from_slice(&id.to_bytes())
            .map(|a| a.into_raw())
            .map_err(|e| e.to_string())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_identityFingerprint<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    identity: jni::objects::JByteArray<'l>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let id = Identity::from_bytes(&bytes_in(env, &identity)?).map_err(|e| e.to_string())?;
        Ok(new_jstring(env, &id.fingerprint().to_hex()))
    })
}

/// Parse a scanned QR link. Returns JSON or throws with a readable reason.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_parsePairingUri<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    uri: JString<'l>,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let s = jstr(env, &uri)?;
        let u = PairingUri::parse(&s).map_err(|e| e.to_string())?;
        // A pairing code can arrive from a web link, not only the PC's
        // screen. Never let one send this phone off the local network.
        let addresses: Vec<IpAddr> = u
            .addresses
            .iter()
            .copied()
            .filter(|a| lanlink_client::lanlink_auth::netpolicy::is_local_address(*a))
            .collect();
        if addresses.is_empty() {
            return Err(
                "This pairing code points outside your local network, so LANLink will not use it."
                    .into(),
            );
        }
        let mut j = Json::object();
        j.str("name", &wire::display_name(&u.name, 48))
            .str("fingerprint", &u.fingerprint.to_hex())
            .str("code", &u.code)
            .int("port", u.port as i64)
            .str_array("addresses", addresses.iter().map(|a| a.to_string()));
        Ok(new_jstring(env, &j.finish()))
    })
}

// ---------------------------------------------------------------- pairing

fn parse_addr(host: &str, port: jint) -> Result<SocketAddr, String> {
    let ip: IpAddr = host
        .trim()
        .parse()
        .map_err(|_| format!("\"{host}\" is not an IP address"))?;
    let port = u16::try_from(port).map_err(|_| "invalid port".to_string())?;
    if port == 0 {
        return Err("invalid port".into());
    }
    Ok(SocketAddr::new(ip, port))
}

fn parse_fp(hex: &str) -> Result<Option<Fingerprint>, String> {
    if hex.trim().is_empty() {
        return Ok(None);
    }
    Fingerprint::from_hex(hex)
        .map(Some)
        .map_err(|_| "invalid host identity".into())
}

// Two-phase pairing so the phone can show the confirmation code while the
// PC user decides: `pairStart` runs the exchange in the background and
// `pairPoll` reports progress.

struct PairJob {
    code: Mutex<Option<String>>,
    result: Mutex<Option<Result<lanlink_client::Paired, String>>>,
    /// Aborted on cancel, which drops the connection so the PC's prompt is
    /// withdrawn instead of the exchange running on unseen.
    task: Mutex<Option<tokio::task::AbortHandle>>,
}

fn pair_jobs() -> &'static Mutex<HashMap<i64, Arc<PairJob>>> {
    static JOBS: OnceLock<Mutex<HashMap<i64, Arc<PairJob>>>> = OnceLock::new();
    JOBS.get_or_init(|| Mutex::new(HashMap::new()))
}

static NEXT_HANDLE: AtomicI64 = AtomicI64::new(1);

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pairStart<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    identity: jni::objects::JByteArray<'l>,
    device_name: JString<'l>,
    device_model: JString<'l>,
    host: JString<'l>,
    port: jint,
    fingerprint_hex: JString<'l>,
    code: JString<'l>,
) -> jlong {
    guard(&mut env, 0, |env| {
        let id = Identity::from_bytes(&bytes_in(env, &identity)?).map_err(|e| e.to_string())?;
        let device = DeviceInfo {
            name: jstr(env, &device_name)?,
            model: jstr(env, &device_model)?,
            app_version: env!("CARGO_PKG_VERSION").into(),
        };
        let addr = parse_addr(&jstr(env, &host)?, port)?;
        let fp = parse_fp(&jstr(env, &fingerprint_hex)?)?;
        let code = jstr(env, &code)?;
        lanlink_client::lanlink_auth::pairing::normalize_code(&code).map_err(|e| e.to_string())?;
        let job = Arc::new(PairJob {
            code: Mutex::new(None),
            result: Mutex::new(None),
            task: Mutex::new(None),
        });
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
        pair_jobs()
            .lock()
            .map_err(|_| "lock")?
            .insert(handle, job.clone());
        let task_job = job.clone();
        let task = runtime().spawn(async move {
            let job = task_job;
            let j = job.clone();
            let r = lanlink_client::pair(&id, &device, addr, fp, &code, move |c| {
                if let Ok(mut slot) = j.code.lock() {
                    *slot = Some(c.to_string());
                }
            })
            .await
            .map_err(|e| e.to_string());
            if let Ok(mut slot) = job.result.lock() {
                *slot = Some(r);
            };
        });
        if let Ok(mut t) = job.task.lock() {
            *t = Some(task.abort_handle());
        }
        Ok(handle)
    })
}

/// JSON: `{state:"connecting"|"confirm"|"done"|"failed", code?, hostName?,
/// fingerprint?, error?}`. A finished job is removed after it is reported.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pairPoll<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let job = pair_jobs()
            .lock()
            .map_err(|_| "lock")?
            .get(&handle)
            .cloned()
            .ok_or("pairing was cancelled")?;
        let mut j = Json::object();
        let result = job.result.lock().map_err(|_| "lock")?.take();
        let code = job.code.lock().map_err(|_| "lock")?.clone();
        match result {
            Some(Ok(p)) => {
                j.str("state", "done")
                    .str("hostName", &p.host_name)
                    .str("fingerprint", &p.host_fingerprint.to_hex())
                    .str("code", &p.confirmation_code);
                pair_jobs().lock().map_err(|_| "lock")?.remove(&handle);
            }
            Some(Err(e)) => {
                j.str("state", "failed").str("error", &e);
                pair_jobs().lock().map_err(|_| "lock")?.remove(&handle);
            }
            None => match code {
                Some(c) => {
                    j.str("state", "confirm").str("code", &c);
                }
                None => {
                    j.str("state", "connecting");
                }
            },
        }
        Ok(new_jstring(env, &j.finish()))
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pairCancel<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        let job = pair_jobs().lock().ok().and_then(|mut j| j.remove(&handle));
        if let Some(t) = job.and_then(|j| j.task.lock().ok().and_then(|mut t| t.take())) {
            t.abort();
        }
        Ok(())
    })
}

// ---------------------------------------------------------------- sessions

/// Cursor shape: width, height, hotspot x, hotspot y, RGBA pixels.
type CursorPixels = (u16, u16, u16, u16, Vec<u8>);

/// Native session state shared by the app's threads.
struct Session {
    conn: Connection,
    events: Mutex<mpsc::Receiver<ClientEvent>>,
    video: Mutex<VideoState>,
    shapes: Mutex<HashMap<u32, CursorPixels>>,
}

struct VideoState {
    rx: mpsc::Receiver<VideoItem>,
    /// Frame held back while its parameter sets go out first, or because
    /// the caller's buffer was too small.
    pending: Option<(FrameHeader, Vec<u8>, u64)>,
    /// Parameter sets already given to the current decoder instance.
    sent_params: Vec<u8>,
    config: Option<VideoConfig>,
}

fn sessions() -> &'static Mutex<HashMap<i64, Arc<Session>>> {
    static S: OnceLock<Mutex<HashMap<i64, Arc<Session>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

fn session(handle: jlong) -> Result<Arc<Session>, String> {
    sessions()
        .lock()
        .map_err(|_| "lock")?
        .get(&handle)
        .cloned()
        .ok_or_else(|| "connection closed".to_string())
}

/// Blocking connect to a paired host. Returns a handle.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_connect<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    identity: jni::objects::JByteArray<'l>,
    device_name: JString<'l>,
    device_model: JString<'l>,
    host: JString<'l>,
    port: jint,
    fingerprint_hex: JString<'l>,
) -> jlong {
    guard(&mut env, 0, |env| {
        let id = Identity::from_bytes(&bytes_in(env, &identity)?).map_err(|e| e.to_string())?;
        let device = DeviceInfo {
            name: jstr(env, &device_name)?,
            model: jstr(env, &device_model)?,
            app_version: env!("CARGO_PKG_VERSION").into(),
        };
        let addr = parse_addr(&jstr(env, &host)?, port)?;
        let fp = parse_fp(&jstr(env, &fingerprint_hex)?)?.ok_or("missing host identity")?;
        let mut conn = runtime()
            .block_on(lanlink_client::connect(&id, &device, addr, fp))
            .map_err(|e| e.to_string())?;
        let events = std::mem::replace(&mut conn.events, mpsc::channel(1).1);
        let video = std::mem::replace(&mut conn.video, mpsc::channel(1).1);
        let s = Arc::new(Session {
            conn,
            events: Mutex::new(events),
            video: Mutex::new(VideoState {
                rx: video,
                pending: None,
                sent_params: Vec::new(),
                config: None,
            }),
            shapes: Mutex::new(HashMap::new()),
        });
        let handle = NEXT_HANDLE.fetch_add(1, Ordering::SeqCst);
        sessions().lock().map_err(|_| "lock")?.insert(handle, s);
        Ok(handle)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_close<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        let s = sessions().lock().map_err(|_| "lock")?.remove(&handle);
        if let Some(s) = s {
            s.conn.close();
        }
        Ok(())
    })
}

fn send(handle: jlong, msg: ControlMsg) -> Result<(), String> {
    let s = session(handle)?;
    if s.conn.try_send(msg) {
        Ok(())
    } else {
        Err("connection busy or closed".into())
    }
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendCapabilities<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    max_w: jint,
    max_h: jint,
    max_fps: jint,
    hardware: jboolean,
    low_latency: jboolean,
    display_w: jint,
    display_h: jint,
    refresh_millihz: jint,
) {
    guard(&mut env, (), |_| {
        let cap = CodecCap {
            codec: Codec::H264,
            hardware: hardware != JNI_FALSE,
            low_latency: low_latency != JNI_FALSE,
            max_width: max_w.clamp(0, 8192) as u16,
            max_height: max_h.clamp(0, 8192) as u16,
            max_fps: max_fps.clamp(0, 240) as u16,
        };
        send(
            handle,
            lanlink_client::capabilities(
                vec![cap],
                (
                    display_w.clamp(0, 65535) as u16,
                    display_h.clamp(0, 65535) as u16,
                ),
                refresh_millihz.max(0) as u32,
            ),
        )
    })
}

fn params(
    quality: jint,
    max_w: jint,
    max_h: jint,
    fps: jint,
    kbps: jint,
    max_kbps: jint,
) -> StreamParams {
    StreamParams {
        quality: QualityMode::from_u8(quality.clamp(0, 3) as u8).unwrap_or(QualityMode::Balanced),
        codec: Codec::H264,
        max_width: max_w.clamp(0, 8192) as u16,
        max_height: max_h.clamp(0, 8192) as u16,
        fps: fps.clamp(0, 240) as u16,
        bitrate_kbps: kbps.max(0) as u32,
        max_bitrate_kbps: max_kbps.max(0) as u32,
    }
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_startSession<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    display_id: jlong,
    quality: jint,
    max_w: jint,
    max_h: jint,
    fps: jint,
    kbps: jint,
    max_kbps: jint,
    view_only: jboolean,
) {
    guard(&mut env, (), |_| {
        send(
            handle,
            ControlMsg::SessionStart {
                display_id: display_id as u32,
                params: params(quality, max_w, max_h, fps, kbps, max_kbps),
                view_only: view_only != JNI_FALSE,
            },
        )
    })
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_setStreamParams<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    quality: jint,
    max_w: jint,
    max_h: jint,
    fps: jint,
    kbps: jint,
    max_kbps: jint,
) {
    guard(&mut env, (), |_| {
        send(
            handle,
            ControlMsg::SetStreamParams {
                params: params(quality, max_w, max_h, fps, kbps, max_kbps),
            },
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_endSession<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        send(
            handle,
            ControlMsg::SessionEnd {
                reason: ErrorCode::Normal,
            },
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_selectDisplay<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    display_id: jlong,
) {
    guard(&mut env, (), |_| {
        send(
            handle,
            ControlMsg::DisplaySelect {
                display_id: display_id as u32,
            },
        )
    })
}

/// The phone's keyboard opened or closed. Returns false if the host is too
/// old to report the caret (protocol < 2) or the queue was full.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_setTextInput<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    active: jboolean,
) -> jboolean {
    guard(&mut env, 0, |_| {
        Ok(session(handle)?.conn.set_text_input(active != 0) as jboolean)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_bindSession<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    session_id: jlong,
) {
    guard(&mut env, (), |_| {
        session(handle)?.conn.bind_session(session_id as u32);
        Ok(())
    })
}

#[allow(clippy::too_many_arguments)]
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendClientStats<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    received_fps_x10: jint,
    rendered_fps_x10: jint,
    decode_p50_us: jint,
    decode_p95_us: jint,
    dropped: jint,
    queue: jint,
) {
    guard(&mut env, (), |_| {
        let _ = send(
            handle,
            ControlMsg::ClientStats(ClientStats {
                received_fps_x10: received_fps_x10.clamp(0, 65535) as u16,
                rendered_fps_x10: rendered_fps_x10.clamp(0, 65535) as u16,
                decode_us_p50: decode_p50_us.max(0) as u32,
                decode_us_p95: decode_p95_us.max(0) as u32,
                frames_dropped: dropped.max(0) as u32,
                decoder_queue_depth: queue.clamp(0, 255) as u8,
            }),
        );
        Ok(())
    })
}

// ---------------------------------------------------------------- events

fn event_json(s: &Session, ev: ClientEvent) -> String {
    let mut j = Json::object();
    match ev {
        ClientEvent::DisplayList(list) => {
            j.str("type", "displays");
            let items: Vec<String> = list
                .iter()
                .map(|d| {
                    let mut o = Json::object();
                    o.int("id", d.id as i64)
                        .str("name", &d.name)
                        .int("width", d.width as i64)
                        .int("height", d.height as i64)
                        .int("x", d.x as i64)
                        .int("y", d.y as i64)
                        .bool("primary", d.primary)
                        .int("refreshMilliHz", d.refresh_millihz as i64)
                        .int("scale", d.scale_percent as i64)
                        .bool("hdr", d.hdr);
                    o.finish()
                })
                .collect();
            j.raw_array("displays", items);
        }
        ClientEvent::SessionStarted {
            session_id,
            display_id,
            view_only,
            encoder,
            capture,
        } => {
            j.str("type", "sessionStarted")
                .int("sessionId", session_id as i64)
                .int("displayId", display_id as i64)
                .bool("viewOnly", view_only)
                .str("encoder", &encoder)
                .str("capture", &capture);
        }
        ClientEvent::SessionEnded(code) => {
            j.str("type", "sessionEnded")
                .int("code", code.as_u16() as i64);
        }
        ClientEvent::Stats(st) => {
            j.str("type", "stats")
                .int("capturedFpsX10", st.captured_fps_x10 as i64)
                .int("encodedFpsX10", st.encoded_fps_x10 as i64)
                .int("bitrateKbps", st.bitrate_kbps as i64)
                .int("targetKbps", st.target_bitrate_kbps as i64)
                .int("encodeP50", st.encode_us_p50 as i64)
                .int("encodeP95", st.encode_us_p95 as i64)
                .int("droppedEncode", st.dropped_before_encode as i64)
                .int("droppedSend", st.dropped_before_send as i64)
                .int("queue", st.send_queue_depth as i64)
                .int("rttUs", st.rtt_us as i64)
                .int("lostPackets", st.lost_packets as i64);
            let c = &s.conn.counters;
            j.int("framesReceived", c.received.load(Ordering::Relaxed) as i64)
                .int("framesSkipped", c.skipped.load(Ordering::Relaxed) as i64);
        }
        ClientEvent::CursorShape {
            shape_id,
            width,
            height,
            hot_x,
            hot_y,
            rgba,
        } => {
            if let Ok(mut m) = s.shapes.lock() {
                if m.len() > 32 {
                    m.clear();
                }
                m.insert(shape_id, (width, height, hot_x, hot_y, rgba));
            }
            j.str("type", "cursorShape")
                .int("id", shape_id as i64)
                .int("width", width as i64)
                .int("height", height as i64)
                .int("hotX", hot_x as i64)
                .int("hotY", hot_y as i64);
        }
        ClientEvent::CursorPos {
            x,
            y,
            visible,
            shape_id,
        } => {
            j.str("type", "cursor")
                .int("x", x as i64)
                .int("y", y as i64)
                .bool("visible", visible)
                .int("shape", shape_id as i64);
        }
        ClientEvent::Caret {
            visible,
            x,
            y,
            w,
            h,
        } => {
            j.str("type", "caret")
                .bool("visible", visible)
                .int("x", x as i64)
                .int("y", y as i64)
                .int("w", w as i64)
                .int("h", h as i64);
        }
        ClientEvent::InputAck { seq } => {
            j.str("type", "inputAck").int("seq", seq as i64);
        }
        ClientEvent::Rtt { rtt_us, offset_us } => {
            j.str("type", "rtt")
                .int("rttUs", rtt_us as i64)
                .int("offsetUs", offset_us);
        }
        ClientEvent::Notice(code, msg) => {
            j.str("type", "notice")
                .int("code", code.as_u16() as i64)
                .str("message", &msg);
        }
        ClientEvent::Error(code, msg) => {
            j.str("type", "error")
                .int("code", code.as_u16() as i64)
                .str("message", &msg);
        }
        ClientEvent::Closed(reason) => {
            j.str("type", "closed").str("message", &reason);
        }
    }
    j.finish()
}

/// Wait up to `timeout_ms` for the next control event. Returns JSON or
/// null on timeout.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_pollEvent<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    timeout_ms: jint,
) -> jstring {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let s = session(handle)?;
        let ev = {
            let mut rx = s.events.lock().map_err(|_| "lock")?;
            let d = Duration::from_millis(timeout_ms.clamp(0, 10_000) as u64);
            runtime().block_on(async { tokio::time::timeout(d, rx.recv()).await })
        };
        match ev {
            Ok(Some(ev)) => Ok(new_jstring(env, &event_json(&s, ev))),
            Ok(None) => {
                let mut j = Json::object();
                j.str("type", "closed").str("message", "connection closed");
                Ok(new_jstring(env, &j.finish()))
            }
            Err(_) => Ok(std::ptr::null_mut()),
        }
    })
}

/// RGBA pixels of a cursor shape announced by a `cursorShape` event.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_cursorPixels<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    shape_id: jlong,
) -> jbyteArray {
    guard(&mut env, std::ptr::null_mut(), |env| {
        let s = session(handle)?;
        let shapes = s.shapes.lock().map_err(|_| "lock")?;
        match shapes.get(&(shape_id as u32)) {
            Some((_, _, _, _, rgba)) => env
                .byte_array_from_slice(rgba)
                .map(|a| a.into_raw())
                .map_err(|e| e.to_string()),
            None => Ok(std::ptr::null_mut()),
        }
    })
}

// ---------------------------------------------------------------- video

/// Return codes of `nextVideo`.
pub const VIDEO_TIMEOUT: jint = -1;
pub const VIDEO_CONFIG: jint = -2;
pub const VIDEO_CLOSED: jint = -3;
pub const VIDEO_BUFFER_SMALL: jint = -4;

/// `meta` flags bit set.
pub const META_KEYFRAME: i64 = 1;
pub const META_CODEC_CONFIG: i64 = 2;
pub const META_DISCONTINUITY: i64 = 4;

/// Extract SPS and PPS NAL units (with start codes) from an access unit.
fn parameter_sets(au: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = au.len();
    let mut starts = Vec::new();
    while i + 3 <= n {
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else if i + 4 <= n && au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 0 && au[i + 3] == 1 {
            starts.push((i, i + 4));
            i += 4;
        } else {
            i += 1;
        }
    }
    for (k, (_, payload)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|s| s.0).unwrap_or(n);
        if *payload < end {
            let ty = au[*payload] & 0x1f;
            if ty == 7 || ty == 8 {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(&au[*payload..end]);
            }
        }
    }
    out
}

/// Write the next decoder input into `buffer` (a direct ByteBuffer, e.g. a
/// MediaCodec input buffer).
///
/// Returns the byte count (> 0) and fills `meta` =
/// `[flags, ptsUs(capture time, host clock), seq, recvUs, captureToSendUs]`,
/// or a negative code:
/// * `-1` timeout;
/// * `-2` new stream configuration; `meta` = `[width, height, fps,
///   configId, displayId, sourceWidth, sourceHeight]`; the caller must
///   (re)configure its decoder and then call `decoderReset`;
/// * `-3` session or connection ended;
/// * `-4` buffer too small; `meta[0]` = bytes needed. The frame is kept.
///
/// Before the first keyframe of each decoder instance (and whenever the
/// parameter sets change), a separate codec-config buffer containing only
/// SPS and PPS is returned with `META_CODEC_CONFIG`.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_nextVideo<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    buffer: JByteBuffer<'l>,
    meta: JLongArray<'l>,
    timeout_ms: jint,
) -> jint {
    guard(&mut env, VIDEO_CLOSED, |env| {
        let s = session(handle)?;
        let mut v = s.video.lock().map_err(|_| "lock")?;
        let cap = env
            .get_direct_buffer_capacity(&buffer)
            .map_err(|e| e.to_string())?;
        let ptr = env
            .get_direct_buffer_address(&buffer)
            .map_err(|e| e.to_string())?;

        if v.pending.is_none() {
            let d = Duration::from_millis(timeout_ms.clamp(0, 5_000) as u64);
            let item = runtime().block_on(async { tokio::time::timeout(d, v.rx.recv()).await });
            match item {
                Err(_) => return Ok(VIDEO_TIMEOUT),
                Ok(None) => return Ok(VIDEO_CLOSED),
                Ok(Some(VideoItem::Config(c))) => {
                    v.config = Some(c);
                    v.sent_params.clear();
                    let m = [
                        c.width as i64,
                        c.height as i64,
                        c.fps as i64,
                        c.config_id as i64,
                        c.display_id as i64,
                        c.source_width as i64,
                        c.source_height as i64,
                    ];
                    env.set_long_array_region(&meta, 0, &m)
                        .map_err(|e| e.to_string())?;
                    return Ok(VIDEO_CONFIG);
                }
                Ok(Some(VideoItem::Frame {
                    header,
                    data,
                    recv_us,
                })) => {
                    v.pending = Some((header, data, recv_us));
                }
            }
        }

        let (header, data, recv_us) = v.pending.as_ref().expect("set above");
        let header = *header;
        let recv_us = *recv_us;
        if header.is_keyframe() {
            let ps = parameter_sets(data);
            if !ps.is_empty() && ps != v.sent_params {
                if ps.len() > cap {
                    env.set_long_array_region(&meta, 0, &[ps.len() as i64])
                        .map_err(|e| e.to_string())?;
                    return Ok(VIDEO_BUFFER_SMALL);
                }
                // SAFETY: `ptr` is the start of a direct buffer of `cap`
                // bytes and `ps.len() <= cap`.
                unsafe { std::ptr::copy_nonoverlapping(ps.as_ptr(), ptr, ps.len()) };
                let m = [
                    META_CODEC_CONFIG,
                    header.capture_us as i64,
                    header.seq as i64,
                    recv_us as i64,
                    0,
                ];
                env.set_long_array_region(&meta, 0, &m)
                    .map_err(|e| e.to_string())?;
                v.sent_params = ps.clone();
                return Ok(ps.len() as jint);
            }
        }
        let len = v.pending.as_ref().map(|p| p.1.len()).unwrap_or(0);
        if len > cap {
            env.set_long_array_region(&meta, 0, &[len as i64])
                .map_err(|e| e.to_string())?;
            return Ok(VIDEO_BUFFER_SMALL);
        }
        let (header, data, _) = v.pending.take().expect("checked");
        // SAFETY: as above, `data.len() <= cap`.
        unsafe { std::ptr::copy_nonoverlapping(data.as_ptr(), ptr, data.len()) };
        let mut flags = 0;
        if header.is_keyframe() {
            flags |= META_KEYFRAME;
        }
        if header.flags & lanlink_client::lanlink_protocol::video::FLAG_DISCONTINUITY != 0 {
            flags |= META_DISCONTINUITY;
        }
        let m = [
            flags,
            header.capture_us as i64,
            header.seq as i64,
            recv_us as i64,
            header.send_us.saturating_sub(header.capture_us) as i64,
        ];
        env.set_long_array_region(&meta, 0, &m)
            .map_err(|e| e.to_string())?;
        Ok(data.len() as jint)
    })
}

/// The app created a new decoder (first start, surface recreated, error
/// recovery): resend parameter sets and start from a fresh keyframe.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_decoderReset<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        let s = session(handle)?;
        if let Ok(mut v) = s.video.lock() {
            v.sent_params.clear();
            // A held delta frame cannot be decoded by a fresh decoder.
            if v.pending.as_ref().is_some_and(|p| !p.0.is_keyframe()) {
                v.pending = None;
            }
        }
        let _ = s.conn.try_send(ControlMsg::KeyframeRequest);
        Ok(())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_requestKeyframe<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| send(handle, ControlMsg::KeyframeRequest))
}

/// Monotonic microseconds on the same clock as `recvUs` in video metadata.
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_nowUs<'l>(
    _env: JNIEnv<'l>,
    _c: JClass<'l>,
) -> jlong {
    lanlink_transport::monotonic_us() as jlong
}

// ---------------------------------------------------------------- input

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendPointer<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    x: jint,
    y: jint,
) {
    guard(&mut env, (), |_| {
        session(handle)?
            .conn
            .send_pointer(x.clamp(0, 65535) as u16, y.clamp(0, 65535) as u16);
        Ok(())
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendPointerRel<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    dx: jint,
    dy: jint,
) {
    guard(&mut env, (), |_| {
        session(handle)?.conn.send_pointer_rel(
            dx.clamp(-32768, 32767) as i16,
            dy.clamp(-32768, 32767) as i16,
        );
        Ok(())
    })
}

fn input(handle: jlong, kind: InputKind) -> Result<jboolean, String> {
    Ok(if session(handle)?.conn.send_input(kind) {
        JNI_TRUE
    } else {
        JNI_FALSE
    })
}

/// `button`: 0 left, 1 right, 2 middle. `x`/`y` < 0 means "no position".
#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendButton<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    button: jint,
    down: jboolean,
    x: jint,
    y: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        let button = match button {
            0 => MouseButton::Left,
            1 => MouseButton::Right,
            2 => MouseButton::Middle,
            3 => MouseButton::X1,
            4 => MouseButton::X2,
            _ => return Err("invalid button".into()),
        };
        let pos = (x >= 0 && y >= 0).then(|| (x.min(65535) as u16, y.min(65535) as u16));
        input(
            handle,
            InputKind::Button {
                button,
                down: down != JNI_FALSE,
                pos,
            },
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendScroll<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    dx: jint,
    dy: jint,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        input(
            handle,
            InputKind::Scroll {
                dx: dx.clamp(-1200, 1200) as i16,
                dy: dy.clamp(-1200, 1200) as i16,
            },
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendKey<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    vk: jint,
    down: jboolean,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |_| {
        if !(1..=0xFE).contains(&vk) {
            return Err("invalid key".into());
        }
        input(
            handle,
            InputKind::Key {
                vk: vk as u16,
                down: down != JNI_FALSE,
            },
        )
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_sendText<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
    text: JString<'l>,
) -> jboolean {
    guard(&mut env, JNI_FALSE, |env| {
        let t = jstr(env, &text)?;
        let t: String = t.chars().filter(|c| *c != '\0').collect();
        if t.is_empty() {
            return Ok(JNI_TRUE);
        }
        // Split long pastes into protocol-sized pieces on char boundaries.
        let mut ok = JNI_TRUE;
        let mut chunk = String::new();
        for c in t.chars() {
            if chunk.len() + c.len_utf8() > limits::MAX_TEXT_INPUT {
                ok &= input(
                    handle,
                    InputKind::Text {
                        text: std::mem::take(&mut chunk),
                    },
                )?;
            }
            chunk.push(c);
        }
        if !chunk.is_empty() {
            ok &= input(handle, InputKind::Text { text: chunk })?;
        }
        Ok(ok)
    })
}

#[no_mangle]
pub extern "system" fn Java_com_lanlink_app_core_NativeBridge_releaseAll<'l>(
    mut env: JNIEnv<'l>,
    _c: JClass<'l>,
    handle: jlong,
) {
    guard(&mut env, (), |_| {
        input(handle, InputKind::ReleaseAll).map(|_| ())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_parameter_sets() {
        let au = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9,
        ];
        assert_eq!(
            parameter_sets(&au),
            [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3]
        );
        assert!(parameter_sets(&[0, 0, 1, 0x41, 5]).is_empty());
        assert!(parameter_sets(&[]).is_empty());
        assert!(parameter_sets(&[0, 0, 1]).is_empty());
    }
}
