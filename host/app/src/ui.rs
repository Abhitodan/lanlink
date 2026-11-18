//! The LANLink window.

use crate::core::{self, Running};
use crate::icon::Mark;
use crate::overlay::{self, Indicator};
use crate::settings::{self, AppSettings};
use crate::tray::{self, TrayState};
use crate::{AppMsg, TrayAction};
use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke,
    Vec2,
};
use lanlink_auth::{Fingerprint, PairedDevice};
use lanlink_protocol::{ClientStats, StreamStats};
use session_manager::{HostCommand, HostEvent};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::oneshot;

// Palette shared with the Android app (night theme).
const NIGHT: Color32 = Color32::from_rgb(0x15, 0x18, 0x1C);
const RAISED: Color32 = Color32::from_rgb(0x1E, 0x22, 0x27);
const RAISED_HI: Color32 = Color32::from_rgb(0x26, 0x2B, 0x31);
const LINE: Color32 = Color32::from_rgb(0x2D, 0x32, 0x39);
const MIST: Color32 = Color32::from_rgb(0xE9, 0xE6, 0xDF);
const MIST_SOFT: Color32 = Color32::from_rgb(0xA9, 0xAE, 0xB6);
const SAND: Color32 = Color32::from_rgb(0xD9, 0xB9, 0x8C);
const BLUE: Color32 = Color32::from_rgb(0x9D, 0xB6, 0xD6);
const OK: Color32 = Color32::from_rgb(0x7F, 0xC4, 0x96);
const DANGER: Color32 = Color32::from_rgb(0xE0, 0x6C, 0x5F);
const DANGER_DEEP: Color32 = Color32::from_rgb(0xA2, 0x3B, 0x33);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Pair,
    Devices,
    Settings,
}

enum Status {
    Starting,
    Ready(Box<Running>),
    Failed(String),
}

struct PairingView {
    uri: String,
    code: String,
    qr: Vec<bool>,
    qr_width: usize,
    opened: Instant,
    lifetime: Duration,
}

enum ApprovalKind {
    Pairing {
        name: String,
        model: String,
        code: String,
    },
    Session {
        name: String,
        view_only: bool,
    },
}

struct Approval {
    kind: ApprovalKind,
    respond: Option<oneshot::Sender<bool>>,
}

struct SessionView {
    device: String,
    display: String,
    encoder: String,
    hardware: bool,
    capture: String,
    size: (u32, u32),
    fps: u32,
    view_only: bool,
    since: Instant,
}

/// A rolling one-minute history per metric, for the sparklines.
#[derive(Default)]
struct Series(VecDeque<f32>);

impl Series {
    const LEN: usize = 60;
    fn push(&mut self, v: f32) {
        if self.0.len() == Self::LEN {
            self.0.pop_front();
        }
        self.0.push_back(v);
    }
    fn last(&self) -> Option<f32> {
        self.0.back().copied()
    }
}

#[derive(Default)]
struct Metrics {
    fps: Series,
    rtt: Series,
    bitrate: Series,
    encode: Series,
    decode: Series,
    dropped: u32,
}

impl Metrics {
    fn record(&mut self, s: &StreamStats, c: Option<&ClientStats>) {
        self.fps
            .push(c.map(|c| c.rendered_fps_x10).unwrap_or(s.encoded_fps_x10) as f32 / 10.0);
        self.rtt.push(s.rtt_us as f32 / 1000.0);
        self.bitrate.push(s.bitrate_kbps as f32 / 1000.0);
        self.encode.push(s.encode_us_p50 as f32 / 1000.0);
        if let Some(c) = c {
            self.decode.push(c.decode_us_p50 as f32 / 1000.0);
            self.dropped = self.dropped.saturating_add(c.frames_dropped);
        }
    }
}

pub struct LanLinkApp {
    rx: Receiver<AppMsg>,
    store_dir: PathBuf,
    status: Status,
    page: Page,
    pairing: Option<PairingView>,
    approval: Option<Approval>,
    session: Option<SessionView>,
    metrics: Metrics,
    activity: VecDeque<(SystemTime, String)>,
    settings: AppSettings,
    name_edit: String,
    run_at_login: bool,
    settings_error: Option<String>,
    hotkey_ok: Option<bool>,
    confirm_revoke: Option<Fingerprint>,
    visible: bool,
    quitting: bool,
    hwnd: Option<isize>,
}

