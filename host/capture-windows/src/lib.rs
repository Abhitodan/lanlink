//! Windows screen capture for LANLink.
//!
//! * [`enumerate_monitors`] lists capturable displays with geometry, DPI,
//!   rotation, HDR state and the GPU adapter DXGI associates with them.
//! * [`open`] starts capturing one display and returns a [`Capture`]. Two
//!   backends implement it:
//!   - [`Backend::GraphicsCapture`] (Windows Graphics Capture). Works with a
//!     D3D11 device on *any* adapter, which matters on hybrid-graphics
//!     laptops where the adapter that owns the display may be unusable.
//!   - [`Backend::DesktopDuplication`] (DXGI). Needs a device on the adapter
//!     that owns the display; reports dirty rectangles and the pointer
//!     without polling.
//! * [`cursor`] converts pointer shapes to straight-alpha RGBA for the
//!   client-side cursor.
//!
//! Both backends wake on new frames; neither spins, and a static desktop
//! produces no frames at all.
//!
//! Thread affinity: a `Capture` must be created and used on one thread, and
//! that thread must have called [`init_thread`]. The D3D11 device has
//! multithread protection enabled because the Media Foundation encoder uses
//! it from its own threads.
//!
//! This crate never reads pixels back to the CPU and never bypasses capture
//! protections: protected content appears black, and the secure desktop
//! (UAC, lock screen) is reported as unavailable rather than worked around.

#![cfg(windows)]

pub mod cursor;
mod cursor_gdi;
mod device;
mod duplication;
mod monitors;
#[cfg(feature = "test-pattern")]
pub mod testpattern;
mod wgc;

pub use cursor::CursorShape;
pub use cursor_gdi::CursorTracker;
pub use device::GpuDevice;
pub use monitors::{enumerate_monitors, init_process_dpi_awareness, MonitorInfo};

use std::time::{Duration, Instant};
use thiserror::Error;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;

/// Capture implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    GraphicsCapture,
    DesktopDuplication,
}

impl Backend {
    pub fn name(self) -> &'static str {
        match self {
            Backend::GraphicsCapture => "Windows Graphics Capture",
            Backend::DesktopDuplication => "DXGI Desktop Duplication",
        }
    }
}

/// Which backend [`open`] should use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackendPreference {
    /// Graphics Capture, falling back to Desktop Duplication.
    #[default]
    Auto,
    Only(Backend),
}

/// Pointer state reported by a capture backend.
#[derive(Debug, Clone)]
pub struct CursorUpdate {
    /// Hotspot position in pixels relative to the captured display.
    pub x: i32,
    pub y: i32,
    pub visible: bool,
    /// Present only when the shape changed.
    pub shape: Option<CursorShape>,
}

/// A captured desktop image. The texture is only valid until the next call
/// to [`Capture::next_frame`]; copy it (on the GPU) before then.
pub struct CapturedFrame<'a> {
    pub texture: &'a ID3D11Texture2D,
    /// Fraction of the display that changed since the previous frame, in
    /// `0.0..=1.0`; `1.0` when the backend does not report it.
    pub dirty_fraction: f32,
    /// Desktop updates coalesced into (or skipped before) this frame (≥ 1).
    pub accumulated_frames: u32,
    /// Time between Windows composing this frame and it being returned
    /// here, when the backend reports composition time.
    pub age_us: Option<u32>,
    pub cursor: Option<CursorUpdate>,
}

pub enum FrameEvent<'a> {
    /// Nothing changed within the timeout.
    Timeout,
    /// The desktop image changed.
    Frame(CapturedFrame<'a>),
    /// Only the pointer moved or changed shape.
    Cursor(CursorUpdate),
}

/// An active capture of one display.
pub trait Capture {
    fn backend(&self) -> Backend;

    /// The device the returned textures live on.
    fn gpu(&self) -> &GpuDevice;

    /// Size of the captured surface in pixels.
    fn size(&self) -> (u32, u32);

    /// Return the newest desktop image available at or after `not_before`,
    /// waiting at most `timeout` from now.
    ///
    /// `not_before` is how the caller paces capture below the display's
    /// refresh rate: images composed before it are superseded, never queued,
    /// so a slow consumer always gets the latest picture.
    fn next_frame(
        &mut self,
        not_before: Instant,
        timeout: Duration,
    ) -> Result<FrameEvent<'_>, CaptureError>;

    /// Hint the interval at which the caller intends to take frames, so the
    /// backend can avoid producing frames nobody will look at. Optional.
    fn set_target_interval(&mut self, _interval: Duration) {}
}

#[derive(Debug, Error)]
pub enum CaptureError {
    /// The capture is no longer valid (mode change, display removed,
    /// desktop switch). Open a new one.
    #[error("capture access lost; capture must be reopened")]
    AccessLost,
    /// The interactive desktop cannot be captured right now: the session is
    /// locked or showing the secure desktop. Retry later; never worked around.
    #[error("the interactive desktop is not available for capture")]
    DesktopUnavailable,
    /// The GPU device was removed or reset. Recreate everything on it.
    #[error("graphics device lost")]
    DeviceLost,
    /// No adapter could both create a device and capture the display.
    /// The message lists what each attempt reported.
    #[error("this display cannot be captured ({0})")]
    NoUsableAdapter(String),
    #[error("display {0:#x} not found")]
    MonitorNotFound(u32),
    #[error("{context}: {source}")]
    Windows {
        context: &'static str,
        #[source]
        source: windows::core::Error,
    },
}

