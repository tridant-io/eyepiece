//! ffmpeg decoders for the gates: software, or hardware decode (direct3d 11 on a device we own; videotoolbox on macos).

use crate::ff::{err, Input, Packet};
use ffmpeg_sys_next as ffi;
use std::ptr;
#[cfg(windows)]
use windows::core::Interface;
#[cfg(windows)]
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11Texture2D};

#[cfg(windows)]
const BIND_SHADER_RESOURCE: u32 = 0x8;

pub struct Decoder {
    pub ctx: *mut ffi::AVCodecContext,
    pub frame: *mut ffi::AVFrame,
    eof_sent: bool,
}

/// ffmpeg asks which output format we want. we take the direct3d 11 one, and make its frame pool readable by a shader:
/// by default the decoder's textures can only be decoded into, which is why players usually copy them out.
#[cfg(windows)]
unsafe extern "C" fn pick_d3d11(ctx: *mut ffi::AVCodecContext, fmts: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut p = fmts;
    while *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == ffi::AVPixelFormat::AV_PIX_FMT_D3D11 {
            let mut frames: *mut ffi::AVBufferRef = ptr::null_mut();
            if ffi::avcodec_get_hw_frames_parameters(ctx, (*ctx).hw_device_ctx, ffi::AVPixelFormat::AV_PIX_FMT_D3D11, &mut frames) < 0 {
                return ffi::AVPixelFormat::AV_PIX_FMT_NONE;
            }
            let fctx = (*frames).data as *mut ffi::AVHWFramesContext;
            let d3d = (*fctx).hwctx as *mut ffi::AVD3D11VAFramesContext;
            (*d3d).BindFlags |= BIND_SHADER_RESOURCE;
            if ffi::av_hwframe_ctx_init(frames) < 0 {
                ffi::av_buffer_unref(&mut frames);
                return ffi::AVPixelFormat::AV_PIX_FMT_NONE;
            }
            (*ctx).hw_frames_ctx = frames;
            return ffi::AVPixelFormat::AV_PIX_FMT_D3D11;
        }
        p = p.add(1);
    }
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

/// on macos we take the videotoolbox format: each frame is then the decoder's own pixel buffer, not a copy.
#[cfg(target_os = "macos")]
unsafe extern "C" fn pick_videotoolbox(_ctx: *mut ffi::AVCodecContext, fmts: *const ffi::AVPixelFormat) -> ffi::AVPixelFormat {
    let mut p = fmts;
    while *p != ffi::AVPixelFormat::AV_PIX_FMT_NONE {
        if *p == ffi::AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX {
            return *p;
        }
        p = p.add(1);
    }
    ffi::AVPixelFormat::AV_PIX_FMT_NONE
}

/// a decoder context for the stream, not opened yet.
unsafe fn prepare(input: &Input, stream: usize) -> Result<(*mut ffi::AVCodecContext, *const ffi::AVCodec), String> {
    let st = input.stream(stream);
    let par = (*st).codecpar;
    let codec = ffi::avcodec_find_decoder((*par).codec_id);
    if codec.is_null() {
        return Err("no decoder for this codec".into());
    }
    let ctx = ffi::avcodec_alloc_context3(codec);
    let r = ffi::avcodec_parameters_to_context(ctx, par);
    if r < 0 {
        return Err(err(r));
    }
    (*ctx).pkt_timebase = (*st).time_base;
    Ok((ctx, codec))
}

unsafe fn finish(ctx: *mut ffi::AVCodecContext, codec: *const ffi::AVCodec) -> Result<Decoder, String> {
    let r = ffi::avcodec_open2(ctx, codec, ptr::null_mut());
    if r < 0 {
        return Err(format!("open decoder: {}", err(r)));
    }
    Ok(Decoder { ctx, frame: ffi::av_frame_alloc(), eof_sent: false })
}

impl Decoder {
    /// `device` = Some: hardware decode into that device's textures. None: software, on `threads` threads (0 = all cores).
    #[cfg(windows)]
    pub fn open(input: &Input, stream: usize, device: Option<&ID3D11Device>, threads: i32) -> Result<Decoder, String> {
        let Some(dev) = device else {
            return Self::open_software(input, stream, threads);
        };
        unsafe {
            let (ctx, codec) = prepare(input, stream)?;
            let hw = ffi::av_hwdevice_ctx_alloc(ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_D3D11VA);
            let dctx = (*hw).data as *mut ffi::AVHWDeviceContext;
            let d3d = (*dctx).hwctx as *mut ffi::AVD3D11VADeviceContext;
            // ffmpeg releases this reference when the context is freed
            (*d3d).device = dev.as_raw() as *mut _;
            std::mem::forget(dev.clone());
            let r = ffi::av_hwdevice_ctx_init(hw);
            if r < 0 {
                return Err(format!("d3d11va device: {}", err(r)));
            }
            (*ctx).hw_device_ctx = hw;
            (*ctx).get_format = Some(pick_d3d11);
            (*ctx).thread_count = 1;
            finish(ctx, codec)
        }
    }

