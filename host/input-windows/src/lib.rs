//! Input injection for LANLink.
//!
//! [`Injector`] applies validated protocol input to one captured display:
//!
//! * coordinates arrive normalized to the display and are mapped onto the
//!   Windows virtual desktop (negative monitor origins included);
//! * every button and key transition is tracked, duplicates are dropped,
//!   and [`Injector::release_all`] (also run on drop) guarantees nothing is
//!   left pressed when a session ends or the connection dies;
//! * a token bucket bounds how much input a client can inject.
//!
//! Injection uses `SendInput` in the interactive user's session. Windows
//! silently drops injected input aimed at windows of a higher integrity
//! level (UIPI) and at the secure desktop; LANLink reports that through
//! [`Injector::foreground_blocked`] and never tries to work around it.
//! Ctrl+Alt+Del cannot be injected by design.

pub mod caret;
pub mod events;
#[cfg(windows)]
mod sendinput;

pub use events::{Event, Rect};
#[cfg(windows)]
pub use sendinput::{foreground_is_higher_integrity, virtual_desktop, SendInputSink};

use events::*;
use lanlink_protocol::InputKind;
use thiserror::Error;

/// Where low-level events go. The Windows implementation calls
/// `SendInput`; tests record them.
pub trait Sink {
    /// Inject events in order. Returns how many were accepted.
    fn send(&mut self, events: &[Event]) -> usize;
    /// Hardware scan code for a virtual key (0 if unknown).
    fn scan_code(&self, vk: u16) -> u16;
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InputError {
    #[error("input rate limit exceeded")]
    RateLimited,
    #[error("Windows accepted {accepted} of {total} input events")]
    Rejected { accepted: usize, total: usize },
}

/// Sustained and burst limits for injected operations. One text message
/// costs one token per character.
pub const RATE_PER_SEC: u32 = 2000;
/// One full-size text message (`MAX_TEXT_INPUT`) fits in a burst, so an
/// ordinary paste is never delayed; the sustained rate is what limits floods.
pub const RATE_BURST: u32 = lanlink_protocol::limits::MAX_TEXT_INPUT as u32;

/// Typed or pasted text is paced to the rate limit instead of refused
/// (dropping part of what the user typed is never right), unless it would
/// have to wait longer than this, which only a flooding client causes.
pub const MAX_TEXT_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Presses (keys, buttons, wheel) wait at most this long for the rate limit
/// instead of being refused, so a keystroke right after a large paste (which
/// drains the bucket) still arrives. A real flood still overruns the input
/// queue and is dropped there.
pub const MAX_PRESS_WAIT: std::time::Duration = std::time::Duration::from_millis(50);

/// Largest wheel delta accepted per event (10 notches).
const MAX_WHEEL: i32 = 1200;
/// Largest relative motion per event, in pixels.
const MAX_REL: i32 = 2000;

pub struct Injector<S: Sink> {
    sink: S,
    display: Rect,
    desktop: Rect,
    held: HeldState,
    limiter: RateLimiter,
}

impl<S: Sink> Injector<S> {
    /// `display` is the captured monitor and `desktop` the whole virtual
    /// desktop, both in physical pixels.
    pub fn new(sink: S, display: Rect, desktop: Rect) -> Self {
        Injector {
            sink,
            display,
            desktop,
            held: HeldState::default(),
            limiter: RateLimiter::new(RATE_PER_SEC, RATE_BURST),
        }
    }

