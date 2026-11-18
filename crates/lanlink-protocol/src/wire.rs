//! Bounded little-endian primitives used by every LANLink message.
//!
//! The reader never allocates more than the limit passed by the caller for a
//! field, and never trusts a length prefix beyond the bytes actually present.

use crate::error::DecodeError;

/// Append-only encoder over a `Vec<u8>`.
#[derive(Debug, Default)]
pub struct Writer {
    buf: Vec<u8>,
}

impl Writer {
    pub fn new() -> Self {
        Self { buf: Vec::new() }
    }

    pub fn with_capacity(n: usize) -> Self {
        Self {
            buf: Vec::with_capacity(n),
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    pub fn bool(&mut self, v: bool) {
        self.buf.push(u8::from(v));
    }

    pub fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i16(&mut self, v: i16) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn i32(&mut self, v: i32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    pub fn raw(&mut self, v: &[u8]) {
        self.buf.extend_from_slice(v);
    }

    /// UTF-8 string with a `u16` byte-length prefix. Strings longer than
    /// `max` bytes are truncated on a character boundary so that an encoder
    /// can never produce a message its peer must reject.
    pub fn str(&mut self, v: &str, max: usize) {
        let v = truncate_utf8(v, max.min(u16::MAX as usize));
        self.u16(v.len() as u16);
        self.buf.extend_from_slice(v.as_bytes());
    }

    /// Byte string with a `u32` length prefix.
    ///
    /// # Panics
    /// Panics if `v` exceeds `u32::MAX` bytes; callers bound payloads far
    /// below that through [`crate::limits`].
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(u32::try_from(v.len()).expect("payload exceeds u32"));
        self.buf.extend_from_slice(v);
    }
}

/// Truncate `s` to at most `max` bytes without splitting a code point.
pub fn truncate_utf8(s: &str, max: usize) -> &str {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Cursor over a received byte slice.
#[derive(Debug)]
pub struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.buf.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.remaining() < n {
            return Err(DecodeError::Truncated);
        }
        let out = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(out)
    }

    pub fn u8(&mut self) -> Result<u8, DecodeError> {
        Ok(self.take(1)?[0])
    }

    /// Booleans are strictly `0` or `1`; anything else is malformed.
    pub fn bool(&mut self) -> Result<bool, DecodeError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(DecodeError::InvalidValue("bool")),
        }
    }

    pub fn u16(&mut self) -> Result<u16, DecodeError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    pub fn i16(&mut self) -> Result<i16, DecodeError> {
        let b = self.take(2)?;
        Ok(i16::from_le_bytes([b[0], b[1]]))
    }

    pub fn u32(&mut self) -> Result<u32, DecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn i32(&mut self) -> Result<i32, DecodeError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    pub fn u64(&mut self) -> Result<u64, DecodeError> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    pub fn array<const N: usize>(&mut self) -> Result<[u8; N], DecodeError> {
        let b = self.take(N)?;
        let mut a = [0u8; N];
        a.copy_from_slice(b);
        Ok(a)
    }

    pub fn str(&mut self, max: usize, field: &'static str) -> Result<String, DecodeError> {
        let len = self.u16()? as usize;
        if len > max {
            return Err(DecodeError::TooLarge(field));
        }
        let b = self.take(len)?;
        std::str::from_utf8(b)
            .map(str::to_owned)
            .map_err(|_| DecodeError::InvalidUtf8(field))
    }

    pub fn bytes(&mut self, max: usize, field: &'static str) -> Result<Vec<u8>, DecodeError> {
        let len = self.u32()? as usize;
        if len > max {
            return Err(DecodeError::TooLarge(field));
        }
        Ok(self.take(len)?.to_vec())
    }

    /// Reject messages that carry bytes the decoder did not consume. Unknown
    /// trailing data is how smuggling and version-confusion bugs start.
    pub fn finish(self) -> Result<(), DecodeError> {
        if self.remaining() != 0 {
            return Err(DecodeError::TrailingBytes);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_char_boundary() {
        assert_eq!(truncate_utf8("héllo", 2), "h");
        assert_eq!(truncate_utf8("héllo", 3), "hé");
        assert_eq!(truncate_utf8("abc", 10), "abc");
    }

    #[test]
    fn rejects_non_canonical_bool() {
        assert_eq!(
            Reader::new(&[2]).bool(),
            Err(DecodeError::InvalidValue("bool"))
        );
    }

    #[test]
    fn length_prefix_cannot_overrun() {
        // Claims 4 GiB of payload but carries none.
        let mut r = Reader::new(&[0xff, 0xff, 0xff, 0xff]);
        assert_eq!(r.bytes(usize::MAX, "x"), Err(DecodeError::Truncated));
        let mut r = Reader::new(&[0x10, 0, 0, 0]);
        assert_eq!(r.bytes(8, "x"), Err(DecodeError::TooLarge("x")));
    }
}

/// A peer-supplied name made safe to show: control characters and Unicode
/// formatting characters that reorder or hide text (bidi overrides and
/// isolates, zero-width marks, BOM) are removed, whitespace runs collapse,
/// and the result is capped at `max_chars`. Without this a device could
/// call itself, say, "\u{202E}enohP s'ibhA" and read as a trusted name in
/// an approval dialog.
pub fn display_name(s: &str, max_chars: usize) -> String {
    fn hidden(c: char) -> bool {
        c.is_control()
            || matches!(c,
                '\u{00AD}'               // soft hyphen
                | '\u{0600}'..='\u{0605}' // Arabic number signs (format)
                | '\u{06DD}'
                | '\u{070F}'
                | '\u{08E2}'
                | '\u{110BD}'
                | '\u{1D173}'..='\u{1D17A}'
                | '\u{E0000}'..='\u{E007F}' // tag characters (invisible)
                | '\u{061C}'             // Arabic letter mark
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}' // zero-width, LRM/RLM
                | '\u{2028}'..='\u{202E}' // separators, bidi embeddings/overrides
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{206F}' // bidi isolates, deprecated formats
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}')
    }
    let mut out = String::new();
    let mut space = false;
    // Line breaks and tabs are whitespace before they are control
    // characters: they separate words, so they become a space.
    for c in s.chars().filter(|c| c.is_whitespace() || !hidden(*c)) {
        if c.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(c);
    }
    out.chars().take(max_chars).collect()
}

#[cfg(test)]
mod display_name_tests {
    use super::display_name;

    #[test]
    fn strips_controls_and_text_direction_tricks() {
        assert_eq!(display_name("Pixel 10 Pro", 64), "Pixel 10 Pro");
        assert_eq!(display_name("\u{202E}enohP\u{202C}", 64), "enohP");
        assert_eq!(display_name("A\u{2066}B\u{2069}\u{200B}C", 64), "ABC");
        assert_eq!(display_name("P\u{E0041}\u{E0042}C\u{0600}", 64), "PC");
        assert_eq!(display_name("  Line\n\tbreak  ", 64), "Line break");
        assert_eq!(display_name("\u{1}\u{7f}", 64), "");
        assert_eq!(display_name("Pixel 10 Pro", 5), "Pixel");
        assert_eq!(display_name("Sam's 💻", 64), "Sam's 💻");
    }
}
