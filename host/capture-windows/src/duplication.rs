//! DXGI Desktop Duplication backend.
//!
//! Requires a D3D11 device on the adapter that owns the display. On
//! hybrid-graphics laptops that adapter may be unusable; [`crate::open`]
//! then uses Graphics Capture instead.

use crate::cursor::{self};
use crate::device::GpuDevice;
use crate::monitors::{adapter_and_output, MonitorInfo};
use crate::{Backend, Capture, CaptureError, CapturedFrame, CursorUpdate, FrameEvent};
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::{E_ACCESSDENIED, RECT};
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::Win32::Graphics::Dxgi::{
    IDXGIOutput1, IDXGIOutput5, IDXGIOutputDuplication, IDXGIResource, DXGI_ERROR_ACCESS_LOST,
    DXGI_ERROR_DEVICE_REMOVED, DXGI_ERROR_DEVICE_RESET, DXGI_ERROR_INVALID_CALL,
    DXGI_ERROR_SESSION_DISCONNECTED, DXGI_ERROR_UNSUPPORTED, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTDUPL_POINTER_SHAPE_INFO,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR, DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR,
    DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME,
};

pub(crate) struct Duplicator {
    gpu: GpuDevice,
    duplication: IDXGIOutputDuplication,
    width: u32,
    height: u32,
    frame_held: bool,
    current: Option<ID3D11Texture2D>,
    shape_buf: Vec<u8>,
    rect_buf: Vec<RECT>,
    /// Hotspot of the current shape; frame info reports the image origin.
    hot: (i32, i32),
}

fn map_dup_error(context: &'static str, e: windows::core::Error) -> CaptureError {
    let code = e.code();
    if code == DXGI_ERROR_ACCESS_LOST || code == DXGI_ERROR_INVALID_CALL {
        CaptureError::AccessLost
    } else if code == E_ACCESSDENIED || code == DXGI_ERROR_SESSION_DISCONNECTED {
        CaptureError::DesktopUnavailable
    } else if code == DXGI_ERROR_DEVICE_REMOVED || code == DXGI_ERROR_DEVICE_RESET {
        CaptureError::DeviceLost
    } else {
        CaptureError::Windows { context, source: e }
    }
}

impl Duplicator {
    pub(crate) fn new(monitor: &MonitorInfo) -> Result<Self, CaptureError> {
        let (adapter, output) = adapter_and_output(monitor)?;
        let gpu = GpuDevice::create(&adapter)?;

        // SAFETY: `output` and the device are live COM objects.
        let duplication = unsafe {
            match output.cast::<IDXGIOutput5>() {
                // Ask for 8-bit BGRA explicitly. On an HDR display this makes
                // Windows hand us its SDR rendition instead of scRGB float.
                Ok(o5) => {
                    match o5.DuplicateOutput1(&gpu.device, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]) {
                        Ok(d) => Ok(d),
                        Err(e) if e.code() == DXGI_ERROR_UNSUPPORTED => output
                            .cast::<IDXGIOutput1>()
                            .and_then(|o1| o1.DuplicateOutput(&gpu.device)),
                        Err(e) => Err(e),
                    }
                }
                Err(_) => output
                    .cast::<IDXGIOutput1>()
                    .and_then(|o1| o1.DuplicateOutput(&gpu.device)),
            }
        }
        .map_err(|e| map_dup_error("DuplicateOutput", e))?;

        // SAFETY: fills a plain struct.
        let desc = unsafe { duplication.GetDesc() };
        tracing::info!(
            display = %monitor.device_name,
            adapter = %gpu.adapter_name,
            width = desc.ModeDesc.Width,
            height = desc.ModeDesc.Height,
            "desktop duplication started"
        );

