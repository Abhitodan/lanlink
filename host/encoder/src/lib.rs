//! Video encoding for LANLink.
//!
//! [`VideoEncoder`] takes captured BGRA textures and produces H.264 access
//! units through a callback. Internally it is three stages:
//!
//! 1. colour conversion and scaling on the GPU (`convert`),
//! 2. an input sample, which is either a pooled NV12 **texture** (the frame
//!    never leaves video memory) or, for encoders that cannot take textures,
//!    a **system-memory** copy read back from the GPU,
//! 3. a Media Foundation H.264 encoder (`mft`).
//!
//! Which path is in use is reported honestly by [`VideoEncoder::path`]:
//! only [`FramePath::GpuTexture`] is free of CPU copies of pixel data. (The
//! *compressed* output is always copied once into a `Vec<u8>`.)
//!
//! Encoder selection order: hardware encoders on the adapter the frames are
//! on (texture path), then the software encoder (system-memory path).
//! Hardware encoders on a *different* adapter are not selected: moving every
//! frame between GPUs through system memory costs more than it saves, and
//! that path cannot be verified on the development machine.
//!
//! Nothing is queued: [`VideoEncoder::submit`] returns [`Submit::Busy`] and
//! drops the frame when the encoder has no capacity, so a slow encoder
//! lowers the frame rate instead of raising latency.
//!
//! Thread affinity: create and use a `VideoEncoder` on one thread. The
//! callback runs on the encoder's own thread for hardware encoders and on
//! the calling thread for the software encoder.

#![cfg(windows)]

pub mod annexb;
mod convert;
mod mft;

use convert::FrameConverter;
use mft::MftEncoder;
use thiserror::Error;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC,
};
use windows::Win32::Media::MediaFoundation::*;

#[derive(Debug, Error)]
pub enum EncoderError {
    #[error("no usable H.264 encoder ({0})")]
    NoEncoder(String),
    #[error("encoder failed and must be recreated")]
    Failed,
    #[error("not supported: {0}")]
    Unsupported(&'static str),
    #[error("invalid encoder configuration: {0}")]
    Config(&'static str),
    #[error("{context}: {source}")]
    Windows {
        context: &'static str,
        #[source]
        source: windows::core::Error,
    },
}

/// Speed/quality trade-off inside the encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tuning {
    Speed,
    Balanced,
    Quality,
}

/// Which encoders may be used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EncoderPreference {
    /// Hardware first, software as a fallback.
    #[default]
    Auto,
    HardwareOnly,
    /// For testing the fallback path.
    SoftwareOnly,
}

#[derive(Debug, Clone, Copy)]
pub struct EncoderConfig {
    /// Coded size. Must be even and at least 64x64.
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_bps: u32,
    pub tuning: Tuning,
    pub preference: EncoderPreference,
    /// Monotonic microsecond clock shared with the caller; used to measure
    /// capture-to-encoded time against the `capture_us` passed to `submit`.
    pub clock: fn() -> u64,
}

/// One encoded access unit.
#[derive(Debug, Clone)]
pub struct EncodedFrame {
    /// H.264 Annex B.
    pub data: Vec<u8>,
    /// Contains an IDR picture.
    pub keyframe: bool,
    /// Keyframe that also carries SPS and PPS, so a decoder can start here.
    pub self_contained: bool,
    /// The `capture_us` given to `submit` for the frame this came from.
    pub capture_us: u64,
    /// Time from `capture_us` until the encoder produced this output.
    pub encode_us: u32,
}

pub type FrameSink = Box<dyn FnMut(EncodedFrame) + Send + 'static>;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submit {
    /// The frame was handed to the encoder.
    Accepted,
    /// The encoder had no capacity; the frame was dropped.
    Busy,
}

/// How pixel data reaches the encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FramePath {
    /// Capture texture → GPU conversion → encoder, all in video memory.
    GpuTexture,
    /// GPU conversion, then one read-back to system memory per frame.
    SystemMemory,
}

