//! H.264 encoding through a Media Foundation Transform (MFT).
//!
//! Hardware encoders (Intel Quick Sync, NVIDIA, AMD) are *asynchronous*
//! MFTs: they raise `NeedInput` / `HaveOutput` events instead of being
//! polled. A dedicated thread blocks on the event queue; `NeedInput` grants
//! one input credit, `HaveOutput` pulls one encoded access unit and hands it
//! to the sink. Nothing is queued on our side: with no credit available the
//! caller is told the encoder is busy and drops the frame.
//!
//! The software encoder is a *synchronous* MFT and is driven inline.
//!
//! Low-delay configuration: no B-frames, low-latency mode, CBR with a short
//! buffer, and an effectively infinite GOP — keyframes are produced on
//! request only, because the transport is reliable and periodic keyframes
//! would only cause bitrate spikes.

use crate::annexb;
use crate::{EncodedFrame, EncoderConfig, EncoderError, FrameSink, Tuning};
use std::mem::ManuallyDrop;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use windows::core::{Interface, GUID, PWSTR};
use windows::Win32::Foundation::LUID;
use windows::Win32::Foundation::VARIANT_BOOL;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::Variant::{VARIANT, VT_BOOL, VT_UI4};

fn win(context: &'static str) -> impl FnOnce(windows::core::Error) -> EncoderError {
    move |source| EncoderError::Windows { context, source }
}

/// Wrapper that lets a COM interface move to the encoder's event thread.
///
/// SAFETY: Media Foundation transforms and their event generators live in
/// the multithreaded apartment and are free-threaded: asynchronous MFTs are
/// documented to be called from arbitrary work-queue threads. The process
/// pins the MTA (see `crate::init`), so the interface stays valid on any
/// thread. We never touch an STA-bound object through this wrapper.
struct SendCom<T>(T);
unsafe impl<T> Send for SendCom<T> {}

/// A selectable encoder.
pub(crate) struct Candidate {
    pub name: String,
    pub hardware: bool,
    activate: IMFActivate,
}

fn activate_name(activate: &IMFActivate) -> String {
    let mut pwstr = PWSTR::null();
    let mut len = 0u32;
    // SAFETY: on success `pwstr` points at a CoTaskMem string of `len`
    // UTF-16 units, which we copy and free.
    unsafe {
        if activate
            .GetAllocatedString(&MFT_FRIENDLY_NAME_Attribute, &mut pwstr, &mut len)
            .is_err()
            || pwstr.is_null()
        {
            return "unknown encoder".into();
        }
        let s = String::from_utf16_lossy(std::slice::from_raw_parts(pwstr.0, len as usize));
        CoTaskMemFree(Some(pwstr.0 as *const _));
        s
    }
}

fn take_activates(ptr: *mut Option<IMFActivate>, count: u32) -> Vec<IMFActivate> {
    if ptr.is_null() {
        return Vec::new();
    }
    // SAFETY: MFTEnum* returned `count` initialised interface pointers in a
    // CoTaskMem array. We move each out exactly once, then free the array.
    unsafe {
        let slice = std::slice::from_raw_parts_mut(ptr, count as usize);
        let out = slice.iter_mut().filter_map(Option::take).collect();
        CoTaskMemFree(Some(ptr as *const _));
        out
    }
}

/// Hardware H.264 encoders that run on the adapter with `adapter_luid`.
pub(crate) fn hardware_candidates(adapter_luid: i64) -> Vec<Candidate> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let luid = LUID {
        LowPart: adapter_luid as u32,
        HighPart: (adapter_luid >> 32) as i32,
    };
    let mut ptr = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: all pointers reference live locals; the LUID blob is copied by
    // the attribute store before the call returns.
    let ok = unsafe {
        let mut attrs = None;
        if MFCreateAttributes(&mut attrs, 1).is_err() {
            return Vec::new();
        }
        let Some(attrs) = attrs else {
            return Vec::new();
        };
        let bytes = std::slice::from_raw_parts(
            (&luid as *const LUID).cast::<u8>(),
            std::mem::size_of::<LUID>(),
        );
        if attrs.SetBlob(&MFT_ENUM_ADAPTER_LUID, bytes).is_err() {
            return Vec::new();
        }
        MFTEnum2(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &attrs,
            &mut ptr,
            &mut count,
        )
        .is_ok()
    };
    if !ok {
        return Vec::new();
    }
    take_activates(ptr, count)
        .into_iter()
        .map(|activate| Candidate {
            name: activate_name(&activate),
            hardware: true,
            activate,
        })
        .collect()
}

