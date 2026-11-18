//! Platform-neutral low-level input events and their translation from the
//! LANLink protocol. Everything here is pure logic and unit-tested; only
//! `sendinput.rs` touches the operating system.

use lanlink_protocol::MouseButton;

/// Pixel rectangle in virtual-desktop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x
            && py >= self.y
            && ((px - self.x) as i64) < self.width as i64
            && ((py - self.y) as i64) < self.height as i64
    }
}

/// One low-level event, as handed to the operating system.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Absolute move, already converted to the 0..=65535 virtual-desktop
    /// space that `SendInput` expects with `MOUSEEVENTF_VIRTUALDESK`.
    MoveAbs {
        x: u16,
        y: u16,
    },
    /// Relative move in pixels (subject to the user's pointer speed).
    MoveRel {
        dx: i32,
        dy: i32,
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    /// Wheel delta in 1/120 notch units; positive = away from user / right.
    Wheel {
        delta: i32,
        horizontal: bool,
    },
    Key {
        vk: u16,
        scan: u16,
        extended: bool,
        down: bool,
    },
    /// One UTF-16 code unit, layout-independent.
    Unicode {
        unit: u16,
        down: bool,
    },
}

/// Map a pixel in virtual-desktop coordinates to the normalized absolute
/// coordinate Windows expects for `MOUSEEVENTF_VIRTUALDESK`.
///
/// Windows converts back with `pixel = origin + floor(abs * extent / 65536)`,
/// so we use the smallest `abs` that lands on the requested pixel (ceiling
/// division). The tests check this inverse for every pixel.
pub fn to_virtual_abs(px: i32, origin: i32, extent: u32) -> u16 {
    if extent == 0 {
        return 0;
    }
    let rel = (px as i64 - origin as i64).clamp(0, extent as i64 - 1);
    let abs = (rel * 65536 + extent as i64 - 1) / extent as i64;
    abs.min(65535) as u16
}

/// Inverse used by tests: what pixel Windows will move to.
pub fn from_virtual_abs(abs: u16, origin: i32, extent: u32) -> i32 {
    origin + ((abs as i64 * extent as i64) >> 16) as i32
}

/// Convert a normalized display coordinate (0..=65535 across `display`)
/// into an absolute event on the virtual desktop `desktop`.
pub fn display_point_to_event(x: u16, y: u16, display: Rect, desktop: Rect) -> Event {
    let px = display.x + lanlink_protocol::denormalize(x, display.width);
    let py = display.y + lanlink_protocol::denormalize(y, display.height);
    Event::MoveAbs {
        x: to_virtual_abs(px, desktop.x, desktop.width),
        y: to_virtual_abs(py, desktop.y, desktop.height),
    }
}

// Virtual-key codes used below (winuser.h).
pub const VK_BACK: u16 = 0x08;
pub const VK_TAB: u16 = 0x09;
pub const VK_RETURN: u16 = 0x0D;
pub const VK_SHIFT: u16 = 0x10;
pub const VK_CONTROL: u16 = 0x11;
pub const VK_MENU: u16 = 0x12;
pub const VK_LSHIFT: u16 = 0xA0;
pub const VK_RSHIFT: u16 = 0xA1;
pub const VK_LCONTROL: u16 = 0xA2;
pub const VK_RCONTROL: u16 = 0xA3;
pub const VK_LMENU: u16 = 0xA4;
pub const VK_RMENU: u16 = 0xA5;
pub const VK_LWIN: u16 = 0x5B;
pub const VK_RWIN: u16 = 0x5C;
pub const VK_APPS: u16 = 0x5D;

/// Generic modifier codes are ambiguous for injection; use the left key.
pub fn canonical_vk(vk: u16) -> u16 {
    match vk {
        VK_SHIFT => VK_LSHIFT,
        VK_CONTROL => VK_LCONTROL,
        VK_MENU => VK_LMENU,
        other => other,
    }
}

