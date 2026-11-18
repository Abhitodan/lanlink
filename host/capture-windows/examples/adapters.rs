//! Diagnostic: DXGI adapters and outputs under each GPU-preference ordering,
//! and whether Desktop Duplication can be started with each (device adapter,
//! output) combination. Useful on hybrid-graphics laptops.
//! `cargo run -p capture-windows --example adapters`

use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::*;

fn s(buf: &[u16]) -> String {
    let n = buf.iter().position(|c| *c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..n])
}

unsafe fn device_on(adapter: &IDXGIAdapter1) -> windows::core::Result<ID3D11Device> {
    let base: IDXGIAdapter = adapter.cast()?;
    let mut dev = None;
    D3D11CreateDevice(
        &base,
        D3D_DRIVER_TYPE_UNKNOWN,
        HMODULE::default(),
        D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
        Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
        D3D11_SDK_VERSION,
        Some(&mut dev),
        None,
        None,
    )?;
    Ok(dev.unwrap())
}

fn main() {
    capture_windows::init_process_dpi_awareness();
    unsafe {
        let factory: IDXGIFactory6 = CreateDXGIFactory1().unwrap();
        for (pname, pref) in [
            ("UNSPECIFIED", DXGI_GPU_PREFERENCE_UNSPECIFIED),
            ("MINIMUM_POWER", DXGI_GPU_PREFERENCE_MINIMUM_POWER),
            ("HIGH_PERFORMANCE", DXGI_GPU_PREFERENCE_HIGH_PERFORMANCE),
        ] {
            println!("== preference {pname} ==");
            let mut adapters = Vec::new();
            let mut i = 0;
            while let Ok(a) = factory.EnumAdapterByGpuPreference::<IDXGIAdapter1>(i, pref) {
                adapters.push(a);
                i += 1;
            }
            let devices: Vec<_> = adapters.iter().map(|a| device_on(a)).collect();
            for (ai, adapter) in adapters.iter().enumerate() {
                let d = adapter.GetDesc1().unwrap();
                println!(
                    "adapter {ai}: '{}' flags {:#x} device: {}",
                    s(&d.Description),
                    d.Flags,
                    match &devices[ai] {
                        Ok(_) => "ok".to_string(),
                        Err(e) => format!("ERR {:#x}", e.code().0),
                    }
                );
                let mut j = 0;
                while let Ok(out) = adapter.EnumOutputs(j) {
                    let od = out.GetDesc().unwrap();
                    println!("  output {j}: {}", s(&od.DeviceName));
                    let o1: IDXGIOutput1 = out.cast().unwrap();
                    for (di, dev) in devices.iter().enumerate() {
                        if let Ok(dev) = dev {
                            let r = o1.DuplicateOutput(dev);
                            println!(
                                "    duplicate with device of adapter {di}: {}",
                                match r {
                                    Ok(_) => "ok".to_string(),
                                    Err(e) => format!("ERR {:#x} {}", e.code().0, e.message()),
                                }
                            );
                        }
                    }
                    j += 1;
                }
            }
        }
    }
}
