//! Minimal JSON object writer for the low-rate control-plane events that
//! cross JNI. Output only; the app parses it with `org.json`.

pub struct Json {
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
    pub fn object() -> Self {
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

    pub fn str(&mut self, k: &str, v: &str) -> &mut Self {
        self.key(k);
        escape(v, &mut self.buf);
        self
    }

    pub fn int(&mut self, k: &str, v: i64) -> &mut Self {
        self.key(k);
        self.buf.push_str(&v.to_string());
        self
    }

    pub fn bool(&mut self, k: &str, v: bool) -> &mut Self {
        self.key(k);
        self.buf.push_str(if v { "true" } else { "false" });
        self
    }

    pub fn str_array(&mut self, k: &str, items: impl Iterator<Item = String>) -> &mut Self {
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
    pub fn raw_array(&mut self, k: &str, items: Vec<String>) -> &mut Self {
        self.key(k);
        self.buf.push('[');
        self.buf.push_str(&items.join(","));
        self.buf.push(']');
        self
    }

    pub fn finish(&mut self) -> String {
        let mut out = std::mem::take(&mut self.buf);
        out.push('}');
        out
    }
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
