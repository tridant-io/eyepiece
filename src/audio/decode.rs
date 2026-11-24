//! a clip's sound through ffmpeg, read through our own shared handle like everything else: decoded, then converted by
//! swresample to interleaved 32-bit float at the rate and channel count the device plays. more channels than the
//! device has are mixed down by swresample's own matrix.

use super::Info;
use crate::media::ff::{err, Input};
use ffmpeg_sys_next as ffi;
use std::ffi::CStr;
use std::ptr;

/// a seek lands this much early and the extra is decoded and dropped: a decoder whose frames overlap (aac) gets the
/// first frame after a seek wrong without the one before it
const PREROLL: f64 = 0.1;

pub struct Decoder {
    input: Input,
    ctx: *mut ffi::AVCodecContext,
    packet: *mut ffi::AVPacket,
    frame: *mut ffi::AVFrame,
    stream: usize,
    /// seconds per time unit
    time_base: f64,
    /// the first stored sample's time: before 0 when the file stores a codec's priming ahead of the clip
    first: i64,
    swr: *mut ffi::SwrContext,
    /// what `swr` converts from, as a frame's (format, rate, channels)
    from: (i32, i32, i32),
    /// what it converts to
    rate: u32,
    channels: u16,
    /// the decoder has been told the file ended, and then has given all it had
    drained: bool,
    done: bool,
    /// after a seek, the time wanted, until the first frame says how much to drop to reach it
    wanted: Option<f64>,
    /// output frames still to drop to land on the time wanted
    drop: usize,
}
// only ever used by the thread that holds it
unsafe impl Send for Decoder {}

impl Decoder {
    /// the file's first sound, ready to decode, and what it is. None when the file has no sound.
    pub fn open(path: &str) -> Result<Option<(Decoder, Info)>, String> {
        let input = Input::open(path)?;
        let Some(stream) = input.first_audio() else { return Ok(None) };
        unsafe {
            let streams = (*input.ctx).streams;
            // this reader is the sound's own: ffmpeg is told to discard the other streams' packets
            for i in 0..(*input.ctx).nb_streams as usize {
                if i != stream {
                    (**streams.add(i)).discard = ffi::AVDiscard::AVDISCARD_ALL;
                }
            }
            let st = *streams.add(stream);
            let par = (*st).codecpar;
            let codec = CStr::from_ptr(ffi::avcodec_get_name((*par).codec_id)).to_string_lossy().into_owned();
            let info = Info { codec, channels: (*par).ch_layout.nb_channels.max(0) as u16, sample_rate: (*par).sample_rate.max(0) as u32 };
            let codec = ffi::avcodec_find_decoder((*par).codec_id);
            if codec.is_null() {
                return Err(format!("{path}: ffmpeg has no decoder for its sound ({})", info.codec));
            }
            let mut ctx = ffi::avcodec_alloc_context3(codec);
            let mut r = ffi::avcodec_parameters_to_context(ctx, par);
            if r >= 0 {
                (*ctx).pkt_timebase = (*st).time_base;
                r = ffi::avcodec_open2(ctx, codec, ptr::null_mut());
            }
            if r < 0 {
                ffi::avcodec_free_context(&mut ctx);
                return Err(format!("{path}: sound: {}", err(r)));
            }
            let time_base = (*st).time_base.num as f64 / (*st).time_base.den as f64;
            let entry = ffi::avformat_index_get_entry(st, 0);
            let first = if entry.is_null() { 0 } else { (*entry).timestamp };
            let decoder = Decoder { input, ctx, packet: ffi::av_packet_alloc(), frame: ffi::av_frame_alloc(), stream, time_base, first, swr: ptr::null_mut(), from: (0, 0, 0), rate: 0, channels: 0, drained: false, done: false, wanted: None, drop: 0 };
            Ok(Some((decoder, info)))
        }
    }

    /// goes to `seconds` into the clip. from there the sound comes as interleaved float at `rate` and `channels`.
    pub fn seek(&mut self, seconds: f64, rate: u32, channels: u16) -> Result<(), String> {
        unsafe {
            // ffmpeg drops what a decoder like aac gets wrong after a seek, as much as the file's priming. at the
            // start that has to be the priming itself, stored before 0, so the seek goes to the first stored sample
            let to = if seconds <= PREROLL { self.first } else { ((seconds - PREROLL) / self.time_base) as i64 };
            let r = ffi::av_seek_frame(self.input.ctx, self.stream as i32, to, ffi::AVSEEK_FLAG_BACKWARD);
            if r < 0 {
                return Err(format!("seek: {}", err(r)));
            }
            ffi::avcodec_flush_buffers(self.ctx);
            // what the converter holds belongs to where the sound was: the first frame from here makes a new one
            ffi::swr_free(&mut self.swr);
        }
        (self.rate, self.channels) = (rate, channels);
        (self.drained, self.done) = (false, false);
        self.wanted = Some(seconds);
        self.drop = 0;
        Ok(())
    }

