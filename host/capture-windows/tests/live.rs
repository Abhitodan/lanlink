//! Live capture against the machine running the tests. These need an
//! interactive desktop session; on a locked or headless session they say so
//! and pass without asserting, so the reason is visible in the output.

#![cfg(windows)]

use capture_windows::*;
use std::time::{Duration, Instant};

fn primary() -> Option<MonitorInfo> {
    init_process_dpi_awareness();
    init_thread();
    enumerate_monitors().ok()?.into_iter().find(|m| m.primary)
}

fn open_or_skip(monitor: &MonitorInfo, pref: BackendPreference) -> Option<Box<dyn Capture>> {
    if !interactive_desktop_active() {
        eprintln!("SKIP: interactive desktop is not active (locked or secure desktop)");
        return None;
    }
    match open(monitor, pref) {
        Ok(c) => Some(c),
        Err(e) => {
            eprintln!("SKIP: {pref:?} cannot capture here: {e}");
            None
        }
    }
}

fn first_frame(cap: &mut dyn Capture) -> bool {
    // Windows delivers the full desktop as the first frame.
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if let FrameEvent::Frame(f) = cap
            .next_frame(Instant::now(), Duration::from_millis(200))
            .expect("capture")
        {
            assert!(f.accumulated_frames >= 1);
            assert!((0.0..=1.0).contains(&f.dirty_fraction));
            return true;
        }
    }
    false
}

/// The default backend selection must produce a picture on this machine.
/// This one does not skip on open failure: if no backend works, LANLink
/// cannot work here and the test should say so loudly.
#[test]
fn auto_backend_delivers_a_frame_of_the_right_size() {
    let Some(monitor) = primary() else {
        eprintln!("SKIP: no primary display");
        return;
    };
    if !interactive_desktop_active() {
        eprintln!("SKIP: interactive desktop is not active");
        return;
    }
    let mut cap = open(&monitor, BackendPreference::Auto).expect("some backend must work");
    assert_eq!(cap.size(), (monitor.width, monitor.height));
    assert_eq!(
        GpuDevice::luid_from_device(&cap.gpu().device),
        Some(cap.gpu().adapter_luid),
        "reported adapter must be the one the device lives on"
    );
    eprintln!(
        "backend: {}; device on '{}' (display listed under '{}')",
        cap.backend().name(),
        cap.gpu().adapter_name,
        monitor.adapter_name
    );
    assert!(first_frame(cap.as_mut()), "no desktop frame within 3 s");
}

#[test]
fn graphics_capture_backend() {
    let Some(monitor) = primary() else { return };
    let Some(mut cap) = open_or_skip(&monitor, BackendPreference::Only(Backend::GraphicsCapture))
    else {
        return;
    };
    assert_eq!(cap.backend(), Backend::GraphicsCapture);
    assert!(first_frame(cap.as_mut()));
}

#[test]
fn desktop_duplication_backend() {
    let Some(monitor) = primary() else { return };
    let Some(mut cap) = open_or_skip(
        &monitor,
        BackendPreference::Only(Backend::DesktopDuplication),
    ) else {
        return;
    };
    assert_eq!(cap.backend(), Backend::DesktopDuplication);
    assert!(first_frame(cap.as_mut()));
}

/// `not_before` must hold a frame back: asking for a frame no earlier than
/// 60 ms from now must not return one sooner.
#[test]
fn not_before_paces_frames() {
    let Some(monitor) = primary() else { return };
    let Some(mut cap) = open_or_skip(&monitor, BackendPreference::Auto) else {
        return;
    };
    assert!(first_frame(cap.as_mut()));
    for _ in 0..3 {
        let start = Instant::now();
        let not_before = start + Duration::from_millis(60);
        match cap
            .next_frame(not_before, Duration::from_millis(150))
            .expect("capture")
        {
            FrameEvent::Frame(_) => {
                assert!(
                    start.elapsed() >= Duration::from_millis(55),
                    "frame returned after {:?}, before not_before",
                    start.elapsed()
                );
            }
            FrameEvent::Cursor(_) | FrameEvent::Timeout => {}
        }
    }
}

/// Waiting on a quiet desktop must block rather than spin.
#[test]
fn idle_wait_blocks() {
    let Some(monitor) = primary() else { return };
    let Some(mut cap) = open_or_skip(&monitor, BackendPreference::Auto) else {
        return;
    };
    let _ = first_frame(cap.as_mut());
    let mut calls = 0u32;
    let start = Instant::now();
    while start.elapsed() < Duration::from_millis(400) {
        let _ = cap.next_frame(Instant::now(), Duration::from_millis(100));
        calls += 1;
    }
    // A spinning implementation would make thousands of calls in 400 ms.
    // Pointer movement and screen updates legitimately return early, so the
    // bound is generous.
    eprintln!("next_frame returned {calls} times in 400 ms");
    assert!(calls < 400, "next_frame appears to spin ({calls} returns)");
}
