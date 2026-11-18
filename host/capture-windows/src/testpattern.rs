//! Synthetic moving content for benchmarks (feature `test-pattern`).
//!
//! A static desktop produces no frames, which makes latency and throughput
//! measurements meaningless. This opens a top-most window that redraws a
//! moving pattern on every timer tick, so capture sees continuous change of
//! a known size. Not compiled into the product.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, InvalidateRect, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

static FRAME: AtomicU32 = AtomicU32::new(0);

/// Running test-pattern window; closes when dropped.
pub struct TestPattern {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for TestPattern {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    // SAFETY: standard window procedure; GDI objects are deleted after use.
    unsafe {
        match msg {
            WM_TIMER => {
                FRAME.fetch_add(1, Ordering::Relaxed);
                let _ = InvalidateRect(Some(hwnd), None, false);
                LRESULT(0)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);
                let mut rc = RECT::default();
                let _ = GetClientRect(hwnd, &mut rc);
                let f = FRAME.load(Ordering::Relaxed) as i32;
                let bands = 16;
                let bw = (rc.right / bands).max(1);
                for i in 0..bands + 1 {
                    let x = (i * bw + f * 7) % (rc.right + bw) - bw;
                    let v = ((i * 37 + f * 3) % 255) as u32;
                    let brush =
                        CreateSolidBrush(COLORREF(v | ((255 - v) << 8) | (((v * 7) % 255) << 16)));
                    let r = RECT {
                        left: x,
                        top: 0,
                        right: x + bw,
                        bottom: rc.bottom,
                    };
                    FillRect(hdc, &r, brush);
                    let _ = DeleteObject(brush.into());
                }
                let _ = EndPaint(hwnd, &ps);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }
}

/// Open the pattern window at (`x`,`y`) with size `w`×`h`, redrawing every
/// `interval_ms`.
pub fn start(x: i32, y: i32, w: i32, h: i32, interval_ms: u32) -> TestPattern {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    let thread = std::thread::spawn(move || {
        // SAFETY: window creation and a message loop on this thread; the
        // window is destroyed before the thread exits.
        unsafe {
            let instance = GetModuleHandleW(None).unwrap_or_default();
            let class = WNDCLASSW {
                lpfnWndProc: Some(wndproc),
                hInstance: instance.into(),
                lpszClassName: w!("LANLinkTestPattern"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let Ok(hwnd) = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                w!("LANLinkTestPattern"),
                w!("LANLink test pattern"),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                w,
                h,
                None,
                None,
                Some(instance.into()),
                None,
            ) else {
                return;
            };
            SetTimer(Some(hwnd), 1, interval_ms.max(1), None);
            let mut msg = MSG::default();
            while !s.load(Ordering::SeqCst) {
                while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            let _ = DestroyWindow(hwnd);
        }
    });
    TestPattern {
        stop,
        thread: Some(thread),
    }
}
