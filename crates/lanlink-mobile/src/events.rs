//! Control-plane events as small JSON strings, plus the minimal JSON object
//! writer that produces them. Output only; the apps parse it themselves.

use crate::runtime::runtime;
use crate::session::{session, Session};
use lanlink_client::ClientEvent;
use std::sync::atomic::Ordering;
use std::time::Duration;

pub(crate) struct Json {
    buf: String,
    first: bool,
}

fn escape(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl Json {
    pub(crate) fn object() -> Self {
        Json {
            buf: String::from("{"),
            first: true,
        }
    }

    fn key(&mut self, k: &str) {
        if !self.first {
            self.buf.push(',');
        }
        self.first = false;
        escape(k, &mut self.buf);
        self.buf.push(':');
    }

    pub(crate) fn str(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k);
        escape(v, &mut self.buf);
        self
    }

    pub(crate) fn int(&mut self, k: &str, v: i64) -> &mut Self {
        self.key(k);
        self.buf.push_str(&v.to_string());
        self
    }

    pub(crate) fn bool(&mut self, k: &str, v: bool) -> &mut Self {
        self.key(k);
        self.buf.push_str(if v { "true" } else { "false" });
        self
    }

    pub(crate) fn str_array(&mut self, k: &str, items: impl Iterator<Item = String>) -> &mut Self {
        self.key(k);
        self.buf.push('[');
        for (i, s) in items.enumerate() {
            if i > 0 {
                self.buf.push(',');
            }
            escape(&s, &mut self.buf);
        }
        self.buf.push(']');
        self
    }

    /// Items must already be valid JSON.
    pub(crate) fn raw_array(&mut self, k: &str, items: Vec<String>) -> &mut Self {
        self.key(k);
        self.buf.push('[');
        self.buf.push_str(&items.join(","));
        self.buf.push(']');
        self
    }

    pub(crate) fn finish(&mut self) -> String {
        let mut out = std::mem::take(&mut self.buf);
        out.push('}');
        out
    }
}

