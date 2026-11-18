//! The on-screen "this PC is being controlled" indicator and the emergency
//! stop hotkey.
//!
//! * A thin coloured border around the primary display (click-through).
//! * A small bar at the top centre: "<phone> is controlling this PC" with a
//!   Stop button.
//! * Ctrl+Alt+Shift+S ends the session from anywhere.
//!
//! Both windows are excluded from screen capture
//! (`WDA_EXCLUDEFROMCAPTURE`), so the phone never sees them, and they never
//! take keyboard focus. Runs on its own thread with its own message loop.

use crate::AppMsg;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::*;

/// What the indicator should show; `None` hides it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Indicator {
    pub device: String,
    pub view_only: bool,
}

static WANTED: Mutex<Option<Indicator>> = Mutex::new(None);
static SHOWN_TEXT: Mutex<String> = Mutex::new(String::new());
static THREAD: OnceLock<u32> = OnceLock::new();
static SINK: OnceLock<(Sender<AppMsg>, eframe::egui::Context)> = OnceLock::new();
static BORDER: AtomicIsize = AtomicIsize::new(0);
static BAR: AtomicIsize = AtomicIsize::new(0);

const HOTKEY_ID: i32 = 0x4C4C; // "LL"
pub const HOTKEY_LABEL: &str = "Ctrl+Alt+Shift+S";

const DANGER: (u8, u8, u8) = (0xE0, 0x6C, 0x5F);
const NIGHT: (u8, u8, u8) = (0x15, 0x18, 0x1C);
const MIST: (u8, u8, u8) = (0xE9, 0xE6, 0xDF);
const STOP_BG: (u8, u8, u8) = (0xA2, 0x3B, 0x33);

fn rgb((r, g, b): (u8, u8, u8)) -> COLORREF {
    COLORREF(r as u32 | (g as u32) << 8 | (b as u32) << 16)
}

fn hwnd(a: &AtomicIsize) -> HWND {
    HWND(a.load(Ordering::Relaxed) as *mut _)
}

fn scale() -> f32 {
    // SAFETY: no preconditions.
    (unsafe { GetDpiForSystem() } as f32 / 96.0).max(1.0)
}

/// Show, update or hide the indicator. Safe from any thread.
pub fn set(indicator: Option<Indicator>) {
    if let Ok(mut w) = WANTED.lock() {
        *w = indicator;
    }
    if let Some(tid) = THREAD.get() {
        // SAFETY: posting to a thread id we own.
        unsafe {
            let _ = PostThreadMessageW(*tid, WM_APP, WPARAM(0), LPARAM(0));
        }
    }
}

fn stop_requested() {
    if let Some((tx, ctx)) = SINK.get() {
        let _ = tx.send(AppMsg::StopRequested);
        ctx.request_repaint();
    }
}

pub fn spawn(tx: Sender<AppMsg>, ctx: eframe::egui::Context) {
    let _ = SINK.set((tx, ctx));
    let r = std::thread::Builder::new()
        .name("lanlink-overlay".into())
        .spawn(|| {
            // SAFETY: Win32 window creation and a message loop on this thread;
            // handles are only used on this thread.
            unsafe { run() }
        });
    if let Err(e) = r {
        tracing::error!(error = %e, "could not start the indicator thread");
    }
}

