//! Video access units and decoder hand-off.

use crate::runtime::runtime;
use crate::session::{send, session};
use lanlink_client::lanlink_protocol::*;
use lanlink_client::VideoItem;
use std::time::Duration;

/// Return codes of `next_video`.
pub const VIDEO_TIMEOUT: i32 = -1;
pub const VIDEO_CONFIG: i32 = -2;
pub const VIDEO_CLOSED: i32 = -3;
pub const VIDEO_BUFFER_SMALL: i32 = -4;

/// `meta` flags bit set.
pub const META_KEYFRAME: i64 = 1;
pub const META_CODEC_CONFIG: i64 = 2;
pub const META_DISCONTINUITY: i64 = 4;

/// Extract SPS and PPS NAL units (with start codes) from an access unit.
fn parameter_sets(au: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    let n = au.len();
    let mut starts = Vec::new();
    while i + 3 <= n {
        if au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 1 {
            starts.push((i, i + 3));
            i += 3;
        } else if i + 4 <= n && au[i] == 0 && au[i + 1] == 0 && au[i + 2] == 0 && au[i + 3] == 1 {
            starts.push((i, i + 4));
            i += 4;
        } else {
            i += 1;
        }
    }
    for (k, (_, payload)) in starts.iter().enumerate() {
        let end = starts.get(k + 1).map(|s| s.0).unwrap_or(n);
        if *payload < end {
            let ty = au[*payload] & 0x1f;
            if ty == 7 || ty == 8 {
                out.extend_from_slice(&[0, 0, 0, 1]);
                out.extend_from_slice(&au[*payload..end]);
            }
        }
    }
    out
}

/// Zero `meta`, then store `vals` at the front.
fn set_meta(meta: &mut [i64; 8], vals: &[i64]) {
    *meta = [0; 8];
    meta[..vals.len()].copy_from_slice(vals);
}

/// Write the next decoder input into `buf`.
///
/// Returns the byte count (> 0) and fills `meta` =
/// `[flags, ptsUs(capture time, host clock), seq, recvUs, captureToSendUs,
/// 0, 0, 0]`, or a negative code:
/// * `-1` timeout;
/// * `-2` new stream configuration; `meta` = `[width, height, fps,
///   configId, displayId, sourceWidth, sourceHeight, 0]`; the caller must
///   (re)configure its decoder and then call `decoder_reset`;
/// * `-3` session or connection ended;
/// * `-4` buffer too small; `meta[0]` = bytes needed. The frame is kept.
///
/// Before the first keyframe of each decoder instance (and whenever the
/// parameter sets change), a separate codec-config buffer containing only
/// SPS and PPS is returned with `META_CODEC_CONFIG`.
pub fn next_video(
    h: i64,
    buf: &mut [u8],
    meta: &mut [i64; 8],
    timeout_ms: i32,
) -> Result<i32, String> {
    let s = session(h)?;
    let mut v = s.video.lock().map_err(|_| "lock")?;
    let cap = buf.len();

    if v.pending.is_none() {
        let d = Duration::from_millis(timeout_ms.clamp(0, 5_000) as u64);
        let item = runtime().block_on(async { tokio::time::timeout(d, v.rx.recv()).await });
        match item {
            Err(_) => return Ok(VIDEO_TIMEOUT),
            Ok(None) => return Ok(VIDEO_CLOSED),
            Ok(Some(VideoItem::Config(c))) => {
                v.config = Some(c);
                v.sent_params.clear();
                set_meta(
                    meta,
                    &[
                        c.width as i64,
                        c.height as i64,
                        c.fps as i64,
                        c.config_id as i64,
                        c.display_id as i64,
                        c.source_width as i64,
                        c.source_height as i64,
                    ],
                );
                return Ok(VIDEO_CONFIG);
            }
            Ok(Some(VideoItem::Frame {
                header,
                data,
                recv_us,
            })) => {
                v.pending = Some((header, data, recv_us));
            }
        }
    }

    let (header, data, recv_us) = v.pending.as_ref().expect("set above");
    let header = *header;
    let recv_us = *recv_us;
    if header.is_keyframe() {
        let ps = parameter_sets(data);
        if !ps.is_empty() && ps != v.sent_params {
            if ps.len() > cap {
                set_meta(meta, &[ps.len() as i64]);
                return Ok(VIDEO_BUFFER_SMALL);
            }
            buf[..ps.len()].copy_from_slice(&ps);
            set_meta(
                meta,
                &[
                    META_CODEC_CONFIG,
                    header.capture_us as i64,
                    header.seq as i64,
                    recv_us as i64,
                    0,
                ],
            );
            let n = ps.len();
            v.sent_params = ps;
            return Ok(n as i32);
        }
    }
    let len = v.pending.as_ref().map(|p| p.1.len()).unwrap_or(0);
    if len > cap {
        set_meta(meta, &[len as i64]);
        return Ok(VIDEO_BUFFER_SMALL);
    }
    let (header, data, _) = v.pending.take().expect("checked");
    buf[..data.len()].copy_from_slice(&data);
    let mut flags = 0;
    if header.is_keyframe() {
        flags |= META_KEYFRAME;
    }
    if header.flags & lanlink_client::lanlink_protocol::video::FLAG_DISCONTINUITY != 0 {
        flags |= META_DISCONTINUITY;
    }
    set_meta(
        meta,
        &[
            flags,
            header.capture_us as i64,
            header.seq as i64,
            recv_us as i64,
            header.send_us.saturating_sub(header.capture_us) as i64,
        ],
    );
    Ok(data.len() as i32)
}

/// The app created a new decoder (first start, surface recreated, error
/// recovery): resend parameter sets and start from a fresh keyframe.
pub fn decoder_reset(h: i64) -> Result<(), String> {
    let s = session(h)?;
    if let Ok(mut v) = s.video.lock() {
        v.sent_params.clear();
        // A held delta frame cannot be decoded by a fresh decoder.
        if v.pending.as_ref().is_some_and(|p| !p.0.is_keyframe()) {
            v.pending = None;
        }
    }
    let _ = s.conn.try_send(ControlMsg::KeyframeRequest);
    Ok(())
}

pub fn request_keyframe(h: i64) -> Result<(), String> {
    send(h, ControlMsg::KeyframeRequest)
}

/// Monotonic microseconds on the same clock as `recvUs` in video metadata.
pub fn now_us() -> i64 {
    lanlink_transport::monotonic_us() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_parameter_sets() {
        let au = [
            0, 0, 0, 1, 0x67, 1, 2, 0, 0, 1, 0x68, 3, 0, 0, 0, 1, 0x65, 9, 9,
        ];
        assert_eq!(
            parameter_sets(&au),
            [0, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3]
        );
        assert!(parameter_sets(&[0, 0, 1, 0x41, 5]).is_empty());
        assert!(parameter_sets(&[]).is_empty());
        assert!(parameter_sets(&[0, 0, 1]).is_empty());
    }
}
