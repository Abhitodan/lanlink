//! Windows Graphics Capture backend.
//!
//! The system compositor copies each composed frame into a texture on *our*
//! device, so this works regardless of which adapter owns the display.
//!
//! Frame pacing: the frame pool holds a small fixed number of buffers and
//! drops new frames while it is full. To always encode the newest picture we
//! take every frame as soon as it arrives and keep only the latest one until
//! the caller's `not_before` time, instead of letting old frames sit in the
//! pool.

use crate::cursor_gdi::CursorTracker;
use crate::device::GpuDevice;
use crate::monitors::{hardware_adapters, MonitorInfo};
use crate::{Backend, Capture, CaptureError, CapturedFrame, CursorUpdate, FrameEvent};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};
use windows::core::{IInspectable, Interface};
use windows::Foundation::{TimeSpan, TypedEventHandler};
use windows::Graphics::Capture::{
    Direct3D11CaptureFrame, Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;

/// How often the pointer is sampled while waiting for frames.
const CURSOR_POLL: Duration = Duration::from_millis(8);
/// Frame-pool depth. Two is enough because frames are taken immediately.
const POOL_BUFFERS: i32 = 2;

/// Microseconds elapsed since a QPC timestamp (in QPC ticks), if plausible.
pub(crate) fn qpc_age_us(then_ticks: i64) -> Option<u32> {
    if then_ticks <= 0 {
        return None;
    }
    let (mut now, mut freq) = (0i64, 0i64);
    // SAFETY: both calls write a single i64.
    unsafe {
        QueryPerformanceCounter(&mut now).ok()?;
        QueryPerformanceFrequency(&mut freq).ok()?;
    }
    if freq <= 0 || now < then_ticks {
        return None;
    }
    let us = (now - then_ticks) as i128 * 1_000_000 / freq as i128;
    u32::try_from(us).ok()
}

/// Microseconds elapsed since a QPC timestamp expressed in 100 ns units
/// (the form WinRT `SystemRelativeTime` uses).
fn hns_age_us(then_hns: i64) -> Option<u32> {
    if then_hns <= 0 {
        return None;
    }
    let (mut now, mut freq) = (0i64, 0i64);
    // SAFETY: both calls write a single i64.
    unsafe {
        QueryPerformanceCounter(&mut now).ok()?;
        QueryPerformanceFrequency(&mut freq).ok()?;
    }
    if freq <= 0 {
        return None;
    }
    let now_hns = now as i128 * 10_000_000 / freq as i128;
    let age = now_hns - then_hns as i128;
    if age < 0 {
        return None;
    }
    u32::try_from(age / 10).ok()
}

struct Signal {
    arrived: Mutex<bool>,
    cv: Condvar,
}

pub(crate) struct WgcCapture {
    gpu: GpuDevice,
    pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    _item: GraphicsCaptureItem,
    arrived_token: i64,
    signal: Arc<Signal>,
    closed: Arc<AtomicBool>,
    size: SizeInt32,
    /// Newest frame taken from the pool but not yet returned.
    pending: Option<Direct3D11CaptureFrame>,
    /// Frames superseded before being returned, since the last return.
    superseded: u32,
    /// Frame and texture lent to the caller by the last `next_frame`.
    current: Option<(Direct3D11CaptureFrame, ID3D11Texture2D)>,
    cursor: CursorTracker,
    pending_cursor: Option<CursorUpdate>,
}

impl WgcCapture {
    pub(crate) fn new(monitor: &MonitorInfo) -> Result<Self, CaptureError> {
        match GraphicsCaptureSession::IsSupported() {
            Ok(true) => {}
            _ => {
                return Err(CaptureError::NoUsableAdapter(
                    "Windows Graphics Capture is not supported on this system".into(),
                ))
            }
        }

        // Prefer the adapter DXGI associates with the display (no
        // cross-adapter copy in the compositor), then any other hardware
        // adapter.
        let mut attempts = Vec::new();
        let mut gpu = None;
        for adapter in hardware_adapters(Some(monitor.adapter_luid))? {
            match GpuDevice::create(&adapter) {
                Ok(g) => {
                    gpu = Some(g);
                    break;
                }
                Err(e) => attempts.push(e.to_string()),
            }
        }
        let Some(gpu) = gpu else {
            return Err(CaptureError::NoUsableAdapter(attempts.join("; ")));
        };
        if !attempts.is_empty() {
            tracing::warn!(
                using = %gpu.adapter_name,
                failed = %attempts.join("; "),
                "display's own adapter is unusable; capturing through another adapter"
            );
        }

        let dxgi: IDXGIDevice = gpu
            .device
            .cast()
            .map_err(CaptureError::win("IDXGIDevice"))?;
        // SAFETY: `dxgi` is a live DXGI device.
        let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi) }
            .map_err(CaptureError::win("CreateDirect3D11DeviceFromDXGIDevice"))?;
        let d3d: IDirect3DDevice = inspectable
            .cast()
            .map_err(CaptureError::win("IDirect3DDevice"))?;

        let interop = windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
            .map_err(CaptureError::win("GraphicsCaptureItem factory"))?;
        // SAFETY: the monitor handle came from DXGI enumeration; an invalid
        // (stale) handle yields an error, not undefined behaviour.
        let item: GraphicsCaptureItem =
            unsafe { interop.CreateForMonitor(HMONITOR(monitor.hmonitor as *mut _)) }
                .map_err(|_| CaptureError::MonitorNotFound(monitor.id))?;
        let size = item.Size().map_err(CaptureError::win("item size"))?;

        let pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
            &d3d,
            DirectXPixelFormat::B8G8R8A8UIntNormalized,
            POOL_BUFFERS,
            size,
        )
        .map_err(CaptureError::win("CreateFreeThreaded"))?;

        let signal = Arc::new(Signal {
            arrived: Mutex::new(false),
            cv: Condvar::new(),
        });
        let sig = signal.clone();
        // The handler runs on a system thread-pool thread. It only flips a
        // flag and wakes the capture thread.
        let arrived_token = pool
            .FrameArrived(
                &TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |_, _| {
                    if let Ok(mut flag) = sig.arrived.lock() {
                        *flag = true;
                    }
                    sig.cv.notify_one();
                    Ok(())
                }),
            )
            .map_err(CaptureError::win("FrameArrived"))?;

        let closed = Arc::new(AtomicBool::new(false));
        let closed_flag = closed.clone();
        let sig = signal.clone();
        let _ = item.Closed(
            &TypedEventHandler::<GraphicsCaptureItem, IInspectable>::new(move |_, _| {
                closed_flag.store(true, Ordering::SeqCst);
                sig.cv.notify_one();
                Ok(())
            }),
        );

        let session = pool
            .CreateCaptureSession(&item)
            .map_err(CaptureError::win("CreateCaptureSession"))?;
        // The client draws the pointer itself, so keep it out of the video.
        let _ = session.SetIsCursorCaptureEnabled(false);
        // Removing the capture border needs Windows 11; where it is refused
        // the border simply stays visible.
        if session.SetIsBorderRequired(false).is_err() {
            tracing::info!("system capture border could not be disabled");
        }
        session
            .StartCapture()
            .map_err(CaptureError::win("StartCapture"))?;

        tracing::info!(
            display = %monitor.device_name,
            adapter = %gpu.adapter_name,
            width = size.Width,
            height = size.Height,
            "graphics capture started"
        );

        Ok(WgcCapture {
            gpu,
            pool,
            session,
            _item: item,
            arrived_token,
            signal,
            closed,
            size,
            pending: None,
            superseded: 0,
            current: None,
            cursor: CursorTracker::new(monitor.x, monitor.y, monitor.width, monitor.height),
            pending_cursor: None,
        })
    }

    /// Ask the compositor not to deliver frames more often than `interval`.
    /// Not available on every Windows build; failure is harmless because
    /// `next_frame` paces on its own.
    pub(crate) fn set_min_interval(&self, interval: Duration) {
        let _ = self.session.SetMinUpdateInterval(TimeSpan {
            Duration: (interval.as_nanos() / 100) as i64,
        });
    }

    fn drain_pool(&mut self) {
        while let Ok(frame) = self.pool.TryGetNextFrame() {
            if let Some(old) = self.pending.replace(frame) {
                self.superseded += 1;
                let _ = old.Close();
            }
        }
    }

    fn merge_cursor(&mut self, update: CursorUpdate) {
        // Keep a shape change even if a later poll only reports movement.
        let shape = update
            .shape
            .clone()
            .or_else(|| self.pending_cursor.take().and_then(|c| c.shape));
        self.pending_cursor = Some(CursorUpdate { shape, ..update });
    }
}

