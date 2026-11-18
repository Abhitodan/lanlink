//! Pointer tracking through the window manager (`GetCursorInfo`), used by the
//! Graphics Capture backend, which has no pointer channel of its own.

use crate::cursor::{self, CursorShape};
use crate::CursorUpdate;
use windows::Win32::Foundation::POINT;
use windows::Win32::Graphics::Gdi::{
    DeleteObject, GetDC, GetDIBits, GetObjectW, ReleaseDC, BITMAP, BITMAPINFO, BITMAPINFOHEADER,
    BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, RGBQUAD,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetCursorInfo, GetIconInfo, CURSORINFO, CURSOR_SHOWING, HCURSOR, ICONINFO,
};

/// Reports pointer changes relative to one display.
pub struct CursorTracker {
    origin: (i32, i32),
    size: (u32, u32),
    last_handle: isize,
    last_pos: (i32, i32),
    last_visible: bool,
    primed: bool,
}

impl CursorTracker {
    /// `x`/`y`/`width`/`height` describe the display in virtual-desktop
    /// physical pixels.
    pub fn new(x: i32, y: i32, width: u32, height: u32) -> Self {
        CursorTracker {
            origin: (x, y),
            size: (width, height),
            last_handle: 0,
            last_pos: (i32::MIN, i32::MIN),
            last_visible: false,
            primed: false,
        }
    }

    /// Returns an update if the pointer moved, changed shape or changed
    /// visibility since the previous call. Costs one syscall when idle.
    pub fn poll(&mut self) -> Option<CursorUpdate> {
        let mut info = CURSORINFO {
            cbSize: std::mem::size_of::<CURSORINFO>() as u32,
            ..Default::default()
        };
        // SAFETY: `info` is a correctly sized, live struct.
        if unsafe { GetCursorInfo(&mut info) }.is_err() {
            return None;
        }
        let POINT { x, y } = info.ptScreenPos;
        let rel = (x - self.origin.0, y - self.origin.1);
        let inside = rel.0 >= 0
            && rel.1 >= 0
            && (rel.0 as u32) < self.size.0
            && (rel.1 as u32) < self.size.1;
        let showing = info.flags.0 & CURSOR_SHOWING.0 != 0;
        let visible = showing && inside;
        let handle = info.hCursor.0 as isize;

        let shape_changed = showing && handle != 0 && handle != self.last_handle;
        let moved = rel != self.last_pos;
        let vis_changed = visible != self.last_visible;
        if self.primed && !shape_changed && !moved && !vis_changed {
            return None;
        }

        let shape = if shape_changed {
            self.last_handle = handle;
            shape_of(info.hCursor)
        } else {
            None
        };
        self.primed = true;
        self.last_pos = rel;
        self.last_visible = visible;
        Some(CursorUpdate {
            x: rel.0,
            y: rel.1,
            visible,
            shape,
        })
    }
}

struct BitmapGuard(HBITMAP);
impl Drop for BitmapGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: we own this bitmap (GetIconInfo returns copies).
            unsafe {
                let _ = DeleteObject(self.0.into());
            }
        }
    }
}

#[repr(C)]
struct MonoBitmapInfo {
    header: BITMAPINFOHEADER,
    colors: [RGBQUAD; 2],
}

fn bitmap_dims(bmp: HBITMAP) -> Option<(u32, u32)> {
    let mut b = BITMAP::default();
    // SAFETY: `b` is a live BITMAP and its size is passed.
    let n = unsafe {
        GetObjectW(
            bmp.into(),
            std::mem::size_of::<BITMAP>() as i32,
            Some((&mut b as *mut BITMAP).cast()),
        )
    };
    if n == 0 || b.bmWidth <= 0 || b.bmHeight <= 0 {
        return None;
    }
    Some((b.bmWidth as u32, b.bmHeight as u32))
}

/// Read a bitmap as top-down 32-bit BGRA.
fn read_bgra(hdc: HDC, bmp: HBITMAP, w: u32, h: u32) -> Option<Vec<u8>> {
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    let mut buf = vec![0u8; w as usize * h as usize * 4];
    // SAFETY: the buffer holds exactly h rows of w 32-bit pixels.
    let lines = unsafe {
        GetDIBits(
            hdc,
            bmp,
            0,
            h,
            Some(buf.as_mut_ptr().cast()),
            &mut bmi,
            DIB_RGB_COLORS,
        )
    };
    (lines as u32 == h).then_some(buf)
}

