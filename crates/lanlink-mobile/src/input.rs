//! Pointer, button, scroll, key and text input.

use crate::session::session;
use lanlink_client::lanlink_protocol::*;

pub fn send_pointer(h: i64, x: i32, y: i32) -> Result<(), String> {
    session(h)?
        .conn
        .send_pointer(x.clamp(0, 65535) as u16, y.clamp(0, 65535) as u16);
    Ok(())
}

pub fn send_pointer_rel(h: i64, dx: i32, dy: i32) -> Result<(), String> {
    session(h)?.conn.send_pointer_rel(
        dx.clamp(-32768, 32767) as i16,
        dy.clamp(-32768, 32767) as i16,
    );
    Ok(())
}

fn input(h: i64, kind: InputKind) -> Result<bool, String> {
    Ok(session(h)?.conn.send_input(kind))
}

/// `button`: 0 left, 1 right, 2 middle, 3 X1, 4 X2. `x`/`y` < 0 means
/// "no position".
pub fn send_button(h: i64, button: i32, down: bool, x: i32, y: i32) -> Result<bool, String> {
    let button = match button {
        0 => MouseButton::Left,
        1 => MouseButton::Right,
        2 => MouseButton::Middle,
        3 => MouseButton::X1,
        4 => MouseButton::X2,
        _ => return Err("invalid button".into()),
    };
    let pos = (x >= 0 && y >= 0).then(|| (x.min(65535) as u16, y.min(65535) as u16));
    input(h, InputKind::Button { button, down, pos })
}

pub fn send_scroll(h: i64, dx: i32, dy: i32) -> Result<bool, String> {
    input(
        h,
        InputKind::Scroll {
            dx: dx.clamp(-1200, 1200) as i16,
            dy: dy.clamp(-1200, 1200) as i16,
        },
    )
}

pub fn send_key(h: i64, vk: i32, down: bool) -> Result<bool, String> {
    if !(1..=0xFE).contains(&vk) {
        return Err("invalid key".into());
    }
    input(
        h,
        InputKind::Key {
            vk: vk as u16,
            down,
        },
    )
}

pub fn send_text(h: i64, text: &str) -> Result<bool, String> {
    let t: String = text.chars().filter(|c| *c != '\0').collect();
    if t.is_empty() {
        return Ok(true);
    }
    // Split long pastes into protocol-sized pieces on char boundaries.
    let mut ok = true;
    let mut chunk = String::new();
    for c in t.chars() {
        if chunk.len() + c.len_utf8() > limits::MAX_TEXT_INPUT {
            ok &= input(
                h,
                InputKind::Text {
                    text: std::mem::take(&mut chunk),
                },
            )?;
        }
        chunk.push(c);
    }
    if !chunk.is_empty() {
        ok &= input(h, InputKind::Text { text: chunk })?;
    }
    Ok(ok)
}

pub fn release_all(h: i64) -> Result<(), String> {
    input(h, InputKind::ReleaseAll).map(|_| ())
}
