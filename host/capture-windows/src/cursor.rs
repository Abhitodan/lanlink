//! Pointer-shape conversion.
//!
//! Desktop Duplication reports the pointer separately from the desktop image
//! in one of three encodings. The client draws the pointer itself (so pointer
//! motion does not wait for video), which needs plain straight-alpha RGBA.
//!
//! Two of the encodings can *invert* the screen underneath (the classic text
//! I-beam). That cannot be expressed in RGBA, so inverting pixels are drawn
//! black with a white outline, which stays visible on any background.

/// A pointer image ready to send to the client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CursorShape {
    pub width: u16,
    pub height: u16,
    pub hot_x: u16,
    pub hot_y: u16,
    /// Straight-alpha RGBA, `width * height * 4` bytes.
    pub rgba: Vec<u8>,
}

/// Largest pointer the protocol carries; larger shapes are dropped.
pub const MAX_DIM: u32 = 256;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Px {
    Transparent,
    Color([u8; 4]),
    Invert,
}

fn finish(width: u32, height: u32, hot_x: u32, hot_y: u32, px: Vec<Px>) -> Option<CursorShape> {
    if width == 0 || height == 0 || width > MAX_DIM || height > MAX_DIM {
        return None;
    }
    let (w, h) = (width as usize, height as usize);
    if px.len() != w * h {
        return None;
    }
    let mut rgba = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let o = (y * w + x) * 4;
            match px[y * w + x] {
                Px::Color(c) => rgba[o..o + 4].copy_from_slice(&c),
                Px::Invert => rgba[o..o + 4].copy_from_slice(&[0, 0, 0, 255]),
                Px::Transparent => {
                    // White outline around inverting pixels.
                    let near_invert =
                        [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)]
                            .iter()
                            .any(|(dx, dy)| {
                                let nx = x as i32 + dx;
                                let ny = y as i32 + dy;
                                nx >= 0
                                    && ny >= 0
                                    && (nx as usize) < w
                                    && (ny as usize) < h
                                    && px[ny as usize * w + nx as usize] == Px::Invert
                            });
                    if near_invert {
                        rgba[o..o + 4].copy_from_slice(&[255, 255, 255, 230]);
                    }
                }
            }
        }
    }
    Some(CursorShape {
        width: width as u16,
        height: height as u16,
        hot_x: hot_x.min(width - 1) as u16,
        hot_y: hot_y.min(height - 1) as u16,
        rgba,
    })
}

/// 32-bit BGRA with a real alpha channel (`DXGI_OUTDUPL_POINTER_SHAPE_TYPE_COLOR`).
pub fn from_color(
    data: &[u8],
    width: u32,
    height: u32,
    pitch: u32,
    hot_x: u32,
    hot_y: u32,
) -> Option<CursorShape> {
    if width > MAX_DIM || height > MAX_DIM || pitch < width.checked_mul(4)? {
        return None;
    }
    let needed = (height as usize).checked_mul(pitch as usize)?;
    if data.len() < needed {
        return None;
    }
    let mut px = Vec::with_capacity((width * height) as usize);
    for y in 0..height as usize {
        let row = &data[y * pitch as usize..];
        for x in 0..width as usize {
            let (b, g, r, a) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
            px.push(if a == 0 {
                Px::Transparent
            } else {
                Px::Color([r, g, b, a])
            });
        }
    }
    finish(width, height, hot_x, hot_y, px)
}

/// 1 bpp AND mask followed by a 1 bpp XOR mask; `height` is the combined
/// height of both masks (`DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MONOCHROME`).
pub fn from_monochrome(
    data: &[u8],
    width: u32,
    height: u32,
    pitch: u32,
    hot_x: u32,
    hot_y: u32,
) -> Option<CursorShape> {
    let real_h = height / 2;
    if width > MAX_DIM || real_h > MAX_DIM || real_h == 0 || pitch < width.div_ceil(8) {
        return None;
    }
    let needed = (height as usize).checked_mul(pitch as usize)?;
    if data.len() < needed {
        return None;
    }
    let bit = |row: usize, x: usize| -> bool {
        data[row * pitch as usize + x / 8] & (0x80 >> (x % 8)) != 0
    };
    let mut px = Vec::with_capacity((width * real_h) as usize);
    for y in 0..real_h as usize {
        for x in 0..width as usize {
            let and = bit(y, x);
            let xor = bit(y + real_h as usize, x);
            px.push(match (and, xor) {
                (false, false) => Px::Color([0, 0, 0, 255]),
                (false, true) => Px::Color([255, 255, 255, 255]),
                (true, false) => Px::Transparent,
                (true, true) => Px::Invert,
            });
        }
    }
    finish(width, real_h, hot_x, hot_y, px)
}