/// Keys that live on the "extended" part of the keyboard and need
/// `KEYEVENTF_EXTENDEDKEY`, otherwise Windows treats e.g. the arrow keys as
/// numeric-keypad keys.
pub fn is_extended(vk: u16) -> bool {
    matches!(
        vk,
        0x21..=0x28   // PageUp, PageDown, End, Home, arrows
        | 0x2C        // PrintScreen
        | 0x2D | 0x2E // Insert, Delete
        | 0x6F        // Numpad divide
        | 0x90        // NumLock
        | VK_RCONTROL | VK_RMENU | VK_LWIN | VK_RWIN | VK_APPS
        | 0xA6..=0xB7 // browser and media keys
    )
}

pub fn is_modifier(vk: u16) -> bool {
    matches!(
        vk,
        VK_SHIFT
            | VK_CONTROL
            | VK_MENU
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_LMENU
            | VK_RMENU
            | VK_LWIN
            | VK_RWIN
    )
}

/// Translate committed text into key events.
///
/// Most characters are sent as `KEYEVENTF_UNICODE` packets, which bypass the
/// keyboard layout, so the PC receives exactly what was typed on the phone,
/// including characters outside the Basic Multilingual Plane (sent as a
/// surrogate pair). Newline and tab become real Enter / Tab key presses
/// because many applications ignore them as Unicode packets. `\r` is
/// dropped so `\r\n` produces a single Enter.
pub fn text_to_events(text: &str, scan_of: impl Fn(u16) -> u16) -> Vec<Event> {
    let mut out = Vec::with_capacity(text.len() * 2);
    let key = |vk: u16, out: &mut Vec<Event>| {
        let scan = scan_of(vk);
        out.push(Event::Key {
            vk,
            scan,
            extended: false,
            down: true,
        });
        out.push(Event::Key {
            vk,
            scan,
            extended: false,
            down: false,
        });
    };
    for c in text.chars() {
        match c {
            '\r' => {}
            '\n' => key(VK_RETURN, &mut out),
            '\t' => key(VK_TAB, &mut out),
            c if c.is_control() => {}
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf).iter() {
                    out.push(Event::Unicode {
                        unit: *unit,
                        down: true,
                    });
                    out.push(Event::Unicode {
                        unit: *unit,
                        down: false,
                    });
                }
            }
        }
    }
    out
}

/// Tracks what this client is currently holding down, so that disconnects
/// and session ends can never leave a button or modifier stuck.
#[derive(Debug, Default)]
pub struct HeldState {
    buttons: [bool; 5],
    keys: Vec<u16>,
}

/// Upper bound on simultaneously held keys; more is not a real keyboard.
pub const MAX_HELD_KEYS: usize = 16;

impl HeldState {
    pub fn button_index(b: MouseButton) -> usize {
        b as usize
    }

    /// Record a button transition. Returns `false` if it would not change
    /// state (a duplicate press or a release of an unpressed button), in
    /// which case the event should not be injected.
    pub fn button(&mut self, b: MouseButton, down: bool) -> bool {
        let slot = &mut self.buttons[Self::button_index(b)];
        if *slot == down {
            return false;
        }
        *slot = down;
        true
    }

    /// Record a key transition. Key-repeat (down while held) is allowed and
    /// passed through; releases of keys we never pressed are dropped.
    /// Returns `false` if the event should not be injected.
    pub fn key(&mut self, vk: u16, down: bool) -> bool {
        let held = self.keys.iter().position(|k| *k == vk);
        match (down, held) {
            (true, Some(_)) => true, // auto-repeat
            (true, None) => {
                if self.keys.len() >= MAX_HELD_KEYS {
                    return false;
                }
                self.keys.push(vk);
                true
            }
            (false, Some(i)) => {
                self.keys.remove(i);
                true
            }
            (false, None) => false,
        }
    }

    pub fn is_button_down(&self, b: MouseButton) -> bool {
        self.buttons[Self::button_index(b)]
    }

    pub fn held_keys(&self) -> &[u16] {
        &self.keys
    }

