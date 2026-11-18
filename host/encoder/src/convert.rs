//! GPU colour conversion and scaling with the D3D11 video processor.
//!
//! Captured frames are BGRA at display resolution; H.264 encoders want NV12
//! at stream resolution. The video processor does both in a single GPU pass,
//! so pixels never leave video memory.
//!
//! Colour: input is treated as full-range RGB, output is BT.709 limited
//! range, which the stream then signals (see `mft.rs`). This is an SDR path;
//! on an HDR display the capture layer already hands us Windows' SDR
//! rendition.

use crate::EncoderError;
use std::collections::HashMap;
use std::mem::ManuallyDrop;
use windows::core::Interface;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::DXGI_RATIONAL;

/// Cached views are keyed by texture address. The cache holds a reference
/// to each texture, so an address cannot be reused while its entry exists.
const MAX_CACHED_VIEWS: usize = 12;

pub(crate) struct FrameConverter {
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    input_views: HashMap<usize, (ID3D11Texture2D, ID3D11VideoProcessorInputView)>,
    output_views: HashMap<usize, (ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
}

fn win(context: &'static str) -> impl FnOnce(windows::core::Error) -> EncoderError {
    move |source| EncoderError::Windows { context, source }
}

impl FrameConverter {
    pub(crate) fn new(
        device: &ID3D11Device,
        context: &ID3D11DeviceContext,
        src: (u32, u32),
        dst: (u32, u32),
        fps: u32,
    ) -> Result<Self, EncoderError> {
        let video_device: ID3D11VideoDevice = device.cast().map_err(win("ID3D11VideoDevice"))?;
        let video_context: ID3D11VideoContext =
            context.cast().map_err(win("ID3D11VideoContext"))?;
        let rate = DXGI_RATIONAL {
            Numerator: fps.max(1),
            Denominator: 1,
        };
        let desc = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
            InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            InputFrameRate: rate,
            InputWidth: src.0,
            InputHeight: src.1,
            OutputFrameRate: rate,
            OutputWidth: dst.0,
            OutputHeight: dst.1,
            Usage: D3D11_VIDEO_USAGE_PLAYBACK_NORMAL,
        };
        // SAFETY: `desc` is fully initialised; the interfaces are live.
        let (enumerator, processor) = unsafe {
            let enumerator = video_device
                .CreateVideoProcessorEnumerator(&desc)
                .map_err(win("CreateVideoProcessorEnumerator"))?;
            let processor = video_device
                .CreateVideoProcessor(&enumerator, 0)
                .map_err(win("CreateVideoProcessor"))?;

            video_context.VideoProcessorSetStreamFrameFormat(
                &processor,
                0,
                D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
            );
            // Driver "enhancements" (denoise, edge enhancement, auto
            // contrast) would alter desktop content and cost time.
            video_context.VideoProcessorSetStreamAutoProcessingMode(&processor, 0, false);

            // Bit layout of D3D11_VIDEO_PROCESSOR_COLOR_SPACE:
            //   bit 0 Usage, bit 1 RGB_Range (0 = full),
            //   bit 2 YCbCr_Matrix (1 = BT.709), bit 3 xvYCC,
            //   bits 4-5 Nominal_Range (1 = 16..235).
            let input_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE { _bitfield: 0 };
            let output_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                _bitfield: (1 << 2) | (1 << 4),
            };
            video_context.VideoProcessorSetStreamColorSpace(&processor, 0, &input_cs);
            video_context.VideoProcessorSetOutputColorSpace(&processor, &output_cs);
            (enumerator, processor)
        };
        Ok(FrameConverter {
            video_device,
            video_context,
            enumerator,
            processor,
            input_views: HashMap::new(),
            output_views: HashMap::new(),
        })
    }

    fn input_view(
        &mut self,
        texture: &ID3D11Texture2D,
    ) -> Result<ID3D11VideoProcessorInputView, EncoderError> {
        let key = texture.as_raw() as usize;
        if let Some((_, view)) = self.input_views.get(&key) {
            return Ok(view.clone());
        }
        if self.input_views.len() >= MAX_CACHED_VIEWS {
            self.input_views.clear();
        }
        let desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: 0,
                },
            },
        };
        let mut view = None;
        // SAFETY: `desc` is initialised and `view` is a live out-slot.
        unsafe {
            self.video_device.CreateVideoProcessorInputView(
                texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )
        }
        .map_err(win("CreateVideoProcessorInputView"))?;
        let view = view.ok_or(EncoderError::Unsupported("video processor input view"))?;
        self.input_views
            .insert(key, (texture.clone(), view.clone()));
        Ok(view)
    }

    fn output_view(
        &mut self,
        texture: &ID3D11Texture2D,
        subresource: u32,
    ) -> Result<ID3D11VideoProcessorOutputView, EncoderError> {
        let key = (texture.as_raw() as usize) ^ ((subresource as usize) << 1);
        if let Some((_, view)) = self.output_views.get(&key) {
            return Ok(view.clone());
        }
        if self.output_views.len() >= MAX_CACHED_VIEWS {
            self.output_views.clear();
        }
        let desc = if subresource == 0 {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            }
        } else {
            D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2DARRAY,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2DArray: D3D11_TEX2D_ARRAY_VPOV {
                        MipSlice: 0,
                        FirstArraySlice: subresource,
                        ArraySize: 1,
                    },
                },
            }
        };
        let mut view = None;
        // SAFETY: `desc` is initialised and `view` is a live out-slot.
        unsafe {
            self.video_device.CreateVideoProcessorOutputView(
                texture,
                &self.enumerator,
                &desc,
                Some(&mut view),
            )
        }
        .map_err(win("CreateVideoProcessorOutputView"))?;
        let view = view.ok_or(EncoderError::Unsupported("video processor output view"))?;
        self.output_views
            .insert(key, (texture.clone(), view.clone()));
        Ok(view)
    }

    /// Convert and scale `src` (BGRA) into `dst` (NV12).
    pub(crate) fn convert(
        &mut self,
        src: &ID3D11Texture2D,
        dst: &ID3D11Texture2D,
        dst_subresource: u32,
    ) -> Result<(), EncoderError> {
        let input = self.input_view(src)?;
        let output = self.output_view(dst, dst_subresource)?;
        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            pInputSurface: ManuallyDrop::new(Some(input)),
            ..Default::default()
        };
        // SAFETY: one fully initialised stream; views belong to this
        // processor's enumerator.
        let result = unsafe {
            self.video_context.VideoProcessorBlt(
                &self.processor,
                &output,
                0,
                std::slice::from_ref(&stream),
            )
        };
        // SAFETY: the field was initialised above and is not used again;
        // this releases the reference the stream struct holds on the view.
        unsafe { ManuallyDrop::drop(&mut stream.pInputSurface) };
        result.map_err(win("VideoProcessorBlt"))
    }

    /// Whether a source texture can be used directly as converter input.
    pub(crate) fn accepts(&mut self, texture: &ID3D11Texture2D) -> bool {
        self.input_view(texture).is_ok()
    }
}
