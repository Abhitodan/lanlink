//! The media pipeline: capture → encode → bounded hand-off to the network.
//!
//! Runs on its own OS thread because capture and encoder objects are COM
//! objects bound to the thread that created them, and both block.
//!
//! Latency rules enforced here:
//!
//! * Capture is paced by `not_before`; images composed in between are
//!   superseded, never queued.
//! * The encoder has no input queue; if it is busy the frame is dropped.
//! * Encoded frames go into a bounded channel (`VIDEO_QUEUE`). If it is full
//!   the frame is dropped, and every following delta frame too, until the
//!   encoder delivers a new self-contained keyframe, which is marked as a
//!   discontinuity. A receiver therefore never sees a delta frame whose
//!   reference it lacks.

use crate::quality::{self, StreamPlan};
use capture_windows::{BackendPreference, Capture, CaptureError, FrameEvent, MonitorInfo};
use encoder::{EncodedFrame, EncoderConfig, EncoderPreference, Submit, Tuning, VideoEncoder};
use input_windows::Rect;
use lanlink_protocol::{
    Codec, CodecCap, ErrorCode, NoticeCode, QualityMode, StreamParams, VideoConfig,
};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Encoded frames that may wait for the network. Small on purpose: every
/// slot is up to one frame time of added latency.
pub const VIDEO_QUEUE: usize = 3;
const CAPTURE_WAIT: Duration = Duration::from_millis(50);
const RETRY_DELAY: Duration = Duration::from_millis(500);
const DESKTOP_CHECK: Duration = Duration::from_millis(500);

pub enum MediaCmd {
    SelectDisplay(MonitorInfo),
    SetParams(StreamParams),
    Keyframe,
    SetBitrate(u32),
    Stop,
}

pub struct FrameOut {
    pub seq: u32,
    pub config_id: u16,
    pub keyframe: bool,
    pub discontinuity: bool,
    pub capture_us: u64,
    pub encode_us: u32,
    pub data: Vec<u8>,
}

pub enum VideoOut {
    Config(VideoConfig),
    Frame(FrameOut),
}

pub enum MediaEvent {
    Started {
        encoder: String,
        hardware: bool,
        capture: String,
        plan: StreamPlan,
        display: Rect,
        monitor: MonitorInfo,
    },
    CursorPos {
        x: u16,
        y: u16,
        visible: bool,
        shape_id: u32,
    },
    CursorShape {
        shape_id: u32,
        shape: capture_windows::CursorShape,
    },
    Notice(NoticeCode, String),
    Fatal(ErrorCode, String),
}

/// Counters read once per second by the connection task.
#[derive(Default)]
pub struct MediaStats {
    pub captured: AtomicU64,
    pub encoded: AtomicU64,
    pub bytes: AtomicU64,
    pub dropped_before_encode: AtomicU64,
    pub dropped_before_send: AtomicU64,
    pub target_bitrate: AtomicU32,
    encode_us: Mutex<VecDeque<u32>>,
}

impl MediaStats {
    fn push_encode_us(&self, v: u32) {
        if let Ok(mut q) = self.encode_us.lock() {
            if q.len() >= 512 {
                q.pop_front();
            }
            q.push_back(v);
        }
    }

    /// p50/p95 of encode times since the last call, then reset.
    pub fn take_encode_percentiles(&self) -> (u32, u32) {
        let mut v: Vec<u32> = match self.encode_us.lock() {
            Ok(mut q) => q.drain(..).collect(),
            Err(_) => return (0, 0),
        };
        if v.is_empty() {
            return (0, 0);
        }
        v.sort_unstable();
        (v[(v.len() - 1) / 2], v[(v.len() - 1) * 95 / 100])
    }
}

pub struct MediaHandle {
    cmd: Sender<MediaCmd>,
    thread: Option<JoinHandle<()>>,
    pub stats: Arc<MediaStats>,
}

impl MediaHandle {
    pub fn send(&self, cmd: MediaCmd) {
        let _ = self.cmd.send(cmd);
    }

