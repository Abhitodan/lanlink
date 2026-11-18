//! Notification-area icon on its own thread with its own message loop, so it
//! keeps working (and changing colour) while the main window is hidden.

use crate::icon::{self, Mark};
use crate::{AppMsg, TrayAction};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, OnceLock};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, GetMessageW, PostThreadMessageW, TranslateMessage, MSG, WM_APP,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrayState {
    pub mark: Mark,
    pub tooltip: String,
    pub session_active: bool,
}

static STATE: Mutex<Option<TrayState>> = Mutex::new(None);
static THREAD: OnceLock<u32> = OnceLock::new();

/// Update the icon, tooltip and menu. Cheap; safe from any thread.
pub fn set(state: TrayState) {
    if let Ok(mut s) = STATE.lock() {
        *s = Some(state);
    }
    if let Some(tid) = THREAD.get() {
        // SAFETY: posting a message to a thread id we own.
        unsafe {
            let _ = PostThreadMessageW(*tid, WM_APP, Default::default(), Default::default());
        }
    }
}

fn tray_icon(mark: Mark) -> Option<Icon> {
    Icon::from_rgba(icon::rgba(mark, 32), 32, 32).ok()
}

pub fn spawn(tx: Sender<AppMsg>, ctx: eframe::egui::Context) {
    let r = std::thread::Builder::new()
        .name("lanlink-tray".into())
        .spawn(move || {
            // SAFETY: no preconditions.
            let _ = THREAD.set(unsafe { GetCurrentThreadId() });

            let open = MenuItem::new("Open LANLink", true, None);
            let pair = MenuItem::new("Pair a phone…", true, None);
            let stop = MenuItem::new("Stop session", false, None);
            let quit = MenuItem::new("Quit LANLink", true, None);
            let menu = Menu::new();
            for item in [&open as &dyn tray_icon::menu::IsMenuItem, &pair, &stop] {
                let _ = menu.append(item);
            }
            let _ = menu.append(&PredefinedMenuItem::separator());
            let _ = menu.append(&quit);

            let ids = (
                open.id().clone(),
                pair.id().clone(),
                stop.id().clone(),
                quit.id().clone(),
            );
            {
                let tx = tx.clone();
                let ctx = ctx.clone();
                MenuEvent::set_event_handler(Some(move |e: MenuEvent| {
                    let action = if e.id == ids.0 {
                        TrayAction::Open
                    } else if e.id == ids.1 {
                        TrayAction::Pair
                    } else if e.id == ids.2 {
                        TrayAction::Stop
                    } else if e.id == ids.3 {
                        TrayAction::Quit
                    } else {
                        return;
                    };
                    let _ = tx.send(AppMsg::Tray(action));
                    ctx.request_repaint();
                }));
            }
            {
                let tx = tx.clone();
                let ctx = ctx.clone();
                TrayIconEvent::set_event_handler(Some(move |e: TrayIconEvent| {
                    let open = matches!(
                        e,
                        TrayIconEvent::Click {
                            button: MouseButton::Left,
                            button_state: MouseButtonState::Up,
                            ..
                        } | TrayIconEvent::DoubleClick {
                            button: MouseButton::Left,
                            ..
                        }
                    );
                    if open {
                        let _ = tx.send(AppMsg::Tray(TrayAction::Open));
                        ctx.request_repaint();
                    }
                }));
            }

            let mut builder = TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .with_tooltip("LANLink — starting");
            if let Some(i) = tray_icon(Mark::Off) {
                builder = builder.with_icon(i);
            }
            let tray = match builder.build() {
                Ok(t) => t,
                Err(e) => {
                    tracing::error!(error = %e, "could not create the tray icon");
                    return;
                }
            };

            let mut current: Option<TrayState> = None;
            let mut msg = MSG::default();
            // SAFETY: standard message loop for windows created on this thread.
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.as_bool() {
                if msg.hwnd.is_invalid() && msg.message == WM_APP {
                    let wanted = STATE.lock().ok().and_then(|s| s.clone());
                    if let Some(w) = wanted {
                        if current.as_ref() != Some(&w) {
                            if current.as_ref().map(|c| c.mark) != Some(w.mark) {
                                let _ = tray.set_icon(tray_icon(w.mark));
                            }
                            let _ = tray.set_tooltip(Some(&w.tooltip));
                            stop.set_enabled(w.session_active);
                            current = Some(w);
                        }
                    }
                    continue;
                }
                unsafe {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            }
        });
    if let Err(e) = r {
        tracing::error!(error = %e, "could not start the tray thread");
    }
}