/// Software H.264 encoders (CPU). Fallback only.
pub(crate) fn software_candidates() -> Vec<Candidate> {
    let input = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_NV12,
    };
    let output = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MFMediaType_Video,
        guidSubtype: MFVideoFormat_H264,
    };
    let mut ptr = std::ptr::null_mut();
    let mut count = 0u32;
    // SAFETY: pointers reference live locals.
    let ok = unsafe {
        MFTEnumEx(
            MFT_CATEGORY_VIDEO_ENCODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            Some(&input),
            Some(&output),
            &mut ptr,
            &mut count,
        )
        .is_ok()
    };
    if !ok {
        return Vec::new();
    }
    take_activates(ptr, count)
        .into_iter()
        .map(|activate| Candidate {
            name: activate_name(&activate),
            hardware: false,
            activate,
        })
        .collect()
}

pub(crate) fn video_type(
    subtype: GUID,
    width: u32,
    height: u32,
    fps: u32,
) -> Result<IMFMediaType, EncoderError> {
    // SAFETY: attribute setters on a freshly created media type.
    unsafe {
        let t = MFCreateMediaType().map_err(win("MFCreateMediaType"))?;
        let set = |r: windows::core::Result<()>| r.map_err(win("media type attribute"));
        set(t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video))?;
        set(t.SetGUID(&MF_MT_SUBTYPE, &subtype))?;
        set(t.SetUINT64(&MF_MT_FRAME_SIZE, ((width as u64) << 32) | height as u64))?;
        set(t.SetUINT64(&MF_MT_FRAME_RATE, ((fps as u64) << 32) | 1))?;
        set(t.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, (1u64 << 32) | 1))?;
        set(t.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32))?;
        // Signal the colour space the converter produces: BT.709, limited
        // range. Encoders that write VUI carry this to the decoder.
        set(t.SetUINT32(&MF_MT_VIDEO_PRIMARIES, MFVideoPrimaries_BT709.0 as u32))?;
        set(t.SetUINT32(&MF_MT_TRANSFER_FUNCTION, MFVideoTransFunc_709.0 as u32))?;
        set(t.SetUINT32(&MF_MT_YUV_MATRIX, MFVideoTransferMatrix_BT709.0 as u32))?;
        set(t.SetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE, MFNominalRange_16_235.0 as u32))?;
        Ok(t)
    }
}

struct Shared {
    sink: Arc<Mutex<FrameSink>>,
    need_input: AtomicI32,
    failed: AtomicBool,
    frames_out: AtomicU64,
    bytes_out: AtomicU64,
    /// SPS/PPS from the output media type, for encoders that do not repeat
    /// them in-band on every keyframe.
    sequence_header: Mutex<Vec<u8>>,
    /// Parameter-set-only output waiting to be prepended to the next frame.
    pending_prefix: Mutex<Vec<u8>>,
    clock: fn() -> u64,
    out_buffer_size: u32,
    provides_samples: bool,
}

pub(crate) struct MftEncoder {
    transform: IMFTransform,
    codec_api: Option<ICodecAPI>,
    is_async: bool,
    shared: Arc<Shared>,
    worker: Option<JoinHandle<()>>,
    pub(crate) name: String,
    pub(crate) hardware: bool,
    /// Whether the transform takes D3D11 texture samples.
    pub(crate) d3d_aware: bool,
}

/// Take one input credit if any is available.
fn claim_credit(credits: &AtomicI32) -> bool {
    let mut current = credits.load(Ordering::Acquire);
    while current > 0 {
        match credits.compare_exchange_weak(
            current,
            current - 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(seen) => current = seen,
        }
    }
    false
}

