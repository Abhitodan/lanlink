//! The Windows `SendInput` sink.

use crate::events::{Event, Rect};
use crate::Sink;
use lanlink_protocol::MouseButton;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenIntegrityLevel,
    TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{
    GetForegroundWindow, GetSystemMetrics, GetWindowThreadProcessId, SM_CXVIRTUALSCREEN,
    SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

/// Marker placed in `dwExtraInfo` of every injected event ("LLNK"), so
/// LANLink-originated input can be identified in diagnostics.
pub const EXTRA_INFO: usize = 0x4C4C_4E4B;

const WHEEL_X_BUTTON1: u32 = 0x0001;
const WHEEL_X_BUTTON2: u32 = 0x0002;

/// The virtual desktop in physical pixels. The process must be per-monitor
/// DPI aware (see `capture_windows::init_process_dpi_awareness`), otherwise
/// Windows reports scaled values.
pub fn virtual_desktop() -> Rect {
    // SAFETY: plain metric queries.
    unsafe {
        Rect {
            x: GetSystemMetrics(SM_XVIRTUALSCREEN),
            y: GetSystemMetrics(SM_YVIRTUALSCREEN),
            width: GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32,
            height: GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32,
        }
    }
}

#[derive(Debug, Default)]
pub struct SendInputSink;

fn mouse(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: EXTRA_INFO,
            },
        },
    }
}

fn key(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: scan,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: EXTRA_INFO,
            },
        },
    }
}

fn to_input(e: &Event) -> INPUT {
    match *e {
        Event::MoveAbs { x, y } => mouse(
            x as i32,
            y as i32,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        ),
        Event::MoveRel { dx, dy } => mouse(dx, dy, 0, MOUSEEVENTF_MOVE),
        Event::Button { button, down } => {
            let (flags, data) = match (button, down) {
                (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, WHEEL_X_BUTTON1),
                (MouseButton::X1, false) => (MOUSEEVENTF_XUP, WHEEL_X_BUTTON1),
                (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, WHEEL_X_BUTTON2),
                (MouseButton::X2, false) => (MOUSEEVENTF_XUP, WHEEL_X_BUTTON2),
            };
            mouse(0, 0, data, flags)
        }
        Event::Wheel { delta, horizontal } => mouse(
            0,
            0,
            delta as u32, // mouseData is a DWORD holding a signed value
            if horizontal {
                MOUSEEVENTF_HWHEEL
            } else {
                MOUSEEVENTF_WHEEL
            },
        ),
        Event::Key {
            vk,
            scan,
            extended,
            down,
        } => {
            let mut flags = KEYBD_EVENT_FLAGS(0);
            if extended {
                flags |= KEYEVENTF_EXTENDEDKEY;
            }
            if !down {
                flags |= KEYEVENTF_KEYUP;
            }
            key(vk, scan, flags)
        }
        Event::Unicode { unit, down } => {
            let mut flags = KEYEVENTF_UNICODE;
            if !down {
                flags |= KEYEVENTF_KEYUP;
            }
            key(0, unit, flags)
        }
    }
}

impl Sink for SendInputSink {
    fn send(&mut self, events: &[Event]) -> usize {
        if events.is_empty() {
            return 0;
        }
        let inputs: Vec<INPUT> = events.iter().map(to_input).collect();
        // SAFETY: `inputs` is a live slice of fully initialised INPUTs and
        // the size argument matches the element type.
        unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) as usize }
    }

    fn scan_code(&self, vk: u16) -> u16 {
        // SAFETY: pure lookup in the active keyboard layout.
        let sc = unsafe { MapVirtualKeyW(vk as u32, MAPVK_VK_TO_VSC_EX) };
        (sc & 0xFF) as u16
    }
}

fn integrity_of(process: HANDLE) -> Option<u32> {
    // SAFETY: the token handle is closed on every path; the buffer is sized
    // by the first query and only read after the second succeeds.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(process, TOKEN_QUERY, &mut token).ok()?;
        let mut needed = 0u32;
        let _ = GetTokenInformation(token, TokenIntegrityLevel, None, 0, &mut needed);
        if needed == 0 || needed > 1024 {
            let _ = CloseHandle(token);
            return None;
        }
        let mut buf = vec![0u8; needed as usize];
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            Some(buf.as_mut_ptr().cast()),
            needed,
            &mut needed,
        )
        .is_ok();
        let _ = CloseHandle(token);
        if !ok {
            return None;
        }
        let label = &*(buf.as_ptr() as *const TOKEN_MANDATORY_LABEL);
        let sid = label.Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return None;
        }
        Some(*GetSidSubAuthority(sid, (count - 1) as u32))
    }
}

/// Whether the foreground window belongs to a process with a higher
/// integrity level than ours (for example an elevated app or Task Manager).
/// Windows drops input injected into such windows (UIPI).
///
/// Returns `None` when it cannot be determined. A process we are not even
/// allowed to query is reported as `Some(true)`, which is the common case
/// for elevated processes.
pub fn foreground_is_higher_integrity() -> Option<bool> {
    // SAFETY: handles are checked and closed; no pointers are retained.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == 0 {
            return None;
        }
        let ours = integrity_of(GetCurrentProcess())?;
        let Ok(process) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return Some(true);
        };
        let theirs = integrity_of(process);
        let _ = CloseHandle(process);
        theirs.map(|t| t > ours)
    }
}

impl<S: Sink> crate::Injector<S> {
    /// See [`foreground_is_higher_integrity`].
    pub fn foreground_blocked(&self) -> bool {
        foreground_is_higher_integrity().unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_struct_layout_matches_win32() {
        // SendInput rejects calls whose cbSize does not match the OS struct.
        assert_eq!(std::mem::size_of::<INPUT>(), 40);
    }

    #[test]
    fn virtual_desktop_is_sane() {
        let d = virtual_desktop();
        assert!(d.width >= 320 && d.height >= 200, "{d:?}");
    }

    #[test]
    fn scan_codes_for_common_keys() {
        let s = SendInputSink;
        // Set 1 scan codes are layout-independent for these keys.
        assert_eq!(s.scan_code(0x0D), 0x1C); // Enter
        assert_eq!(s.scan_code(0x1B), 0x01); // Escape
        assert_eq!(s.scan_code(0x20), 0x39); // Space
    }

    #[test]
    fn integrity_query_does_not_fail_for_ourselves() {
        // SAFETY: pseudo-handle for the current process.
        let ours = integrity_of(unsafe { GetCurrentProcess() });
        assert!(ours.is_some());
    }
}