impl FramePath {
    pub fn describe(self) -> &'static str {
        match self {
            FramePath::GpuTexture => "GPU textures (no CPU pixel copies)",
            FramePath::SystemMemory => "system memory (one GPU read-back per frame)",
        }
    }
}

fn win(context: &'static str) -> impl FnOnce(windows::core::Error) -> EncoderError {
    move |source| EncoderError::Windows { context, source }
}

/// Prepare the process for encoding: pins the COM multithreaded apartment
/// and starts Media Foundation. Idempotent.
pub fn init() -> Result<(), EncoderError> {
    use std::sync::OnceLock;
    use windows::Win32::System::Com::CoIncrementMTAUsage;
    static INIT: OnceLock<Result<(), i32>> = OnceLock::new();
    let r = INIT.get_or_init(|| {
        // SAFETY: process-wide initialisation with no preconditions. Neither
        // is undone: both must outlive every encoder.
        unsafe {
            let _ = CoIncrementMTAUsage();
            MFStartup(MF_VERSION, MFSTARTUP_FULL).map_err(|e| e.code().0)
        }
    });
    r.map_err(|code| EncoderError::Windows {
        context: "MFStartup",
        source: windows::core::Error::from_hresult(windows::core::HRESULT(code)),
    })
}

/// Source of input samples for the transform.
enum SampleSource {
    /// Pooled NV12 textures on the encoder's device.
    Textures {
        allocator: IMFVideoSampleAllocatorEx,
    },
    /// One render target plus a staging copy for read-back.
    Memory {
        target: ID3D11Texture2D,
        staging: ID3D11Texture2D,
    },
}

pub struct VideoEncoder {
    context: ID3D11DeviceContext,
    device: ID3D11Device,
    converter: FrameConverter,
    source: SampleSource,
    mft: MftEncoder,
    /// Kept so a keyframe can be produced while the screen is static. While
    /// held, the pool cannot hand this sample's texture out again.
    last_sample: Option<IMFSample>,
    /// Private BGRA copy used when the capture texture cannot be read by the
    /// video processor directly.
    copy_texture: Option<ID3D11Texture2D>,
    direct_input: Option<bool>,
    src: (u32, u32),
    dst: (u32, u32),
    frame_duration_hns: i64,
    submitted: u64,
    busy: u64,
    _manager: IMFDXGIDeviceManager,
}

/// Names of the encoders that would be considered for frames on the adapter
/// with `adapter_luid`, in selection order, with whether each is hardware.
pub fn list_encoders(adapter_luid: i64) -> Result<Vec<(String, bool)>, EncoderError> {
    init()?;
    Ok(mft::hardware_candidates(adapter_luid)
        .into_iter()
        .chain(mft::software_candidates())
        .map(|c| (c.name, c.hardware))
        .collect())
}

