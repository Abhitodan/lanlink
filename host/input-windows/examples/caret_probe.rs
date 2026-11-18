//! Print where LANLink finds the text caret in the foreground app, a few
//! times a second. Read-only. `cargo run -p input-windows --example caret_probe -- 10`

#[cfg(windows)]
fn main() {
    use input_windows::caret::CaretFinder;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::HiDpi::{
        SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    };
    let secs: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    // SAFETY: process setup calls with constant arguments.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
    }
    let f = CaretFinder::new();
    let end = std::time::Instant::now() + std::time::Duration::from_secs(secs);
    while std::time::Instant::now() < end {
        let t = std::time::Instant::now();
        let c = f.find();
        println!("{:>4} ms  {:?}", t.elapsed().as_millis(), c);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
}

#[cfg(not(windows))]
fn main() {}
