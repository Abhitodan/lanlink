//! Input events.
//!
//! State *transitions* (buttons, keys, text) travel on a reliable ordered
//! stream because losing one leaves the host in the wrong state. Pointer
//! *positions* travel as datagrams: a newer position supersedes an older one,
//! so retransmitting a lost one would only add latency.
//!
//! Coordinates are normalized to the captured display: `0` is the left/top
//! edge and `65535` the right/bottom edge. The client resolves letterboxing,
//! zoom, pan and rotation before sending; the host resolves DPI and the
//! virtual-desktop offset. See `docs/protocol.md`.

use crate::error::DecodeError;
use crate::limits::*;
use crate::wire::{Reader, Writer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum MouseButton {
    Left = 0,
    Right = 1,
    Middle = 2,
    X1 = 3,
    X2 = 4,
}

impl MouseButton {
    pub const ALL: [MouseButton; 5] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::X1,
        MouseButton::X2,
    ];

    fn from_u8(v: u8) -> Result<Self, DecodeError> {
        match v {
            0 => Ok(MouseButton::Left),
            1 => Ok(MouseButton::Right),
            2 => Ok(MouseButton::Middle),
            3 => Ok(MouseButton::X1),
            4 => Ok(MouseButton::X2),
            _ => Err(DecodeError::InvalidValue("button")),
        }
    }
}

/// Body of a reliable input record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputKind {
    /// Button transition. When `pos` is set the host first moves the pointer
    /// there, so a click lands where the user touched even if the preceding
    /// position datagram was lost.
    Button {
        button: MouseButton,
        down: bool,
        pos: Option<(u16, u16)>,
    },
    /// Wheel movement in units of 1/120 of a notch. Positive `dy` scrolls
    /// content up (wheel away from the user); positive `dx` scrolls right.
    Scroll { dx: i16, dy: i16 },
    /// Key transition. `vk` is a Windows virtual-key code (key space 1).
    Key { vk: u16, down: bool },
    /// Committed Unicode text, injected independent of keyboard layout.
    Text { text: String },
    /// Release every button and key the host believes this client holds.
    ReleaseAll,
    /// Reliable absolute move (used when datagrams are unavailable).
    MoveAbs { x: u16, y: u16 },
    /// Reliable relative move in host pixels.
    MoveRel { dx: i16, dy: i16 },
}

/// One reliable input record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputEvent {
    /// Session the event belongs to. Events for any other session are stale
    /// and dropped by the host.
    pub session_id: u32,
    /// Strictly increasing per session, starting at 1.
    pub seq: u32,
    pub kind: InputKind,
}

mod tag {
    pub const BUTTON: u8 = 1;
    pub const SCROLL: u8 = 2;
    pub const KEY: u8 = 3;
    pub const TEXT: u8 = 4;
    pub const RELEASE_ALL: u8 = 5;
    pub const MOVE_ABS: u8 = 6;
    pub const MOVE_REL: u8 = 7;
}

impl InputEvent {
    /// Encode as `u16 body_len | u8 tag | u32 session | u32 seq | fields`.
    pub fn encode_framed(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(24);
        w.u16(0); // patched below
        let t = match &self.kind {
            InputKind::Button { .. } => tag::BUTTON,
            InputKind::Scroll { .. } => tag::SCROLL,
            InputKind::Key { .. } => tag::KEY,
            InputKind::Text { .. } => tag::TEXT,
            InputKind::ReleaseAll => tag::RELEASE_ALL,
            InputKind::MoveAbs { .. } => tag::MOVE_ABS,
            InputKind::MoveRel { .. } => tag::MOVE_REL,
        };
        w.u8(t);
        w.u32(self.session_id);
        w.u32(self.seq);
        match &self.kind {
            InputKind::Button { button, down, pos } => {
                w.u8(*button as u8);
                w.bool(*down);
                w.bool(pos.is_some());
                let (x, y) = pos.unwrap_or((0, 0));
                w.u16(x);
                w.u16(y);
            }
            InputKind::Scroll { dx, dy } => {
                w.i16(*dx);
                w.i16(*dy);
            }
            InputKind::Key { vk, down } => {
                w.u16(*vk);
                w.bool(*down);
            }
            InputKind::Text { text } => w.str(text, MAX_TEXT_INPUT),
            InputKind::ReleaseAll => {}
            InputKind::MoveAbs { x, y } => {
                w.u16(*x);
                w.u16(*y);
            }
            InputKind::MoveRel { dx, dy } => {
                w.i16(*dx);
                w.i16(*dy);
            }
        }
        let mut out = w.into_bytes();
        let body_len = (out.len() - 2) as u16;
        out[..2].copy_from_slice(&body_len.to_le_bytes());
        out
    }

