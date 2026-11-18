//! Capture the primary display, encode it, and write an Annex B `.h264`
//! file that any player can open. Prints real measurements.
//!
//! ```text
//! cargo run --release -p encoder --example capture_encode -- \
//!     [--seconds 5] [--fps 60] [--width 1920] [--kbps 12000] \
//!     [--software] [--out capture.h264]
//! ```

use capture_windows::{BackendPreference, FrameEvent};
use encoder::{EncodedFrame, EncoderConfig, EncoderPreference, Submit, Tuning, VideoEncoder};
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

fn clock() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<Instant> = OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_micros() as u64
}

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter()
        .position(|a| a == name)
        .and_then(|i| args.get(i + 1).cloned())
}

fn percentile(sorted: &[u32], p: usize) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(sorted.len() - 1) * p / 100]
}

fn main() {
    let seconds: u64 = arg("--seconds").and_then(|s| s.parse().ok()).unwrap_or(5);
    let fps: u32 = arg("--fps").and_then(|s| s.parse().ok()).unwrap_or(60);
    let max_width: u32 = arg("--width").and_then(|s| s.parse().ok()).unwrap_or(1920);
    let kbps: u32 = arg("--kbps").and_then(|s| s.parse().ok()).unwrap_or(12_000);
    let software = std::env::args().any(|a| a == "--software");
    let animate = std::env::args().any(|a| a == "--animate");
    let out_path = arg("--out").unwrap_or_else(|| "capture.h264".into());

    capture_windows::init_process_dpi_awareness();
    capture_windows::init_thread();
    let monitor = capture_windows::enumerate_monitors()
        .expect("enumerate")
        .into_iter()
        .find(|m| m.primary)
        .expect("primary display");
    let mut capture =
        capture_windows::open(&monitor, BackendPreference::Auto).expect("open capture");
    let interval = Duration::from_secs_f64(1.0 / fps as f64);
    capture.set_target_interval(interval);
    let src = capture.size();
    let (width, height) = encoder::fit_size(src, (max_width, 0));
    let gpu = capture.gpu().clone();

    println!(
        "capture : {} on '{}', {}x{} @ {} Hz",
        capture.backend().name(),
        gpu.adapter_name,
        src.0,
        src.1,
        monitor.refresh_millihz / 1000
    );
    for (name, hw) in encoder::list_encoders(gpu.adapter_luid).unwrap_or_default() {
        println!(
            "          available encoder: {name} ({})",
            if hw { "hardware" } else { "software" }
        );
    }

    let (tx, rx) = mpsc::channel::<EncodedFrame>();
    let mut enc = VideoEncoder::new(
        &gpu.device,
        &gpu.context,
        gpu.adapter_luid,
        src,
        EncoderConfig {
            width,
            height,
            fps,
            bitrate_bps: kbps * 1000,
            tuning: Tuning::Balanced,
            preference: if software {
                EncoderPreference::SoftwareOnly
            } else {
                EncoderPreference::Auto
            },
            clock,
        },
        Box::new(move |f| {
            let _ = tx.send(f);
        }),
    )
    .expect("create encoder");
    println!(
        "encoder : {} ({}), {}x{} @ {fps} fps, {kbps} kbps, frames via {}",
        enc.name(),
        if enc.is_hardware() {
            "hardware"
        } else {
            "software"
        },
        width,
        height,
        enc.path().describe()
    );

    // Moving content of a known size, so frames are produced continuously.
    let _pattern = animate.then(|| capture_windows::testpattern::start(200, 200, 960, 540, 8));
    if animate {
        std::thread::sleep(Duration::from_millis(300));
    }
    let mut file = std::fs::File::create(&out_path).expect("create output");
    let start = Instant::now();
    let mut next_slot = Instant::now();
    let (mut captured, mut busy, mut idle) = (0u32, 0u32, 0u32);
    let mut capture_ages = Vec::new();
    let mut submit_us = Vec::new();
    let mut force_key = true;
    let mut frames: Vec<EncodedFrame> = Vec::new();

    while start.elapsed() < Duration::from_secs(seconds) {
        match capture.next_frame(next_slot, Duration::from_millis(100)) {
            Ok(FrameEvent::Frame(f)) => {
                captured += 1;
                if let Some(a) = f.age_us {
                    capture_ages.push(a);
                }
                let t = Instant::now();
                // Timestamp the frame with when Windows composed it, so the
                // encoder's capture-to-output figure includes capture delay.
                let capture_us = clock().saturating_sub(f.age_us.unwrap_or(0) as u64);
                match enc
                    .submit(f.texture, capture_us, force_key)
                    .expect("submit")
                {
                    Submit::Accepted => {
                        force_key = false;
                        submit_us.push(t.elapsed().as_micros() as u32);
                    }
                    Submit::Busy => busy += 1,
                }
                next_slot = Instant::now().max(next_slot + interval);
                if next_slot < Instant::now() {
                    next_slot = Instant::now();
                }
            }
            Ok(FrameEvent::Cursor(_)) => {}
            Ok(FrameEvent::Timeout) => idle += 1,
            Err(e) => {
                eprintln!("capture error: {e}");
                break;
            }
        }
        while let Ok(f) = rx.try_recv() {
            file.write_all(&f.data).expect("write");
            frames.push(f);
        }
    }
    // Give the encoder a moment to flush what it still holds.
    let flush_deadline = Instant::now() + Duration::from_millis(300);
    while Instant::now() < flush_deadline {
        while let Ok(f) = rx.recv_timeout(Duration::from_millis(50)) {
            file.write_all(&f.data).expect("write");
            frames.push(f);
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    drop(file);

    let bytes: usize = frames.iter().map(|f| f.data.len()).sum();
    let keyframes = frames.iter().filter(|f| f.keyframe).count();
    let self_contained = frames.iter().filter(|f| f.self_contained).count();
    let mut enc_us: Vec<u32> = frames.iter().map(|f| f.encode_us).collect();
    enc_us.sort_unstable();
    capture_ages.sort_unstable();
    submit_us.sort_unstable();
    let mut sizes: Vec<u32> = frames.iter().map(|f| f.data.len() as u32).collect();
    sizes.sort_unstable();

    println!("--- results over {elapsed:.2} s ---");
    println!(
        "captured frames        : {captured} ({:.1} fps), idle waits {idle}",
        captured as f64 / elapsed
    );
    println!("dropped (encoder busy) : {busy}");
    println!(
        "encoded frames         : {} ({:.1} fps), keyframes {keyframes} (self-contained {self_contained})",
        frames.len(),
        frames.len() as f64 / elapsed
    );
    println!(
        "bitrate                : {:.0} kbps actual vs {kbps} kbps target",
        bytes as f64 * 8.0 / elapsed / 1000.0
    );
    println!(
        "frame size bytes       : p50 {} p95 {} max {}",
        percentile(&sizes, 50),
        percentile(&sizes, 95),
        sizes.last().copied().unwrap_or(0)
    );
    println!(
        "compose -> acquired us : p50 {} p95 {} p99 {} (n={})",
        percentile(&capture_ages, 50),
        percentile(&capture_ages, 95),
        percentile(&capture_ages, 99),
        capture_ages.len()
    );
    println!(
        "convert + submit us    : p50 {} p95 {} p99 {} (n={})",
        percentile(&submit_us, 50),
        percentile(&submit_us, 95),
        percentile(&submit_us, 99),
        submit_us.len()
    );
    println!(
        "compose -> encoded us  : p50 {} p95 {} p99 {} (n={})",
        percentile(&enc_us, 50),
        percentile(&enc_us, 95),
        percentile(&enc_us, 99),
        enc_us.len()
    );
    println!(
        "first frame keyframe   : {:?}",
        frames.first().map(|f| f.self_contained)
    );
    println!("wrote {bytes} bytes to {out_path}");
}
