//! Injects validated client input on a dedicated thread.
//!
//! The network side validates (session, sequence, view-only) and forwards
//! here over a bounded channel. Injection happens off the async runtime so
//! a slow `SendInput` can never stall networking. When the worker stops for
//! any reason the injector is dropped, which releases every held button and
//! key.

use input_windows::{Injector, InputError, Rect, Sink};
use lanlink_protocol::{ControlMsg, InputKind, NoticeCode};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Pending input operations. Beyond this the client is flooding.
pub const INPUT_QUEUE: usize = 256;

pub enum InputMsg {
    Reliable {
        seq: u32,
        kind: InputKind,
        t_recv_us: u64,
    },
    PointerAbs(u16, u16),
    PointerRel(i16, i16),
    Geometry {
        display: Rect,
        desktop: Rect,
    },
    ReleaseAll,
}

pub type SinkFactory = std::sync::Arc<dyn Fn() -> Box<dyn Sink + Send> + Send + Sync>;

pub struct InputWorker {
    tx: Option<SyncSender<InputMsg>>,
    thread: Option<JoinHandle<()>>,
}

impl InputWorker {
    pub fn spawn(
        sink: SinkFactory,
        display: Rect,
        desktop: Rect,
        out: mpsc::Sender<ControlMsg>,
    ) -> std::io::Result<Self> {
        let (tx, rx) = sync_channel(INPUT_QUEUE);
        let thread = std::thread::Builder::new()
            .name("lanlink-input".into())
            .spawn(move || run(rx, sink(), display, desktop, out))?;
        Ok(InputWorker {
            tx: Some(tx),
            thread: Some(thread),
        })
    }

    /// Returns `false` if the queue is full (the caller treats that as the
    /// client exceeding the input rate).
    pub fn send(&self, msg: InputMsg) -> bool {
        match &self.tx {
            Some(tx) => match tx.try_send(msg) {
                Ok(()) => true,
                Err(TrySendError::Full(_)) => false,
                Err(TrySendError::Disconnected(_)) => false,
            },
            None => false,
        }
    }

    /// Release everything and stop. Blocks until held input is released.
    pub fn stop(&mut self) {
        if let Some(tx) = self.tx.take() {
            // Blocking send: the release must not be lost to a full queue.
            let _ = tx.send(InputMsg::ReleaseAll);
            drop(tx);
        }
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

impl Drop for InputWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn run(
    rx: Receiver<InputMsg>,
    sink: Box<dyn Sink + Send>,
    display: Rect,
    desktop: Rect,
    out: mpsc::Sender<ControlMsg>,
) {
    let mut inj = Injector::new(sink, display, desktop);
    let mut last_rate_notice = Instant::now() - Duration::from_secs(60);
    let mut last_uipi_check = Instant::now() - Duration::from_secs(60);
    let mut uipi_blocked = false;
    // Honour the Windows lock: while the session is locked nothing but
    // releases is injected. Checked at most every 250 ms (one WTS call).
    let mut locked = capture_windows::session_locked();
    let mut last_lock_check = Instant::now();

    while let Ok(msg) = rx.recv() {
        if last_lock_check.elapsed() > Duration::from_millis(250) {
            last_lock_check = Instant::now();
            locked = capture_windows::session_locked();
        }
        let is_release = matches!(
            &msg,
            InputMsg::ReleaseAll
                | InputMsg::Geometry { .. }
                | InputMsg::Reliable {
                    kind: InputKind::Button { down: false, .. },
                    ..
                }
                | InputMsg::Reliable {
                    kind: InputKind::Key { down: false, .. },
                    ..
                }
                | InputMsg::Reliable {
                    kind: InputKind::ReleaseAll,
                    ..
                }
        );
        // A release may carry a pointer position; while locked, release
        // without moving the pointer.
        let msg = match msg {
            InputMsg::Reliable {
                seq,
                kind:
                    InputKind::Button {
                        button,
                        down: false,
                        ..
                    },
                t_recv_us,
            } if locked => InputMsg::Reliable {
                seq,
                kind: InputKind::Button {
                    button,
                    down: false,
                    pos: None,
                },
                t_recv_us,
            },
            other => other,
        };
        if locked && !is_release {
            if let InputMsg::Reliable { seq, t_recv_us, .. } = msg {
                // Still acknowledged so the phone's sequence stays in step.
                let _ = out.try_send(ControlMsg::InputAck {
                    seq,
                    t_recv_us,
                    t_inject_us: 0,
                });
            }
            continue;
        }
        let result = match msg {
            InputMsg::Reliable {
                seq,
                kind,
                t_recv_us,
            } => {
                // A press into an elevated window is silently dropped by
                // Windows; tell the user instead of failing mysteriously.
                let is_press = matches!(
                    kind,
                    InputKind::Button { down: true, .. }
                        | InputKind::Key { down: true, .. }
                        | InputKind::Text { .. }
                );
                if is_press && last_uipi_check.elapsed() > Duration::from_millis(500) {
                    last_uipi_check = Instant::now();
                    let blocked = inj.foreground_blocked();
                    if blocked && !uipi_blocked {
                        let _ = out.try_send(ControlMsg::Notice {
                            code: NoticeCode::InputBlockedByIntegrity,
                            message: "The window in front runs as administrator. Windows does not allow LANLink to control it; switch to another window on the PC.".into(),
                        });
                    }
                    uipi_blocked = blocked;
                }
                let r = inj.apply(&kind);
                let _ = out.try_send(ControlMsg::InputAck {
                    seq,
                    t_recv_us,
                    t_inject_us: lanlink_transport::monotonic_us(),
                });
                r
            }
            InputMsg::PointerAbs(x, y) => inj.pointer_abs(x, y),
            InputMsg::PointerRel(dx, dy) => inj.pointer_rel(dx as i32, dy as i32),
            InputMsg::Geometry { display, desktop } => {
                inj.set_geometry(display, desktop);
                Ok(())
            }
            InputMsg::ReleaseAll => inj.release_all(),
        };
        match result {
            Ok(()) => {}
            Err(InputError::RateLimited) => {
                if last_rate_notice.elapsed() > Duration::from_secs(5) {
                    last_rate_notice = Instant::now();
                    tracing::warn!("client exceeded the input rate limit");
                }
            }
            Err(e) => tracing::debug!(error = %e, "input not fully injected"),
        }
    }
    // Channel closed: `inj` drops here and releases held input.
}