        Ok(Duplicator {
            gpu,
            duplication,
            width: desc.ModeDesc.Width,
            height: desc.ModeDesc.Height,
            frame_held: false,
            current: None,
            shape_buf: Vec::new(),
            rect_buf: Vec::new(),
            hot: (0, 0),
        })
    }

    fn read_cursor(&mut self, info: &DXGI_OUTDUPL_FRAME_INFO) -> Option<CursorUpdate> {
        if info.LastMouseUpdateTime == 0 {
            return None;
        }
        let mut shape = None;
        if info.PointerShapeBufferSize > 0 {
            let size = info.PointerShapeBufferSize as usize;
            if self.shape_buf.len() < size {
                self.shape_buf.resize(size, 0);
            }
            let mut required = 0u32;
            let mut si = DXGI_OUTDUPL_POINTER_SHAPE_INFO::default();
            // SAFETY: the buffer is at least `size` bytes, as passed.
            let ok = unsafe {
                self.duplication.GetFramePointerShape(
                    size as u32,
                    self.shape_buf.as_mut_ptr().cast(),
                    &mut required,
                    &mut si,
                )
            }
            .is_ok();
            if ok {
                let data = &self.shape_buf[..size];
                let (hx, hy) = (si.HotSpot.x.max(0) as u32, si.HotSpot.y.max(0) as u32);
                let kind = si.Type as i32;
                shape = if kind == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR.0 {
                    cursor::from_color(data, si.Width, si.Height, si.Pitch, hx, hy)
                } else if kind == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME.0 {
                    cursor::from_monochrome(data, si.Width, si.Height, si.Pitch, hx, hy)
                } else if kind == DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR.0 {
                    cursor::from_masked_color(data, si.Width, si.Height, si.Pitch, hx, hy)
                } else {
                    None
                };
                if let Some(s) = &shape {
                    self.hot = (s.hot_x as i32, s.hot_y as i32);
                }
            }
        }
        Some(CursorUpdate {
            x: info.PointerPosition.Position.x + self.hot.0,
            y: info.PointerPosition.Position.y + self.hot.1,
            visible: info.PointerPosition.Visible.as_bool(),
            shape,
        })
    }

    fn dirty_fraction(&mut self, info: &DXGI_OUTDUPL_FRAME_INFO) -> f32 {
        if info.TotalMetadataBufferSize == 0 {
            return 1.0;
        }
        let cap = info.TotalMetadataBufferSize as usize / std::mem::size_of::<RECT>() + 1;
        if self.rect_buf.len() < cap {
            self.rect_buf.resize(cap, RECT::default());
        }
        let mut required = 0u32;
        // SAFETY: the buffer holds `cap` RECTs and its byte size is passed.
        let ok = unsafe {
            self.duplication.GetFrameDirtyRects(
                (self.rect_buf.len() * std::mem::size_of::<RECT>()) as u32,
                self.rect_buf.as_mut_ptr(),
                &mut required,
            )
        }
        .is_ok();
        if !ok {
            return 1.0;
        }
        let n = (required as usize / std::mem::size_of::<RECT>()).min(self.rect_buf.len());
        let total = (self.width as u64 * self.height as u64).max(1);
        let mut area = 0u64;
        for r in &self.rect_buf[..n] {
            let w = (r.right - r.left).max(0) as u64;
            let h = (r.bottom - r.top).max(0) as u64;
            area += w * h;
        }
        // Rectangles may overlap; clamp rather than compute a union.
        (area as f32 / total as f32).min(1.0)
    }
}

impl Capture for Duplicator {
    fn backend(&self) -> Backend {
        Backend::DesktopDuplication
    }

    fn gpu(&self) -> &GpuDevice {
        &self.gpu
    }

    fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn next_frame(
        &mut self,
        not_before: Instant,
        timeout: Duration,
    ) -> Result<FrameEvent<'_>, CaptureError> {
        self.current = None;
        // While no frame is held Windows keeps coalescing desktop updates,
        // so sleeping here and acquiring afterwards yields the newest image.
        let now = Instant::now();
        if not_before > now {
            std::thread::sleep(not_before - now);
        }
        if self.frame_held {
            self.frame_held = false;
            // SAFETY: a frame is currently held.
            unsafe { self.duplication.ReleaseFrame() }
                .map_err(|e| map_dup_error("ReleaseFrame", e))?;
        }

        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut resource: Option<IDXGIResource> = None;
        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        // SAFETY: out-pointers reference live locals.
        match unsafe {
            self.duplication
                .AcquireNextFrame(timeout_ms, &mut info, &mut resource)
        } {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(FrameEvent::Timeout),
            Err(e) => return Err(map_dup_error("AcquireNextFrame", e)),
        }
        self.frame_held = true;

        let cursor = self.read_cursor(&info);
        let has_image = info.LastPresentTime != 0 && info.AccumulatedFrames > 0;
        if !has_image {
            return Ok(match cursor {
                Some(c) => FrameEvent::Cursor(c),
                None => FrameEvent::Timeout,
            });
        }

        let dirty_fraction = self.dirty_fraction(&info);
        let age_us = crate::wgc::qpc_age_us(info.LastPresentTime);
        let texture: ID3D11Texture2D = resource
            .ok_or(CaptureError::AccessLost)?
            .cast()
            .map_err(CaptureError::win("frame texture"))?;
        self.current = Some(texture);
        Ok(FrameEvent::Frame(CapturedFrame {
            texture: self.current.as_ref().expect("just set"),
            dirty_fraction,
            accumulated_frames: info.AccumulatedFrames,
            age_us,
            cursor,
        }))
    }
}

impl Drop for Duplicator {
    fn drop(&mut self) {
        self.current = None;
        if self.frame_held {
            // SAFETY: a frame is held; failure during teardown is harmless.
            unsafe {
                let _ = self.duplication.ReleaseFrame();
            }
        }
    }
}