fn variant_u32(v: u32) -> VARIANT {
    let mut var = VARIANT::default();
    // SAFETY: a default VARIANT is VT_EMPTY; writing the tag and the
    // matching union member yields a valid VT_UI4 value.
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_UI4;
        inner.Anonymous.ulVal = v;
    }
    var
}

fn variant_bool(v: bool) -> VARIANT {
    let mut var = VARIANT::default();
    // SAFETY: as above, for VT_BOOL (VARIANT_TRUE is -1).
    unsafe {
        let inner = &mut *var.Anonymous.Anonymous;
        inner.vt = VT_BOOL;
        inner.Anonymous.boolVal = VARIANT_BOOL(if v { -1 } else { 0 });
    }
    var
}

fn set_codec(api: &ICodecAPI, key: &GUID, value: VARIANT, what: &str) -> bool {
    // SAFETY: `key` and `value` are live for the duration of the call.
    let r = unsafe { api.SetValue(key, &value) };
    if let Err(e) = &r {
        tracing::debug!(setting = what, error = %e, "encoder rejected optional setting");
    }
    r.is_ok()
}

fn read_sequence_header(transform: &IMFTransform) -> Vec<u8> {
    // SAFETY: blob size is queried first and the buffer sized to match.
    unsafe {
        let Ok(t) = transform.GetOutputCurrentType(0) else {
            return Vec::new();
        };
        let Ok(size) = t.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) else {
            return Vec::new();
        };
        if size == 0 || size > 4096 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        if t.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut buf, None)
            .is_err()
        {
            return Vec::new();
        }
        buf
    }
}

impl MftEncoder {
    /// Activate and configure `candidate`. `manager` is passed to transforms
    /// that can take GPU textures.
    pub(crate) fn open(
        candidate: &Candidate,
        cfg: &EncoderConfig,
        manager: Option<&IMFDXGIDeviceManager>,
        sink: Arc<Mutex<FrameSink>>,
    ) -> Result<Self, EncoderError> {
        match Self::configure(candidate, cfg, manager) {
            Ok((transform, codec_api, is_async, d3d_aware, provides_samples, out_buffer_size)) => {
                let shared = Arc::new(Shared {
                    sink,
                    need_input: AtomicI32::new(0),
                    failed: AtomicBool::new(false),
                    frames_out: AtomicU64::new(0),
                    bytes_out: AtomicU64::new(0),
                    sequence_header: Mutex::new(read_sequence_header(&transform)),
                    pending_prefix: Mutex::new(Vec::new()),
                    clock: cfg.clock,
                    out_buffer_size,
                    provides_samples,
                });
                let mut enc = MftEncoder {
                    transform,
                    codec_api,
                    is_async,
                    shared,
                    worker: None,
                    name: candidate.name.clone(),
                    hardware: candidate.hardware,
                    d3d_aware,
                };
                enc.start()?;
                Ok(enc)
            }
            Err(e) => {
                // SAFETY: releases whatever the activation object created.
                unsafe {
                    let _ = candidate.activate.ShutdownObject();
                }
                Err(e)
            }
        }
    }

