//! The LANLink mark, drawn in code so the tray and window icons can change
//! colour with the host state without image files.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mark {
    /// Listening, nobody connected.
    Ready,
    /// A phone is viewing or controlling this PC.
    Controlled,
    /// The host is not running (start-up failed).
    Off,
}

impl Mark {
    fn rgb(self) -> [u8; 3] {
        match self {
            Mark::Ready => [0xD9, 0xB9, 0x8C],      // sandstone
            Mark::Controlled => [0xE0, 0x6C, 0x5F], // alert red
            Mark::Off => [0x7A, 0x80, 0x88],        // grey
        }
    }
}

/// Straight-alpha RGBA, `size`×`size`: a filled disc with two linked rings
/// cut out of it (phone ↔ PC).
pub fn rgba(mark: Mark, size: u32) -> Vec<u8> {
    let [r, g, b] = mark.rgb();
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let c = s / 2.0;
    let disc = s * 0.48;
    let ring_r = s * 0.15;
    let ring_w = s * 0.055;
    let left = (c - s * 0.09, c);
    let right = (c + s * 0.09, c);
    for y in 0..size {
        for x in 0..size {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let d = ((px - c).powi(2) + (py - c).powi(2)).sqrt();
            // Antialiased coverage of the disc.
            let mut a = (disc - d + 0.5).clamp(0.0, 1.0);
            // Rings are cut out (transparent), antialiased too.
            for (cx, cy) in [left, right] {
                let dr = ((px - cx).powi(2) + (py - cy).powi(2)).sqrt();
                let ring = (ring_w / 2.0 - (dr - ring_r).abs() + 0.5).clamp(0.0, 1.0);
                a *= 1.0 - ring;
            }
            let i = ((y * size + x) * 4) as usize;
            out[i] = r;
            out[i + 1] = g;
            out[i + 2] = b;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corners_are_transparent_and_disc_is_opaque() {
        let px = rgba(Mark::Ready, 32);
        assert_eq!(px.len(), 32 * 32 * 4);
        assert_eq!(px[3], 0, "corner must be transparent");
        // A point inside the disc but away from the rings.
        let i = ((4 * 32 + 16) * 4) as usize;
        assert_eq!(px[i + 3], 255);
        assert_ne!(rgba(Mark::Controlled, 32), px);
    }
}
