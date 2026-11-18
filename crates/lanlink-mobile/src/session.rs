//! Connection registry and control-plane commands.

use crate::links::{parse_addr, parse_fp};
use crate::runtime::{next_handle, runtime};
use lanlink_client::lanlink_auth::Identity;
use lanlink_client::lanlink_protocol::*;
use lanlink_client::{ClientEvent, Connection, DeviceInfo, VideoItem};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

/// Cursor shape: width, height, hotspot x, hotspot y, RGBA pixels.
pub(crate) type CursorPixels = (u16, u16, u16, u16, Vec<u8>);

/// Native session state shared by the app's threads.
pub(crate) struct Session {
    pub(crate) conn: Connection,
    pub(crate) events: Mutex<mpsc::Receiver<ClientEvent>>,
    pub(crate) video: Mutex<VideoState>,
    pub(crate) shapes: Mutex<HashMap<u32, CursorPixels>>,
}

pub(crate) struct VideoState {
    pub(crate) rx: mpsc::Receiver<VideoItem>,
    /// Frame held back while its parameter sets go out first, or because
    /// the caller's buffer was too small.
    pub(crate) pending: Option<(FrameHeader, Vec<u8>, u64)>,
    /// Parameter sets already given to the current decoder instance.
    pub(crate) sent_params: Vec<u8>,
    pub(crate) config: Option<VideoConfig>,
}

fn sessions() -> &'static Mutex<HashMap<i64, Arc<Session>>> {
    static S: OnceLock<Mutex<HashMap<i64, Arc<Session>>>> = OnceLock::new();
    S.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn session(handle: i64) -> Result<Arc<Session>, String> {
    sessions()
        .lock()
        .map_err(|_| "lock")?
        .get(&handle)
        .cloned()
        .ok_or_else(|| "connection closed".to_string())
}

/// Blocking connect to a paired host. Returns a handle.
pub fn connect(
    identity: &[u8],
    name: &str,
    model: &str,
    host: &str,
    port: i32,
    fp_hex: &str,
) -> Result<i64, String> {
    let id = Identity::from_bytes(identity).map_err(|e| e.to_string())?;
    let device = DeviceInfo {
        name: name.to_string(),
        model: model.to_string(),
        app_version: env!("CARGO_PKG_VERSION").into(),
    };
    let addr = parse_addr(host, port)?;
    let fp = parse_fp(fp_hex)?.ok_or("missing host identity")?;
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
    let handle = next_handle();
    sessions().lock().map_err(|_| "lock")?.insert(handle, s);
    Ok(handle)
}

pub fn close(h: i64) {
    let s = sessions().lock().ok().and_then(|mut m| m.remove(&h));
    if let Some(s) = s {
        s.conn.close();
    }
}

pub(crate) fn send(handle: i64, msg: ControlMsg) -> Result<(), String> {
    let s = session(handle)?;
    if s.conn.try_send(msg) {
        Ok(())
    } else {
        Err("connection busy or closed".into())
    }
}

#[allow(clippy::too_many_arguments)]
pub fn send_capabilities(
    h: i64,
    max_w: i32,
    max_h: i32,
    max_fps: i32,
    hardware: bool,
    low_latency: bool,
    display_w: i32,
    display_h: i32,
    refresh_millihz: i32,
) -> Result<(), String> {
    let cap = CodecCap {
        codec: Codec::H264,
        hardware,
        low_latency,
        max_width: max_w.clamp(0, 8192) as u16,
        max_height: max_h.clamp(0, 8192) as u16,
        max_fps: max_fps.clamp(0, 240) as u16,
    };
    send(
        h,
        lanlink_client::capabilities(
            vec![cap],
            (
                display_w.clamp(0, 65535) as u16,
                display_h.clamp(0, 65535) as u16,
            ),
            refresh_millihz.max(0) as u32,
        ),
    )
}

fn params(
    quality: i32,
    max_w: i32,
    max_h: i32,
    fps: i32,
    kbps: i32,
    max_kbps: i32,
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
pub fn start_session(
    h: i64,
    display_id: i64,
    quality: i32,
    max_w: i32,
    max_h: i32,
    fps: i32,
    kbps: i32,
    max_kbps: i32,
    view_only: bool,
) -> Result<(), String> {
    send(
        h,
        ControlMsg::SessionStart {
            display_id: display_id as u32,
            params: params(quality, max_w, max_h, fps, kbps, max_kbps),
            view_only,
        },
    )
}

pub fn set_stream_params(
    h: i64,
    quality: i32,
    max_w: i32,
    max_h: i32,
    fps: i32,
    kbps: i32,
    max_kbps: i32,
) -> Result<(), String> {
    send(
        h,
        ControlMsg::SetStreamParams {
            params: params(quality, max_w, max_h, fps, kbps, max_kbps),
        },
    )
}

pub fn end_session(h: i64) -> Result<(), String> {
    send(
        h,
        ControlMsg::SessionEnd {
            reason: ErrorCode::Normal,
        },
    )
}

pub fn select_display(h: i64, id: i64) -> Result<(), String> {
    send(
        h,
        ControlMsg::DisplaySelect {
            display_id: id as u32,
        },
    )
}

/// The phone's keyboard opened or closed. Returns false if the host is too
/// old to report the caret (protocol < 2) or the queue was full.
pub fn set_text_input(h: i64, active: bool) -> Result<bool, String> {
    Ok(session(h)?.conn.set_text_input(active))
}

pub fn bind_session(h: i64, session_id: i64) -> Result<(), String> {
    session(h)?.conn.bind_session(session_id as u32);
    Ok(())
}

pub fn send_client_stats(
    h: i64,
    received_fps_x10: i32,
    rendered_fps_x10: i32,
    decode_p50_us: i32,
    decode_p95_us: i32,
    dropped: i32,
    queue: i32,
) -> Result<(), String> {
    // Stats are best effort: a full queue is not an error.
    let _ = send(
        h,
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
}