    /// Decode a record body (without the `u16` length prefix).
    pub fn decode_body(body: &[u8]) -> Result<Self, DecodeError> {
        if body.len() > MAX_INPUT_MSG {
            return Err(DecodeError::FrameTooLarge(body.len()));
        }
        let mut r = Reader::new(body);
        let t = r.u8()?;
        let session_id = r.u32()?;
        let seq = r.u32()?;
        let kind = match t {
            tag::BUTTON => {
                let button = MouseButton::from_u8(r.u8()?)?;
                let down = r.bool()?;
                let has_pos = r.bool()?;
                let x = r.u16()?;
                let y = r.u16()?;
                InputKind::Button {
                    button,
                    down,
                    pos: has_pos.then_some((x, y)),
                }
            }
            tag::SCROLL => InputKind::Scroll {
                dx: r.i16()?,
                dy: r.i16()?,
            },
            tag::KEY => {
                let vk = r.u16()?;
                // Windows virtual-key codes occupy 1..=254.
                if vk == 0 || vk > 0xFE {
                    return Err(DecodeError::InvalidValue("vk"));
                }
                InputKind::Key {
                    vk,
                    down: r.bool()?,
                }
            }
            tag::TEXT => {
                let text = r.str(MAX_TEXT_INPUT, "text")?;
                if text.chars().any(|c| c == '\0') {
                    return Err(DecodeError::InvalidValue("text"));
                }
                InputKind::Text { text }
            }
            tag::RELEASE_ALL => InputKind::ReleaseAll,
            tag::MOVE_ABS => InputKind::MoveAbs {
                x: r.u16()?,
                y: r.u16()?,
            },
            tag::MOVE_REL => InputKind::MoveRel {
                dx: r.i16()?,
                dy: r.i16()?,
            },
            other => return Err(DecodeError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(InputEvent {
            session_id,
            seq,
            kind,
        })
    }
}

/// Unreliable datagrams. The first byte is the tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Datagram {
    /// C→H absolute pointer position.
    PointerAbs {
        session_id: u32,
        seq: u32,
        x: u16,
        y: u16,
    },
    /// C→H relative pointer motion in host pixels.
    PointerRel {
        session_id: u32,
        seq: u32,
        dx: i16,
        dy: i16,
    },
    /// H→C authoritative cursor state on the captured display.
    CursorPos {
        session_id: u32,
        seq: u32,
        x: u16,
        y: u16,
        shape_id: u32,
        visible: bool,
    },
}

mod dtag {
    pub const POINTER_ABS: u8 = 0x10;
    pub const POINTER_REL: u8 = 0x11;
    pub const CURSOR_POS: u8 = 0x20;
}

impl Datagram {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(20);
        match *self {
            Datagram::PointerAbs {
                session_id,
                seq,
                x,
                y,
            } => {
                w.u8(dtag::POINTER_ABS);
                w.u32(session_id);
                w.u32(seq);
                w.u16(x);
                w.u16(y);
            }
            Datagram::PointerRel {
                session_id,
                seq,
                dx,
                dy,
            } => {
                w.u8(dtag::POINTER_REL);
                w.u32(session_id);
                w.u32(seq);
                w.i16(dx);
                w.i16(dy);
            }
            Datagram::CursorPos {
                session_id,
                seq,
                x,
                y,
                shape_id,
                visible,
            } => {
                w.u8(dtag::CURSOR_POS);
                w.u32(session_id);
                w.u32(seq);
                w.u16(x);
                w.u16(y);
                w.u32(shape_id);
                w.bool(visible);
            }
        }
        w.into_bytes()
    }

    pub fn decode(buf: &[u8]) -> Result<Self, DecodeError> {
        if buf.len() > MAX_DATAGRAM {
            return Err(DecodeError::FrameTooLarge(buf.len()));
        }
        let mut r = Reader::new(buf);
        let d = match r.u8()? {
            dtag::POINTER_ABS => Datagram::PointerAbs {
                session_id: r.u32()?,
                seq: r.u32()?,
                x: r.u16()?,
                y: r.u16()?,
            },
            dtag::POINTER_REL => Datagram::PointerRel {
                session_id: r.u32()?,
                seq: r.u32()?,
                dx: r.i16()?,
                dy: r.i16()?,
            },
            dtag::CURSOR_POS => Datagram::CursorPos {
                session_id: r.u32()?,
                seq: r.u32()?,
                x: r.u16()?,
                y: r.u16()?,
                shape_id: r.u32()?,
                visible: r.bool()?,
            },
            other => return Err(DecodeError::UnknownTag(other)),
        };
        r.finish()?;
        Ok(d)
    }
}