impl LanLinkApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        store_dir: PathBuf,
        port: u16,
        visible: bool,
    ) -> Self {
        style(&cc.egui_ctx);
        let (tx, rx): (Sender<AppMsg>, Receiver<AppMsg>) = mpsc::channel();
        let settings = AppSettings::load(&store_dir, &crate::computer_name());
        core::spawn(
            core::StartConfig {
                name: settings.name.clone(),
                port,
                ask_each_session: settings.ask_each_session,
            },
            tx.clone(),
            cc.egui_ctx.clone(),
        );
        tray::spawn(tx.clone(), cc.egui_ctx.clone());
        overlay::spawn(tx, cc.egui_ctx.clone());
        // Development aid: show the indicator without a phone, to check how
        // it looks and that the phone-side capture excludes it.
        if std::env::var_os("LANLINK_DEMO_INDICATOR").is_some() {
            overlay::set(Some(Indicator {
                device: "Test phone".into(),
                view_only: false,
            }));
        }
        let hwnd = {
            use raw_window_handle::{HasWindowHandle, RawWindowHandle};
            cc.window_handle().ok().and_then(|h| match h.as_raw() {
                RawWindowHandle::Win32(w) => Some(w.hwnd.get()),
                _ => None,
            })
        };
        let app = LanLinkApp {
            rx,
            name_edit: settings.name.clone(),
            settings,
            store_dir,
            status: Status::Starting,
            page: Page::Home,
            pairing: None,
            approval: None,
            session: None,
            metrics: Metrics::default(),
            activity: VecDeque::new(),
            run_at_login: settings::run_at_login(),
            settings_error: None,
            hotkey_ok: None,
            confirm_revoke: None,
            visible,
            quitting: false,
            hwnd,
        };
        app.sync_tray();
        app
    }

    // ------------------------------------------------------------ plumbing

    fn command(&self, c: HostCommand) {
        if let Status::Ready(r) = &self.status {
            let _ = r.handle.commands.send(c);
        }
    }

    fn log(&mut self, text: impl Into<String>) {
        if self.activity.len() == 40 {
            self.activity.pop_back();
        }
        self.activity.push_front((SystemTime::now(), text.into()));
    }

    fn show_window(&mut self, ctx: &egui::Context) {
        self.visible = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
        ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
        if let Some(h) = self.hwnd {
            use windows::Win32::Foundation::HWND;
            use windows::Win32::UI::WindowsAndMessaging::{
                SetForegroundWindow, ShowWindow, SW_SHOW,
            };
            // SAFETY: our own top-level window.
            unsafe {
                let h = HWND(h as *mut _);
                let _ = ShowWindow(h, SW_SHOW);
                let _ = SetForegroundWindow(h);
            }
        }
    }

    fn sync_tray(&self) {
        let (mark, tooltip) = match (&self.status, &self.session) {
            (Status::Ready(_), Some(s)) => (
                Mark::Controlled,
                format!(
                    "LANLink — {} is {} this PC",
                    s.device,
                    if s.view_only {
                        "viewing"
                    } else {
                        "controlling"
                    }
                ),
            ),
            (Status::Ready(_), None) => (Mark::Ready, "LANLink — ready for phones".to_string()),
            (Status::Starting, _) => (Mark::Off, "LANLink — starting".to_string()),
            (Status::Failed(_), _) => (Mark::Off, "LANLink — not running".to_string()),
        };
        tray::set(TrayState {
            mark,
            tooltip,
            session_active: self.session.is_some(),
        });
    }

    fn sync_indicator(&self) {
        let ind = match &self.session {
            Some(s) if self.settings.indicator => Some(Indicator {
                device: s.device.clone(),
                view_only: s.view_only,
            }),
            _ => None,
        };
        overlay::set(ind);
    }

    fn open_pairing(&mut self) {
        self.page = Page::Pair;
        self.command(HostCommand::OpenPairing);
    }

    fn stop_session(&mut self) {
        if self.session.is_some() {
            self.command(HostCommand::StopSession);
            self.log("Session stopped on this PC");
        }
    }

    fn handle(&mut self, msg: AppMsg, ctx: &egui::Context) {
        match msg {
            AppMsg::Ready(r) => {
                self.log(format!("Listening on UDP {} as \"{}\"", r.port, r.name));
                if let Some(d) = &r.discovery {
                    self.log(d.clone());
                }
                self.status = Status::Ready(r);
                self.sync_tray();
            }
            AppMsg::Failed(e) => {
                tracing::error!(error = %e, "host failed to start");
                self.status = Status::Failed(e);
                self.sync_tray();
                self.show_window(ctx);
            }
            AppMsg::Stopped => {
                if !self.quitting {
                    self.status =
                        Status::Failed("The host stopped unexpectedly. Restart LANLink.".into());
                    self.sync_tray();
                }
            }
            AppMsg::Hotkey(ok) => self.hotkey_ok = Some(ok),
            AppMsg::StopRequested => self.stop_session(),
            AppMsg::Tray(a) => match a {
                TrayAction::Open => self.show_window(ctx),
                TrayAction::Pair => {
                    self.show_window(ctx);
                    self.open_pairing();
                }
                TrayAction::Stop => self.stop_session(),
                TrayAction::Quit => self.quit(ctx),
            },
            AppMsg::Host(ev) => self.host_event(ev, ctx),
        }
    }

    fn host_event(&mut self, ev: HostEvent, ctx: &egui::Context) {
        match ev {
            HostEvent::Listening { .. } => {}
            HostEvent::PairingWindowOpened {
                uri,
                code,
                expires_in,
            } => {
                let (qr, qr_width) = match qrcode::QrCode::new(uri.as_bytes()) {
                    Ok(c) => (
                        c.to_colors()
                            .into_iter()
                            .map(|m| m == qrcode::Color::Dark)
                            .collect(),
                        c.width(),
                    ),
                    Err(_) => (Vec::new(), 0),
                };
                self.pairing = Some(PairingView {
                    uri,
                    code,
                    qr,
                    qr_width,
                    opened: Instant::now(),
                    lifetime: expires_in,
                });
                self.page = Page::Pair;
                self.log("Pairing opened");
            }
            HostEvent::PairingWindowClosed => {
                self.pairing = None;
            }
            HostEvent::PairingApproval {
                device_name,
                device_model,
                confirmation_code,
                respond,
                ..
            } => {
                self.approval = Some(Approval {
                    kind: ApprovalKind::Pairing {
                        name: device_name,
                        model: device_model,
                        code: confirmation_code,
                    },
                    respond: Some(respond),
                });
                self.show_window(ctx);
            }
            HostEvent::Paired { device_name, .. } => {
                self.pairing = None;
                self.log(format!("Paired with {device_name}"));
                self.page = Page::Home;
            }
            HostEvent::SessionApproval {
                device_name,
                view_only,
                respond,
                ..
            } => {
                self.approval = Some(Approval {
                    kind: ApprovalKind::Session {
                        name: device_name,
                        view_only,
                    },
                    respond: Some(respond),
                });
                self.show_window(ctx);
            }
            HostEvent::SessionStarted {
                device_name,
                display,
                encoder,
                hardware,
                capture,
                width,
                height,
                fps,
                view_only,
                ..
            } => {
                self.log(format!(
                    "{device_name} connected ({width}×{height} @ {fps} fps)"
                ));
                self.metrics = Metrics::default();
                self.session = Some(SessionView {
                    device: device_name,
                    display,
                    encoder,
                    hardware,
                    capture,
                    size: (width, height),
                    fps,
                    view_only,
                    since: Instant::now(),
                });
                self.sync_tray();
                self.sync_indicator();
            }
            HostEvent::SessionEnded {
                device_name,
                reason,
            } => {
                self.log(format!("{device_name} disconnected: {reason}"));
                self.session = None;
                self.sync_tray();
                self.sync_indicator();
            }
            HostEvent::Stats { stream, client } => self.metrics.record(&stream, client.as_ref()),
            HostEvent::Notice(n) => self.log(n),
            HostEvent::Rejected { addr, reason } => self.log(format!("Refused {addr}: {reason}")),
        }
    }

    fn quit(&mut self, ctx: &egui::Context) {
        self.quitting = true;
        overlay::set(None);
        self.command(HostCommand::StopSession);
        self.command(HostCommand::Shutdown);
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn answer(&mut self, yes: bool) {
        if let Some(mut a) = self.approval.take() {
            if let Some(tx) = a.respond.take() {
                let _ = tx.send(yes);
            }
            let what = match &a.kind {
                ApprovalKind::Pairing { name, .. } => format!("pairing with {name}"),
                ApprovalKind::Session { name, .. } => format!("session for {name}"),
            };
            self.log(format!(
                "{} {what}",
                if yes { "Approved" } else { "Declined" }
            ));
        }
    }
}

