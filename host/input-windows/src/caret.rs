//! Finding the PC's text caret, so the phone can keep the place being typed
//! into visible above its on-screen keyboard.
//!
//! Two sources, tried in order:
//!
//! 1. `GetGUIThreadInfo` for the foreground thread: the classic Win32 caret
//!    (Notepad, Win32 edit controls, many older apps). Cheap.
//! 2. UI Automation: the focused element's caret range (`TextPattern2`), or
//!    its selection, or failing both the bounds of the focused text control.
//!    Covers browsers, Office, WinUI and other modern apps.
//!
//! UI Automation calls cross into other processes and can block on a hung
//! app, so [`CaretFinder`] sets short UIA timeouts and is meant to live on a
//! dedicated thread. Nothing here injects input or changes any state.

use crate::events::Rect;

/// Where the caret was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaretSource {
    /// `GetGUIThreadInfo` (the system caret).
    Win32,
    /// UI Automation text range.
    Automation,
    /// UI Automation, bounds of the focused text control (no caret range).
    Field,
}

/// A caret rectangle in virtual-desktop pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caret {
    pub rect: Rect,
    pub source: CaretSource,
}

/// Map a caret rectangle (virtual-desktop pixels) onto the streamed display
/// as protocol units: `(x, y, w, h)`, each 0..65535. `None` if the caret's
/// top-left is not on that display.
pub fn caret_on_display(caret: Rect, display: Rect) -> Option<(u16, u16, u16, u16)> {
    if display.width == 0 || display.height == 0 || !display.contains(caret.x, caret.y) {
        return None;
    }
    let rx = (caret.x - display.x) as i64;
    let ry = (caret.y - display.y) as i64;
    let x = lanlink_protocol::normalize(rx, display.width);
    let y = lanlink_protocol::normalize(ry, display.height);
    // Sizes scale without the -1 used for positions, and are clipped to the
    // part of the display below/right of the caret.
    let size = |len: u32, from: i64, extent: u32| -> u16 {
        let len = (len as i64).min(extent as i64 - from).max(0);
        ((len * 65535) / extent as i64).min(65535) as u16
    };
    let w = size(caret.width.max(1), rx, display.width);
    let h = size(caret.height.max(1), ry, display.height);
    Some((x, y, w, h))
}

#[cfg(windows)]
pub use imp::{CaretFinder, CaretWatcher};