unsafe fn run() {
    let _ = THREAD.set(GetCurrentThreadId());
    let instance = match GetModuleHandleW(None) {
        Ok(m) => windows::Win32::Foundation::HINSTANCE(m.0),
        Err(_) => return,
    };
    let border_class = w!("LANLinkIndicatorBorder");
    let bar_class = w!("LANLinkIndicatorBar");
    let wc = WNDCLASSW {
        lpfnWndProc: Some(border_proc),
        hInstance: instance,
        lpszClassName: border_class,
        ..Default::default()
    };
    RegisterClassW(&wc);
    let wc = WNDCLASSW {
        lpfnWndProc: Some(bar_proc),
        hInstance: instance,
        lpszClassName: bar_class,
        hCursor: LoadCursorW(None, IDC_HAND).unwrap_or_default(),
        ..Default::default()
    };
    RegisterClassW(&wc);

    let common = WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE | WS_EX_LAYERED;
    let border = CreateWindowExW(
        common | WS_EX_TRANSPARENT,
        border_class,
        w!("LANLink indicator"),
        WS_POPUP,
        0,
        0,
        1,
        1,
        None,
        None,
        Some(instance),
        None,
    );
    let bar = CreateWindowExW(
        common,
        bar_class,
        w!("LANLink"),
        WS_POPUP,
        0,
        0,
        1,
        1,
        None,
        None,
        Some(instance),
        None,
    );
    for h in [&border, &bar].into_iter().flatten() {
        // Keep the indicator out of every capture, including ours.
        if SetWindowDisplayAffinity(*h, WDA_EXCLUDEFROMCAPTURE).is_err() {
            tracing::warn!("could not exclude the indicator from capture");
        }
    }
    if let Ok(b) = border {
        BORDER.store(b.0 as isize, Ordering::Relaxed);
    }
    if let Ok(b) = bar {
        BAR.store(b.0 as isize, Ordering::Relaxed);
        let _ = SetLayeredWindowAttributes(b, COLORREF(0), 245, LWA_ALPHA);
    }

    let hotkey = RegisterHotKey(
        None,
        HOTKEY_ID,
        MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
        'S' as u32,
    );
    if let Some((tx, ctx)) = SINK.get() {
        let _ = tx.send(AppMsg::Hotkey(hotkey.is_ok()));
        ctx.request_repaint();
    }

    // A state requested before this thread was ready (no thread id to post
    // to yet) is applied now.
    let mut shown: Option<Indicator> = WANTED.lock().ok().and_then(|w| w.clone());
    if shown.is_some() {
        apply(shown.as_ref());
    }
    let mut msg = MSG::default();
    while GetMessageW(&mut msg, None, 0, 0).as_bool() {
        if msg.hwnd.is_invalid() {
            match msg.message {
                WM_HOTKEY if msg.wParam.0 as i32 == HOTKEY_ID => stop_requested(),
                WM_APP => {
                    let wanted = WANTED.lock().ok().and_then(|w| w.clone());
                    if wanted != shown {
                        apply(wanted.as_ref());
                        shown = wanted;
                    }
                }
                _ => {}
            }
            continue;
        }
        let _ = TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

unsafe fn apply(ind: Option<&Indicator>) {
    let border = hwnd(&BORDER);
    let bar = hwnd(&BAR);
    let Some(ind) = ind else {
        let _ = ShowWindow(border, SW_HIDE);
        let _ = ShowWindow(bar, SW_HIDE);
        return;
    };
    if let Ok(mut t) = SHOWN_TEXT.lock() {
        *t = format!(
            "{} is {} this PC",
            ind.device,
            if ind.view_only {
                "viewing"
            } else {
                "controlling"
            }
        );
    }
    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let s = scale();
    if !border.is_invalid() {
        paint_border(border, sw, sh, (3.0 * s).round() as i32);
    }
    if !bar.is_invalid() {
        let w = (430.0 * s) as i32;
        let h = (38.0 * s) as i32;
        let r = h;
        let rgn = CreateRoundRectRgn(0, 0, w + 1, h + 1, r, r);
        SetWindowRgn(bar, Some(rgn), false);
        let _ = SetWindowPos(
            bar,
            Some(HWND_TOPMOST),
            (sw - w) / 2,
            (10.0 * s) as i32,
            w,
            h,
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
        let _ = InvalidateRect(Some(bar), None, true);
    }
    // (Re)assert capture exclusion once the windows are visible.
    for h in [border, bar] {
        if !h.is_invalid() {
            if let Err(e) = SetWindowDisplayAffinity(h, WDA_EXCLUDEFROMCAPTURE) {
                tracing::warn!(error = %e, "could not exclude the indicator from capture");
            }
        }
    }
}

/// A premultiplied-alpha frame around the display; the inside is fully
/// transparent (and the window is click-through).
unsafe fn paint_border(hwnd: HWND, w: i32, h: i32, t: i32) {
    let screen = GetDC(None);
    let mem = CreateCompatibleDC(Some(screen));
    let bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
    let Ok(dib) = CreateDIBSection(Some(mem), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) else {
        let _ = DeleteDC(mem);
        ReleaseDC(None, screen);
        return;
    };
    let old = SelectObject(mem, dib.into());
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (w * h) as usize);
    let a = 230u32;
    let (r, g, b) = DANGER;
    let pre = |c: u8| (c as u32 * a / 255) & 0xFF;
    let color = (a << 24) | (pre(r) << 16) | (pre(g) << 8) | pre(b);
    for y in 0..h {
        let row = &mut px[(y * w) as usize..((y + 1) * w) as usize];
        if y < t || y >= h - t {
            row.fill(color);
        } else {
            row[..t as usize].fill(color);
            row[(w - t) as usize..].fill(color);
        }
    }
    let blend = BLENDFUNCTION {
        BlendOp: AC_SRC_OVER as u8,
        BlendFlags: 0,
        SourceConstantAlpha: 255,
        AlphaFormat: AC_SRC_ALPHA as u8,
    };
    let _ = UpdateLayeredWindow(
        hwnd,
        Some(screen),
        Some(&POINT { x: 0, y: 0 }),
        Some(&SIZE { cx: w, cy: h }),
        Some(mem),
        Some(&POINT { x: 0, y: 0 }),
        COLORREF(0),
        Some(&blend),
        ULW_ALPHA,
    );
    SelectObject(mem, old);
    let _ = DeleteObject(dib.into());
    let _ = DeleteDC(mem);
    ReleaseDC(None, screen);
    let _ = SetWindowPos(
        hwnd,
        Some(HWND_TOPMOST),
        0,
        0,
        w,
        h,
        SWP_NOACTIVATE | SWP_SHOWWINDOW,
    );
}

unsafe extern "system" fn border_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    DefWindowProcW(hwnd, msg, wp, lp)
}

fn stop_rect(client: &RECT) -> RECT {
    let s = scale();
    let pad = (5.0 * s) as i32;
    RECT {
        left: client.right - (78.0 * s) as i32,
        top: client.top + pad,
        right: client.right - pad,
        bottom: client.bottom - pad,
    }
}

unsafe extern "system" fn bar_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_MOUSEACTIVATE => LRESULT(MA_NOACTIVATE as isize),
        WM_LBUTTONUP => {
            let x = (lp.0 & 0xFFFF) as i16 as i32;
            let y = ((lp.0 >> 16) & 0xFFFF) as i16 as i32;
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let stop = stop_rect(&rc);
            if x >= stop.left && x < stop.right && y >= stop.top && y < stop.bottom {
                stop_requested();
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            // Windows sends a first paint while the bar is still hidden and
            // 1×1 px, before any text exists. Drawing then handed DrawTextW an
            // empty buffer (a dangling pointer) and crashed in user32.
            let text_now = SHOWN_TEXT.lock().map(|t| t.clone()).unwrap_or_default();
            if !IsWindowVisible(hwnd).as_bool() || text_now.is_empty() {
                let _ = EndPaint(hwnd, &ps);
                return LRESULT(0);
            }
            let mut rc = RECT::default();
            let _ = GetClientRect(hwnd, &mut rc);
            let s = scale();
            let bg = CreateSolidBrush(rgb(NIGHT));
            FillRect(hdc, &rc, bg);
            let _ = DeleteObject(bg.into());

            // Pulsing-red dot (static here; the colour carries the meaning).
            let dot = CreateSolidBrush(rgb(DANGER));
            let old_brush = SelectObject(hdc, dot.into());
            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            let d = (10.0 * s) as i32;
            let cy = (rc.bottom - rc.top) / 2;
            let dx = (16.0 * s) as i32;
            let _ = Ellipse(hdc, dx, cy - d / 2, dx + d, cy + d / 2);

            let font = CreateFontW(
                (-15.0 * s) as i32,
                0,
                0,
                0,
                FW_SEMIBOLD.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                (DEFAULT_PITCH.0 | FF_SWISS.0) as u32,
                w!("Segoe UI"),
            );
            let old_font = SelectObject(hdc, font.into());
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, rgb(MIST));
            let stop = stop_rect(&rc);
            let mut text_rc = RECT {
                left: dx + d + (10.0 * s) as i32,
                top: rc.top,
                right: stop.left - (8.0 * s) as i32,
                bottom: rc.bottom,
            };
            let mut text: Vec<u16> = SHOWN_TEXT
                .lock()
                .map(|t| t.clone())
                .unwrap_or_default()
                .encode_utf16()
                .collect();
            DrawTextW(
                hdc,
                &mut text,
                &mut text_rc,
                DT_SINGLELINE | DT_VCENTER | DT_END_ELLIPSIS | DT_NOPREFIX,
            );

            let stop_brush = CreateSolidBrush(rgb(STOP_BG));
            SelectObject(hdc, stop_brush.into());
            let r = stop.bottom - stop.top;
            let _ = RoundRect(hdc, stop.left, stop.top, stop.right, stop.bottom, r, r);
            SetTextColor(hdc, rgb((0xFF, 0xFF, 0xFF)));
            let mut label: Vec<u16> = "Stop".encode_utf16().collect();
            let mut stop_text = stop;
            DrawTextW(
                hdc,
                &mut label,
                &mut stop_text,
                DT_SINGLELINE | DT_VCENTER | DT_CENTER | DT_NOPREFIX,
            );

            SelectObject(hdc, old_font);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
            let _ = DeleteObject(font.into());
            let _ = DeleteObject(dot.into());
            let _ = DeleteObject(stop_brush.into());
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}
