//! the few ffmpeg calls the gates need, behind small safe wrappers.

use ffmpeg_sys_next as ffi;
use std::ffi::{CStr, CString};
use std::ptr;

pub use ffi::{AVCodecID, AVMediaType, AVPixelFormat};

pub fn err(code: i32) -> String {
    let mut buf = [0i8; 128];
    unsafe {
        ffi::av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

pub fn fourcc(tag: u32) -> String {
    tag.to_le_bytes().iter().map(|&b| if (32..127).contains(&b) { b as char } else { '.' }).collect()
}

pub fn version() -> String {
    unsafe { CStr::from_ptr(ffi::av_version_info()).to_string_lossy().into_owned() }
}

pub struct Input {
    pub ctx: *mut ffi::AVFormatContext,
}

pub struct StreamInfo {
    pub index: usize,
    pub kind: AVMediaType,
    pub codec_id: AVCodecID,
    pub codec_name: String,
    pub tag: u32,
    pub width: i32,
    pub height: i32,
    pub nb_frames: i64,
    pub avg_rate: (i32, i32),
    pub time_base: (i32, i32),
}

impl Input {
    /// opens a file. `probe` runs avformat_find_stream_info, which may decode frames; the player skips it where it can.
    pub fn open(path: &str, probe: bool) -> Result<Input, String> {
        let c = CString::new(path).map_err(|e| e.to_string())?;
        let mut ctx: *mut ffi::AVFormatContext = ptr::null_mut();
        let r = unsafe { ffi::avformat_open_input(&mut ctx, c.as_ptr(), ptr::null(), ptr::null_mut()) };
        if r < 0 {
            return Err(format!("open {path}: {}", err(r)));
        }
        if probe {
            let r = unsafe { ffi::avformat_find_stream_info(ctx, ptr::null_mut()) };
            if r < 0 {
                unsafe { ffi::avformat_close_input(&mut ctx) };
                return Err(format!("stream info {path}: {}", err(r)));
            }
        }
        Ok(Input { ctx })
    }

    pub fn streams(&self) -> Vec<StreamInfo> {
        let n = unsafe { (*self.ctx).nb_streams } as usize;
        (0..n)
            .map(|i| unsafe {
                let st = *(*self.ctx).streams.add(i);
                let par = (*st).codecpar;
                StreamInfo {
                    index: i,
                    kind: (*par).codec_type,
                    codec_id: (*par).codec_id,
                    codec_name: CStr::from_ptr(ffi::avcodec_get_name((*par).codec_id)).to_string_lossy().into_owned(),
                    tag: (*par).codec_tag,
                    width: (*par).width,
                    height: (*par).height,
                    nb_frames: (*st).nb_frames,
                    avg_rate: ((*st).avg_frame_rate.num, (*st).avg_frame_rate.den),
                    time_base: ((*st).time_base.num, (*st).time_base.den),
                }
            })
            .collect()
    }

    pub fn first_video(&self) -> Option<StreamInfo> {
        self.streams().into_iter().find(|s| s.kind == AVMediaType::AVMEDIA_TYPE_VIDEO)
    }

    pub fn stream(&self, index: usize) -> *mut ffi::AVStream {
        unsafe { *(*self.ctx).streams.add(index) }
    }

    /// seeks to the keyframe at or before `ts` (in the stream's time base) on that stream.
    pub fn seek(&mut self, stream: usize, ts: i64) -> Result<(), String> {
        let r = unsafe { ffi::av_seek_frame(self.ctx, stream as i32, ts, ffi::AVSEEK_FLAG_BACKWARD as i32) };
        if r < 0 { Err(err(r)) } else { Ok(()) }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe { ffi::avformat_close_input(&mut self.ctx) };
    }
}

pub struct Packet {
    pub ptr: *mut ffi::AVPacket,
}

impl Packet {
    pub fn new() -> Packet {
        Packet { ptr: unsafe { ffi::av_packet_alloc() } }
    }
    /// reads the next packet of the file. Ok(false) at end of file.
    pub fn read(&mut self, input: &Input) -> Result<bool, String> {
        unsafe {
            ffi::av_packet_unref(self.ptr);
            let r = ffi::av_read_frame(input.ctx, self.ptr);
            if r == ffi::AVERROR_EOF {
                return Ok(false);
            }
            if r < 0 {
                return Err(err(r));
            }
            Ok(true)
        }
    }
    pub fn stream(&self) -> usize {
        unsafe { (*self.ptr).stream_index as usize }
    }
    pub fn data(&self) -> &[u8] {
        unsafe {
            let p = &*self.ptr;
            if p.data.is_null() { &[] } else { std::slice::from_raw_parts(p.data, p.size as usize) }
        }
    }
    pub fn pts(&self) -> i64 {
        unsafe { (*self.ptr).pts }
    }
    pub fn key(&self) -> bool {
        unsafe { (*self.ptr).flags & ffi::AV_PKT_FLAG_KEY as i32 != 0 }
    }
}

impl Default for Packet {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { ffi::av_packet_free(&mut self.ptr) };
    }
}

impl Input {
    /// where each frame of a stream lies in the file: (offset, size), in decode order. built by the demuxer from the header alone.
    pub fn index(&self, stream: usize) -> Vec<(u64, usize)> {
        unsafe {
            let st = self.stream(stream);
            let n = ffi::avformat_index_get_entries_count(st);
            (0..n)
                .filter_map(|i| {
                    let e = ffi::avformat_index_get_entry(st, i);
                    if e.is_null() { None } else { Some(((*e).pos as u64, (*e).size() as usize)) }
                })
                .collect()
        }
    }
}