    /// software decode on `threads` threads (0 = all cores).
    pub fn open_software(input: &Input, stream: usize, threads: i32) -> Result<Decoder, String> {
        unsafe {
            let (ctx, codec) = prepare(input, stream)?;
            (*ctx).thread_count = threads;
            finish(ctx, codec)
        }
    }

    /// a software decoder for stepping: all cores work on the slices of ONE frame, so a frame comes out for every packet
    /// that goes in. the default (frame threading) is faster straight through but holds back a frame per thread.
    pub fn open_stepping(input: &Input, stream: usize) -> Result<Decoder, String> {
        unsafe {
            let (ctx, codec) = prepare(input, stream)?;
            (*ctx).thread_count = 0;
            (*ctx).thread_type = ffi::FF_THREAD_SLICE as i32;
            finish(ctx, codec)
        }
    }

    /// hardware decode through videotoolbox.
    #[cfg(target_os = "macos")]
    pub fn open_videotoolbox(input: &Input, stream: usize) -> Result<Decoder, String> {
        unsafe {
            let (ctx, codec) = prepare(input, stream)?;
            let mut hw: *mut ffi::AVBufferRef = ptr::null_mut();
            let r = ffi::av_hwdevice_ctx_create(&mut hw, ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VIDEOTOOLBOX, ptr::null(), ptr::null_mut(), 0);
            if r < 0 {
                return Err(format!("videotoolbox device: {}", err(r)));
            }
            (*ctx).hw_device_ctx = hw;
            (*ctx).get_format = Some(pick_videotoolbox);
            finish(ctx, codec)
        }
    }

    /// decodes the next frame of `stream` into `self.frame`. Ok(false) at the end of the file.
    pub fn next(&mut self, input: &Input, pkt: &mut Packet, stream: usize) -> Result<bool, String> {
        unsafe {
            loop {
                let r = ffi::avcodec_receive_frame(self.ctx, self.frame);
                if r == 0 {
                    return Ok(true);
                }
                if r == ffi::AVERROR_EOF {
                    return Ok(false);
                }
                if r != ffi::AVERROR(ffi::EAGAIN) {
                    return Err(err(r));
                }
                if self.eof_sent {
                    return Ok(false);
                }
                loop {
                    if !pkt.read(input)? {
                        ffi::avcodec_send_packet(self.ctx, ptr::null());
                        self.eof_sent = true;
                        break;
                    }
                    if pkt.stream() == stream {
                        let r = ffi::avcodec_send_packet(self.ctx, pkt.ptr);
                        if r < 0 {
                            return Err(format!("send packet: {}", err(r)));
                        }
                        break;
                    }
                }
            }
        }
    }

    pub fn flush(&mut self) {
        unsafe { ffi::avcodec_flush_buffers(self.ctx) };
        self.eof_sent = false;
    }

    pub fn format(&self) -> ffi::AVPixelFormat {
        // the frame's format is a plain int in the c struct
        unsafe { std::mem::transmute::<i32, ffi::AVPixelFormat>((*self.frame).format) }
    }

    pub fn size(&self) -> (usize, usize) {
        unsafe { ((*self.frame).width as usize, (*self.frame).height as usize) }
    }

    /// for a hardware frame: the decoder's texture array and the slice this frame is in. borrowed, not copied.
    #[cfg(windows)]
    pub fn texture(&self) -> (ID3D11Texture2D, u32) {
        unsafe {
            let raw = (*self.frame).data[0] as *mut std::ffi::c_void;
            let tex = ID3D11Texture2D::from_raw_borrowed(&raw).expect("texture").clone();
            (tex, (*self.frame).data[1] as usize as u32)
        }
    }

    /// for a videotoolbox frame: the decoder's pixel buffer (a CVPixelBufferRef). borrowed: it lives as long as the frame.
    #[cfg(target_os = "macos")]
    pub fn pixel_buffer(&self) -> *mut std::ffi::c_void {
        unsafe { (*self.frame).data[3] as *mut std::ffi::c_void }
    }

    /// for a software frame: plane `i` as 16-bit samples and its stride in samples.
    pub fn plane16(&self, i: usize, rows: usize) -> (&[u16], usize) {
        unsafe {
            let stride = (*self.frame).linesize[i] as usize / 2;
            (std::slice::from_raw_parts((*self.frame).data[i] as *const u16, stride * rows), stride)
        }
    }

    /// for a software frame: plane `i` as bytes and its stride in bytes.
    pub fn plane8(&self, i: usize, rows: usize) -> (&[u8], usize) {
        unsafe {
            let stride = (*self.frame).linesize[i] as usize;
            (std::slice::from_raw_parts((*self.frame).data[i] as *const u8, stride * rows), stride)
        }
    }

    pub fn pts(&self) -> i64 {
        unsafe { (*self.frame).pts }
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            ffi::av_frame_free(&mut self.frame);
            ffi::avcodec_free_context(&mut self.ctx);
        }
    }
}
