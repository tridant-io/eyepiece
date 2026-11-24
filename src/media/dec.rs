//! ffmpeg's decoders: in software, or the system's hardware decoder writing into surfaces on the renderer's own device,
//! so a frame is drawn where it was decoded and never copied through system memory (gate 1).

use super::color::{Matrix, Stored};
use super::ff::{err, Input};
use ffmpeg_sys_next as ffi;
use std::ffi::{c_void, CStr};
use std::ptr;

/// surfaces a hardware decoder is given beyond its own needs and the frames the caller keeps: the frames ready around
/// the one on screen, and the one being drawn.
const SPARE_SURFACES: usize = 8;

/// what a hardware decoder needs from the renderer: its device, and the one lock both sides take around device calls.
/// ffmpeg requires that lock to be recursive.
#[cfg(windows)]
#[derive(Clone, Copy)]
pub struct Hardware {
    /// the renderer's device: an ID3D11Device on windows. borrowed; the decoder takes its own reference.
    pub device: *mut c_void,
    pub lock: unsafe extern "C" fn(*mut c_void),
    pub unlock: unsafe extern "C" fn(*mut c_void),
    pub lock_ctx: *mut c_void,
}
// the device is free-threaded behind the lock it comes with
#[cfg(windows)]
unsafe impl Send for Hardware {}

/// what a hardware decoder needs from the renderer. off windows, nothing: videotoolbox decodes into io surfaces,
/// which any metal device reads.
#[cfg(not(windows))]
#[derive(Clone, Copy)]
pub struct Hardware;

/// how a decoded picture's samples are laid out, in the terms the renderer and the color math need.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// a decoder surface on the gpu (luma plane and an interleaved chroma plane), or planes in ordinary memory
    pub hardware: bool,
    pub stored: Stored,
    /// how many times smaller the chroma planes are than luma, as powers of two: (1, 1) for 4:2:0
    pub chroma_shift: (u32, u32),
    pub matrix: Matrix,
    /// true when the file did not name its matrix and `matrix` is the usual assumption
    pub matrix_assumed: bool,
    /// where a chroma sample sits among the luma samples it stands for: how far right and down of the first of
    /// them, in luma samples. (0, 0.5) for 4:2:0 as h.264 and hevc site it; 0 the way a plane is not smaller
    pub siting: (f32, f32),
    /// true when the file did not say and `siting` is the usual place
    pub siting_assumed: bool,
    /// true when the picture has a plane of alpha, as big as luma (prores 4444)
    pub alpha: bool,
}

/// one decoded frame, owned. software: its planes live in ffmpeg's buffers. hardware: it holds a decoder surface, which
/// goes back to the decoder when this is dropped.
pub struct Picture(*mut ffi::AVFrame);
// a decoded frame is not written again; ffmpeg's frame references are safe to move and to read from any thread
unsafe impl Send for Picture {}
unsafe impl Sync for Picture {}

impl Drop for Picture {
    fn drop(&mut self) {
        unsafe { ffi::av_frame_free(&mut self.0) };
    }
}

impl Picture {
    /// when this frame is shown, in the stream's time units.
    pub fn time(&self) -> i64 {
        unsafe {
            let f = &*self.0;
            if f.best_effort_timestamp != ffi::AV_NOPTS_VALUE { f.best_effort_timestamp } else { f.pts }
        }
    }

    /// true when this picture is a decoder surface on the gpu.
    pub fn hardware(&self) -> bool {
        unsafe { !(*self.0).hw_frames_ctx.is_null() }
    }

    fn format(&self) -> ffi::AVPixelFormat {
        // the frame's format is a plain int in the c struct
        unsafe { std::mem::transmute::<i32, ffi::AVPixelFormat>((*self.0).format) }
    }

