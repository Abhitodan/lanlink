//! Choosing stream parameters from what the user asked for, what the phone
//! can decode and what is being captured. Pure logic, unit-tested.

use lanlink_protocol::{Codec, CodecCap, QualityMode, StreamParams};

/// Concrete encoder settings for one stream configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamPlan {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    /// Starting bitrate, also the ceiling the adaptive controller returns to.
    pub bitrate_bps: u32,
    /// Floor for the adaptive controller.
    pub min_bitrate_bps: u32,
}

/// Bits per pixel per frame for H.264 desktop content. Desktop content is
/// mostly static with sharp edges; these are starting points that the
/// adaptive controller adjusts from measurements.
fn bits_per_pixel(mode: QualityMode) -> f64 {
    match mode {
        QualityMode::Smooth => 0.06,
        QualityMode::Balanced => 0.08,
        QualityMode::Sharp => 0.12,
        QualityMode::Custom => 0.08,
    }
}

/// Longest edge for each preset before client limits are applied.
fn preset_long_edge(mode: QualityMode) -> u32 {
    match mode {
        QualityMode::Smooth => 1600,
        QualityMode::Balanced => 1920,
        QualityMode::Sharp => 4096,
        QualityMode::Custom => 4096,
    }
}

const MAX_BITRATE: u32 = 60_000_000;
const MIN_BITRATE: u32 = 1_000_000;

/// `source`: captured size; `refresh_hz`: display refresh; `caps`: decoders
/// the client reported (empty = assume a basic 1080p60 H.264 decoder).
pub fn plan(
    params: &StreamParams,
    caps: &[CodecCap],
    source: (u32, u32),
    refresh_hz: u32,
) -> StreamPlan {
    let cap = caps
        .iter()
        .filter(|c| c.codec == Codec::H264)
        .max_by_key(|c| (c.hardware, c.max_width as u32 * c.max_height as u32))
        .copied()
        .unwrap_or(CodecCap {
            codec: Codec::H264,
            hardware: true,
            low_latency: false,
            max_width: 1920,
            max_height: 1080,
            max_fps: 60,
        });

    // Size: preset, then client request, then decoder limit. The decoder
    // limit is orientation-agnostic (a 1920x1080 decoder takes 1080x1920).
    let long = preset_long_edge(params.quality);
    let mut max = (long, long);
    if params.max_width > 0 {
        max.0 = max.0.min(params.max_width as u32);
    }
    if params.max_height > 0 {
        max.1 = max.1.min(params.max_height as u32);
    }
    let (dec_long, dec_short) = (
        cap.max_width.max(cap.max_height) as u32,
        cap.max_width.min(cap.max_height) as u32,
    );
    let (dec_w, dec_h) = if source.0 >= source.1 {
        (dec_long, dec_short)
    } else {
        (dec_short, dec_long)
    };
    if dec_w > 0 && dec_h > 0 {
        max.0 = max.0.min(dec_w);
        max.1 = max.1.min(dec_h);
    }
    let (width, height) = fit(source, max);

    // Frame rate: preset, then request, display refresh and decoder limit.
    let preset_fps = 60;
    let mut fps = if params.fps > 0 {
        params.fps as u32
    } else {
        preset_fps
    };
    fps = fps.min(refresh_hz.max(1));
    if cap.max_fps > 0 {
        fps = fps.min(cap.max_fps as u32);
    }
    let fps = fps.clamp(1, 240);

    let auto = (width as f64 * height as f64 * fps as f64 * bits_per_pixel(params.quality)) as u64;
    let mut bitrate = if params.bitrate_kbps > 0 {
        params.bitrate_kbps as u64 * 1000
    } else {
        auto
    };
    if params.max_bitrate_kbps > 0 {
        bitrate = bitrate.min(params.max_bitrate_kbps as u64 * 1000);
    }
    let bitrate = bitrate.clamp(MIN_BITRATE as u64, MAX_BITRATE as u64) as u32;
    StreamPlan {
        width,
        height,
        fps,
        bitrate_bps: bitrate,
        min_bitrate_bps: (bitrate / 8).max(MIN_BITRATE).min(bitrate),
    }
}

/// Fit preserving aspect ratio, never upscaling, even dimensions, ≥ 64.
pub fn fit(src: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (src.0.max(1) as u64, src.1.max(1) as u64);
    let (mw, mh) = (max.0.max(64) as u64, max.1.max(64) as u64);
    let (num, den) = if mw * sh <= mh * sw {
        (mw, sw)
    } else {
        (mh, sh)
    };
    let (num, den) = if num >= den { (1, 1) } else { (num, den) };
    let even = |v: u64| ((v & !1).max(64)) as u32;
    (even(sw * num / den), even(sh * num / den))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(q: QualityMode) -> StreamParams {
        StreamParams {
            quality: q,
            codec: Codec::H264,
            max_width: 0,
            max_height: 0,
            fps: 0,
            bitrate_kbps: 0,
            max_bitrate_kbps: 0,
        }
    }

    fn cap(w: u16, h: u16, fps: u16) -> CodecCap {
        CodecCap {
            codec: Codec::H264,
            hardware: true,
            low_latency: true,
            max_width: w,
            max_height: h,
            max_fps: fps,
        }
    }

    #[test]
    fn balanced_on_this_laptop() {
        let p = plan(
            &params(QualityMode::Balanced),
            &[cap(4096, 2304, 120)],
            (2560, 1600),
            240,
        );
        assert_eq!((p.width, p.height, p.fps), (1920, 1200, 60));
        assert_eq!(p.bitrate_bps, (1920.0 * 1200.0 * 60.0 * 0.08) as u32);
        assert!(p.min_bitrate_bps < p.bitrate_bps);
    }

    #[test]
    fn decoder_limit_applies_in_either_orientation() {
        let caps = [cap(1920, 1080, 60)];
        let p = plan(&params(QualityMode::Sharp), &caps, (2560, 1600), 60);
        assert_eq!((p.width, p.height), (1728, 1080));
        let p = plan(&params(QualityMode::Sharp), &caps, (1600, 2560), 60);
        assert_eq!((p.width, p.height), (1080, 1728));
    }

    #[test]
    fn requests_and_refresh_cap_fps_and_size() {
        let mut q = params(QualityMode::Custom);
        q.fps = 120;
        q.max_width = 1280;
        q.bitrate_kbps = 5000;
        q.max_bitrate_kbps = 4000;
        let p = plan(&q, &[cap(3840, 2160, 120)], (2560, 1600), 90);
        assert_eq!((p.width, p.height, p.fps), (1280, 800, 90));
        assert_eq!(p.bitrate_bps, 4_000_000);
    }

    #[test]
    fn missing_caps_fall_back_to_1080p60() {
        let p = plan(&params(QualityMode::Sharp), &[], (3840, 2160), 144);
        assert_eq!((p.width, p.height, p.fps), (1920, 1080, 60));
    }

    #[test]
    fn bitrate_is_bounded() {
        let p = plan(
            &params(QualityMode::Sharp),
            &[cap(8192, 8192, 240)],
            (7680, 4320),
            240,
        );
        assert_eq!(p.bitrate_bps, MAX_BITRATE);
        let mut q = params(QualityMode::Custom);
        q.bitrate_kbps = 10;
        let p = plan(&q, &[], (1280, 720), 60);
        assert_eq!(p.bitrate_bps, MIN_BITRATE);
        assert_eq!(p.min_bitrate_bps, MIN_BITRATE);
    }
}