/// Wrapping "is `a` newer than `b`" for datagram sequence numbers.
///
/// Datagrams can be reordered or duplicated; receivers keep the newest
/// sequence number seen and drop anything that is not newer.
pub fn seq_newer(a: u32, b: u32) -> bool {
    a != b && a.wrapping_sub(b) < 0x8000_0000
}

/// Tracks the reliable-input sequence for one session and rejects stale,
/// duplicated or out-of-session events.
#[derive(Debug)]
pub struct InputSequencer {
    session_id: u32,
    last_seq: u32,
}

/// Why an input event was not accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputReject {
    WrongSession,
    Duplicate,
    OutOfOrder,
}

impl InputSequencer {
    pub fn new(session_id: u32) -> Self {
        Self {
            session_id,
            last_seq: 0,
        }
    }

    pub fn session_id(&self) -> u32 {
        self.session_id
    }

    /// Accept `seq` only if it is exactly the next one. The reliable stream
    /// already guarantees order, so anything else indicates a replayed or
    /// forged record and is refused rather than "repaired".
    pub fn accept(&mut self, session_id: u32, seq: u32) -> Result<(), InputReject> {
        if session_id != self.session_id {
            return Err(InputReject::WrongSession);
        }
        if seq == self.last_seq {
            return Err(InputReject::Duplicate);
        }
        if seq != self.last_seq.wrapping_add(1) {
            return Err(InputReject::OutOfOrder);
        }
        self.last_seq = seq;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seq_wraps() {
        assert!(seq_newer(1, 0));
        assert!(!seq_newer(0, 1));
        assert!(!seq_newer(5, 5));
        assert!(seq_newer(2, u32::MAX));
        assert!(!seq_newer(u32::MAX, 2));
    }

    #[test]
    fn sequencer_rejects_duplicates_and_gaps() {
        let mut s = InputSequencer::new(9);
        assert_eq!(s.accept(9, 1), Ok(()));
        assert_eq!(s.accept(9, 1), Err(InputReject::Duplicate));
        assert_eq!(s.accept(9, 3), Err(InputReject::OutOfOrder));
        assert_eq!(s.accept(8, 2), Err(InputReject::WrongSession));
        assert_eq!(s.accept(9, 2), Ok(()));
    }

    #[test]
    fn button_test_vector() {
        let e = InputEvent {
            session_id: 1,
            seq: 2,
            kind: InputKind::Button {
                button: MouseButton::Right,
                down: true,
                pos: Some((0x1234, 0xffff)),
            },
        };
        assert_eq!(
            e.encode_framed(),
            [
                16, 0, // body length
                1, // tag
                1, 0, 0, 0, // session
                2, 0, 0, 0, // seq
                1, 1, 1, // button, down, has_pos
                0x34, 0x12, 0xff, 0xff,
            ]
        );
    }

    #[test]
    fn key_rejects_out_of_range_vk() {
        for vk in [0u16, 0xFF, 0x1000] {
            let e = InputEvent {
                session_id: 1,
                seq: 1,
                kind: InputKind::Key { vk, down: true },
            };
            let framed = e.encode_framed();
            assert!(InputEvent::decode_body(&framed[2..]).is_err(), "vk {vk}");
        }
    }

    #[test]
    fn text_rejects_nul() {
        let e = InputEvent {
            session_id: 1,
            seq: 1,
            kind: InputKind::Text {
                text: "a\0b".into(),
            },
        };
        assert!(InputEvent::decode_body(&e.encode_framed()[2..]).is_err());
    }

    #[test]
    fn datagram_roundtrip_and_strictness() {
        let d = Datagram::CursorPos {
            session_id: 3,
            seq: 4,
            x: 5,
            y: 6,
            shape_id: 7,
            visible: true,
        };
        let mut b = d.encode();
        assert_eq!(Datagram::decode(&b), Ok(d));
        b.push(0);
        assert_eq!(Datagram::decode(&b), Err(DecodeError::TrailingBytes));
        assert_eq!(Datagram::decode(&[0x10, 1]), Err(DecodeError::Truncated));
    }
}
