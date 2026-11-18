//! Lists the displays LANLink can see and measures a short capture on each
//! with both backends.
//! `cargo run --release -p capture-windows --example probe [seconds]`

use capture_windows::*;
use std::time::{Duration, Instant};

fn run(monitor: &MonitorInfo, backend: Backend, seconds: u64) {
    print!("  {:<26} ", backend.name());
    let mut cap = match open(monitor, BackendPreference::Only(backend)) {
        Ok(c) => c,
        Err(e) => {
            println!("UNAVAILABLE: {e}");
            return;
        }
    };
    let (w, h) = cap.size();
    let start = Instant::now();
    let (mut frames, mut cursors, mut timeouts) = (0u32, 0u32, 0u32);
    let mut ages: Vec<u32> = Vec::new();
    let mut shape = None;
    while start.elapsed() < Duration::from_secs(seconds) {
        match cap.next_frame(Instant::now(), Duration::from_millis(100)) {
            Ok(FrameEvent::Frame(f)) => {
                frames += 1;
                if let Some(a) = f.age_us {
                    ages.push(a);
                }
                if let Some(s) = f.cursor.and_then(|c| c.shape) {
                    shape = Some((s.width, s.height));
                }
            }
            Ok(FrameEvent::Cursor(c)) => {
                cursors += 1;
                if let Some(s) = c.shape {
                    shape = Some((s.width, s.height));
                }
            }
            Ok(FrameEvent::Timeout) => timeouts += 1,
            Err(e) => {
                println!("ERROR after {frames} frames: {e}");
                return;
            }
        }
    }
    ages.sort_unstable();
    let pct = |p: usize| ages.get(ages.len().saturating_sub(1) * p / 100).copied();
    println!(
        "{w}x{h} on '{}': {frames} frames, {cursors} pointer updates, {timeouts} idle waits in {seconds} s; \
         compose-to-acquire p50 {:?} us p95 {:?} us; pointer shape {:?}",
        cap.gpu().adapter_name,
        pct(50),
        pct(95),
        shape
    );
}

fn main() {
    let seconds = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(2);
    init_process_dpi_awareness();
    init_thread();
    println!(
        "session locked: {}  interactive desktop active: {}",
        session_locked(),
        interactive_desktop_active()
    );
    let monitors = match enumerate_monitors() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("enumeration failed: {e}");
            return;
        }
    };
    for m in &monitors {
        println!(
            "{:#010x} {} [{}] at ({},{}) {} Hz, scale {}%, rotation {}, hdr {}, primary {}, DXGI adapter '{}'",
            m.id,
            m.label,
            m.device_name,
            m.x,
            m.y,
            m.refresh_millihz as f64 / 1000.0,
            m.scale_percent,
            m.rotation,
            m.hdr,
            m.primary,
            m.adapter_name,
        );
        run(m, Backend::GraphicsCapture, seconds);
        run(m, Backend::DesktopDuplication, seconds);
    }
}
