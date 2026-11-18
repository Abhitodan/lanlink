//! App preferences (plain text next to the DPAPI-protected host store) and
//! the per-user "start at login" entry.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppSettings {
    /// Name phones show for this PC. Applied when the host starts.
    pub name: String,
    /// Ask on the PC before every session, not only at pairing.
    pub ask_each_session: bool,
    /// Show the border and "being controlled" bar while a phone is connected.
    pub indicator: bool,
}

impl AppSettings {
    fn file(dir: &Path) -> PathBuf {
        dir.join("app-settings.txt")
    }

    pub fn load(dir: &Path, default_name: &str) -> Self {
        let mut s = AppSettings {
            name: default_name.to_string(),
            ask_each_session: false,
            indicator: true,
        };
        let Ok(text) = std::fs::read_to_string(Self::file(dir)) else {
            return s;
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else {
                continue;
            };
            let v = v.trim();
            match k.trim() {
                "name" if !v.is_empty() => s.name = v.chars().take(48).collect(),
                "ask_each_session" => s.ask_each_session = v == "true",
                "indicator" => s.indicator = v != "false",
                _ => {}
            }
        }
        s
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        let name: String = self
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(48)
            .collect();
        std::fs::write(
            Self::file(dir),
            format!(
                "# LANLink app settings\nname={name}\nask_each_session={}\nindicator={}\n",
                self.ask_each_session, self.indicator
            ),
        )
    }
}

const RUN_KEY: windows::core::PCWSTR =
    windows::core::w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const RUN_VALUE: windows::core::PCWSTR = windows::core::w!("LANLink");

/// Whether LANLink starts when this user signs in.
pub fn run_at_login() -> bool {
    use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_SZ};
    // SAFETY: only queries existence; no output buffers are passed.
    unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_KEY,
            RUN_VALUE,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
        .is_ok()
    }
}

/// Add or remove the per-user Run entry. Only called when the user flips
/// the switch in Settings.
pub fn set_run_at_login(on: bool) -> Result<(), String> {
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
    };
    if on {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let cmd = format!("\"{}\" --minimized", exe.display());
        let wide: Vec<u16> = cmd.encode_utf16().chain(std::iter::once(0)).collect();
        // SAFETY: `wide` is a NUL-terminated UTF-16 buffer whose byte length is passed.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN_KEY,
                RUN_VALUE,
                REG_SZ.0,
                Some(wide.as_ptr().cast()),
                (wide.len() * 2) as u32,
            )
            .ok()
            .map_err(|e| e.to_string())
        }
    } else {
        // SAFETY: plain value deletion.
        let r = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN_KEY, RUN_VALUE) };
        match r.ok() {
            Ok(()) => Ok(()),
            Err(e) if e.code() == windows::Win32::Foundation::ERROR_FILE_NOT_FOUND.to_hresult() => {
                Ok(())
            }
            Err(e) => Err(e.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_sanitises() {
        let dir = std::env::temp_dir().join(format!("lanlink-settings-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let s = AppSettings {
            name: "Desk\nPC".into(),
            ask_each_session: true,
            indicator: false,
        };
        s.save(&dir).unwrap();
        let back = AppSettings::load(&dir, "fallback");
        assert_eq!(back.name, "DeskPC");
        assert!(back.ask_each_session);
        assert!(!back.indicator);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn missing_file_gives_defaults() {
        let dir = std::env::temp_dir().join("lanlink-settings-none-xyz");
        let s = AppSettings::load(&dir, "My PC");
        assert_eq!(s.name, "My PC");
        assert!(!s.ask_each_session);
        assert!(s.indicator);
    }
}