impl Capture for WgcCapture {
    fn backend(&self) -> Backend {
        Backend::GraphicsCapture
    }

    fn gpu(&self) -> &GpuDevice {
        &self.gpu
    }

    fn size(&self) -> (u32, u32) {
        (
            self.size.Width.max(0) as u32,
            self.size.Height.max(0) as u32,
        )
    }

    fn set_target_interval(&mut self, interval: Duration) {
        self.set_min_interval(interval);
    }

    fn next_frame(
        &mut self,
        not_before: Instant,
        timeout: Duration,
    ) -> Result<FrameEvent<'_>, CaptureError> {
        if let Some((frame, _)) = self.current.take() {
            let _ = frame.Close();
        }
        let deadline = Instant::now() + timeout;

        loop {
            if self.closed.load(Ordering::SeqCst) {
                return Err(CaptureError::AccessLost);
            }
            self.drain_pool();
            if let Some(c) = self.cursor.poll() {
                self.merge_cursor(c);
            }

            let now = Instant::now();
            if self.pending.is_some() && now >= not_before {
                break;
            }
            if self.pending.is_none() {
                if let Some(c) = self.pending_cursor.take() {
                    return Ok(FrameEvent::Cursor(c));
                }
                if now >= deadline {
                    return Ok(FrameEvent::Timeout);
                }
            }

            let wake = if self.pending.is_some() {
                not_before
            } else {
                deadline
            }
            .min(now + CURSOR_POLL);
            let wait = wake.saturating_duration_since(now);
            let guard = self
                .signal
                .arrived
                .lock()
                .map_err(|_| CaptureError::AccessLost)?;
            let (mut guard, _) = self
                .signal
                .cv
                .wait_timeout_while(guard, wait, |arrived| !*arrived)
                .map_err(|_| CaptureError::AccessLost)?;
            *guard = false;
        }