fn event_json(s: &Session, ev: ClientEvent) -> String {
    let mut j = Json::object();
    match ev {
        ClientEvent::DisplayList(list) => {
            j.str("type", "displays");
            let items: Vec<String> = list
                .iter()
                .map(|d| {
                    let mut o = Json::object();
                    o.int("id", d.id as i64)
                        .str("name", &d.name)
                        .int("width", d.width as i64)
                        .int("height", d.height as i64)
                        .int("x", d.x as i64)
                        .int("y", d.y as i64)
                        .bool("primary", d.primary)
                        .int("refreshMilliHz", d.refresh_millihz as i64)
                        .int("scale", d.scale_percent as i64)
                        .bool("hdr", d.hdr);
                    o.finish()
                })
                .collect();
            j.raw_array("displays", items);
        }
        ClientEvent::SessionStarted {
            session_id,
            display_id,
            view_only,
            encoder,
            capture,
        } => {
            j.str("type", "sessionStarted")
                .int("sessionId", session_id as i64)
                .int("displayId", display_id as i64)
                .bool("viewOnly", view_only)
                .str("encoder", &encoder)
                .str("capture", &capture);
        }
        ClientEvent::SessionEnded(code) => {
            j.str("type", "sessionEnded")
                .int("code", code.as_u16() as i64);
        }
        ClientEvent::Stats(st) => {
            j.str("type", "stats")
                .int("capturedFpsX10", st.captured_fps_x10 as i64)
                .int("encodedFpsX10", st.encoded_fps_x10 as i64)
                .int("bitrateKbps", st.bitrate_kbps as i64)
                .int("targetKbps", st.target_bitrate_kbps as i64)
                .int("encodeP50", st.encode_us_p50 as i64)
                .int("encodeP95", st.encode_us_p95 as i64)
                .int("droppedEncode", st.dropped_before_encode as i64)
                .int("droppedSend", st.dropped_before_send as i64)
                .int("queue", st.send_queue_depth as i64)
                .int("rttUs", st.rtt_us as i64)
                .int("lostPackets", st.lost_packets as i64);
            let c = &s.conn.counters;
            j.int("framesReceived", c.received.load(Ordering::Relaxed) as i64)
                .int("framesSkipped", c.skipped.load(Ordering::Relaxed) as i64);
        }
        ClientEvent::CursorShape {
            shape_id,
            width,
            height,
            hot_x,
            hot_y,
            rgba,
        } => {
            if let Ok(mut m) = s.shapes.lock() {
                if m.len() > 32 {
                    m.clear();
                }
                m.insert(shape_id, (width, height, hot_x, hot_y, rgba));
            }
            j.str("type", "cursorShape")
                .int("id", shape_id as i64)
                .int("width", width as i64)
                .int("height", height as i64)
                .int("hotX", hot_x as i64)
                .int("hotY", hot_y as i64);
        }
        ClientEvent::CursorPos {
            x,
            y,
            visible,
            shape_id,
        } => {
            j.str("type", "cursor")
                .int("x", x as i64)
                .int("y", y as i64)
                .bool("visible", visible)
                .int("shape", shape_id as i64);
        }
        ClientEvent::Caret {
            visible,
            x,
            y,
            w,
            h,
        } => {
            j.str("type", "caret")
                .bool("visible", visible)
                .int("x", x as i64)
                .int("y", y as i64)
                .int("w", w as i64)
                .int("h", h as i64);
        }
        ClientEvent::InputAck { seq } => {
            j.str("type", "inputAck").int("seq", seq as i64);
        }
        ClientEvent::Rtt { rtt_us, offset_us } => {
            j.str("type", "rtt")
                .int("rttUs", rtt_us as i64)
                .int("offsetUs", offset_us);
        }
        ClientEvent::Notice(code, msg) => {
            j.str("type", "notice")
                .int("code", code.as_u16() as i64)
                .str("message", &msg);
        }
        ClientEvent::Error(code, msg) => {
            j.str("type", "error")
                .int("code", code.as_u16() as i64)
                .str("message", &msg);
        }
        ClientEvent::Closed(reason) => {
            j.str("type", "closed").str("message", &reason);
        }
    }
    j.finish()
}

/// Wait up to `timeout_ms` for the next control event. Returns JSON, or
/// `None` on timeout.
pub fn poll_event(h: i64, timeout_ms: i32) -> Result<Option<String>, String> {
    let s = session(h)?;
    let ev = {
        let mut rx = s.events.lock().map_err(|_| "lock")?;
        let d = Duration::from_millis(timeout_ms.clamp(0, 10_000) as u64);
        runtime().block_on(async { tokio::time::timeout(d, rx.recv()).await })
    };
    match ev {
        Ok(Some(ev)) => Ok(Some(event_json(&s, ev))),
        Ok(None) => {
            let mut j = Json::object();
            j.str("type", "closed").str("message", "connection closed");
            Ok(Some(j.finish()))
        }
        Err(_) => Ok(None),
    }
}

/// RGBA pixels of a cursor shape announced by a `cursorShape` event.
pub fn cursor_pixels(h: i64, shape_id: i64) -> Result<Option<Vec<u8>>, String> {
    let s = session(h)?;
    let shapes = s.shapes.lock().map_err(|_| "lock")?;
    Ok(shapes
        .get(&(shape_id as u32))
        .map(|(_, _, _, _, rgba)| rgba.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_and_nests() {
        let mut inner = Json::object();
        inner.int("a", -1).bool("b", true);
        let mut j = Json::object();
        j.str("name", "Sam's \"PC\"\n\u{1}")
            .str_array("xs", ["1".to_string(), "\\".to_string()].into_iter())
            .raw_array("objs", vec![inner.finish()]);
        assert_eq!(
            j.finish(),
            r#"{"name":"Sam's \"PC\"\n\u0001","xs":["1","\\"],"objs":[{"a":-1,"b":true}]}"#
        );
    }
}