    #[allow(clippy::type_complexity)]
    fn configure(
        candidate: &Candidate,
        cfg: &EncoderConfig,
        manager: Option<&IMFDXGIDeviceManager>,
    ) -> Result<(IMFTransform, Option<ICodecAPI>, bool, bool, bool, u32), EncoderError> {
        // SAFETY: Media Foundation configuration calls on live interfaces;
        // every pointer argument references a live local.
        unsafe {
            let transform: IMFTransform = candidate
                .activate
                .ActivateObject()
                .map_err(win("ActivateObject"))?;

            let attrs = transform.GetAttributes().ok();
            let is_async = attrs
                .as_ref()
                .and_then(|a| a.GetUINT32(&MF_TRANSFORM_ASYNC).ok())
                .unwrap_or(0)
                != 0;
            let d3d_capable = attrs
                .as_ref()
                .and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE).ok())
                .unwrap_or(0)
                != 0;
            if let Some(a) = &attrs {
                if is_async {
                    a.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1)
                        .map_err(win("unlock async transform"))?;
                }
                let _ = a.SetUINT32(&MF_LOW_LATENCY, 1);
            }

            let mut d3d_aware = false;
            if let (true, Some(manager)) = (d3d_capable, manager) {
                transform
                    .ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, manager.as_raw() as usize)
                    .map_err(win("set D3D manager"))?;
                d3d_aware = true;
            }

            // Encoder properties must be set before the media types.
            let codec_api = transform.cast::<ICodecAPI>().ok();
            if let Some(api) = &codec_api {
                set_codec(
                    api,
                    &CODECAPI_AVEncCommonRateControlMode,
                    variant_u32(eAVEncCommonRateControlMode_CBR.0 as u32),
                    "rate control",
                );
                set_codec(
                    api,
                    &CODECAPI_AVEncCommonMeanBitRate,
                    variant_u32(cfg.bitrate_bps),
                    "bitrate",
                );
                // A short rate-control buffer keeps individual frames from
                // ballooning, which is what makes frame delivery time even.
                set_codec(
                    api,
                    &CODECAPI_AVEncCommonBufferSize,
                    variant_u32(cfg.bitrate_bps / 4),
                    "buffer size",
                );
                set_codec(
                    api,
                    &CODECAPI_AVLowLatencyMode,
                    variant_bool(true),
                    "low latency",
                );
                set_codec(
                    api,
                    &CODECAPI_AVEncMPVDefaultBPictureCount,
                    variant_u32(0u32),
                    "B-frame count",
                );
                set_codec(
                    api,
                    &CODECAPI_AVEncMPVGOPSize,
                    variant_u32(cfg.fps.saturating_mul(600)),
                    "GOP size",
                );
                set_codec(
                    api,
                    &CODECAPI_AVEncCommonQualityVsSpeed,
                    variant_u32(match cfg.tuning {
                        Tuning::Speed => 0u32,
                        Tuning::Balanced => 33,
                        Tuning::Quality => 66,
                    }),
                    "quality vs speed",
                );
                set_codec(
                    api,
                    &CODECAPI_AVEncH264CABACEnable,
                    variant_bool(true),
                    "CABAC",
                );
            }

            let out_type = video_type(MFVideoFormat_H264, cfg.width, cfg.height, cfg.fps)?;
            out_type
                .SetUINT32(&MF_MT_AVG_BITRATE, cfg.bitrate_bps)
                .map_err(win("bitrate"))?;
            out_type
                .SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                .map_err(win("profile"))?;
            if transform.SetOutputType(0, &out_type, 0).is_err() {
                // Some encoders only offer Main.
                out_type
                    .SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)
                    .map_err(win("profile"))?;
                transform
                    .SetOutputType(0, &out_type, 0)
                    .map_err(win("SetOutputType"))?;
            }

            let in_type = video_type(MFVideoFormat_NV12, cfg.width, cfg.height, cfg.fps)?;
            transform
                .SetInputType(0, &in_type, 0)
                .map_err(win("SetInputType"))?;

            let info = transform
                .GetOutputStreamInfo(0)
                .map_err(win("GetOutputStreamInfo"))?;
            let provides_samples = info.dwFlags
                & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0)
                    as u32
                != 0;
            let out_buffer_size = info.cbSize.max(cfg.width * cfg.height);

            Ok((
                transform,
                codec_api,
                is_async,
                d3d_aware,
                provides_samples,
                out_buffer_size,
            ))
        }
    }

    fn start(&mut self) -> Result<(), EncoderError> {
        let events = if self.is_async {
            Some(
                self.transform
                    .cast::<IMFMediaEventGenerator>()
                    .map_err(win("IMFMediaEventGenerator"))?,
            )
        } else {
            None
        };
        // SAFETY: streaming notifications on a configured transform.
        unsafe {
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)
                .map_err(win("begin streaming"))?;
            self.transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)
                .map_err(win("start of stream"))?;
        }
        if let Some(events) = events {
            let transform = SendCom(self.transform.clone());
            let events = SendCom(events);
            let shared = self.shared.clone();
            let handle = std::thread::Builder::new()
                .name("lanlink-encoder-events".into())
                .spawn(move || event_loop(transform, events, shared))
                .map_err(|_| EncoderError::Unsupported("could not start encoder thread"))?;
            self.worker = Some(handle);
        }
        Ok(())
    }

    /// Whether the encoder can take a frame right now.
    pub(crate) fn ready(&self) -> bool {
        !self.is_async || self.shared.need_input.load(Ordering::Acquire) > 0
    }

    pub(crate) fn failed(&self) -> bool {
        self.shared.failed.load(Ordering::Acquire)
    }

    pub(crate) fn counters(&self) -> (u64, u64) {
        (
            self.shared.frames_out.load(Ordering::Relaxed),
            self.shared.bytes_out.load(Ordering::Relaxed),
        )
    }

    /// Feed one frame. Returns `Ok(false)` if the encoder is busy and the
    /// frame was not taken.
    pub(crate) fn input(
        &mut self,
        sample: &IMFSample,
        force_keyframe: bool,
    ) -> Result<bool, EncoderError> {
        if self.failed() {
            return Err(EncoderError::Failed);
        }
        if self.is_async {
            // Claim one credit, or report busy.
            let claimed = claim_credit(&self.shared.need_input);
            if !claimed {
                return Ok(false);
            }
        }
        if force_keyframe {
            if let Some(api) = &self.codec_api {
                set_codec(
                    api,
                    &CODECAPI_AVEncVideoForceKeyFrame,
                    variant_u32(1u32),
                    "force keyframe",
                );
            }
        }
        // SAFETY: the sample is a live IMFSample matching the input type.
        match unsafe { self.transform.ProcessInput(0, sample, 0) } {
            Ok(()) => {}
            Err(e) if e.code() == MF_E_NOTACCEPTING => {
                if self.is_async {
                    // Return the credit we claimed.
                    self.shared.need_input.fetch_add(1, Ordering::AcqRel);
                }
                return Ok(false);
            }
            Err(e) => return Err(win("ProcessInput")(e)),
        }
        if !self.is_async {
            // Synchronous transform: drain everything it produced.
            while pull_output(&self.transform, &self.shared) {}
        }
        Ok(true)
    }

    pub(crate) fn set_bitrate(&mut self, bps: u32) -> bool {
        match &self.codec_api {
            Some(api) => {
                let ok = set_codec(
                    api,
                    &CODECAPI_AVEncCommonMeanBitRate,
                    variant_u32(bps),
                    "bitrate",
                );
                set_codec(
                    api,
                    &CODECAPI_AVEncCommonBufferSize,
                    variant_u32(bps / 4),
                    "buffer size",
                );
                ok
            }
            None => false,
        }
    }
}

