//! LANLink host for Windows: a window and a tray icon over the host core.
//!
//! Options:
//!   --minimized   start in the tray (used by "Start at login")
//!   --port <n>    UDP port (default 47990)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod core;
mod icon;
mod overlay;
mod settings;
mod tray;
mod ui;

use session_manager::HostEvent;

/// Everything the background threads tell the window.
pub enum AppMsg {
    Ready(Box<core::Running>),
    Failed(String),
    Stopped,
    Host(HostEvent),
    Tray(TrayAction),
    /// Stop pressed on the on-screen bar, or the hotkey.
    StopRequested,
    /// Whether the emergency-stop hotkey could be registered.
    Hotkey(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Open,
    Pair,
    Stop,
    Quit,
}

fn arg(name: &str) -> Option<String> {
    let a: Vec<String> = std::env::args().collect();
    a.iter()
        .position(|x| x == name)
        .and_then(|i| a.get(i + 1).cloned())
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME")
        .map(|n| {
            let lower = n.to_lowercase();
            let mut c = lower.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => n,
            }
        })
        .unwrap_or_else(|_| "Windows PC".into())
}

/// One LANLink host per user session: a second launch brings the first
/// one's window forward instead.
fn already_running() -> bool {
    use windows::core::w;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::Win32::UI::WindowsAndMessaging::{
        FindWindowW, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    };
    // SAFETY: the mutex handle is intentionally kept for the process lifetime.
    unsafe {
        let _m = CreateMutexW(None, false, w!("Local\\LANLinkHostSingleInstance"));
        if GetLastError() != ERROR_ALREADY_EXISTS {
            return false;
        }
        if let Ok(h) = FindWindowW(None, w!("LANLink")) {
            let _ = ShowWindow(h, SW_SHOW);
            let _ = ShowWindow(h, SW_RESTORE);
            let _ = SetForegroundWindow(h);
        }
        true
    }
}

fn init_logging(dir: &std::path::Path) {
    let _ = std::fs::create_dir_all(dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("lanlink-host.log"));
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,quinn=warn,mdns_sd=warn".into());
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false);
    match file {
        Ok(f) => builder.with_writer(std::sync::Mutex::new(f)).init(),
        Err(_) => builder.init(),
    }
}

fn main() -> eframe::Result {
    if already_running() {
        return Ok(());
    }
    let store_dir = session_manager::storage::HostStore::default_dir();
    init_logging(&store_dir);
    capture_windows::init_process_dpi_awareness();
    capture_windows::init_thread();

    let start_hidden = std::env::args().any(|a| a == "--minimized");
    let port: u16 = arg("--port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(lanlink_protocol::limits::DEFAULT_PORT);

    let icon = eframe::egui::IconData {
        rgba: icon::rgba(icon::Mark::Ready, 64),
        width: 64,
        height: 64,
    };
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("LANLink")
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([720.0, 520.0])
            .with_icon(icon)
            .with_visible(!start_hidden),
        ..Default::default()
    };
    eframe::run_native(
        "LANLink",
        options,
        Box::new(move |cc| {
            Ok(Box::new(ui::LanLinkApp::new(
                cc,
                store_dir,
                port,
                !start_hidden,
            )))
        }),
    )
}