        let frame = self.pending.take().expect("checked above");
        let content = frame
            .ContentSize()
            .map_err(CaptureError::win("ContentSize"))?;
        if content.Width != self.size.Width || content.Height != self.size.Height {
            // Resolution changed. The caller rebuilds the whole pipeline for
            // the new size, so report it the same way as any mode change.
            let _ = frame.Close();
            return Err(CaptureError::AccessLost);
        }
        let age_us = frame
            .SystemRelativeTime()
            .ok()
            .and_then(|t| hns_age_us(t.Duration));
        let surface = frame
            .Surface()
            .map_err(CaptureError::win("frame surface"))?;
        let access: IDirect3DDxgiInterfaceAccess = surface
            .cast()
            .map_err(CaptureError::win("IDirect3DDxgiInterfaceAccess"))?;
        // SAFETY: the surface is a live capture surface backed by a texture.
        let texture: ID3D11Texture2D = unsafe { access.GetInterface() }.map_err(|e| {
            if self.gpu_removed() {
                CaptureError::DeviceLost
            } else {
                CaptureError::Windows {
                    context: "frame texture",
                    source: e,
                }
            }
        })?;
        let accumulated = 1 + std::mem::take(&mut self.superseded);
        let cursor = self.pending_cursor.take();
        self.current = Some((frame, texture));
        Ok(FrameEvent::Frame(CapturedFrame {
            texture: &self.current.as_ref().expect("just set").1,
            dirty_fraction: 1.0,
            accumulated_frames: accumulated,
            age_us,
            cursor,
        }))
    }
}

impl WgcCapture {
    fn gpu_removed(&self) -> bool {
        // SAFETY: simple query on a live device.
        unsafe { self.gpu.device.GetDeviceRemovedReason() }.is_err()
    }
}

impl Drop for WgcCapture {
    fn drop(&mut self) {
        let _ = self.pool.RemoveFrameArrived(self.arrived_token);
        if let Some((frame, _)) = self.current.take() {
            let _ = frame.Close();
        }
        if let Some(frame) = self.pending.take() {
            let _ = frame.Close();
        }
        let _ = self.session.Close();
        let _ = self.pool.Close();
    }
}