    pub fn anything_held(&self) -> bool {
        self.buttons.iter().any(|b| *b) || !self.keys.is_empty()
    }

    /// Events that release everything held, keys in reverse press order,
    /// then buttons. Clears the state.
    pub fn release_all(&mut self, scan_of: impl Fn(u16) -> u16) -> Vec<Event> {
        let mut out = Vec::new();
        for vk in self.keys.drain(..).rev() {
            out.push(Event::Key {
                vk,
                scan: scan_of(vk),
                extended: is_extended(vk),
                down: false,
            });
        }
        for b in MouseButton::ALL {
            let slot = &mut self.buttons[Self::button_index(b)];
            if *slot {
                *slot = false;
                out.push(Event::Button {
                    button: b,
                    down: false,
                });
            }
        }
        out
    }
}

/// Token bucket bounding how many input operations a client may inject.
#[derive(Debug)]
pub struct RateLimiter {
    capacity: f64,
    tokens: f64,
    per_sec: f64,
    last: std::time::Instant,
}

impl RateLimiter {
    pub fn new(per_sec: u32, burst: u32) -> Self {
        RateLimiter {
            capacity: burst as f64,
            tokens: burst as f64,
            per_sec: per_sec as f64,
            last: std::time::Instant::now(),
        }
    }