impl VideoEncoder {
    /// Create an encoder for frames of size `src` on `device`.
    pub fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        adapter_luid: i64,
        src: (u32, u32),
        cfg: EncoderConfig,
        sink: FrameSink,
    ) -> Result<Self, EncoderError> {
        if cfg.width < 64 || cfg.height < 64 || cfg.width % 2 != 0 || cfg.height % 2 != 0 {
            return Err(EncoderError::Config("size must be even and at least 64x64"));
        }
        if cfg.fps == 0 || cfg.fps > 240 {
            return Err(EncoderError::Config("fps must be 1..=240"));
        }
        if cfg.bitrate_bps < 100_000 {
            return Err(EncoderError::Config("bitrate must be at least 100 kbps"));
        }
        init()?;

        // The device manager lets Media Foundation share our D3D11 device.
        let mut token = 0u32;
        let mut manager = None;
        // SAFETY: out-pointers reference live locals.
        unsafe { MFCreateDXGIDeviceManager(&mut token, &mut manager) }
            .map_err(win("MFCreateDXGIDeviceManager"))?;
        let manager = manager.ok_or(EncoderError::Unsupported("DXGI device manager"))?;
        // SAFETY: `device` is a live D3D11 device.
        unsafe { manager.ResetDevice(device, token) }.map_err(win("ResetDevice"))?;

        let mut candidates = Vec::new();
        if cfg.preference != EncoderPreference::SoftwareOnly {
            candidates.extend(mft::hardware_candidates(adapter_luid));
        }
        if cfg.preference != EncoderPreference::HardwareOnly {
            candidates.extend(mft::software_candidates());
        }

        let mut attempts = Vec::new();
        let sink = std::sync::Arc::new(std::sync::Mutex::new(sink));
        let mut chosen = None;
        for candidate in &candidates {
            match MftEncoder::open(candidate, &cfg, Some(&manager), sink.clone()) {
                Ok(enc) => {
                    // A hardware encoder that will not take textures would
                    // need a read-back per frame; the software encoder is
                    // the honest fallback for that, so move on.
                    if enc.hardware && !enc.d3d_aware {
                        attempts.push(format!("{}: does not accept GPU textures", enc.name));
                        continue;
                    }
                    chosen = Some(enc);
                    break;
                }
                Err(e) => {
                    tracing::warn!(encoder = %candidate.name, error = %e, "encoder unavailable");
                    attempts.push(format!("{}: {e}", candidate.name));
                }
            }
        }
        let Some(mft) = chosen else {
            if attempts.is_empty() {
                attempts.push("no H.264 encoder is installed".into());
            }
            return Err(EncoderError::NoEncoder(attempts.join("; ")));
        };

        let dst = (cfg.width, cfg.height);
        let source = if mft.d3d_aware {
            SampleSource::Textures {
                allocator: create_allocator(&manager, &cfg)?,
            }
        } else {
            let (target, staging) = create_readback_textures(device, dst)?;
            SampleSource::Memory { target, staging }
        };
        let converter = FrameConverter::new(device, context, src, dst, cfg.fps)?;

        tracing::info!(
            encoder = %mft.name,
            hardware = mft.hardware,
            path = if mft.d3d_aware { "gpu-texture" } else { "system-memory" },
            src_width = src.0,
            src_height = src.1,
            width = dst.0,
            height = dst.1,
            fps = cfg.fps,
            bitrate_kbps = cfg.bitrate_bps / 1000,
            "video encoder ready"
        );

        Ok(VideoEncoder {
            context: context.clone(),
            device: device.clone(),
            converter,
            source,
            mft,
            last_sample: None,
            copy_texture: None,
            direct_input: None,
            src,
            dst,
            frame_duration_hns: 10_000_000 / cfg.fps as i64,
            submitted: 0,
            busy: 0,
            _manager: manager,
        })
    }

    pub fn name(&self) -> &str {
        &self.mft.name
    }

    pub fn is_hardware(&self) -> bool {
        self.mft.hardware
    }

    pub fn path(&self) -> FramePath {
        match self.source {
            SampleSource::Textures { .. } => FramePath::GpuTexture,
            SampleSource::Memory { .. } => FramePath::SystemMemory,
        }
    }

    /// Coded size.
    pub fn size(&self) -> (u32, u32) {
        self.dst
    }

    /// `(frames submitted, frames dropped because the encoder was busy,
    /// frames produced, bytes produced)`.
    pub fn counters(&self) -> (u64, u64, u64, u64) {
        let (out, bytes) = self.mft.counters();
        (self.submitted, self.busy, out, bytes)
    }

    /// Whether the encoder reported an unrecoverable error.
    pub fn failed(&self) -> bool {
        self.mft.failed()
    }

    /// Change the target bitrate without restarting the encoder. Returns
    /// `false` if this encoder cannot change bitrate while running.
    pub fn set_bitrate(&mut self, bps: u32) -> bool {
        self.mft.set_bitrate(bps.max(100_000))
    }

    /// Encode `src` (BGRA, on this encoder's device).
    pub fn submit(
        &mut self,
        src: &ID3D11Texture2D,
        capture_us: u64,
        force_keyframe: bool,
    ) -> Result<Submit, EncoderError> {
        if self.mft.failed() {
            return Err(EncoderError::Failed);
        }
        // Check capacity before spending GPU time on conversion.
        if !self.mft.ready() {
            self.busy += 1;
            return Ok(Submit::Busy);
        }
        let Some(sample) = self.make_sample(src)? else {
            self.busy += 1;
            return Ok(Submit::Busy);
        };
        self.feed(sample, capture_us, force_keyframe)
    }

    /// Encode the most recently submitted picture again, as a keyframe.
    /// Used when a client needs a keyframe while the desktop is static.
    /// Returns `Busy` if there is no previous picture or no capacity.
    pub fn resubmit_last(&mut self, capture_us: u64) -> Result<Submit, EncoderError> {
        if self.mft.failed() {
            return Err(EncoderError::Failed);
        }
        let Some(sample) = self.last_sample.clone() else {
            return Ok(Submit::Busy);
        };
        if !self.mft.ready() {
            return Ok(Submit::Busy);
        }
        self.feed(sample, capture_us, true)
    }

    fn feed(
        &mut self,
        sample: IMFSample,
        capture_us: u64,
        force_keyframe: bool,
    ) -> Result<Submit, EncoderError> {
        // SAFETY: timestamp setters on a live sample.
        unsafe {
            sample
                .SetSampleTime(capture_us as i64 * 10)
                .map_err(win("SetSampleTime"))?;
            sample
                .SetSampleDuration(self.frame_duration_hns)
                .map_err(win("SetSampleDuration"))?;
        }
        if self.mft.input(&sample, force_keyframe)? {
            self.submitted += 1;
            self.last_sample = Some(sample);
            Ok(Submit::Accepted)
        } else {
            self.busy += 1;
            Ok(Submit::Busy)
        }
    }

    /// Pick the texture the converter reads: the capture texture itself when
    /// the video processor accepts it, otherwise a private GPU copy.
    fn converter_input(&mut self, src: &ID3D11Texture2D) -> Result<ID3D11Texture2D, EncoderError> {
        let direct = match self.direct_input {
            Some(d) => d,
            None => {
                let d = self.converter.accepts(src);
                if !d {
                    tracing::info!(
                        "capture texture is not a valid video-processor input; using a GPU copy"
                    );
                }
                self.direct_input = Some(d);
                d
            }
        };
        if direct {
            return Ok(src.clone());
        }
        if self.copy_texture.is_none() {
            let desc = D3D11_TEXTURE2D_DESC {
                Width: self.src.0,
                Height: self.src.1,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                ..Default::default()
            };
            let mut tex = None;
            // SAFETY: `desc` is initialised; `tex` is a live out-slot.
            unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut tex)) }
                .map_err(win("create copy texture"))?;
            self.copy_texture = tex;
        }
        let copy = self.copy_texture.clone().expect("created above");
        // SAFETY: both textures are on this device with identical size and
        // format.
        unsafe { self.context.CopyResource(&copy, src) };
        Ok(copy)
    }

    /// Convert `src` into a fresh input sample. `None` means the pool is
    /// exhausted (the encoder still holds every texture).
    fn make_sample(&mut self, src: &ID3D11Texture2D) -> Result<Option<IMFSample>, EncoderError> {
        let input = self.converter_input(src)?;
        match &self.source {
            SampleSource::Textures { allocator } => {
                // SAFETY: allocator is initialised; errors are handled.
                let sample = match unsafe { allocator.AllocateSample() } {
                    Ok(s) => s,
                    Err(e) if e.code() == MF_E_SAMPLEALLOCATOR_EMPTY => return Ok(None),
                    Err(e) => return Err(win("AllocateSample")(e)),
                };
                // SAFETY: pooled samples carry exactly one DXGI buffer.
                let (texture, subresource) = unsafe {
                    let buffer = sample
                        .GetBufferByIndex(0)
                        .map_err(win("GetBufferByIndex"))?;
                    let dxgi: IMFDXGIBuffer = buffer.cast().map_err(win("IMFDXGIBuffer"))?;
                    let mut raw = std::ptr::null_mut();
                    dxgi.GetResource(&ID3D11Texture2D::IID, &mut raw)
                        .map_err(win("GetResource"))?;
                    let texture = ID3D11Texture2D::from_raw(raw);
                    let sub = dxgi
                        .GetSubresourceIndex()
                        .map_err(win("GetSubresourceIndex"))?;
                    (texture, sub)
                };
                self.converter.convert(&input, &texture, subresource)?;
                Ok(Some(sample))
            }
            SampleSource::Memory { target, staging } => {
                let (target, staging) = (target.clone(), staging.clone());
                self.converter.convert(&input, &target, 0)?;
                Ok(Some(self.read_back(&target, &staging)?))
            }
        }
    }

    /// Copy the converted NV12 picture into a system-memory sample.
    fn read_back(
        &mut self,
        target: &ID3D11Texture2D,
        staging: &ID3D11Texture2D,
    ) -> Result<IMFSample, EncoderError> {
        let (w, h) = (self.dst.0 as usize, self.dst.1 as usize);
        let size = w * h * 3 / 2;
        // SAFETY: `Map` blocks until the copy is complete and gives us a
        // pointer valid until `Unmap`; row copies stay inside both buffers
        // (staging rows are `RowPitch >= w` bytes, the NV12 UV plane follows
        // the Y plane at `RowPitch * h`).
        unsafe {
            self.context.CopyResource(staging, target);
            let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
            self.context
                .Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
                .map_err(win("Map staging texture"))?;
            let result = (|| {
                let buffer =
                    MFCreateMemoryBuffer(size as u32).map_err(win("MFCreateMemoryBuffer"))?;
                let mut dst = std::ptr::null_mut();
                buffer.Lock(&mut dst, None, None).map_err(win("Lock"))?;
                let pitch = mapped.RowPitch as usize;
                let src = mapped.pData as *const u8;
                for row in 0..h {
                    std::ptr::copy_nonoverlapping(src.add(row * pitch), dst.add(row * w), w);
                }
                for row in 0..h / 2 {
                    std::ptr::copy_nonoverlapping(
                        src.add((h + row) * pitch),
                        dst.add(w * h + row * w),
                        w,
                    );
                }
                let _ = buffer.Unlock();
                buffer
                    .SetCurrentLength(size as u32)
                    .map_err(win("SetCurrentLength"))?;
                let sample = MFCreateSample().map_err(win("MFCreateSample"))?;
                sample.AddBuffer(&buffer).map_err(win("AddBuffer"))?;
                Ok(sample)
            })();
            self.context.Unmap(staging, 0);
            result
        }
    }
}

