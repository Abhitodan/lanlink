//! Display enumeration.

use crate::CaptureError;
use windows::core::Interface;
use windows::Win32::Foundation::LUID;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_MODE_ROTATION_ROTATE180,
    DXGI_MODE_ROTATION_ROTATE270, DXGI_MODE_ROTATION_ROTATE90,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput, IDXGIOutput6,
    DXGI_ADAPTER_FLAG_SOFTWARE, DXGI_ERROR_NOT_FOUND,
};
use windows::Win32::Graphics::Gdi::{EnumDisplaySettingsW, DEVMODEW, ENUM_CURRENT_SETTINGS};
use windows::Win32::UI::HiDpi::{
    GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
    MDT_EFFECTIVE_DPI,
};

/// One display attached to the desktop.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Stable for as long as Windows keeps the same device name
    /// (`\\.\DISPLAYn`). Never `u32::MAX`, which the protocol reserves.
    pub id: u32,
    /// GDI device name, e.g. `\\.\DISPLAY1`.
    pub device_name: String,
    /// Human-readable label.
    pub label: String,
    /// Position and size in physical pixels of the virtual desktop. `x`/`y`
    /// are negative for displays left of or above the primary.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
    pub refresh_millihz: u32,
    /// Windows display scale, e.g. 150 for 150 %.
    pub scale_percent: u16,
    /// Clockwise rotation in degrees.
    pub rotation: u16,
    pub hdr: bool,
    pub adapter_luid: i64,
    pub adapter_name: String,
    pub(crate) hmonitor: isize,
    pub(crate) adapter_index: u32,
    pub(crate) output_index: u32,
}

/// Make the process per-monitor DPI aware so that every coordinate this
/// process sees (monitor rectangles, cursor positions, injected input) is in
/// physical pixels. Call once at startup, before creating any window.
/// Returns `false` if awareness was already fixed (for example by a manifest).
pub fn init_process_dpi_awareness() -> bool {
    // SAFETY: plain Win32 call with a constant argument.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_ok() }
}

pub(crate) fn luid_to_i64(l: LUID) -> i64 {
    ((l.HighPart as i64) << 32) | l.LowPart as i64
}

pub(crate) fn utf16_to_string(buf: &[u16]) -> String {
    let len = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..len])
}

/// FNV-1a of the device name; avoids the reserved value.
fn stable_id(device_name: &str) -> u32 {
    let mut h: u32 = 0x811c_9dc5;
    for b in device_name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    if h == u32::MAX {
        0
    } else {
        h
    }
}

pub(crate) fn factory() -> Result<IDXGIFactory1, CaptureError> {
    // SAFETY: no preconditions.
    unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }
        .map_err(CaptureError::win("CreateDXGIFactory1"))
}

pub(crate) fn adapter_and_output(
    m: &MonitorInfo,
) -> Result<(IDXGIAdapter1, IDXGIOutput), CaptureError> {
    let factory = factory()?;
    // SAFETY: indices come from a previous enumeration; errors are handled.
    unsafe {
        let adapter = factory
            .EnumAdapters1(m.adapter_index)
            .map_err(|_| CaptureError::MonitorNotFound(m.id))?;
        let output = adapter
            .EnumOutputs(m.output_index)
            .map_err(|_| CaptureError::MonitorNotFound(m.id))?;
        // Indices can shift after a hotplug; make sure this is still the
        // display the caller meant.
        let desc = output
            .GetDesc()
            .map_err(CaptureError::win("IDXGIOutput::GetDesc"))?;
        if utf16_to_string(&desc.DeviceName) != m.device_name {
            return Err(CaptureError::MonitorNotFound(m.id));
        }
        Ok((adapter, output))
    }
}

/// Every non-software adapter, with the one matching `prefer_luid` first and
/// the rest in DXGI order.
pub(crate) fn hardware_adapters(
    prefer_luid: Option<i64>,
) -> Result<Vec<IDXGIAdapter1>, CaptureError> {
    let factory = factory()?;
    let mut preferred = Vec::new();
    let mut others = Vec::new();
    // SAFETY: DXGI enumeration with handled errors.
    unsafe {
        let mut i = 0u32;
        while let Ok(adapter) = factory.EnumAdapters1(i) {
            i += 1;
            let Ok(desc) = adapter.GetDesc1() else {
                continue;
            };
            if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
                continue;
            }
            if Some(luid_to_i64(desc.AdapterLuid)) == prefer_luid {
                preferred.push(adapter);
            } else {
                others.push(adapter);
            }
        }
    }
    preferred.extend(others);
    Ok(preferred)
}