    /// the layout, or why this picture cannot be drawn yet.
    pub fn layout(&self) -> Result<Layout, String> {
        use ffi::AVPixelFormat as F;
        unsafe {
            let f = &*self.0;
            let hardware = self.hardware();
            let samples = if hardware { (*((*f.hw_frames_ctx).data as *const ffi::AVHWFramesContext)).sw_format } else { self.format() };
            let name = || CStr::from_ptr(ffi::av_get_pix_fmt_name(samples)).to_string_lossy().into_owned();
            let (bits, chroma_shift) = match samples {
                F::AV_PIX_FMT_NV12 | F::AV_PIX_FMT_YUV420P | F::AV_PIX_FMT_YUVJ420P => (8, (1, 1)),
                F::AV_PIX_FMT_NV16 | F::AV_PIX_FMT_YUV422P | F::AV_PIX_FMT_YUVJ422P => (8, (1, 0)),
                F::AV_PIX_FMT_NV24 | F::AV_PIX_FMT_YUV444P | F::AV_PIX_FMT_YUVJ444P => (8, (0, 0)),
                F::AV_PIX_FMT_P010LE | F::AV_PIX_FMT_YUV420P10LE => (10, (1, 1)),
                F::AV_PIX_FMT_P210LE | F::AV_PIX_FMT_YUV422P10LE => (10, (1, 0)),
                F::AV_PIX_FMT_P410LE | F::AV_PIX_FMT_YUV444P10LE | F::AV_PIX_FMT_YUVA444P10LE => (10, (0, 0)),
                F::AV_PIX_FMT_YUV422P12LE => (12, (1, 0)),
                F::AV_PIX_FMT_YUV444P12LE | F::AV_PIX_FMT_YUVA444P12LE => (12, (0, 0)),
                _ => return Err(format!("pictures stored as {} do not play yet", name())),
            };
            let full_range = f.color_range == ffi::AVColorRange::AVCOL_RANGE_JPEG || matches!(samples, F::AV_PIX_FMT_YUVJ420P | F::AV_PIX_FMT_YUVJ422P | F::AV_PIX_FMT_YUVJ444P);
            let (matrix, matrix_assumed) = match f.colorspace {
                ffi::AVColorSpace::AVCOL_SPC_BT709 => (Matrix::Bt709, false),
                ffi::AVColorSpace::AVCOL_SPC_BT470BG | ffi::AVColorSpace::AVCOL_SPC_SMPTE170M => (Matrix::Bt601, false),
                ffi::AVColorSpace::AVCOL_SPC_BT2020_NCL => (Matrix::Bt2020, false),
                _ => (Matrix::assumed(f.height as usize), true),
            };
            let stored = Stored { bits, texel_bits: if bits > 8 { 16 } else { 8 }, top_aligned: hardware && bits > 8, full_range };
            // where the file does not say: on the left and between two rows for video, in the middle for jpeg's
            // full-range pictures
            let jpeg = matches!(samples, F::AV_PIX_FMT_YUVJ420P | F::AV_PIX_FMT_YUVJ422P | F::AV_PIX_FMT_YUVJ444P);
            let (across, down, siting_assumed) = match f.chroma_location {
                ffi::AVChromaLocation::AVCHROMA_LOC_LEFT => (0.0, 0.5, false),
                ffi::AVChromaLocation::AVCHROMA_LOC_CENTER => (0.5, 0.5, false),
                ffi::AVChromaLocation::AVCHROMA_LOC_TOPLEFT => (0.0, 0.0, false),
                ffi::AVChromaLocation::AVCHROMA_LOC_TOP => (0.5, 0.0, false),
                ffi::AVChromaLocation::AVCHROMA_LOC_BOTTOMLEFT => (0.0, 1.0, false),
                ffi::AVChromaLocation::AVCHROMA_LOC_BOTTOM => (0.5, 1.0, false),
                _ if jpeg => (0.5, 0.5, true),
                _ => (0.0, 0.5, true),
            };
            let siting = (if chroma_shift.0 > 0 { across } else { 0.0 }, if chroma_shift.1 > 0 { down } else { 0.0 });
            Ok(Layout { hardware, stored, chroma_shift, matrix, matrix_assumed, siting, siting_assumed: siting_assumed && chroma_shift != (0, 0), alpha: matches!(samples, F::AV_PIX_FMT_YUVA444P10LE | F::AV_PIX_FMT_YUVA444P12LE) })
        }
    }

    /// for a software picture: plane `i` (0 luma, 1 cb, 2 cr, 3 alpha), its bytes a row, and its rows.
    pub fn plane(&self, i: usize, chroma_shift: (u32, u32)) -> (&[u8], usize, usize) {
        unsafe {
            let f = &*self.0;
            let rows = if i == 0 || i == 3 { f.height as usize } else { (f.height as usize).div_ceil(1 << chroma_shift.1) };
            let stride = f.linesize[i] as usize;
            (std::slice::from_raw_parts(f.data[i], stride * rows), stride, rows)
        }
    }