/// 32-bit BGRA where the alpha byte is a mask: `0x00` replaces the screen
/// pixel, `0xFF` XORs with it (`DXGI_OUTDUPL_POINTER_SHAPE_TYPE_MASKED_COLOR`).
pub fn from_masked_color(
    data: &[u8],
    width: u32,
    height: u32,
    pitch: u32,
    hot_x: u32,
    hot_y: u32,
) -> Option<CursorShape> {
    if width > MAX_DIM || height > MAX_DIM || pitch < width.checked_mul(4)? {
        return None;
    }
    let needed = (height as usize).checked_mul(pitch as usize)?;
    if data.len() < needed {
        return None;
    }
    let mut px = Vec::with_capacity((width * height) as usize);
    for y in 0..height as usize {
        let row = &data[y * pitch as usize..];
        for x in 0..width as usize {
            let (b, g, r, mask) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
            px.push(if mask == 0 {
                Px::Color([r, g, b, 255])
            } else if r == 0 && g == 0 && b == 0 {
                // XOR with black leaves the screen unchanged.
                Px::Transparent
            } else {
                Px::Invert
            });
        }
    }
    finish(width, height, hot_x, hot_y, px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_swaps_channels_and_keeps_alpha() {
        // 2x1: opaque blue-ish pixel, fully transparent pixel.
        let bgra = [10, 20, 30, 255, 1, 2, 3, 0];
        let s = from_color(&bgra, 2, 1, 8, 0, 0).unwrap();
        assert_eq!(s.rgba, [30, 20, 10, 255, 0, 0, 0, 0]);
    }

    #[test]
    fn monochrome_decodes_all_four_cases() {
        // 4x1 image => combined height 2, pitch 1.
        // AND row: 0 0 1 1 ; XOR row: 0 1 0 1
        let data = [0b0011_0000, 0b0101_0000];
        let s = from_monochrome(&data, 4, 2, 1, 9, 9).unwrap();
        assert_eq!((s.width, s.height), (4, 1));
        // Hotspot is clamped inside the image.
        assert_eq!((s.hot_x, s.hot_y), (3, 0));
        assert_eq!(&s.rgba[0..4], &[0, 0, 0, 255]); // black
        assert_eq!(&s.rgba[4..8], &[255, 255, 255, 255]); // white
        assert_eq!(&s.rgba[8..12], &[255, 255, 255, 230]); // outline next to invert
        assert_eq!(&s.rgba[12..16], &[0, 0, 0, 255]); // invert drawn black
    }

    #[test]
    fn masked_color_handles_replace_xor_and_noop() {
        let data = [
            1, 2, 3, 0x00, // replace
            0, 0, 0, 0xff, // xor with black = unchanged
            9, 9, 9, 0xff, // xor = invert
        ];
        let s = from_masked_color(&data, 3, 1, 12, 0, 0).unwrap();
        assert_eq!(&s.rgba[0..4], &[3, 2, 1, 255]);
        assert_eq!(&s.rgba[4..8], &[255, 255, 255, 230]);
        assert_eq!(&s.rgba[8..12], &[0, 0, 0, 255]);
    }

    #[test]
    fn rejects_short_buffers_and_oversize() {
        assert!(from_color(&[0; 7], 2, 1, 8, 0, 0).is_none());
        assert!(from_color(&[0; 8], 2, 1, 4, 0, 0).is_none()); // pitch too small
        assert!(from_monochrome(&[0; 1], 4, 2, 1, 0, 0).is_none());
        assert!(from_monochrome(&[0; 4], 4, 1, 1, 0, 0).is_none()); // zero real height
        assert!(from_color(&[], 0, 0, 0, 0, 0).is_none());
        assert!(from_color(&[0; 16], 257, 1, 1028, 0, 0).is_none());
        assert!(from_masked_color(&[0; 3], 1, 1, 4, 0, 0).is_none());
        // Overflowing dimensions must not panic.
        assert!(from_color(&[0; 16], u32::MAX, u32::MAX, u32::MAX, 0, 0).is_none());
    }
}