impl eframe::App for LanLinkApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(msg) = self.rx.try_recv() {
            self.handle(msg, ctx);
        }
        // Closing the window keeps LANLink running in the tray.
        if ctx.input(|i| i.viewport().close_requested()) && !self.quitting {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.visible = false;
            if self
                .activity
                .iter()
                .all(|(_, t)| !t.starts_with("LANLink keeps running"))
            {
                self.log(
                    "LANLink keeps running in the notification area. Right-click its icon to quit.",
                );
            }
        }
        // Countdown and session timer.
        if self.visible && (self.pairing.is_some() || self.session.is_some()) {
            ctx.request_repaint_after(Duration::from_millis(500));
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        egui::Panel::left("nav")
            .exact_size(210.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(NIGHT)
                    .inner_margin(Margin::symmetric(14, 18)),
            )
            .show(ui, |ui| self.nav(ui));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(RAISED)
                    .inner_margin(Margin::symmetric(28, 24)),
            )
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| match self.page {
                        Page::Home => self.home(ui),
                        Page::Pair => self.pair(ui),
                        Page::Devices => self.devices(ui),
                        Page::Settings => self.settings_page(ui),
                    });
            });
        self.approval_modal(&ctx);
    }
}

// ---------------------------------------------------------------- pages

impl LanLinkApp {
    fn nav(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            draw_mark(
                ui.painter(),
                rect,
                if self.session.is_some() { DANGER } else { SAND },
            );
            ui.label(RichText::new("LANLink").size(20.0).strong().color(MIST));
        });
        ui.add_space(22.0);
        for (page, label) in [
            (Page::Home, "Home"),
            (Page::Pair, "Pair a phone"),
            (Page::Devices, "Phones"),
            (Page::Settings, "Settings"),
        ] {
            let selected = self.page == page;
            let resp = nav_item(ui, label, selected);
            if resp.clicked() {
                self.page = page;
            }
        }
        ui.with_layout(Layout::bottom_up(Align::Min), |ui| match &self.status {
            Status::Ready(r) => {
                let addr = r
                    .addresses
                    .iter()
                    .find(|a| {
                        a.is_ipv4() && !matches!(a, std::net::IpAddr::V4(v) if v.is_link_local())
                    })
                    .map(|a| a.to_string())
                    .unwrap_or_else(|| "no network".into());
                ui.label(
                    RichText::new(format!("{addr} · UDP {}", r.port))
                        .size(12.0)
                        .color(MIST_SOFT),
                );
                status_dot(ui, OK, "Listening");
            }
            Status::Starting => status_dot(ui, SAND, "Starting…"),
            Status::Failed(_) => status_dot(ui, DANGER, "Not running"),
        });
    }

    fn home(&mut self, ui: &mut egui::Ui) {
        if let Status::Failed(e) = &self.status {
            card(ui, |ui| {
                ui.label(
                    RichText::new("LANLink could not start")
                        .size(22.0)
                        .strong()
                        .color(DANGER),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(MIST));
            });
            return;
        }
        let paired = self.paired().len();
        // Hero.
        let mut stop = false;
        let mut pair = false;
        card(ui, |ui| match &self.session {
            Some(s) => {
                ui.horizontal(|ui| {
                    pulse_dot(ui, DANGER);
                    ui.label(
                        RichText::new(format!(
                            "{} is {} this PC",
                            s.device,
                            if s.view_only {
                                "viewing"
                            } else {
                                "controlling"
                            }
                        ))
                        .size(24.0)
                        .strong()
                        .color(MIST),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "{} · {}×{} @ {} fps · {}{} · {} · for {}",
                        s.display,
                        s.size.0,
                        s.size.1,
                        s.fps,
                        s.encoder,
                        if s.hardware { "" } else { " (software)" },
                        s.capture,
                        short_duration(s.since.elapsed())
                    ))
                    .color(MIST_SOFT),
                );
                ui.add_space(14.0);
                ui.horizontal(|ui| {
                    if big_button(ui, "Stop session", DANGER_DEEP, Color32::WHITE).clicked() {
                        stop = true;
                    }
                    ui.add_space(8.0);
                    let hint = match self.hotkey_ok {
                        Some(false) => format!("{} is taken by another app", overlay::HOTKEY_LABEL),
                        _ => format!("or press {} anywhere", overlay::HOTKEY_LABEL),
                    };
                    ui.label(RichText::new(hint).color(MIST_SOFT));
                });
            }
            None => {
                let ready = matches!(self.status, Status::Ready(_));
                ui.horizontal(|ui| {
                    pulse_dot(ui, if ready { OK } else { SAND });
                    ui.label(
                        RichText::new(if ready {
                            "Ready for your phone"
                        } else {
                            "Starting…"
                        })
                        .size(24.0)
                        .strong()
                        .color(MIST),
                    );
                });
                ui.add_space(4.0);
                ui.label(
                    RichText::new(if paired == 0 {
                        "Pair a phone once; after that it connects with a tap in the LANLink app."
                    } else {
                        "Open LANLink on a paired phone and tap this PC to connect."
                    })
                    .color(MIST_SOFT),
                );
                ui.add_space(14.0);
                if big_button(ui, "Pair a phone", SAND, NIGHT).clicked() {
                    pair = true;
                }
            }
        });
        if stop {
            self.stop_session();
        }
        if pair {
            self.open_pairing();
        }

        ui.add_space(16.0);
        ui.label(RichText::new("Live").size(15.0).strong().color(MIST));
        ui.add_space(4.0);
        let m = &self.metrics;
        let live = self.session.is_some();
        let tiles: [(&str, &Series, &str, Color32, usize); 5] = [
            ("Frames on phone", &m.fps, "fps", BLUE, 0),
            ("Network round trip", &m.rtt, "ms", SAND, 1),
            ("Bitrate", &m.bitrate, "Mb/s", BLUE, 1),
            ("Encode", &m.encode, "ms", SAND, 1),
            ("Phone decode", &m.decode, "ms", BLUE, 1),
        ];
        // Tiles are a fixed width; lay them out in as many columns as fit
        // (egui cannot wrap a frame before it knows its size).
        const TILE_OUTER: f32 = 150.0 + 2.0 * 14.0 + 2.0 + 12.0;
        let cols = ((ui.available_width() + 12.0) / TILE_OUTER)
            .floor()
            .max(1.0) as usize;
        for row in tiles.chunks(cols) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(12.0);
                for (title, series, unit, color, digits) in row {
                    tile(
                        ui,
                        title,
                        if live { series.last() } else { None },
                        unit,
                        *digits,
                        series,
                        *color,
                    );
                }
            });
            ui.add_space(4.0);
        }

        ui.add_space(16.0);
        ui.label(RichText::new("Activity").size(15.0).strong().color(MIST));
        ui.add_space(4.0);
        card(ui, |ui| {
            if self.activity.is_empty() {
                ui.label(RichText::new("Nothing yet.").color(MIST_SOFT));
            }
            for (t, text) in self.activity.iter().take(10) {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new(clock(*t))
                            .monospace()
                            .size(12.0)
                            .color(MIST_SOFT),
                    );
                    ui.label(RichText::new(text).color(MIST));
                });
            }
        });
    }

    fn pair(&mut self, ui: &mut egui::Ui) {
        ui.label(
            RichText::new("Pair a phone")
                .size(24.0)
                .strong()
                .color(MIST),
        );
        ui.label(
            RichText::new("Open LANLink on your phone, tap Pair, and scan this code. Both screens then show the same six digits; approve only if they match.")
                .color(MIST_SOFT),
        );
        ui.add_space(16.0);
        let mut cancel = false;
        let mut start = false;
        match &self.pairing {
            Some(p) => {
                let remaining = p.lifetime.saturating_sub(p.opened.elapsed());
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        let side = 260.0;
                        let (rect, _) = ui.allocate_exact_size(Vec2::splat(side), Sense::hover());
                        draw_qr(ui.painter(), rect, &p.qr, p.qr_width);
                        ui.add_space(24.0);
                        ui.vertical(|ui| {
                            ui.label(RichText::new("Or type this code").color(MIST_SOFT));
                            let code = format!(
                                "{} {}",
                                &p.code[..4.min(p.code.len())],
                                &p.code[4.min(p.code.len())..]
                            );
                            ui.label(
                                RichText::new(code)
                                    .size(40.0)
                                    .monospace()
                                    .strong()
                                    .color(SAND),
                            );
                            ui.add_space(12.0);
                            let frac = remaining.as_secs_f32() / p.lifetime.as_secs_f32().max(1.0);
                            countdown(ui, frac, remaining.as_secs());
                            ui.add_space(16.0);
                            if ghost_button(ui, "Cancel").clicked() {
                                cancel = true;
                            }
                            ui.add_space(8.0);
                            ui.label(RichText::new(&p.uri).size(10.0).monospace().color(LINE));
                        });
                    });
                });
            }
            None => {
                card(ui, |ui| {
                    ui.label(RichText::new("Pairing is closed.").color(MIST));
                    ui.label(
                        RichText::new("Opening it shows a QR code for two minutes. Only a phone that scans it (or types the code) and that you approve here can pair.")
                            .color(MIST_SOFT),
                    );
                    ui.add_space(12.0);
                    if big_button(ui, "Show pairing code", SAND, NIGHT).clicked() {
                        start = true;
                    }
                });
            }
        }
        if cancel {
            self.command(HostCommand::ClosePairing);
        }
        if start {
            self.open_pairing();
        }
    }

    fn paired(&self) -> Vec<PairedDevice> {
        match &self.status {
            Status::Ready(r) => r.handle.paired_devices(),
            _ => Vec::new(),
        }
    }

    fn devices(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Phones").size(24.0).strong().color(MIST));
        ui.label(
            RichText::new(
                "Phones paired with this PC. Removing one disconnects it and it must pair again.",
            )
            .color(MIST_SOFT),
        );
        ui.add_space(16.0);
        let devices = self.paired();
        if devices.is_empty() {
            card(ui, |ui| {
                ui.label(RichText::new("No phones yet.").color(MIST));
            });
            return;
        }
        let now = unix_now();
        let mut revoke: Option<Fingerprint> = None;
        for d in &devices {
            let connected = self.session.as_ref().is_some_and(|s| s.device == d.name);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&d.name).size(17.0).strong().color(MIST));
                            if connected {
                                ui.label(RichText::new("connected").size(12.0).color(DANGER));
                            }
                        });
                        ui.label(
                            RichText::new(format!(
                                "{} · paired {} · last used {} · id {}",
                                d.model,
                                ago(now, d.paired_at),
                                if d.last_seen == 0 {
                                    "never".to_string()
                                } else {
                                    ago(now, d.last_seen)
                                },
                                d.fingerprint.short()
                            ))
                            .size(12.5)
                            .color(MIST_SOFT),
                        );
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self
                            .confirm_revoke
                            .as_ref()
                            .is_some_and(|f| f.ct_eq(&d.fingerprint))
                        {
                            if small_button(ui, "Remove", DANGER_DEEP, Color32::WHITE).clicked() {
                                revoke = Some(d.fingerprint);
                            }
                            if ghost_button(ui, "Keep").clicked() {
                                self.confirm_revoke = None;
                            }
                        } else if ghost_button(ui, "Remove…").clicked() {
                            self.confirm_revoke = Some(d.fingerprint);
                        }
                    });
                });
            });
            ui.add_space(8.0);
        }
        if let Some(fp) = revoke {
            self.command(HostCommand::Revoke(fp));
            self.confirm_revoke = None;
            self.log("Removed a paired phone");
        }
    }

    fn settings_page(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Settings").size(24.0).strong().color(MIST));
        ui.add_space(16.0);
        let mut changed = false;
        card(ui, |ui| {
            ui.label(RichText::new("Name shown on phones").color(MIST));
            ui.horizontal(|ui| {
                let edit = egui::TextEdit::singleline(&mut self.name_edit)
                    .desired_width(260.0)
                    .char_limit(48);
                ui.add(edit);
                if self.name_edit.trim() != self.settings.name
                    && small_button(ui, "Save", SAND, NIGHT).clicked()
                {
                    self.settings.name = self.name_edit.trim().to_string();
                    changed = true;
                }
            });
            ui.label(
                RichText::new("Takes effect the next time LANLink starts.")
                    .size(12.0)
                    .color(MIST_SOFT),
            );
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            if toggle_row(
                ui,
                "Ask before every session",
                "Not only when a phone pairs: approve each connection here.",
                &mut self.settings.ask_each_session,
            ) {
                self.command(HostCommand::SetSessionApproval(
                    self.settings.ask_each_session,
                ));
                changed = true;
            }
            ui.add_space(6.0);
            if toggle_row(
                ui,
                "Show the on-screen indicator",
                "A red border and a Stop bar while a phone is connected. Phones never see it.",
                &mut self.settings.indicator,
            ) {
                self.sync_indicator();
                changed = true;
            }
            ui.add_space(6.0);
            let mut login = self.run_at_login;
            if toggle_row(
                ui,
                "Start LANLink when I sign in",
                "Starts in the notification area.",
                &mut login,
            ) {
                match settings::set_run_at_login(login) {
                    Ok(()) => {
                        self.run_at_login = login;
                        self.settings_error = None;
                    }
                    Err(e) => {
                        self.settings_error =
                            Some(format!("Could not change the startup setting: {e}"))
                    }
                }
            }
            if let Some(e) = &self.settings_error {
                ui.label(RichText::new(e).color(DANGER));
            }
        });
        if changed {
            if let Err(e) = self.settings.save(&self.store_dir) {
                self.settings_error = Some(format!("Could not save settings: {e}"));
            }
        }
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.label(RichText::new("This PC").color(MIST));
            if let Status::Ready(r) = &self.status {
                ui.label(
                    RichText::new(format!("Identity {}  ·  UDP {}", r.fingerprint, r.port))
                        .monospace()
                        .color(MIST_SOFT),
                );
                ui.label(
                    RichText::new(format!(
                        "Addresses: {}",
                        r.addresses
                            .iter()
                            .map(|a| a.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ))
                    .size(12.0)
                    .color(MIST_SOFT),
                );
            }
            ui.label(
                RichText::new("Windows Firewall must allow LANLink on your network; Windows asks the first time it runs.")
                    .size(12.0)
                    .color(MIST_SOFT),
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ghost_button(ui, "Open log folder").clicked() {
                    let _ = std::process::Command::new("explorer")
                        .arg(&self.store_dir)
                        .spawn();
                }
                if ghost_button(ui, "Quit LANLink").clicked() {
                    let ctx = ui.ctx().clone();
                    self.quit(&ctx);
                }
            });
            ui.label(
                RichText::new(format!("LANLink {}", env!("CARGO_PKG_VERSION")))
                    .size(12.0)
                    .color(LINE),
            );
        });
    }

    fn approval_modal(&mut self, ctx: &egui::Context) {
        // The phone gave up or disconnected: the question no longer stands,
        // and approving it must not be possible.
        if self
            .approval
            .as_ref()
            .is_some_and(|a| a.respond.as_ref().is_none_or(|tx| tx.is_closed()))
        {
            self.approval = None;
            self.log("A request was withdrawn by the phone".to_string());
        }
        let Some(a) = &self.approval else { return };
        let mut answer: Option<bool> = None;
        let (title, body, code, yes) = match &a.kind {
            ApprovalKind::Pairing { name, model, code } => (
                format!("Pair with “{name}”?"),
                format!("{model} wants to pair with this PC. Approve only if the phone shows exactly this code:"),
                Some(code.clone()),
                "Pair",
            ),
            ApprovalKind::Session { name, view_only } => (
                format!("Let “{name}” {} this PC?", if *view_only { "view" } else { "control" }),
                if *view_only {
                    "The phone will see your screen.".to_string()
                } else {
                    "The phone will see your screen and use your mouse and keyboard.".to_string()
                },
                None,
                "Allow",
            ),
        };
        egui::Modal::new(egui::Id::unique("lanlink-approval")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.label(RichText::new(title).size(20.0).strong().color(MIST));
            ui.add_space(6.0);
            ui.label(RichText::new(body).color(MIST_SOFT));
            if let Some(code) = code {
                ui.add_space(8.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} {}",
                            &code[..3.min(code.len())],
                            &code[3.min(code.len())..]
                        ))
                        .size(42.0)
                        .monospace()
                        .strong()
                        .color(SAND),
                    );
                });
            }
            ui.add_space(14.0);
            ui.horizontal(|ui| {
                if big_button(ui, yes, SAND, NIGHT).clicked() {
                    answer = Some(true);
                }
                if ghost_button(ui, "Decline").clicked() {
                    answer = Some(false);
                }
            });
        });
        if let Some(yes) = answer {
            self.answer(yes);
        }
    }
}