    /// for a hardware picture, where it is on the gpu: on windows the decoder's texture array (an ID3D11Texture2D)
    /// and this frame's slice; on macos the frame's CVPixelBuffer, and nothing.
    pub fn surface(&self) -> (*mut c_void, u32) {
        unsafe {
            let f = &*self.0;
            if cfg!(target_os = "macos") { (f.data[3] as *mut c_void, 0) } else { (f.data[0] as *mut c_void, f.data[1] as usize as u32) }
        }
    }
}

pub struct Decoder {
    ctx: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    stream: usize,
    drained: bool,
}
// only ever used by the thread that holds it
unsafe impl Send for Decoder {}

/// ffmpeg asks which output we want. on windows we take the direct3d 11 one and make its surfaces readable by a shader:
/// by default a decoder's surfaces can only be decoded into, which is why players usually copy frames out.
/// when the gpu does not take this stream (10-bit h.264, 4:2:2 on most cards), ffmpeg's own software choice stands.
#[cfg(windows)]
unsafe extern "C" fn pick_format(ctx: *mut ffi::AVCodecContext, formats: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    const BIND_SHADER_RESOURCE: u32 = 0x8;
    let mut p = formats;
    while *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == ffi::AVPixelFormat::AV_PIX_FMT_D3D11 {
            let mut frames: *mut ffi::AVBufferRef = ptr::null_mut();
            if ffi::avcodec_get_hw_frames_parameters(ctx, (*ctx).hw_device_ctx, ffi::AVPixelFormat::AV_PIX_FMT_D3D11, &mut frames) < 0 {
                break;
            }
            let fctx = (*frames).data as *mut ffi::AVHWFramesContext;
            (*((*fctx).hwctx as *mut ffi::AVD3D11VAFramesContext)).BindFlags |= BIND_SHADER_RESOURCE;
            if ffi::av_hwframe_ctx_init(frames) < 0 {
                ffi::av_buffer_unref(&mut frames);
                break;
            }
            (*ctx).hw_frames_ctx = frames;
            return ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
        }
        p = p.add(1);
    }
    ffi::avcodec_default_get_format(ctx, formats)
}

/// hands ffmpeg the renderer's device and lock.
#[cfg(windows)]
unsafe fn attach(ctx: *mut ffi::AVCodecContext, hardware: &Hardware, kept: usize) -> Result<(), String> {
    use windows::core::Interface;
    use windows::Win32::Graphics::Direct3D11::ID3D11Device;
    let hw = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
    let d3d = (*((*hw).data as *mut ffi::AVHWDeviceContext)).hwctx as *mut ffi::AVD3D11VADeviceContext;
    // ffmpeg releases one reference when its context is freed: give it one of its own
    let device = ID3D11Device::from_raw_borrowed(&hardware.device).ok_or("no device")?.clone();
    (*d3d).device = device.into_raw() as *mut _;
    (*d3d).lock = Some(hardware.lock);
    (*d3d).unlock = Some(hardware.unlock);
    (*d3d).lock_ctx = hardware.lock_ctx;
    let r = ffi::av_hwdevice_ctx_init(hw);
    if r < 0 {
        return Err(format!("hardware decoder: {}", err(r)));
    }
    (*ctx).hw_device_ctx = hw;
    (*ctx).get_format = Some(pick_format);
    // the decoder's surfaces are one pool of a fixed size: a picture the caller still holds is a surface it cannot
    // decode into, so the pool has to cover what the caller keeps
    (*ctx).extra_hw_frames = (kept + SPARE_SURFACES) as i32;
    // the gpu does the decoding: threads of our own would only hold frames back
    (*ctx).thread_count = 1;
    Ok(())
}

/// on macos we take videotoolbox's output when ffmpeg offers it; its own software choice stands when it does not.
#[cfg(target_os = "macos")]
unsafe extern "C" fn pick_format(ctx: *mut ffi::AVCodecContext, formats: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut p = formats;
    while *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
            return *p;
        }
        p = p.add(1);
    }
    ffi::avcodec_default_get_format(ctx, formats)
}

/// asks ffmpeg for videotoolbox, which makes its own surfaces as it needs them.
#[cfg(target_os = "macos")]
unsafe fn attach(ctx: *mut ffi::AVCodecContext, _hardware: &Hardware, _kept: usize) -> Result<(), String> {
    let mut hw = ptr::null_mut();
    let r = ffi::av_hwdevice_ctx_create(&mut hw, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX, ptr::null(), ptr::null_mut(), 0);
    if r < 0 {
        return Err(format!("hardware decoder: {}", err(r)));
    }
    (*ctx).hw_device_ctx = hw;
    (*ctx).get_format = Some(pick_format);
    // the system does the decoding: threads of our own would only hold frames back
    (*ctx).thread_count = 1;
    Ok(())
}