impl Drop for MftEncoder {
    fn drop(&mut self) {
        // SAFETY: teardown calls on a live transform; failures are ignored
        // because the object is being discarded.
        unsafe {
            let _ = self
                .transform
                .ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0);
            // Shutting the transform down makes the event thread's blocking
            // GetEvent return MF_E_SHUTDOWN.
            match self.transform.cast::<IMFShutdown>() {
                Ok(s) => {
                    let _ = s.Shutdown();
                    if let Some(h) = self.worker.take() {
                        let _ = h.join();
                    }
                }
                Err(_) => {
                    if self.worker.take().is_some() {
                        tracing::warn!("encoder has no IMFShutdown; event thread detached");
                    }
                }
            }
        }
    }
}

fn event_loop(
    transform: SendCom<IMFTransform>,
    events: SendCom<IMFMediaEventGenerator>,
    shared: Arc<Shared>,
) {
    let (transform, events) = (transform.0, events.0);
    loop {
        // SAFETY: blocking wait on a live event generator.
        let event = match unsafe { events.GetEvent(MEDIA_EVENT_GENERATOR_GET_EVENT_FLAGS(0)) } {
            Ok(e) => e,
            Err(e) => {
                if e.code() != MF_E_SHUTDOWN {
                    tracing::error!(error = %e, "encoder event queue failed");
                    shared.failed.store(true, Ordering::Release);
                }
                return;
            }
        };
        // SAFETY: reading the type of a live event.
        let ty = unsafe { event.GetType() }.unwrap_or(0);
        if ty == METransformNeedInput.0 as u32 {
            shared.need_input.fetch_add(1, Ordering::AcqRel);
        } else if ty == METransformHaveOutput.0 as u32 {
            pull_output(&transform, &shared);
        } else if ty == MEError.0 as u32 {
            // SAFETY: reading the status of a live event.
            let status = unsafe { event.GetStatus() }.ok();
            tracing::error!(?status, "encoder reported an error event");
            shared.failed.store(true, Ordering::Release);
        }
    }
}