// ---------------------------------------------------------------- widgets

fn style(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|s| {
        s.spacing.item_spacing = Vec2::new(10.0, 8.0);
        s.spacing.button_padding = Vec2::new(14.0, 8.0);
        let v = &mut s.visuals;
        v.panel_fill = RAISED;
        v.window_fill = RAISED;
        v.extreme_bg_color = NIGHT;
        v.window_stroke = Stroke::new(1.0, LINE);
        v.selection.bg_fill = SAND.gamma_multiply(0.5);
        v.override_text_color = Some(MIST);
        v.window_corner_radius = CornerRadius::same(14);
        for w in [
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.noninteractive,
        ] {
            w.corner_radius = CornerRadius::same(8);
        }
        v.widgets.inactive.weak_bg_fill = RAISED_HI;
        v.widgets.inactive.bg_fill = RAISED_HI;
        v.widgets.hovered.weak_bg_fill = LINE;
        v.widgets.hovered.bg_fill = LINE;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    });
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(NIGHT)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

fn nav_item(ui: &mut egui::Ui, label: &str, selected: bool) -> egui::Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 38.0), Sense::click());
    let bg = if selected {
        RAISED
    } else if resp.hovered() {
        RAISED.gamma_multiply(0.6)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
    if selected {
        let bar =
            egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 9.0), Vec2::new(3.0, 20.0));
        ui.painter().rect_filled(bar, CornerRadius::same(2), SAND);
    }
    ui.painter().text(
        rect.left_center() + Vec2::new(16.0, 0.0),
        Align2::LEFT_CENTER,
        label,
        FontId::proportional(15.0),
        if selected { MIST } else { MIST_SOFT },
    );
    resp
}