#[cfg(not(any(windows, target_os = "macos")))]
unsafe fn attach(_ctx: *mut ffi::AVCodecContext, _hardware: &Hardware, _kept: usize) -> Result<(), String> {
    Ok(())
}

impl Decoder {
    /// a decoder for `stream`. with `hardware`, the system's decoder is tried first and software is the fallback.
    /// `one_frame_at_a_time` makes all cores work on the slices of one frame, so a frame comes out for every packet
    /// that goes in: right for codecs whose frames stand alone, where a lone step must not wait behind other frames.
    /// `kept` is how many decoded pictures the caller will hold on to at once.
    pub fn open(input: &Input, stream: usize, hardware: Option<&Hardware>, one_frame_at_a_time: bool, kept: usize) -> Result<Decoder, String> {
        unsafe {
            let st = *(*input.ctx).streams.add(stream);
            let par = (*st).codecpar;
            let codec = ffi::avcodec_find_decoder((*par).codec_id);
            if codec.is_null() {
                return Err("ffmpeg has no decoder for this codec".into());
            }
            let mut ctx = ffi::avcodec_alloc_context3(codec);
            let r = ffi::avcodec_parameters_to_context(ctx, par);
            if r < 0 {
                ffi::avcodec_free_context(&mut ctx);
                return Err(err(r));
            }
            (*ctx).pkt_timebase = (*st).time_base;
            // 0 lets ffmpeg use every core
            (*ctx).thread_count = 0;
            if one_frame_at_a_time {
                (*ctx).thread_type = ffi::FF_THREAD_SLICE;
            }
            let attached = match hardware {
                Some(h) => attach(ctx, h, kept),
                None => Ok(()),
            };
            let opened = attached.and_then(|()| match ffi::avcodec_open2(ctx, codec, ptr::null_mut()) {
                r if r < 0 => Err(format!("open decoder: {}", err(r))),
                _ => Ok(()),
            });
            if let Err(why) = opened {
                ffi::avcodec_free_context(&mut ctx);
                return Err(why);
            }
            Ok(Decoder { ctx, packet: ffi::av_packet_alloc(), stream, drained: false })
        }
    }

    /// the next picture in display order, or None at the end of the file.
    pub fn next(&mut self, input: &Input) -> Result<Option<Picture>, String> {
        unsafe {
            let mut frame = ffi::av_frame_alloc();
            loop {
                let r = ffi::avcodec_receive_frame(self.ctx, frame);
                if r == 0 {
                    return Ok(Some(Picture(frame)));
                }
                if r != ffi::AVERROR(ffi::EAGAIN) || self.drained {
                    ffi::av_frame_free(&mut frame);
                    return if r == ffi::AVERROR_EOF || r == ffi::AVERROR(ffi::EAGAIN) { Ok(None) } else { Err(err(r)) };
                }
                // the decoder wants more: feed it the stream's next packet, or tell it the file has ended
                loop {
                    ffi::av_packet_unref(self.packet);
                    let r = ffi::av_read_frame(input.ctx, self.packet);
                    if r < 0 {
                        ffi::avcodec_send_packet(self.ctx, ptr::null());
                        self.drained = true;
                        break;
                    }
                    if (*self.packet).stream_index as usize == self.stream {
                        let r = ffi::avcodec_send_packet(self.ctx, self.packet);
                        if r < 0 {
                            ffi::av_frame_free(&mut frame);
                            return Err(format!("decode: {}", err(r)));
                        }
                        break;
                    }
                }
            }
        }
    }

    /// moves the file to the key frame at or before `time` (the stream's units, decode time) and empties the decoder.
    pub fn seek(&mut self, input: &Input, time: i64) -> Result<(), String> {
        unsafe {
            let r = ffi::av_seek_frame(input.ctx, self.stream as i32, time, ffi::AVSEEK_FLAG_BACKWARD);
            if r < 0 {
                return Err(format!("seek: {}", err(r)));
            }
            ffi::avcodec_flush_buffers(self.ctx);
        }
        self.drained = false;
        Ok(())
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            ffi::av_packet_free(&mut self.packet);
            ffi::avcodec_free_context(&mut self.ctx);
        }
    }
}