    /// Call after a display switch or topology change.
    pub fn set_geometry(&mut self, display: Rect, desktop: Rect) {
        self.display = display;
        self.desktop = desktop;
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn held(&self) -> &HeldState {
        &self.held
    }

    fn inject(&mut self, events: &[Event]) -> Result<(), InputError> {
        if events.is_empty() {
            return Ok(());
        }
        let accepted = self.sink.send(events);
        if accepted == events.len() {
            Ok(())
        } else {
            Err(InputError::Rejected {
                accepted,
                total: events.len(),
            })
        }
    }

    /// Move the pointer to a normalized position on the captured display.
    pub fn pointer_abs(&mut self, x: u16, y: u16) -> Result<(), InputError> {
        if !self.limiter.allow(1) {
            return Err(InputError::RateLimited);
        }
        let ev = display_point_to_event(x, y, self.display, self.desktop);
        self.inject(&[ev])
    }

    /// Move the pointer by a pixel offset.
    pub fn pointer_rel(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        if !self.limiter.allow(1) {
            return Err(InputError::RateLimited);
        }
        let ev = Event::MoveRel {
            dx: dx.clamp(-MAX_REL, MAX_REL),
            dy: dy.clamp(-MAX_REL, MAX_REL),
        };
        self.inject(&[ev])
    }

    /// Apply one reliable input event.
    pub fn apply(&mut self, kind: &InputKind) -> Result<(), InputError> {
        let cost = match kind {
            InputKind::Text { text } => text.chars().count().max(1) as u32,
            // Releasing must always be possible, even when rate limited.
            InputKind::ReleaseAll => 0,
            _ => 1,
        };
        // Releases (button/key up) are never rate limited either: dropping
        // one would leave something stuck.
        let is_release = matches!(
            kind,
            InputKind::Button { down: false, .. } | InputKind::Key { down: false, .. }
        );
        if let InputKind::Text { .. } = kind {
            let wait = self.limiter.wait_at(std::time::Instant::now(), cost);
            if wait > MAX_TEXT_WAIT {
                return Err(InputError::RateLimited);
            }
            if !wait.is_zero() {
                // Only ever on the dedicated input thread.
                std::thread::sleep(wait);
            }
            self.limiter.take_at(std::time::Instant::now(), cost);
        } else if cost > 0 && !is_release {
            let wait = self.limiter.wait_at(std::time::Instant::now(), cost);
            if wait > MAX_PRESS_WAIT {
                return Err(InputError::RateLimited);
            }
            if !wait.is_zero() {
                std::thread::sleep(wait);
            }
            self.limiter.take_at(std::time::Instant::now(), cost);
        }

        let mut events = Vec::with_capacity(4);
        match kind {
            InputKind::Button { button, down, pos } => {
                if let Some((x, y)) = pos {
                    events.push(display_point_to_event(*x, *y, self.display, self.desktop));
                }
                if self.held.button(*button, *down) {
                    events.push(Event::Button {
                        button: *button,
                        down: *down,
                    });
                }
            }
            InputKind::Scroll { dx, dy } => {
                let dy = (*dy as i32).clamp(-MAX_WHEEL, MAX_WHEEL);
                let dx = (*dx as i32).clamp(-MAX_WHEEL, MAX_WHEEL);
                if dy != 0 {
                    events.push(Event::Wheel {
                        delta: dy,
                        horizontal: false,
                    });
                }
                if dx != 0 {
                    events.push(Event::Wheel {
                        delta: dx,
                        horizontal: true,
                    });
                }
            }
            InputKind::Key { vk, down } => {
                let vk = canonical_vk(*vk);
                if self.held.key(vk, *down) {
                    events.push(Event::Key {
                        vk,
                        scan: self.sink.scan_code(vk),
                        extended: is_extended(vk),
                        down: *down,
                    });
                }
            }
            InputKind::Text { text } => {
                let sink = &self.sink;
                events = text_to_events(text, |vk| sink.scan_code(vk));
            }
            InputKind::ReleaseAll => {
                return self.release_all();
            }
            InputKind::MoveAbs { x, y } => {
                events.push(display_point_to_event(*x, *y, self.display, self.desktop));
            }
            InputKind::MoveRel { dx, dy } => {
                events.push(Event::MoveRel {
                    dx: (*dx as i32).clamp(-MAX_REL, MAX_REL),
                    dy: (*dy as i32).clamp(-MAX_REL, MAX_REL),
                });
            }
        }
        self.inject(&events)
    }

    /// Release every button and key this client holds.
    pub fn release_all(&mut self) -> Result<(), InputError> {
        let sink = &self.sink;
        let events = self.held.release_all(|vk| sink.scan_code(vk));
        if !events.is_empty() {
            tracing::info!(count = events.len(), "releasing held input");
        }
        self.inject(&events)
    }
}

impl<S: Sink> Drop for Injector<S> {
    fn drop(&mut self) {
        let _ = self.release_all();
    }
}

impl Sink for Box<dyn Sink + Send> {
    fn send(&mut self, events: &[Event]) -> usize {
        (**self).send(events)
    }
    fn scan_code(&self, vk: u16) -> u16 {
        (**self).scan_code(vk)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lanlink_protocol::MouseButton;

    #[derive(Default)]
    struct Recorder {
        events: Vec<Event>,
        accept_limit: Option<usize>,
    }

    impl Sink for &mut Recorder {
        fn send(&mut self, events: &[Event]) -> usize {
            let n = self
                .accept_limit
                .map_or(events.len(), |l| l.min(events.len()));
            self.events.extend_from_slice(&events[..n]);
            n
        }
        fn scan_code(&self, vk: u16) -> u16 {
            vk + 0x100
        }
    }

    const DISPLAY: Rect = Rect {
        x: 0,
        y: 0,
        width: 2560,
        height: 1600,
    };

    #[test]
    fn click_moves_then_presses_and_disconnect_releases() {
        let mut rec = Recorder::default();
        {
            let mut inj = Injector::new(&mut rec, DISPLAY, DISPLAY);
            inj.apply(&InputKind::Button {
                button: MouseButton::Left,
                down: true,
                pos: Some((65535, 0)),
            })
            .unwrap();
            inj.apply(&InputKind::Key {
                vk: events::VK_CONTROL,
                down: true,
            })
            .unwrap();
            // Session dies here: the injector is dropped with both held.
        }
        assert_eq!(
            rec.events,
            [
                Event::MoveAbs {
                    x: to_virtual_abs(2559, 0, 2560),
                    y: 0
                },
                Event::Button {
                    button: MouseButton::Left,
                    down: true
                },
                Event::Key {
                    vk: events::VK_LCONTROL,
                    scan: 0x1A2,
                    extended: false,
                    down: true
                },
                Event::Key {
                    vk: events::VK_LCONTROL,
                    scan: 0x1A2,
                    extended: false,
                    down: false
                },
                Event::Button {
                    button: MouseButton::Left,
                    down: false
                },
            ]
        );
    }

    #[test]
    fn duplicate_transitions_are_not_injected() {
        let mut rec = Recorder::default();
        {
            let mut inj = Injector::new(&mut rec, DISPLAY, DISPLAY);
            let up = InputKind::Button {
                button: MouseButton::Right,
                down: false,
                pos: None,
            };
            inj.apply(&up).unwrap();
            inj.apply(&InputKind::Key {
                vk: 0x41,
                down: false,
            })
            .unwrap();
        }
        assert!(rec.events.is_empty());
    }

    #[test]
    fn extended_flag_and_scroll_clamping() {
        let mut rec = Recorder::default();
        {
            let mut inj = Injector::new(&mut rec, DISPLAY, DISPLAY);
            inj.apply(&InputKind::Key {
                vk: 0x25,
                down: true,
            })
            .unwrap(); // Left arrow
            inj.apply(&InputKind::Key {
                vk: 0x25,
                down: false,
            })
            .unwrap();
            inj.apply(&InputKind::Scroll {
                dx: -30000,
                dy: 120,
            })
            .unwrap();
        }
        assert_eq!(
            rec.events[0],
            Event::Key {
                vk: 0x25,
                scan: 0x125,
                extended: true,
                down: true
            }
        );
        assert_eq!(
            rec.events[2],
            Event::Wheel {
                delta: 120,
                horizontal: false
            }
        );
        assert_eq!(
            rec.events[3],
            Event::Wheel {
                delta: -MAX_WHEEL,
                horizontal: true
            }
        );
    }

    #[test]
    fn presses_are_paced_to_the_rate_and_releases_never_wait() {
        let mut rec = Recorder::default();
        let mut inj = Injector::new(&mut rec, DISPLAY, DISPLAY);
        inj.apply(&InputKind::Key {
            vk: 0x41,
            down: true,
        })
        .unwrap();
        // Use the whole burst, then 200 more: those wait for the rate
        // instead of being refused, so the sustained rate still holds.
        let t0 = std::time::Instant::now();
        for _ in 0..RATE_BURST + 200 {
            inj.apply(&InputKind::Scroll { dx: 0, dy: 1 }).unwrap();
        }
        let min = 200.0 / RATE_PER_SEC as f64 - 0.03;
        assert!(t0.elapsed().as_secs_f64() >= min, "{:?}", t0.elapsed());
        // A key press right after a drained bucket still goes through.
        inj.apply(&InputKind::Key {
            vk: 0x42,
            down: true,
        })
        .unwrap();
        inj.apply(&InputKind::Key {
            vk: 0x42,
            down: false,
        })
        .unwrap();
        // The release still goes through.
        inj.apply(&InputKind::Key {
            vk: 0x41,
            down: false,
        })
        .unwrap();
        assert!(!inj.held().anything_held());
    }

    /// A paste longer than the burst used to be refused outright; text is
    /// paced instead, and still counts fully against the rate.
    #[test]
    fn long_text_is_paced_not_dropped() {
        let mut rec = Recorder::default();
        let mut inj = Injector::new(&mut rec, DISPLAY, DISPLAY);
        let t0 = std::time::Instant::now();
        let long = "y".repeat(1000);
        for _ in 0..3 {
            inj.apply(&InputKind::Text { text: long.clone() }).unwrap();
        }
        // 3000 characters at RATE_PER_SEC with a RATE_BURST head start: the
        // rate still holds, the text just waits instead of being dropped.
        let min = (3000 - RATE_BURST) as f64 / RATE_PER_SEC as f64 - 0.1;
        assert!(t0.elapsed().as_secs_f64() >= min, "{:?}", t0.elapsed());
        drop(inj);
        let chars = rec
            .events
            .iter()
            .filter(|e| matches!(e, Event::Unicode { down: true, .. }))
            .count();
        assert_eq!(chars, 3000);
    }

    #[test]
    fn partial_acceptance_is_reported() {
        let mut rec = Recorder {
            accept_limit: Some(1),
            ..Default::default()
        };
        let mut inj = Injector::new(&mut rec, DISPLAY, DISPLAY);
        let r = inj.apply(&InputKind::Text { text: "ab".into() });
        assert_eq!(
            r,
            Err(InputError::Rejected {
                accepted: 1,
                total: 4
            })
        );
    }
}