fn status_dot(ui: &mut egui::Ui, color: Color32, text: &str) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
        ui.painter().circle_filled(r.center(), 4.5, color);
        ui.label(RichText::new(text).size(13.0).color(MIST));
    });
}

fn pulse_dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
    let t = ui.input(|i| i.time) as f32;
    let halo = 0.35 + 0.25 * (t * 2.4).sin().abs();
    ui.painter()
        .circle_filled(r.center(), 8.5, color.gamma_multiply(halo));
    ui.painter().circle_filled(r.center(), 5.0, color);
}

fn big_button(ui: &mut egui::Ui, label: &str, fill: Color32, text: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(15.5).strong().color(text))
            .fill(fill)
            .corner_radius(CornerRadius::same(10))
            .min_size(Vec2::new(150.0, 42.0)),
    )
}

fn small_button(ui: &mut egui::Ui, label: &str, fill: Color32, text: Color32) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).strong().color(text))
            .fill(fill)
            .corner_radius(CornerRadius::same(8)),
    )
}

fn ghost_button(ui: &mut egui::Ui, label: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(label).color(MIST))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0, LINE))
            .corner_radius(CornerRadius::same(8)),
    )
}

/// A labelled switch; returns true when flipped.
fn toggle_row(ui: &mut egui::Ui, title: &str, detail: &str, on: &mut bool) -> bool {
    let mut flipped = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(RichText::new(title).color(MIST));
            ui.label(RichText::new(detail).size(12.0).color(MIST_SOFT));
        });
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let size = Vec2::new(42.0, 24.0);
            let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
            if resp.clicked() {
                *on = !*on;
                flipped = true;
            }
            let t = ui.ctx().animate_bool(resp.id, *on);
            let track = if *on { SAND } else { LINE };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(12), track);
            let x = egui::lerp(rect.left() + 12.0..=rect.right() - 12.0, t);
            ui.painter().circle_filled(
                egui::pos2(x, rect.center().y),
                9.0,
                if *on { NIGHT } else { MIST },
            );
        });
    });
    flipped
}