/// List every display attached to the desktop, primary first.
pub fn enumerate_monitors() -> Result<Vec<MonitorInfo>, CaptureError> {
    let factory = factory()?;
    let mut out = Vec::new();
    // SAFETY: DXGI enumeration; every out-structure is fully written by the
    // callee before we read it, and failures are propagated.
    unsafe {
        let mut ai = 0u32;
        loop {
            let adapter = match factory.EnumAdapters1(ai) {
                Ok(a) => a,
                Err(e) if e.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(e) => return Err(CaptureError::win("EnumAdapters1")(e)),
            };
            let adesc = adapter.GetDesc1().map_err(CaptureError::win("GetDesc1"))?;
            let software = adesc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0;
            let adapter_name = utf16_to_string(&adesc.Description);
            let mut oi = 0u32;
            loop {
                let output = match adapter.EnumOutputs(oi) {
                    Ok(o) => o,
                    Err(_) => break,
                };
                let desc = output
                    .GetDesc()
                    .map_err(CaptureError::win("IDXGIOutput::GetDesc"))?;
                if desc.AttachedToDesktop.as_bool() && !software {
                    let device_name = utf16_to_string(&desc.DeviceName);
                    let r = desc.DesktopCoordinates;
                    let width = (r.right - r.left).max(0) as u32;
                    let height = (r.bottom - r.top).max(0) as u32;

                    let mut mode = DEVMODEW {
                        dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                        ..Default::default()
                    };
                    let refresh_millihz = if EnumDisplaySettingsW(
                        windows::core::PCWSTR(desc.DeviceName.as_ptr()),
                        ENUM_CURRENT_SETTINGS,
                        &mut mode,
                    )
                    .as_bool()
                    {
                        mode.dmDisplayFrequency.saturating_mul(1000)
                    } else {
                        60_000
                    };

                    let (mut dpi_x, mut dpi_y) = (96u32, 96u32);
                    let _ =
                        GetDpiForMonitor(desc.Monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);

                    let rotation = match desc.Rotation {
                        DXGI_MODE_ROTATION_ROTATE90 => 90,
                        DXGI_MODE_ROTATION_ROTATE180 => 180,
                        DXGI_MODE_ROTATION_ROTATE270 => 270,
                        _ => 0,
                    };

                    let hdr = output
                        .cast::<IDXGIOutput6>()
                        .ok()
                        .and_then(|o6| o6.GetDesc1().ok())
                        .map(|d| d.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020)
                        .unwrap_or(false);

                    let number = device_name
                        .trim_start_matches(r"\\.\DISPLAY")
                        .parse::<u32>()
                        .ok();
                    let label = match number {
                        Some(n) => format!("Display {n} ({width}\u{00d7}{height})"),
                        None => format!("{device_name} ({width}\u{00d7}{height})"),
                    };

                    out.push(MonitorInfo {
                        id: stable_id(&device_name),
                        device_name,
                        label,
                        x: r.left,
                        y: r.top,
                        width,
                        height,
                        primary: r.left == 0 && r.top == 0,
                        refresh_millihz,
                        scale_percent: ((dpi_x * 100 + 48) / 96) as u16,
                        rotation,
                        hdr,
                        adapter_luid: luid_to_i64(adesc.AdapterLuid),
                        adapter_name: adapter_name.clone(),
                        hmonitor: desc.Monitor.0 as isize,
                        adapter_index: ai,
                        output_index: oi,
                    });
                }
                oi += 1;
            }
            ai += 1;
        }
    }
    out.sort_by_key(|m| (!m.primary, m.x, m.y));
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_is_deterministic_and_never_reserved() {
        assert_eq!(stable_id(r"\\.\DISPLAY1"), stable_id(r"\\.\DISPLAY1"));
        assert_ne!(stable_id(r"\\.\DISPLAY1"), stable_id(r"\\.\DISPLAY2"));
        for n in 0..500 {
            assert_ne!(stable_id(&format!(r"\\.\DISPLAY{n}")), u32::MAX);
        }
    }

    /// Runs against the real machine: there must be exactly one primary
    /// display and every display must have a sane geometry.
    #[test]
    fn enumerates_real_displays() {
        let monitors = enumerate_monitors().expect("enumeration works on a desktop session");
        if monitors.is_empty() {
            eprintln!("no attached displays (headless session); skipping assertions");
            return;
        }
        assert_eq!(monitors.iter().filter(|m| m.primary).count(), 1);
        assert!(monitors[0].primary, "primary sorts first");
        for m in &monitors {
            assert!(m.width >= 320 && m.height >= 200, "{m:?}");
            assert!(m.scale_percent >= 100 && m.scale_percent <= 500, "{m:?}");
            assert!(matches!(m.rotation, 0 | 90 | 180 | 270));
            assert!(!m.adapter_name.is_empty());
        }
    }
}
