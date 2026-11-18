//! D3D11 device creation shared by both capture backends.

use crate::monitors::{luid_to_i64, utf16_to_string};
use crate::CaptureError;
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
};
use windows::Win32::Graphics::Dxgi::{IDXGIAdapter, IDXGIAdapter1, IDXGIDevice};

/// A D3D11 device and the adapter it was created on.
///
/// Captured textures live on this device. An encoder on the same adapter can
/// consume them without leaving the GPU; an encoder on another adapter needs
/// an explicit copy.
#[derive(Clone)]
pub struct GpuDevice {
    pub device: ID3D11Device,
    pub context: ID3D11DeviceContext,
    pub adapter_luid: i64,
    pub adapter_name: String,
}

impl GpuDevice {
    /// Create a device on `adapter` with video support and multithread
    /// protection (Media Foundation uses the device from its own threads).
    pub(crate) fn create(adapter: &IDXGIAdapter1) -> Result<Self, CaptureError> {
        let base: IDXGIAdapter = adapter.cast().map_err(CaptureError::win("IDXGIAdapter"))?;
        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        // SAFETY: out-pointers reference live locals; the adapter is valid.
        unsafe {
            D3D11CreateDevice(
                &base,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&[D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0]),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
        }
        .map_err(CaptureError::win("D3D11CreateDevice"))?;
        let device = device.expect("D3D11CreateDevice succeeded without a device");
        let context = context.expect("D3D11CreateDevice succeeded without a context");

        if let Ok(mt) = context.cast::<ID3D11Multithread>() {
            // SAFETY: simple setter on a live interface.
            unsafe {
                let _ = mt.SetMultithreadProtected(true);
            }
        }

        // SAFETY: fills a plain struct.
        let desc = unsafe { adapter.GetDesc1() }.map_err(CaptureError::win("GetDesc1"))?;
        Ok(GpuDevice {
            device,
            context,
            adapter_luid: luid_to_i64(desc.AdapterLuid),
            adapter_name: utf16_to_string(&desc.Description),
        })
    }

    /// LUID of the adapter as reported by the device itself.
    pub fn luid_from_device(device: &ID3D11Device) -> Option<i64> {
        let dxgi: IDXGIDevice = device.cast().ok()?;
        // SAFETY: live interface; `GetDesc` fills a plain struct.
        unsafe {
            let adapter = dxgi.GetAdapter().ok()?;
            let desc = adapter.GetDesc().ok()?;
            Some(luid_to_i64(desc.AdapterLuid))
        }
    }
}