impl CaptureError {
    pub(crate) fn win(context: &'static str) -> impl FnOnce(windows::core::Error) -> CaptureError {
        move |source| CaptureError::Windows { context, source }
    }

    /// Whether reopening the capture (possibly after a delay) can help.
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            CaptureError::AccessLost | CaptureError::DesktopUnavailable | CaptureError::DeviceLost
        )
    }
}

/// Prepare the process for capture. Call before [`open`]; safe to call from
/// any thread, any number of times.
///
/// This pins the COM multithreaded apartment for the lifetime of the
/// process instead of initialising it per thread. Per-thread initialisation
/// tears the apartment down when the last such thread exits, which
/// invalidates the WinRT activation factories the `windows` crate caches
/// process-wide; the next capture on a new thread then crashes. (Found by
/// the live tests, which open captures on successive threads.)
pub fn init_thread() {
    use std::sync::Once;
    use windows::Win32::System::Com::CoIncrementMTAUsage;
    static PIN_MTA: Once = Once::new();
    PIN_MTA.call_once(|| {
        // SAFETY: no preconditions. The returned cookie is intentionally
        // never released: the apartment must outlive every capture.
        unsafe {
            if let Err(e) = CoIncrementMTAUsage() {
                tracing::error!(error = %e, "could not pin the COM multithreaded apartment");
            }
        }
    });
}

/// Start capturing `monitor`.
pub fn open(
    monitor: &MonitorInfo,
    preference: BackendPreference,
) -> Result<Box<dyn Capture>, CaptureError> {
    match preference {
        BackendPreference::Only(Backend::GraphicsCapture) => {
            Ok(Box::new(wgc::WgcCapture::new(monitor)?))
        }
        BackendPreference::Only(Backend::DesktopDuplication) => {
            Ok(Box::new(duplication::Duplicator::new(monitor)?))
        }
        BackendPreference::Auto => match wgc::WgcCapture::new(monitor) {
            Ok(c) => Ok(Box::new(c)),
            Err(wgc_err) => {
                tracing::warn!(error = %wgc_err, "Graphics Capture unavailable; trying Desktop Duplication");
                match duplication::Duplicator::new(monitor) {
                    Ok(c) => Ok(Box::new(c)),
                    Err(dd_err) if dd_err.is_recoverable() => Err(dd_err),
                    Err(dd_err) => Err(CaptureError::NoUsableAdapter(format!(
                        "Graphics Capture: {wgc_err}; Desktop Duplication: {dd_err}"
                    ))),
                }
            }
        },
    }
}

/// Whether the Windows session is locked.
///
/// On Windows 10/11 the lock screen runs on the *normal* desktop (only the
/// credential prompt switches desktops), so a desktop-name check alone does
/// not detect it. The session's lock flag does. (Found when a capture on a
/// locked machine returned a single stale frame while the desktop check
/// still said "available".)
pub fn session_locked() -> bool {
    use windows::Win32::System::RemoteDesktop::{
        WTSFreeMemory, WTSQuerySessionInformationW, WTSSessionInfoEx, WTSINFOEXW,
        WTS_CURRENT_SERVER_HANDLE, WTS_CURRENT_SESSION,
    };
    const WTS_SESSIONSTATE_LOCK: i32 = 0;
    let mut buf = windows::core::PWSTR::null();
    let mut len = 0u32;
    // SAFETY: on success WTS allocates `buf` with at least a WTSINFOEXW,
    // which we read (checking the size) and free exactly once.
    unsafe {
        if WTSQuerySessionInformationW(
            Some(WTS_CURRENT_SERVER_HANDLE),
            WTS_CURRENT_SESSION,
            WTSSessionInfoEx,
            &mut buf,
            &mut len,
        )
        .is_err()
            || buf.is_null()
        {
            return false;
        }
        let locked = if len as usize >= std::mem::size_of::<WTSINFOEXW>() {
            let info = &*(buf.0 as *const WTSINFOEXW);
            info.Level == 1 && info.Data.WTSInfoExLevel1.SessionFlags == WTS_SESSIONSTATE_LOCK
        } else {
            false
        };
        WTSFreeMemory(buf.0 as *mut _);
        locked
    }
}

/// Whether the interactive desktop can be shown and controlled: `false`
/// while the session is locked or a UAC prompt (secure desktop) is up.
pub fn interactive_desktop_active() -> bool {
    if session_locked() {
        return false;
    }
    use windows::Win32::System::StationsAndDesktops::{
        CloseDesktop, GetUserObjectInformationW, OpenInputDesktop, DESKTOP_CONTROL_FLAGS,
        DESKTOP_READOBJECTS, UOI_NAME,
    };
    // SAFETY: the name buffer is sized and its byte length passed; the
    // desktop handle is closed on every path.
    unsafe {
        let Ok(desktop) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        else {
            // The secure desktop cannot be opened from a normal process.
            return false;
        };
        let mut name = [0u16; 64];
        let mut needed = 0u32;
        let ok = GetUserObjectInformationW(
            windows::Win32::Foundation::HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            (name.len() * 2) as u32,
            Some(&mut needed),
        )
        .is_ok();
        let _ = CloseDesktop(desktop);
        if !ok {
            return false;
        }
        let len = name.iter().position(|c| *c == 0).unwrap_or(name.len());
        String::from_utf16_lossy(&name[..len]).eq_ignore_ascii_case("Default")
    }
}