/// Pull one output sample and deliver it. Returns `true` if a sample was
/// produced (so a synchronous caller should try again).
fn pull_output(transform: &IMFTransform, shared: &Shared) -> bool {
    // SAFETY: the output buffer struct is fully initialised; samples taken
    // out of its ManuallyDrop fields are released exactly once below.
    unsafe {
        let own_sample = if shared.provides_samples {
            None
        } else {
            let Ok(sample) = MFCreateSample() else {
                return false;
            };
            let Ok(buffer) = MFCreateMemoryBuffer(shared.out_buffer_size) else {
                return false;
            };
            if sample.AddBuffer(&buffer).is_err() {
                return false;
            }
            Some(sample)
        };
        let mut out = MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(own_sample),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        };
        let mut status = 0u32;
        let result = transform.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status);
        let sample = ManuallyDrop::take(&mut out.pSample);
        drop(ManuallyDrop::take(&mut out.pEvents));

        match result {
            Ok(()) => match sample {
                Some(sample) => {
                    deliver(&sample, shared);
                    true
                }
                None => false,
            },
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => false,
            Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                // The encoder refined its output type; accept it and refresh
                // the stored parameter sets.
                let renegotiated = transform
                    .GetOutputAvailableType(0, 0)
                    .and_then(|t| transform.SetOutputType(0, &t, 0))
                    .is_ok();
                if renegotiated {
                    if let Ok(mut h) = shared.sequence_header.lock() {
                        *h = read_sequence_header(transform);
                    }
                    true
                } else {
                    shared.failed.store(true, Ordering::Release);
                    false
                }
            }
            Err(e) => {
                tracing::error!(error = %e, "ProcessOutput failed");
                shared.failed.store(true, Ordering::Release);
                false
            }
        }
    }
}

fn deliver(sample: &IMFSample, shared: &Shared) {
    // SAFETY: the buffer is locked for the duration of the copy and
    // unlocked on every path; `len` bytes at `ptr` are valid while locked.
    let (mut data, time_hns) = unsafe {
        let Ok(buffer) = sample.ConvertToContiguousBuffer() else {
            return;
        };
        let mut ptr = std::ptr::null_mut();
        let mut len = 0u32;
        if buffer.Lock(&mut ptr, None, Some(&mut len)).is_err() {
            return;
        }
        let data = std::slice::from_raw_parts(ptr, len as usize).to_vec();
        let _ = buffer.Unlock();
        (data, sample.GetSampleTime().unwrap_or(0))
    };
    if data.is_empty() {
        return;
    }

    let info = annexb::inspect(&data);
    if !info.has_picture() {
        // Parameter sets on their own: keep them for the next picture.
        if let Ok(mut p) = shared.pending_prefix.lock() {
            if p.len() + data.len() <= 8192 {
                p.extend_from_slice(&data);
            }
        }
        return;
    }
    if let Ok(mut p) = shared.pending_prefix.lock() {
        if !p.is_empty() {
            let mut joined = std::mem::take(&mut *p);
            joined.extend_from_slice(&data);
            data = joined;
        }
    }
    if info.has_idr {
        if let Ok(header) = shared.sequence_header.lock() {
            annexb::ensure_parameter_sets(&mut data, &header);
        }
    }

    let capture_us = (time_hns.max(0) / 10) as u64;
    let now = (shared.clock)();
    shared.frames_out.fetch_add(1, Ordering::Relaxed);
    shared
        .bytes_out
        .fetch_add(data.len() as u64, Ordering::Relaxed);
    let frame = EncodedFrame {
        keyframe: info.has_idr,
        self_contained: annexb::inspect(&data).is_self_contained_keyframe(),
        capture_us,
        encode_us: now.saturating_sub(capture_us).min(u32::MAX as u64) as u32,
        data,
    };
    if let Ok(mut sink) = shared.sink.lock() {
        (sink)(frame);
    }
}