    /// A sender other tasks can use to request keyframes. The pipeline stops
    /// on an explicit `Stop` (sent on drop), not when senders go away.
    pub fn requester(&self) -> Sender<MediaCmd> {
        self.cmd.clone()
    }
}

impl Drop for MediaHandle {
    fn drop(&mut self) {
        let _ = self.cmd.send(MediaCmd::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

pub struct MediaParams {
    pub monitor: MonitorInfo,
    pub params: StreamParams,
    pub caps: Vec<CodecCap>,
    pub backend: BackendPreference,
    pub encoder: EncoderPreference,
}

pub fn spawn(
    p: MediaParams,
    video_tx: mpsc::Sender<VideoOut>,
    events_tx: mpsc::Sender<MediaEvent>,
) -> std::io::Result<MediaHandle> {
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let stats = Arc::new(MediaStats::default());
    let st = stats.clone();
    let thread = std::thread::Builder::new()
        .name("lanlink-media".into())
        .spawn(move || {
            Pipeline::run(p, cmd_rx, video_tx, events_tx, st);
        })?;
    Ok(MediaHandle {
        cmd: cmd_tx,
        thread: Some(thread),
        stats,
    })
}

fn clock() -> u64 {
    lanlink_transport::monotonic_us()
}

/// State shared between the pipeline thread and the encoder's output
/// callback (which runs on the encoder's own thread).
struct SinkShared {
    seq: AtomicU32,
    /// Drop delta frames until the next self-contained keyframe.
    awaiting_key: AtomicBool,
    /// A frame was dropped since the last one sent.
    dropped: AtomicBool,
    /// Ask the pipeline to force a keyframe on its next submit.
    force_key: AtomicBool,
}

struct Active {
    capture: Box<dyn Capture>,
    encoder: VideoEncoder,
}

struct Pipeline {
    monitor: MonitorInfo,
    params: StreamParams,
    caps: Vec<CodecCap>,
    backend: BackendPreference,
    encoder_pref: EncoderPreference,
    video_tx: mpsc::Sender<VideoOut>,
    events_tx: mpsc::Sender<MediaEvent>,
    stats: Arc<MediaStats>,
    shared: Arc<SinkShared>,
    config_id: u16,
    plan: Option<StreamPlan>,
    bitrate_override: Option<u32>,
    last_shape: Option<Vec<u8>>,
    shape_id: u32,
    last_cursor: Option<(u16, u16, bool)>,
}

enum Flow {
    Continue,
    Stop,
}

impl Pipeline {
    fn run(
        p: MediaParams,
        cmd_rx: Receiver<MediaCmd>,
        video_tx: mpsc::Sender<VideoOut>,
        events_tx: mpsc::Sender<MediaEvent>,
        stats: Arc<MediaStats>,
    ) {
        capture_windows::init_thread();
        if let Err(e) = encoder::init() {
            let _ =
                events_tx.blocking_send(MediaEvent::Fatal(ErrorCode::EncoderFailed, e.to_string()));
            return;
        }
        let mut pl = Pipeline {
            monitor: p.monitor,
            params: p.params,
            caps: p.caps,
            backend: p.backend,
            encoder_pref: p.encoder,
            video_tx,
            events_tx,
            stats,
            shared: Arc::new(SinkShared {
                seq: AtomicU32::new(0),
                awaiting_key: AtomicBool::new(true),
                dropped: AtomicBool::new(false),
                force_key: AtomicBool::new(true),
            }),
            config_id: 0,
            plan: None,
            bitrate_override: None,
            last_shape: None,
            shape_id: 0,
            last_cursor: None,
        };
        pl.main_loop(&cmd_rx);
        tracing::info!("media pipeline stopped");
    }

    fn notice(&self, code: NoticeCode, msg: impl Into<String>) {
        let _ = self
            .events_tx
            .blocking_send(MediaEvent::Notice(code, msg.into()));
    }

    fn fatal(&self, code: ErrorCode, msg: impl Into<String>) {
        let _ = self
            .events_tx
            .blocking_send(MediaEvent::Fatal(code, msg.into()));
    }

    /// Re-resolve the monitor after topology changes (its size or adapter
    /// may have changed, or it may be gone).
    fn refresh_monitor(&mut self) {
        if let Ok(list) = capture_windows::enumerate_monitors() {
            if let Some(m) = list.iter().find(|m| m.id == self.monitor.id) {
                self.monitor = m.clone();
            } else if let Some(primary) = list.iter().find(|m| m.primary) {
                self.notice(
                    NoticeCode::CaptureRecovering,
                    "The selected display was disconnected; showing the primary display.",
                );
                self.monitor = primary.clone();
            }
        }
    }

    fn build(&mut self) -> Result<Active, (bool, String)> {
        let capture = capture_windows::open(&self.monitor, self.backend)
            .map_err(|e| (e.is_recoverable(), e.to_string()))?;
        let src = capture.size();
        let refresh = (self.monitor.refresh_millihz / 1000).max(1);
        let plan = quality::plan(&self.params, &self.caps, src, refresh);
        let mut capture = capture;
        capture.set_target_interval(Duration::from_secs_f64(1.0 / plan.fps as f64));

        self.config_id = self.config_id.wrapping_add(1);
        let config_id = self.config_id;
        let shared = self.shared.clone();
        let tx = self.video_tx.clone();
        let stats = self.stats.clone();
        let sink = Box::new(move |f: EncodedFrame| {
            Self::on_encoded(&shared, &tx, &stats, config_id, f);
        });
        let gpu = capture.gpu().clone();
        let bitrate = self
            .bitrate_override
            .unwrap_or(plan.bitrate_bps)
            .min(plan.bitrate_bps);
        let encoder = VideoEncoder::new(
            &gpu.device,
            &gpu.context,
            gpu.adapter_luid,
            src,
            EncoderConfig {
                width: plan.width,
                height: plan.height,
                fps: plan.fps,
                bitrate_bps: bitrate,
                tuning: match self.params.quality {
                    QualityMode::Smooth => Tuning::Speed,
                    QualityMode::Sharp => Tuning::Quality,
                    _ => Tuning::Balanced,
                },
                preference: self.encoder_pref,
                clock,
            },
            sink,
        )
        .map_err(|e| (false, e.to_string()))?;

        self.stats.target_bitrate.store(bitrate, Ordering::Relaxed);
        // The config record must precede every frame of this configuration.
        let cfg = VideoConfig {
            config_id,
            codec: Codec::H264,
            width: plan.width as u16,
            height: plan.height as u16,
            fps: plan.fps as u16,
            display_id: self.monitor.id,
            source_width: src.0,
            source_height: src.1,
            rotation: 0,
        };
        if self.video_tx.blocking_send(VideoOut::Config(cfg)).is_err() {
            return Err((false, "network side closed".into()));
        }
        self.shared.awaiting_key.store(true, Ordering::SeqCst);
        self.shared.force_key.store(true, Ordering::SeqCst);
        self.last_shape = None;
        self.last_cursor = None;
        self.plan = Some(plan);

        let _ = self.events_tx.blocking_send(MediaEvent::Started {
            encoder: encoder.name().to_string(),
            hardware: encoder.is_hardware(),
            capture: capture.backend().name().to_string(),
            plan,
            display: Rect {
                x: self.monitor.x,
                y: self.monitor.y,
                width: self.monitor.width,
                height: self.monitor.height,
            },
            monitor: self.monitor.clone(),
        });
        Ok(Active { capture, encoder })
    }

    /// Encoder output callback (encoder thread).
    fn on_encoded(
        shared: &SinkShared,
        tx: &mpsc::Sender<VideoOut>,
        stats: &MediaStats,
        config_id: u16,
        f: EncodedFrame,
    ) {
        let seq = shared.seq.fetch_add(1, Ordering::SeqCst).wrapping_add(1);
        stats.encoded.fetch_add(1, Ordering::Relaxed);
        stats
            .bytes
            .fetch_add(f.data.len() as u64, Ordering::Relaxed);
        stats.push_encode_us(f.encode_us);
        if shared.awaiting_key.load(Ordering::SeqCst) && !f.self_contained {
            stats.dropped_before_send.fetch_add(1, Ordering::Relaxed);
            shared.dropped.store(true, Ordering::SeqCst);
            shared.force_key.store(true, Ordering::SeqCst);
            return;
        }
        let discontinuity = f.self_contained && shared.dropped.load(Ordering::SeqCst);
        let out = VideoOut::Frame(FrameOut {
            seq,
            config_id,
            keyframe: f.self_contained,
            discontinuity,
            capture_us: f.capture_us,
            encode_us: f.encode_us,
            data: f.data,
        });
        match tx.try_send(out) {
            Ok(()) => {
                shared.dropped.store(false, Ordering::SeqCst);
                if f.self_contained {
                    shared.awaiting_key.store(false, Ordering::SeqCst);
                }
            }
            Err(_) => {
                stats.dropped_before_send.fetch_add(1, Ordering::Relaxed);
                shared.dropped.store(true, Ordering::SeqCst);
                shared.awaiting_key.store(true, Ordering::SeqCst);
                shared.force_key.store(true, Ordering::SeqCst);
            }
        }
    }

    fn handle_cmds(&mut self, rx: &Receiver<MediaCmd>, active: &mut Option<Active>) -> Flow {
        loop {
            match rx.try_recv() {
                Ok(MediaCmd::Stop) | Err(TryRecvError::Disconnected) => return Flow::Stop,
                Err(TryRecvError::Empty) => return Flow::Continue,
                Ok(MediaCmd::Keyframe) => self.shared.force_key.store(true, Ordering::SeqCst),
                Ok(MediaCmd::SetBitrate(bps)) => {
                    self.bitrate_override = Some(bps);
                    if let Some(a) = active.as_mut() {
                        if a.encoder.set_bitrate(bps) {
                            self.stats.target_bitrate.store(bps, Ordering::Relaxed);
                        }
                    }
                }
                Ok(MediaCmd::SelectDisplay(m)) => {
                    self.monitor = m;
                    *active = None;
                }
                Ok(MediaCmd::SetParams(p)) => {
                    self.params = p;
                    self.bitrate_override = None;
                    *active = None;
                }
            }
        }
    }

    fn cursor(&mut self, c: capture_windows::CursorUpdate, size: (u32, u32)) {
        if let Some(shape) = c.shape {
            if self.last_shape.as_deref() != Some(shape.rgba.as_slice()) {
                self.shape_id = self.shape_id.wrapping_add(1);
                self.last_shape = Some(shape.rgba.clone());
                let _ = self.events_tx.blocking_send(MediaEvent::CursorShape {
                    shape_id: self.shape_id,
                    shape,
                });
            }
        }
        let x = lanlink_protocol::normalize(c.x as i64, size.0);
        let y = lanlink_protocol::normalize(c.y as i64, size.1);
        let state = (x, y, c.visible);
        if self.last_cursor != Some(state) {
            self.last_cursor = Some(state);
            // Positions are superseded by newer ones; dropping is fine.
            let _ = self.events_tx.try_send(MediaEvent::CursorPos {
                x,
                y,
                visible: c.visible,
                shape_id: self.shape_id,
            });
        }
    }

    fn main_loop(&mut self, rx: &Receiver<MediaCmd>) {
        let mut active: Option<Active> = None;
        let mut next_slot = Instant::now();
        let mut desktop_ok = true;
        let mut last_desktop_check = Instant::now();
        let mut recovering = false;

        loop {
            if let Flow::Stop = self.handle_cmds(rx, &mut active) {
                return;
            }

            if last_desktop_check.elapsed() >= DESKTOP_CHECK {
                last_desktop_check = Instant::now();
                let ok = capture_windows::interactive_desktop_active();
                if ok != desktop_ok {
                    desktop_ok = ok;
                    if ok {
                        self.notice(
                            NoticeCode::CaptureResumed,
                            "The desktop is available again.",
                        );
                        self.shared.force_key.store(true, Ordering::SeqCst);
                    } else {
                        self.notice(
                            NoticeCode::DesktopUnavailable,
                            "The PC is locked or showing a secure prompt. LANLink cannot show or control it until it is unlocked on the PC.",
                        );
                    }
                }
            }

            if active.is_none() {
                self.refresh_monitor();
                match self.build() {
                    Ok(a) => {
                        if recovering {
                            recovering = false;
                            self.notice(NoticeCode::CaptureResumed, "Screen capture resumed.");
                        }
                        active = Some(a);
                        next_slot = Instant::now();
                    }
                    Err((true, msg)) => {
                        if !recovering {
                            recovering = true;
                            self.notice(NoticeCode::CaptureRecovering, msg);
                        }
                        std::thread::sleep(RETRY_DELAY);
                        continue;
                    }
                    Err((false, msg)) => {
                        tracing::error!(error = %msg, "media pipeline cannot start");
                        let code = if msg.contains("encoder") || msg.contains("H.264") {
                            ErrorCode::EncoderFailed
                        } else {
                            ErrorCode::CaptureFailed
                        };
                        self.fatal(code, msg);
                        return;
                    }
                }
            }

            let a = active.as_mut().expect("built above");
            if a.encoder.failed() {
                self.notice(
                    NoticeCode::EncoderFallback,
                    "The video encoder failed and is being restarted.",
                );
                active = None;
                continue;
            }
            let plan = self.plan.expect("set by build");
            let interval = Duration::from_secs_f64(1.0 / plan.fps as f64);
            let size = a.capture.size();

            let event = a.capture.next_frame(next_slot, CAPTURE_WAIT);
            let mut cursor_update = None;
            let mut rebuild = false;
            match event {
                Ok(FrameEvent::Frame(f)) => {
                    self.stats.captured.fetch_add(1, Ordering::Relaxed);
                    cursor_update = f.cursor.clone();
                    let capture_us = clock().saturating_sub(f.age_us.unwrap_or(0) as u64);
                    let key = self.shared.force_key.swap(false, Ordering::SeqCst);
                    match a.encoder.submit(f.texture, capture_us, key) {
                        Ok(Submit::Accepted) => {}
                        Ok(Submit::Busy) => {
                            self.stats
                                .dropped_before_encode
                                .fetch_add(1, Ordering::Relaxed);
                            if key {
                                self.shared.force_key.store(true, Ordering::SeqCst);
                            }
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "encoder submit failed");
                            rebuild = true;
                        }
                    }
                    let now = Instant::now();
                    next_slot = (next_slot + interval).max(now);
                }
                Ok(FrameEvent::Cursor(c)) => cursor_update = Some(c),
                Ok(FrameEvent::Timeout) => {
                    // A keyframe is owed but the screen is static: encode the
                    // last picture again.
                    if self.shared.force_key.load(Ordering::SeqCst) {
                        if let Ok(Submit::Accepted) = a.encoder.resubmit_last(clock()) {
                            self.shared.force_key.store(false, Ordering::SeqCst);
                        }
                    }
                }
                Err(e) => {
                    let recoverable = e.is_recoverable();
                    tracing::warn!(error = %e, "capture interrupted");
                    if recoverable {
                        recovering = true;
                        let (code, msg) = match e {
                            CaptureError::DesktopUnavailable => (
                                NoticeCode::DesktopUnavailable,
                                "The PC is locked or showing a secure prompt.".to_string(),
                            ),
                            other => (NoticeCode::CaptureRecovering, other.to_string()),
                        };
                        self.notice(code, msg);
                        rebuild = true;
                    } else {
                        self.fatal(ErrorCode::CaptureFailed, e.to_string());
                        return;
                    }
                }
            }
            if let Some(c) = cursor_update {
                self.cursor(c, size);
            }
            if rebuild {
                active = None;
            }
        }
    }
}