fn tile(
    ui: &mut egui::Ui,
    title: &str,
    value: Option<f32>,
    unit: &str,
    digits: usize,
    series: &Series,
    color: Color32,
) {
    egui::Frame::new()
        .fill(NIGHT)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(14))
        .show(ui, |ui| {
            // The parent row is horizontal; a tile stacks its own content.
            ui.vertical(|ui| {
                ui.set_width(150.0);
                ui.label(RichText::new(title).size(12.0).color(MIST_SOFT));
                ui.horizontal(|ui| {
                    let v = value
                        .map(|v| format!("{v:.digits$}"))
                        .unwrap_or_else(|| "–".into());
                    ui.label(RichText::new(v).size(26.0).strong().color(MIST));
                    ui.label(RichText::new(unit).size(12.0).color(MIST_SOFT));
                });
                let (rect, _) = ui.allocate_exact_size(Vec2::new(150.0, 34.0), Sense::hover());
                sparkline(ui.painter(), rect, series, color);
            });
        });
}

fn sparkline(p: &egui::Painter, rect: egui::Rect, s: &Series, color: Color32) {
    if s.0.len() < 2 {
        p.line_segment(
            [rect.left_bottom(), rect.right_bottom()],
            Stroke::new(1.0, LINE),
        );
        return;
    }
    let max = s.0.iter().copied().fold(0.0f32, f32::max).max(1e-3);
    let n = Series::LEN as f32 - 1.0;
    let offset = Series::LEN - s.0.len();
    let pts: Vec<egui::Pos2> =
        s.0.iter()
            .enumerate()
            .map(|(i, v)| {
                let x = rect.left() + rect.width() * ((i + offset) as f32 / n);
                let y = rect.bottom() - rect.height() * (v / max).clamp(0.0, 1.0);
                egui::pos2(x, y)
            })
            .collect();
    p.line(pts, Stroke::new(1.6, color));
}