#[cfg(windows)]
mod imp {
    use super::{Caret, CaretSource};
    use crate::events::Rect;
    use windows::core::{Interface, BOOL};
    use windows::Win32::Foundation::{HWND, POINT};
    use windows::Win32::Graphics::Gdi::MapWindowPoints;
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER, SAFEARRAY};
    use windows::Win32::System::Ole::{
        SafeArrayAccessData, SafeArrayDestroy, SafeArrayGetLBound, SafeArrayGetUBound,
        SafeArrayUnaccessData,
    };
    use windows::Win32::UI::Accessibility::{
        CUIAutomation8, IUIAutomation, IUIAutomation2, IUIAutomationElement,
        IUIAutomationTextPattern, IUIAutomationTextPattern2, IUIAutomationTextRange,
        IUIAutomationValuePattern, TextUnit_Character, UIA_EditControlTypeId, UIA_TextPattern2Id,
        UIA_TextPatternId, UIA_ValuePatternId,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GetForegroundWindow, GetGUIThreadInfo, GetWindowThreadProcessId, GUITHREADINFO,
    };

    /// UIA connection and transaction timeout, so a hung app cannot stall
    /// the caret thread for long.
    const UIA_TIMEOUT_MS: u32 = 300;

    /// Finds the caret. Create and use it on one thread that has COM
    /// initialised (MTA).
    pub struct CaretFinder {
        uia: Option<IUIAutomation>,
    }

    impl CaretFinder {
        pub fn new() -> Self {
            // SAFETY: plain COM activation; the caller initialised COM.
            let uia: Option<IUIAutomation> =
                unsafe { CoCreateInstance(&CUIAutomation8, None, CLSCTX_INPROC_SERVER) }.ok();
            if let Some(u2) = uia.as_ref().and_then(|u| u.cast::<IUIAutomation2>().ok()) {
                // SAFETY: setters on a live interface.
                unsafe {
                    let _ = u2.SetConnectionTimeout(UIA_TIMEOUT_MS);
                    let _ = u2.SetTransactionTimeout(UIA_TIMEOUT_MS);
                }
            }
            if uia.is_none() {
                tracing::warn!("UI Automation unavailable; caret only from Win32 apps");
            }
            Self { uia }
        }

        /// The caret of the focused control, if any can be found.
        pub fn find(&self) -> Option<Caret> {
            win32_caret().or_else(|| self.automation_caret())
        }

        /// UI Automation only, skipping the Win32 caret (for diagnostics and
        /// tests of that path).
        pub fn find_automation(&self) -> Option<Caret> {
            self.automation_caret()
        }

        fn automation_caret(&self) -> Option<Caret> {
            let uia = self.uia.as_ref()?;
            // SAFETY: COM calls on live interfaces; results are checked.
            unsafe {
                let el = uia.GetFocusedElement().ok()?;
                // Only somewhere the user can type. A web page is a
                // "document" with text access too, but typing does not go
                // into it unless it is editable.
                let is_edit = el.CurrentControlType().ok() == Some(UIA_EditControlTypeId);
                if !is_edit && !writable_value(&el) {
                    return None;
                }
                if let Ok(tp2) =
                    el.GetCurrentPatternAs::<IUIAutomationTextPattern2>(UIA_TextPattern2Id)
                {
                    let mut active = BOOL(0);
                    if let Ok(range) = tp2.GetCaretRange(&mut active) {
                        if let Some(rect) = range_rect(&range) {
                            return Some(Caret {
                                rect,
                                source: CaretSource::Automation,
                            });
                        }
                    }
                }
                let tp = el
                    .GetCurrentPatternAs::<IUIAutomationTextPattern>(UIA_TextPatternId)
                    .ok()?;
                if let Ok(sel) = tp.GetSelection() {
                    if sel.Length().unwrap_or(0) > 0 {
                        if let Ok(range) = sel.GetElement(0) {
                            if let Some(rect) = range_rect(&range) {
                                return Some(Caret {
                                    rect,
                                    source: CaretSource::Automation,
                                });
                            }
                        }
                    }
                }
                // No caret range: fall back to the box itself, but only for a
                // real edit box (never a whole page or pane).
                if !is_edit {
                    return None;
                }
                field_rect(&el).map(|rect| Caret {
                    rect,
                    source: CaretSource::Field,
                })
            }
        }
    }

    impl Default for CaretFinder {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Polls [`CaretFinder`] on its own thread (with COM initialised) and
    /// calls `on_change` whenever the result changes. Stops when dropped;
    /// the thread is not joined, because a UI Automation call into a hung
    /// app may still be waiting out its timeout.
    pub struct CaretWatcher {
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    impl CaretWatcher {
        pub fn spawn(
            interval: std::time::Duration,
            mut on_change: impl FnMut(Option<Caret>) + Send + 'static,
        ) -> std::io::Result<Self> {
            use std::sync::atomic::{AtomicBool, Ordering};
            use windows::Win32::System::Com::{
                CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED,
            };
            let stop = std::sync::Arc::new(AtomicBool::new(false));
            let flag = stop.clone();
            std::thread::Builder::new()
                .name("lanlink-caret".into())
                .spawn(move || {
                    // SAFETY: per-thread COM initialisation, undone below.
                    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
                    {
                        let finder = CaretFinder::new();
                        let mut last: Option<Option<Caret>> = None;
                        while !flag.load(Ordering::Relaxed) {
                            let now = finder.find();
                            if last != Some(now) && !flag.load(Ordering::Relaxed) {
                                on_change(now);
                                last = Some(now);
                            }
                            std::thread::sleep(interval);
                        }
                    }
                    if com {
                        // SAFETY: balances the successful CoInitializeEx.
                        unsafe { CoUninitialize() };
                    }
                })?;
            Ok(Self { stop })
        }
    }

    impl Drop for CaretWatcher {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn win32_caret() -> Option<Caret> {
        // SAFETY: read-only window queries with a correctly sized struct.
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_invalid() {
                return None;
            }
            let tid = GetWindowThreadProcessId(fg, None);
            if tid == 0 {
                return None;
            }
            let mut gti = GUITHREADINFO {
                cbSize: std::mem::size_of::<GUITHREADINFO>() as u32,
                ..Default::default()
            };
            GetGUIThreadInfo(tid, &mut gti).ok()?;
            if gti.hwndCaret.is_invalid() {
                return None;
            }
            let r = gti.rcCaret;
            if r.bottom <= r.top {
                return None;
            }
            let mut pts = [
                POINT {
                    x: r.left,
                    y: r.top,
                },
                POINT {
                    x: r.right,
                    y: r.bottom,
                },
            ];
            MapWindowPoints(Some(gti.hwndCaret), Some(HWND::default()), &mut pts);
            Some(Caret {
                rect: Rect {
                    x: pts[0].x,
                    y: pts[0].y,
                    width: (pts[1].x - pts[0].x).max(1) as u32,
                    height: (pts[1].y - pts[0].y).max(1) as u32,
                },
                source: CaretSource::Win32,
            })
        }
    }

    /// Bounding rectangle of a text range; an empty (caret) range is widened
    /// to one character, since many providers report no rectangle for it.
    unsafe fn range_rect(range: &IUIAutomationTextRange) -> Option<Rect> {
        // SAFETY: forwarded from the caller.
        unsafe {
            if let Some(r) = bounding(range) {
                return Some(r);
            }
            let wide = range.Clone().ok()?;
            wide.ExpandToEnclosingUnit(TextUnit_Character).ok()?;
            bounding(&wide).map(|r| Rect { width: 1, ..r })
        }
    }

    /// Last rectangle of `GetBoundingRectangles` (the end of the range is
    /// where the caret is).
    unsafe fn bounding(range: &IUIAutomationTextRange) -> Option<Rect> {
        // SAFETY: the SAFEARRAY is owned by us and destroyed on every path.
        unsafe {
            let sa: *mut SAFEARRAY = range.GetBoundingRectangles().ok()?;
            if sa.is_null() {
                return None;
            }
            let out = read_last_rect(sa);
            let _ = SafeArrayDestroy(sa);
            out
        }
    }

    unsafe fn read_last_rect(sa: *mut SAFEARRAY) -> Option<Rect> {
        // SAFETY: bounds come from the array itself; elements are doubles
        // per the UIA contract (x, y, width, height repeated).
        unsafe {
            let lo = SafeArrayGetLBound(sa, 1).ok()?;
            let hi = SafeArrayGetUBound(sa, 1).ok()?;
            let n = (hi - lo + 1).max(0) as usize;
            if n < 4 {
                return None;
            }
            let mut data = std::ptr::null_mut();
            SafeArrayAccessData(sa, &mut data).ok()?;
            let d = std::slice::from_raw_parts(data as *const f64, n);
            let i = (n / 4 - 1) * 4;
            let (x, y, w, h) = (d[i], d[i + 1], d[i + 2], d[i + 3]);
            let _ = SafeArrayUnaccessData(sa);
            if !(h > 0.0 && x.is_finite() && y.is_finite()) {
                return None;
            }
            Some(Rect {
                x: x as i32,
                y: y as i32,
                width: w.max(1.0) as u32,
                height: h as u32,
            })
        }
    }

    /// The element exposes a value the user may change (e.g. a
    /// contenteditable region or a custom text box).
    unsafe fn writable_value(el: &IUIAutomationElement) -> bool {
        // SAFETY: forwarded from the caller.
        unsafe {
            el.GetCurrentPatternAs::<IUIAutomationValuePattern>(UIA_ValuePatternId)
                .and_then(|v| v.CurrentIsReadOnly())
                .is_ok_and(|ro| !ro.as_bool())
        }
    }

    unsafe fn field_rect(el: &IUIAutomationElement) -> Option<Rect> {
        // SAFETY: forwarded from the caller.
        let r = unsafe { el.CurrentBoundingRectangle() }.ok()?;
        (r.right > r.left && r.bottom > r.top).then(|| Rect {
            x: r.left,
            y: r.top,
            width: (r.right - r.left) as u32,
            height: (r.bottom - r.top) as u32,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DISPLAY: Rect = Rect {
        x: -2560,
        y: 0,
        width: 2560,
        height: 1600,
    };

    #[test]
    fn caret_maps_onto_the_streamed_display() {
        // Top-left corner of a monitor left of the primary one.
        let c = Rect {
            x: -2560,
            y: 0,
            width: 2,
            height: 32,
        };
        let (x, y, w, h) = caret_on_display(c, DISPLAY).unwrap();
        assert_eq!((x, y), (0, 0));
        assert_eq!(w, (2 * 65535 / 2560) as u16);
        assert_eq!(h, (32 * 65535 / 1600) as u16);
        // Bottom-right pixel.
        let c = Rect {
            x: -1,
            y: 1599,
            width: 1,
            height: 20,
        };
        let (x, y, _, h) = caret_on_display(c, DISPLAY).unwrap();
        assert_eq!((x, y), (65535, 65535));
        assert_eq!(h, (65535 / 1600) as u16, "clipped to the display");
    }

    #[test]
    fn caret_on_another_display_is_not_reported() {
        let c = Rect {
            x: 100,
            y: 100,
            width: 2,
            height: 20,
        };
        assert_eq!(caret_on_display(c, DISPLAY), None);
        let empty = Rect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        assert_eq!(caret_on_display(c, empty), None);
    }
}