    pub fn allow_at(&mut self, now: std::time::Instant, cost: u32) -> bool {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.per_sec).min(self.capacity);
        if self.tokens >= cost as f64 {
            self.tokens -= cost as f64;
            true
        } else {
            false
        }
    }

    pub fn allow(&mut self, cost: u32) -> bool {
        self.allow_at(std::time::Instant::now(), cost)
    }

    /// How long until `min(cost, burst)` tokens are available. A cost larger
    /// than the burst can never be fully saved up, so it waits for a full
    /// bucket and the rest becomes debt ([`take_at`](Self::take_at)).
    pub fn wait_at(&mut self, now: std::time::Instant, cost: u32) -> std::time::Duration {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.per_sec).min(self.capacity);
        let need = (cost as f64).min(self.capacity) - self.tokens;
        if need <= 0.0 {
            std::time::Duration::ZERO
        } else {
            std::time::Duration::from_secs_f64(need / self.per_sec)
        }
    }

    /// Spend `cost` tokens unconditionally; the balance may go negative,
    /// which delays whatever comes next. Keeps the long-run rate exact for
    /// operations that are paced rather than refused.
    pub fn take_at(&mut self, now: std::time::Instant, cost: u32) {
        let dt = now.saturating_duration_since(self.last).as_secs_f64();
        self.last = now;
        self.tokens = (self.tokens + dt * self.per_sec).min(self.capacity) - cost as f64;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn virtual_abs_hits_every_pixel_exactly() {
        for (origin, extent) in [(0, 2560u32), (-1920, 4480), (0, 1), (-100, 3), (0, 7680)] {
            for px in origin..origin + extent as i32 {
                let abs = to_virtual_abs(px, origin, extent);
                assert_eq!(
                    from_virtual_abs(abs, origin, extent),
                    px,
                    "origin {origin} extent {extent}"
                );
            }
            // Out-of-range pixels clamp to the edges.
            assert_eq!(
                from_virtual_abs(to_virtual_abs(origin - 50, origin, extent), origin, extent),
                origin
            );
            assert_eq!(
                from_virtual_abs(
                    to_virtual_abs(origin + extent as i32 + 50, origin, extent),
                    origin,
                    extent
                ),
                origin + extent as i32 - 1
            );
        }
    }

    #[test]
    fn display_corners_map_to_display_corners_on_negative_monitor() {
        // Secondary display left of the primary: virtual desktop spans both.
        let display = Rect {
            x: -1920,
            y: 200,
            width: 1920,
            height: 1080,
        };
        let desktop = Rect {
            x: -1920,
            y: 0,
            width: 1920 + 2560,
            height: 1600,
        };
        for (nx, ny, ex, ey) in [
            (0u16, 0u16, -1920, 200),
            (65535, 65535, -1, 1279),
            (32768, 0, -960, 200),
        ] {
            let Event::MoveAbs { x, y } = display_point_to_event(nx, ny, display, desktop) else {
                panic!()
            };
            assert_eq!(from_virtual_abs(x, desktop.x, desktop.width), ex);
            assert_eq!(from_virtual_abs(y, desktop.y, desktop.height), ey);
        }
    }

    #[test]
    fn extended_keys() {
        for vk in [
            0x25,
            0x26,
            0x27,
            0x28,
            0x2D,
            0x2E,
            0x24,
            0x23,
            0x21,
            0x22,
            VK_RCONTROL,
            VK_RMENU,
            VK_LWIN,
        ] {
            assert!(is_extended(vk), "{vk:#x}");
        }
        for vk in [0x41, VK_RETURN, VK_LSHIFT, VK_LCONTROL, 0x70] {
            assert!(!is_extended(vk), "{vk:#x}");
        }
        assert_eq!(canonical_vk(VK_CONTROL), VK_LCONTROL);
        assert_eq!(canonical_vk(0x41), 0x41);
    }

    #[test]
    fn text_uses_unicode_packets_and_surrogates() {
        let ev = text_to_events("a😀\r\nb\t\u{7}", |_| 0x1c);
        let units: Vec<u16> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Unicode { unit, down: true } => Some(*unit),
                _ => None,
            })
            .collect();
        assert_eq!(units, [0x61, 0xD83D, 0xDE00, 0x62]);
        let keys: Vec<u16> = ev
            .iter()
            .filter_map(|e| match e {
                Event::Key { vk, down: true, .. } => Some(*vk),
                _ => None,
            })
            .collect();
        assert_eq!(keys, [VK_RETURN, VK_TAB]);
        // Every down has a matching up, in order.
        assert_eq!(ev.len(), 12);
    }

    #[test]
    fn held_state_suppresses_duplicates_and_releases_everything() {
        let mut s = HeldState::default();
        assert!(s.button(MouseButton::Left, true));
        assert!(!s.button(MouseButton::Left, true));
        assert!(!s.button(MouseButton::Right, false));
        assert!(s.key(VK_LCONTROL, true));
        assert!(s.key(VK_LSHIFT, true));
        assert!(s.key(VK_LSHIFT, true)); // repeat
        assert!(!s.key(0x41, false));
        assert!(s.anything_held());

        let rel = s.release_all(|_| 0);
        assert_eq!(
            rel,
            [
                Event::Key {
                    vk: VK_LSHIFT,
                    scan: 0,
                    extended: false,
                    down: false
                },
                Event::Key {
                    vk: VK_LCONTROL,
                    scan: 0,
                    extended: false,
                    down: false
                },
                Event::Button {
                    button: MouseButton::Left,
                    down: false
                },
            ]
        );
        assert!(!s.anything_held());
        assert!(s.release_all(|_| 0).is_empty());
    }

    #[test]
    fn held_keys_are_bounded() {
        let mut s = HeldState::default();
        for vk in 0x41..0x41 + MAX_HELD_KEYS as u16 {
            assert!(s.key(vk, true));
        }
        assert!(!s.key(0x70, true));
    }

    #[test]
    fn rate_limiter_refills() {
        let t0 = Instant::now();
        let mut r = RateLimiter::new(100, 10);
        r.last = t0;
        for _ in 0..10 {
            assert!(r.allow_at(t0, 1));
        }
        assert!(!r.allow_at(t0, 1));
        assert!(r.allow_at(t0 + Duration::from_millis(20), 1));
        assert!(r.allow_at(t0 + Duration::from_millis(20), 1));
        assert!(!r.allow_at(t0 + Duration::from_millis(20), 1));
        // Refill never exceeds the burst.
        assert!(r.allow_at(t0 + Duration::from_secs(10), 10));
        assert!(!r.allow_at(t0 + Duration::from_secs(10), 1));
    }
}