fn countdown(ui: &mut egui::Ui, frac: f32, secs: u64) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(220.0, 6.0), Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(3), LINE);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, CornerRadius::same(3), SAND);
    ui.label(
        RichText::new(format!("Closes in {secs} s"))
            .size(12.0)
            .color(MIST_SOFT),
    );
}

fn draw_qr(p: &egui::Painter, rect: egui::Rect, modules: &[bool], width: usize) {
    p.rect_filled(rect, CornerRadius::same(12), Color32::WHITE);
    if width == 0 {
        return;
    }
    let quiet = 3.0;
    let cell = rect.width() / (width as f32 + 2.0 * quiet);
    for (i, dark) in modules.iter().enumerate() {
        if !*dark {
            continue;
        }
        let x = (i % width) as f32;
        let y = (i / width) as f32;
        let min = rect.min + Vec2::new((x + quiet) * cell, (y + quiet) * cell);
        // Slight overlap avoids hairline gaps between modules.
        p.rect_filled(
            egui::Rect::from_min_size(min, Vec2::splat(cell + 0.4)),
            CornerRadius::ZERO,
            NIGHT,
        );
    }
}

fn draw_mark(p: &egui::Painter, rect: egui::Rect, color: Color32) {
    let c = rect.center();
    let s = rect.width();
    p.circle_filled(c, s * 0.48, color);
    for dx in [-0.09, 0.09] {
        p.circle_stroke(
            c + Vec2::new(s * dx, 0.0),
            s * 0.15,
            Stroke::new(s * 0.055, NIGHT),
        );
    }
}