/// Read a monochrome bitmap as top-down 1 bpp; returns `(bits, pitch)`.
fn read_mono(hdc: HDC, bmp: HBITMAP, w: u32, h: u32) -> Option<(Vec<u8>, u32)> {
    let pitch = w.div_ceil(32) * 4;
    let mut bmi = MonoBitmapInfo {
        header: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w as i32,
            biHeight: -(h as i32),
            biPlanes: 1,
            biBitCount: 1,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        colors: [RGBQUAD::default(); 2],
    };
    let mut buf = vec![0u8; pitch as usize * h as usize];
    // SAFETY: `MonoBitmapInfo` is a BITMAPINFO with room for the two-entry
    // palette GetDIBits writes for 1 bpp; the buffer holds h rows of `pitch`.
    let lines = unsafe {
        GetDIBits(
            hdc,
            bmp,
            0,
            h,
            Some(buf.as_mut_ptr().cast()),
            (&mut bmi as *mut MonoBitmapInfo).cast(),
            DIB_RGB_COLORS,
        )
    };
    (lines as u32 == h).then_some((buf, pitch))
}

/// Render a cursor handle to RGBA.
fn shape_of(cursor: HCURSOR) -> Option<CursorShape> {
    let mut ii = ICONINFO::default();
    // SAFETY: `ii` is a live struct; on success we own both bitmaps.
    unsafe { GetIconInfo(cursor.into(), &mut ii) }.ok()?;
    let mask = BitmapGuard(ii.hbmMask);
    let color = BitmapGuard(ii.hbmColor);
    let (hot_x, hot_y) = (ii.xHotspot, ii.yHotspot);

    // SAFETY: screen DC, released below on every path.
    let hdc = unsafe { GetDC(None) };
    if hdc.is_invalid() {
        return None;
    }
    let result = (|| {
        if color.0.is_invalid() {
            // Monochrome: the mask holds the AND mask above the XOR mask.
            let (w, h2) = bitmap_dims(mask.0)?;
            if w > cursor::MAX_DIM || h2 > cursor::MAX_DIM * 2 {
                return None;
            }
            let (bits, pitch) = read_mono(hdc, mask.0, w, h2)?;
            return cursor::from_monochrome(&bits, w, h2, pitch, hot_x, hot_y);
        }
        let (w, h) = bitmap_dims(color.0)?;
        if w > cursor::MAX_DIM || h > cursor::MAX_DIM {
            return None;
        }
        let mut bgra = read_bgra(hdc, color.0, w, h)?;
        if bgra.chunks_exact(4).any(|p| p[3] != 0) {
            return cursor::from_color(&bgra, w, h, w * 4, hot_x, hot_y);
        }
        // No alpha channel: transparency comes from the AND mask. Rewrite
        // into the "masked colour" layout the converter understands.
        let (bits, pitch) = read_mono(hdc, mask.0, w, h)?;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let and = bits[y * pitch as usize + x / 8] & (0x80 >> (x % 8)) != 0;
                bgra[(y * w as usize + x) * 4 + 3] = if and { 0xff } else { 0x00 };
            }
        }
        cursor::from_masked_color(&bgra, w, h, w * 4, hot_x, hot_y)
    })();
    // SAFETY: releasing the DC obtained above.
    unsafe {
        ReleaseDC(None, hdc);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against the real session: the first poll always reports, and the
    /// standard arrow (or whatever is showing) converts to a sane bitmap.
    #[test]
    fn first_poll_reports_position_and_shape() {
        let mut t = CursorTracker::new(-100_000, -100_000, 200_000, 200_000);
        let Some(first) = t.poll() else {
            eprintln!("SKIP: no pointer information in this session");
            return;
        };
        if let Some(shape) = &first.shape {
            assert!(shape.width > 0 && shape.width as u32 <= cursor::MAX_DIM);
            assert_eq!(
                shape.rgba.len(),
                shape.width as usize * shape.height as usize * 4
            );
            assert!(shape.hot_x < shape.width && shape.hot_y < shape.height);
            assert!(
                shape.rgba.chunks_exact(4).any(|p| p[3] != 0),
                "a visible pointer has at least one non-transparent pixel"
            );
        }
        // Nothing changed in the meantime (unless the user is moving the
        // mouse right now, which we tolerate).
        let _ = t.poll();
    }

    #[test]
    fn pointer_outside_display_is_not_visible() {
        // A 1x1 "display" far away from any real pointer position.
        let mut t = CursorTracker::new(-30_000, -30_000, 1, 1);
        if let Some(u) = t.poll() {
            assert!(!u.visible);
        }
    }
}