fn create_allocator(
    manager: &IMFDXGIDeviceManager,
    cfg: &EncoderConfig,
) -> Result<IMFVideoSampleAllocatorEx, EncoderError> {
    // SAFETY: standard allocator setup; out-pointer references a live local
    // and is converted to an owned interface exactly once.
    unsafe {
        let mut raw = std::ptr::null_mut();
        MFCreateVideoSampleAllocatorEx(&IMFVideoSampleAllocatorEx::IID, &mut raw)
            .map_err(win("MFCreateVideoSampleAllocatorEx"))?;
        let allocator = IMFVideoSampleAllocatorEx::from_raw(raw);
        allocator
            .SetDirectXManager(manager)
            .map_err(win("allocator SetDirectXManager"))?;
        let mut attrs = None;
        MFCreateAttributes(&mut attrs, 2).map_err(win("MFCreateAttributes"))?;
        let attrs = attrs.ok_or(EncoderError::Unsupported("attribute store"))?;
        // The video processor renders into these textures.
        attrs
            .SetUINT32(
                &MF_SA_D3D11_BINDFLAGS,
                (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
            )
            .map_err(win("allocator bind flags"))?;
        attrs
            .SetUINT32(&MF_SA_D3D11_USAGE, D3D11_USAGE_DEFAULT.0 as u32)
            .map_err(win("allocator usage"))?;
        let ty = mft::video_type(MFVideoFormat_NV12, cfg.width, cfg.height, cfg.fps)?;
        // Small, fixed pool: when the encoder holds every texture the caller
        // drops frames instead of allocating more.
        allocator
            .InitializeSampleAllocatorEx(2, 6, &attrs, &ty)
            .map_err(win("InitializeSampleAllocatorEx"))?;
        Ok(allocator)
    }
}

fn create_readback_textures(
    device: &ID3D11Device,
    size: (u32, u32),
) -> Result<(ID3D11Texture2D, ID3D11Texture2D), EncoderError> {
    let base = D3D11_TEXTURE2D_DESC {
        Width: size.0,
        Height: size.1,
        MipLevels: 1,
        ArraySize: 1,
        Format: DXGI_FORMAT_NV12,
        SampleDesc: DXGI_SAMPLE_DESC {
            Count: 1,
            Quality: 0,
        },
        ..Default::default()
    };
    let target_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        ..base
    };
    let staging_desc = D3D11_TEXTURE2D_DESC {
        Usage: D3D11_USAGE_STAGING,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        ..base
    };
    let (mut target, mut staging) = (None, None);
    // SAFETY: descriptors are initialised; out-slots are live locals.
    unsafe {
        device
            .CreateTexture2D(&target_desc, None, Some(&mut target))
            .map_err(win("create NV12 target"))?;
        device
            .CreateTexture2D(&staging_desc, None, Some(&mut staging))
            .map_err(win("create NV12 staging"))?;
    }
    Ok((
        target.ok_or(EncoderError::Unsupported("NV12 render target"))?,
        staging.ok_or(EncoderError::Unsupported("NV12 staging texture"))?,
    ))
}