    /// appends the next of the sound to `out`. false at its end.
    pub fn next(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        unsafe {
            while !self.done {
                let r = ffi::avcodec_receive_frame(self.ctx, self.frame);
                if r == 0 {
                    let kept = self.convert(out);
                    ffi::av_frame_unref(self.frame);
                    if kept? > 0 {
                        return Ok(true);
                    }
                } else if r == ffi::AVERROR(ffi::EAGAIN) && !self.drained {
                    self.feed()?;
                } else if r == ffi::AVERROR_EOF || r == ffi::AVERROR(ffi::EAGAIN) {
                    // the decoder has given all it had: what the converter still holds is the end of the sound
                    self.done = true;
                    return Ok(self.append(out, ptr::null(), 0)? > 0);
                } else {
                    return Err(format!("decode: {}", err(r)));
                }
            }
            Ok(false)
        }
    }

    /// gives the decoder the sound's next packet, or tells it the file has ended.
    unsafe fn feed(&mut self) -> Result<(), String> {
        loop {
            ffi::av_packet_unref(self.packet);
            if ffi::av_read_frame(self.input.ctx, self.packet) < 0 {
                ffi::avcodec_send_packet(self.ctx, ptr::null());
                self.drained = true;
                return Ok(());
            }
            if (*self.packet).stream_index as usize == self.stream {
                let r = ffi::avcodec_send_packet(self.ctx, self.packet);
                return if r < 0 { Err(format!("decode: {}", err(r))) } else { Ok(()) };
            }
        }
    }

    /// converts the frame just decoded and appends it. returns the output frames kept.
    unsafe fn convert(&mut self, out: &mut Vec<f32>) -> Result<usize, String> {
        let f = &*self.frame;
        let from = (f.format, f.sample_rate, f.ch_layout.nb_channels);
        if self.swr.is_null() || from != self.from {
            self.build(f)?;
            self.from = from;
        }
        if let Some(wanted) = self.wanted.take() {
            let time = if f.best_effort_timestamp != ffi::AV_NOPTS_VALUE { f.best_effort_timestamp } else { f.pts };
            // a frame with no time is taken to be the one wanted
            if time != ffi::AV_NOPTS_VALUE {
                self.drop = ((wanted - time as f64 * self.time_base) * self.rate as f64).round().max(0.0) as usize;
            }
        }
        self.append(out, f.extended_data as *const *const u8, f.nb_samples)
    }

    /// makes the converter for frames like `f`.
    unsafe fn build(&mut self, f: &ffi::AVFrame) -> Result<(), String> {
        ffi::swr_free(&mut self.swr);
        let mut from: ffi::AVChannelLayout = std::mem::zeroed();
        let mut to: ffi::AVChannelLayout = std::mem::zeroed();
        // a file that does not say which channels it has gets the usual layout for its count, so they can be mixed
        let mut r = if f.ch_layout.order == ffi::AVChannelOrder::AV_CHANNEL_ORDER_UNSPEC {
            ffi::av_channel_layout_default(&mut from, f.ch_layout.nb_channels);
            0
        } else {
            ffi::av_channel_layout_copy(&mut from, &f.ch_layout)
        };
        ffi::av_channel_layout_default(&mut to, self.channels as i32);
        if r >= 0 {
            // the frame's format is a plain int in the c struct
            let format = std::mem::transmute::<i32, ffi::AVSampleFormat>(f.format);
            r = ffi::swr_alloc_set_opts2(&mut self.swr, &to, ffi::AVSampleFormat::AV_SAMPLE_FMT_FLT, self.rate as i32, &from, format, f.sample_rate, 0, ptr::null_mut());
        }
        if r >= 0 {
            r = ffi::swr_init(self.swr);
        }
        ffi::av_channel_layout_uninit(&mut from);
        ffi::av_channel_layout_uninit(&mut to);
        if r < 0 {
            ffi::swr_free(&mut self.swr);
            return Err(format!("sound conversion: {}", err(r)));
        }
        Ok(())
    }

    /// runs the converter on `samples` frames at `data`, or on none to empty it, and appends what comes out less what
    /// is still to be dropped. returns the output frames kept.
    unsafe fn append(&mut self, out: &mut Vec<f32>, data: *const *const u8, samples: i32) -> Result<usize, String> {
        if self.swr.is_null() {
            return Ok(0);
        }
        let room = ffi::swr_get_out_samples(self.swr, samples);
        if room < 0 {
            return Err(format!("sound conversion: {}", err(room)));
        }
        let (ch, at) = (self.channels as usize, out.len());
        out.resize(at + room as usize * ch, 0.0);
        let to = out[at..].as_mut_ptr() as *mut u8;
        let n = ffi::swr_convert(self.swr, &to, room, data, samples);
        if n < 0 {
            out.truncate(at);
            return Err(format!("sound conversion: {}", err(n)));
        }
        let n = n as usize;
        let dropped = self.drop.min(n);
        self.drop -= dropped;
        out.truncate(at + n * ch);
        out.drain(at..at + dropped * ch);
        Ok(n - dropped)
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            ffi::swr_free(&mut self.swr);
            ffi::av_frame_free(&mut self.frame);
            ffi::av_packet_free(&mut self.packet);
            ffi::avcodec_free_context(&mut self.ctx);
        }
    }
}