// ---------------------------------------------------------------- time

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn ago(now: u64, then: u64) -> String {
    let d = now.saturating_sub(then);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => format!("{} days ago", d / 86_400),
    }
}

fn short_duration(d: Duration) -> String {
    let s = d.as_secs();
    if s < 60 {
        format!("{s} s")
    } else if s < 3600 {
        format!("{} min", s / 60)
    } else {
        format!("{} h {} min", s / 3600, (s % 3600) / 60)
    }
}

/// Local wall-clock time as HH:MM (Windows local time zone).
fn clock(t: SystemTime) -> String {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{FileTimeToSystemTime, SystemTimeToTzSpecificLocalTime};
    let Ok(d) = t.duration_since(UNIX_EPOCH) else {
        return "--:--".into();
    };
    // FILETIME counts 100 ns since 1601-01-01.
    let ft = (d.as_nanos() / 100) as u64 + 116_444_736_000_000_000;
    let ft = FILETIME {
        dwLowDateTime: ft as u32,
        dwHighDateTime: (ft >> 32) as u32,
    };
    let mut utc = SYSTEMTIME::default();
    let mut local = SYSTEMTIME::default();
    // SAFETY: plain conversions between stack values.
    unsafe {
        if FileTimeToSystemTime(&ft, &mut utc).is_err()
            || SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_err()
        {
            return "--:--".into();
        }
    }
    format!("{:02}:{:02}", local.wHour, local.wMinute)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_times() {
        assert_eq!(ago(1000, 990), "just now");
        assert_eq!(ago(10_000, 10_000 - 600), "10 min ago");
        assert_eq!(ago(200_000, 200_000 - 7200), "2 h ago");
        assert_eq!(ago(1_000_000, 1_000_000 - 3 * 86_400), "3 days ago");
        assert_eq!(short_duration(Duration::from_secs(3725)), "1 h 2 min");
    }

    #[test]
    fn series_keeps_one_minute() {
        let mut s = Series::default();
        for i in 0..100 {
            s.push(i as f32);
        }
        assert_eq!(s.0.len(), Series::LEN);
        assert_eq!(s.last(), Some(99.0));
    }
}