/// Fit `src` inside `max` (0 = unlimited) preserving aspect ratio, never
/// upscaling, with both dimensions rounded down to even and at least 64.
pub fn fit_size(src: (u32, u32), max: (u32, u32)) -> (u32, u32) {
    let (sw, sh) = (src.0.max(1) as u64, src.1.max(1) as u64);
    let mw = if max.0 == 0 { sw } else { max.0 as u64 };
    let mh = if max.1 == 0 { sh } else { max.1 as u64 };
    // Scale = min(mw/sw, mh/sh, 1), compared with cross-multiplication.
    let (num, den) = if mw * sh <= mh * sw {
        (mw, sw)
    } else {
        (mh, sh)
    };
    let (num, den) = if num >= den { (1, 1) } else { (num, den) };
    let even = |v: u64| ((v & !1).max(64)) as u32;
    (even(sw * num / den), even(sh * num / den))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_size_preserves_aspect_and_parity() {
        assert_eq!(fit_size((2560, 1600), (0, 0)), (2560, 1600));
        assert_eq!(fit_size((2560, 1600), (1920, 1200)), (1920, 1200));
        assert_eq!(fit_size((2560, 1600), (1920, 1080)), (1728, 1080));
        assert_eq!(fit_size((1920, 1080), (1280, 0)), (1280, 720));
        // Never upscales.
        assert_eq!(fit_size((1280, 720), (3840, 2160)), (1280, 720));
        // Odd sources round down to even.
        assert_eq!(fit_size((1367, 769), (0, 0)), (1366, 768));
        // Portrait displays.
        assert_eq!(fit_size((1080, 1920), (1280, 1280)), (720, 1280));
        // Degenerate inputs stay valid.
        assert_eq!(fit_size((0, 0), (0, 0)), (64, 64));
        assert_eq!(fit_size((10_000, 10), (100, 100)), (100, 64));
    }
}
