//! Live injection on the machine running the tests. These move the real
//! pointer (and restore it) and type into a window the test creates, so
//! they are `#[ignore]`d by default:
//!
//! `cargo test -p input-windows --test live -- --ignored --test-threads=1`

#![cfg(windows)]

use input_windows::{virtual_desktop, Injector, Rect, SendInputSink};
use lanlink_protocol::InputKind;
use std::time::Duration;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::*;

fn dpi_aware() {
    // SAFETY: constant argument; failure means it was already set.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
}

fn cursor() -> POINT {
    let mut p = POINT::default();
    // SAFETY: writes one POINT.
    unsafe { GetCursorPos(&mut p).unwrap() };
    p
}

#[test]
#[ignore = "moves the real pointer"]
fn absolute_moves_land_on_exact_pixels() {
    dpi_aware();
    let desktop = virtual_desktop();
    let original = cursor();
    let mut inj = Injector::new(SendInputSink, desktop, desktop);
    let mut misses = Vec::new();
    for (nx, ny) in [(0u16, 0u16), (65535, 65535), (32768, 16384), (12345, 54321)] {
        inj.pointer_abs(nx, ny).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        let got = cursor();
        let ex = desktop.x + lanlink_protocol::denormalize(nx, desktop.width);
        let ey = desktop.y + lanlink_protocol::denormalize(ny, desktop.height);
        println!(
            "normalized ({nx},{ny}) -> expected ({ex},{ey}) got ({},{})",
            got.x, got.y
        );
        if (got.x, got.y) != (ex, ey) {
            misses.push(((ex, ey), (got.x, got.y)));
        }
    }
    // Restore the user's pointer.
    // SAFETY: plain call.
    unsafe {
        let _ = SetCursorPos(original.x, original.y);
    }
    assert!(misses.is_empty(), "pointer missed: {misses:?}");
}

unsafe extern "system" fn wndproc(
    h: HWND,
    m: u32,
    w: WPARAM,
    l: LPARAM,
) -> windows::Win32::Foundation::LRESULT {
    DefWindowProcW(h, m, w, l)
}

#[test]
#[ignore = "injects real keystrokes into a test window"]
fn unicode_text_and_keys_reach_a_focused_edit_control() {
    dpi_aware();
    // SAFETY: standard window creation; all handles are destroyed below.
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("LANLinkInputTest"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let parent = CreateWindowExW(
            WS_EX_TOPMOST,
            w!("LANLinkInputTest"),
            w!("LANLink input test"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            100,
            100,
            500,
            200,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!(""),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            10,
            10,
            460,
            30,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let _ = ShowWindow(parent, SW_SHOW);
        let fg = SetForegroundWindow(parent).as_bool();
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
        pump(Duration::from_millis(200));
        if GetForegroundWindow() != parent {
            DestroyWindow(parent).ok();
            eprintln!(
                "SKIP: Windows did not give the test window focus (foreground lock, set={fg})"
            );
            return;
        }

        let desktop = virtual_desktop();
        let display = Rect { ..desktop };
        let mut inj = Injector::new(SendInputSink, display, desktop);
        inj.apply(&InputKind::Text {
            text: "Héllo 😀 ✓".into(),
        })
        .unwrap();
        // Shift+End then Backspace? Keep it simple: Backspace twice removes
        // the check mark and the space before it.
        inj.apply(&InputKind::Key {
            vk: 0x08,
            down: true,
        })
        .unwrap();
        inj.apply(&InputKind::Key {
            vk: 0x08,
            down: false,
        })
        .unwrap();
        inj.apply(&InputKind::Key {
            vk: 0x08,
            down: true,
        })
        .unwrap();
        inj.apply(&InputKind::Key {
            vk: 0x08,
            down: false,
        })
        .unwrap();
        pump(Duration::from_millis(300));

        let mut buf = [0u16; 128];
        let n = GetWindowTextW(edit, &mut buf);
        let text = String::from_utf16_lossy(&buf[..n as usize]);
        DestroyWindow(parent).ok();
        println!("edit control contains: {text:?}");
        assert_eq!(text, "Héllo 😀");
        assert!(!inj.held().anything_held());
    }
}

unsafe fn pump(d: Duration) {
    let end = std::time::Instant::now() + d;
    while std::time::Instant::now() < end {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "briefly shows and focuses a test window"]
fn caret_is_found_in_a_focused_edit_control() {
    use input_windows::caret::{CaretFinder, CaretSource};
    use windows::Win32::Foundation::RECT;
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    dpi_aware();
    // SAFETY: standard window creation; the window is destroyed below.
    unsafe {
        let instance = GetModuleHandleW(None).unwrap();
        let class = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: instance.into(),
            lpszClassName: w!("LANLinkCaretTest"),
            ..Default::default()
        };
        RegisterClassW(&class);
        let parent = CreateWindowExW(
            WS_EX_TOPMOST,
            w!("LANLinkCaretTest"),
            w!("LANLink caret test"),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            120,
            140,
            500,
            200,
            None,
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            w!("caret here"),
            WS_CHILD | WS_VISIBLE | WINDOW_STYLE(ES_AUTOHSCROLL as u32),
            10,
            10,
            460,
            30,
            Some(parent),
            None,
            Some(instance.into()),
            None,
        )
        .unwrap();
        let fg = SetForegroundWindow(parent).as_bool();
        let _ = windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(edit));
        pump(Duration::from_millis(200));
        if GetForegroundWindow() != parent {
            DestroyWindow(parent).ok();
            eprintln!("SKIP: Windows did not give the test window focus (set={fg})");
            return;
        }
        let mut er = RECT::default();
        GetWindowRect(edit, &mut er).unwrap();

        // The finder runs on its own thread, as in the host, while this
        // thread keeps the window responsive.
        let finder = std::thread::spawn(|| {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let f = CaretFinder::new();
            // Like the host, poll: the first UI Automation request to a
            // window can take most of a second while Windows loads its
            // provider.
            let t = std::time::Instant::now();
            let mut uia = None;
            while uia.is_none() && t.elapsed() < Duration::from_secs(3) {
                uia = f.find_automation();
            }
            println!("automation answered after {:?}", t.elapsed());
            (f.find(), uia)
        });
        let mut found = None;
        while found.is_none() {
            pump(Duration::from_millis(20));
            if finder.is_finished() {
                found = Some(finder.join().unwrap());
                break;
            }
        }
        DestroyWindow(parent).ok();
        let (win32, uia) = found.unwrap();
        println!(
            "edit {er:?}
  win32 {win32:?}
  automation {uia:?}"
        );
        let caret = win32.expect("Win32 caret should be found");
        assert_eq!(caret.source, CaretSource::Win32);
        let uia = uia.expect("UI Automation caret should be found");
        for (what, r) in [("win32", caret.rect), ("automation", uia.rect)] {
            assert!(
                r.x >= er.left && r.x <= er.right,
                "{what}: x inside the edit control"
            );
            assert!(
                r.y >= er.top && r.y <= er.bottom,
                "{what}: y inside the edit control"
            );
            assert!(r.height > 4, "{what}: has a height");
        }
    }
}
