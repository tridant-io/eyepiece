//! ffmpeg, behind small safe wrappers. it reads containers and decodes; it never opens a file itself.

use ffmpeg_sys_next as ffi;
use std::ffi::{c_int, c_void, CStr, CString};
use std::fs::File;
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use ffi::AVMediaType;

// the two errno values ffmpeg's error codes are built from, the same on every system we build for
const EIO: c_int = 5;
const EINVAL: c_int = 22;
const IO_BUFFER: usize = 1 << 16;

pub fn err(code: i32) -> String {
    let mut buf = [0i8; 128];
    unsafe {
        ffi::av_strerror(code, buf.as_mut_ptr(), buf.len());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// a codec tag as its four letters.
pub fn fourcc(tag: u32) -> String {
    tag.to_le_bytes().iter().map(|&b| if (32..127).contains(&b) { b as char } else { '.' }).collect()
}

/// reads up to `buf.len()` bytes at `pos`, without a shared file position: safe from any thread.
pub fn read_some(f: &File, buf: &mut [u8], pos: u64) -> std::io::Result<usize> {
    #[cfg(windows)]
    {
        std::os::windows::fs::FileExt::seek_read(f, buf, pos)
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::FileExt::read_at(f, buf, pos)
    }
}

/// reads exactly `buf.len()` bytes at `pos`.
pub fn read_at(f: &File, buf: &mut [u8], pos: u64) -> std::io::Result<()> {
    let mut done = 0;
    while done < buf.len() {
        match read_some(f, &mut buf[done..], pos + done as u64)? {
            0 => return Err(std::io::ErrorKind::UnexpectedEof.into()),
            n => done += n,
        }
    }
    Ok(())
}

/// opens a media file the only way the player may: shared for read, write and delete, so a render can overwrite,
/// rename over or delete and recreate a clip that is open (gate 5). rust's own open does this on windows, and
/// nothing is ever refused on the other systems; ffmpeg's own open does not, which is why it never gets the path.
pub fn open_shared(path: &str) -> Result<File, String> {
    File::open(path).map_err(|e| format!("{path}: {e}"))
}

/// what ffmpeg reads through: our own handle and our own position.
struct Io {
    file: File,
    pos: u64,
}

unsafe extern "C" fn io_read(opaque: *mut c_void, buf: *mut u8, size: c_int) -> c_int {
    let io = &mut *(opaque as *mut Io);
    match read_some(&io.file, std::slice::from_raw_parts_mut(buf, size as usize), io.pos) {
        Ok(0) => ffi::AVERROR_EOF,
        Ok(n) => {
            io.pos += n as u64;
            n as c_int
        }
        Err(_) => -EIO,
    }
}

unsafe extern "C" fn io_seek(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let io = &mut *(opaque as *mut Io);
    let len = io.file.metadata().map(|m| m.len() as i64).unwrap_or(-1);
    if whence & ffi::AVSEEK_SIZE as c_int != 0 {
        return len;
    }
    let to = match whence & 3 {
        0 => offset,
        1 => io.pos as i64 + offset,
        2 => len + offset,
        _ => -1,
    };
    if to < 0 {
        return -EINVAL as i64;
    }
    io.pos = to as u64;
    to
}

/// one video stream as the container describes it. the frame count and rate are not here on purpose: they come from
/// the sample table (see `Timeline`), never from ffmpeg's summary fields (adr 0003).
pub struct Video {
    pub index: usize,
    pub codec_name: String,
    pub tag: u32,
    pub width: usize,
    pub height: usize,
    /// seconds per time unit, as a ratio
    pub time_base: (i64, i64),
}

/// one stored sample of a stream: where it lies in the file, its decode time in the stream's units (or, from a
/// container that gives none, such as mkv, when it is shown: what its seeking goes by), and whether decoding can start
/// at it.
#[derive(Clone, Copy)]
pub struct Sample {
    pub pos: u64,
    pub size: usize,
    pub time: i64,
    pub key: bool,
    /// stored but not shown: an edit list starts the clip after it (a clip cut out of a longer one keeps the frames
    /// its first shown frame was built from). the decoder still needs it; it is not a frame of the clip
    pub hidden: bool,
}

pub struct Input {
    pub ctx: *mut ffi::AVFormatContext,
    pb: *mut ffi::AVIOContext,
    io: *mut Io,
}

// the context is only ever used by the thread that holds the Input
unsafe impl Send for Input {}

impl Input {
    /// reads a container's header through our own shared handle. nothing is decoded.
    pub fn open(path: &str) -> Result<Input, String> {
        let file = open_shared(path)?;
        let name = CString::new(path).map_err(|e| e.to_string())?;
        unsafe {
            let io = Box::into_raw(Box::new(Io { file, pos: 0 }));
            let mut pb = ffi::avio_alloc_context(ffi::av_malloc(IO_BUFFER) as *mut u8, IO_BUFFER as c_int, 0, io as *mut c_void, Some(io_read), None, Some(io_seek));
            let mut ctx = ffi::avformat_alloc_context();
            (*ctx).pb = pb;
            // the name is only a hint for telling the container apart; ffmpeg has the handle already and opens nothing
            let r = ffi::avformat_open_input(&mut ctx, name.as_ptr(), ptr::null(), ptr::null_mut());
            if r < 0 {
                // ffmpeg freed its own context on failure; the i/o side is ours
                ffi::av_freep(&mut (*pb).buffer as *mut *mut u8 as *mut c_void);
                ffi::avio_context_free(&mut pb);
                drop(Box::from_raw(io));
                return Err(format!("{path}: {}", err(r)));
            }
            Ok(Input { ctx, pb, io })
        }
    }

    pub fn first_video(&self) -> Option<Video> {
        let n = unsafe { (*self.ctx).nb_streams } as usize;
        (0..n).find_map(|i| unsafe {
            let st = *(*self.ctx).streams.add(i);
            let par = (*st).codecpar;
            ((*par).codec_type == AVMediaType::AVMEDIA_TYPE_VIDEO).then(|| Video {
                index: i,
                codec_name: CStr::from_ptr(ffi::avcodec_get_name((*par).codec_id)).to_string_lossy().into_owned(),
                tag: (*par).codec_tag,
                width: (*par).width.max(0) as usize,
                height: (*par).height.max(0) as usize,
                time_base: ((*st).time_base.num as i64, (*st).time_base.den as i64),
            })
        })
    }

    /// the index of the first sound stream.
    pub fn first_audio(&self) -> Option<usize> {
        let n = unsafe { (*self.ctx).nb_streams } as usize;
        (0..n).find(|&i| unsafe { (*(**(*self.ctx).streams.add(i)).codecpar).codec_type == AVMediaType::AVMEDIA_TYPE_AUDIO })
    }

    /// every stored sample of a stream, in file order, from the container's own table. the header alone gives this.
    pub fn samples(&self, stream: usize) -> Vec<Sample> {
        unsafe {
            let st = *(*self.ctx).streams.add(stream);
            let n = ffi::avformat_index_get_entries_count(st);
            (0..n)
                .filter_map(|i| {
                    let e = ffi::avformat_index_get_entry(st, i);
                    (!e.is_null()).then(|| Sample { pos: (*e).pos as u64, size: (*e).size() as usize, time: (*e).timestamp, key: (*e).flags() & ffi::AVINDEX_KEYFRAME != 0, hidden: (*e).flags() & ffi::AVINDEX_DISCARD_FRAME != 0 })
                })
                .collect()
        }
    }

    /// how many samples the container's header says a stream has, which for mov and mp4 is the moov's own table:
    /// none, or only the first, for a fragmented mp4. not a frame count (adr 0003).
    pub fn listed(&self, stream: usize) -> i64 {
        unsafe { (**(*self.ctx).streams.add(stream)).nb_frames }
    }

    /// the packet pass: every stored sample of a stream, counted by reading the file's packets from where the reader
    /// is to the end, for a container that lists none up front or whose times do not say when frames are shown.
    /// nothing is decoded, but every packet of the stream is read whole. also gives the times the frames are shown,
    /// in order, leaving out what an edit list hides. `counted` follows how many samples are counted so far.
    pub fn count(&mut self, stream: usize, counted: &AtomicUsize) -> Result<(Vec<Sample>, Vec<i64>), String> {
        let (mut samples, mut shown) = (Vec::new(), Vec::new());
        unsafe {
            for i in 0..self.streams() {
                (**(*self.ctx).streams.add(i)).discard = if i == stream { ffi::AVDiscard::AVDISCARD_DEFAULT } else { ffi::AVDiscard::AVDISCARD_ALL };
            }
            let mut packet = ffi::av_packet_alloc();
            let end = loop {
                let r = ffi::av_read_frame(self.ctx, packet);
                if r < 0 {
                    break r;
                }
                let p = &*packet;
                if p.stream_index as usize == stream {
                    let hidden = p.flags & ffi::AV_PKT_FLAG_DISCARD != 0;
                    let time = if p.dts != ffi::AV_NOPTS_VALUE { p.dts } else { p.pts };
                    samples.push(Sample { pos: p.pos.max(0) as u64, size: p.size.max(0) as usize, time, key: p.flags & ffi::AV_PKT_FLAG_KEY != 0, hidden });
                    if !hidden {
                        shown.push(if p.pts != ffi::AV_NOPTS_VALUE { p.pts } else { p.dts });
                    }
                    counted.store(samples.len(), Ordering::Relaxed);
                }
                ffi::av_packet_unref(packet);
            };
            ffi::av_packet_free(&mut packet);
            if end != ffi::AVERROR_EOF {
                return Err(err(end));
            }
        }
        shown.sort_unstable();
        Ok((samples, shown))
    }

    /// when a stream's first stored sample is, in its units: from the index, or where the index has none of it (mkv's
    /// sound) from the stream's first packet, read from where the reader is. 0 for a stream with no packet.
    pub fn first_time(&mut self, stream: usize) -> i64 {
        unsafe {
            let entry = ffi::avformat_index_get_entry(*(*self.ctx).streams.add(stream), 0);
            if !entry.is_null() {
                return (*entry).timestamp;
            }
            let mut packet = ffi::av_packet_alloc();
            let mut time = 0;
            while ffi::av_read_frame(self.ctx, packet) >= 0 {
                let p = &*packet;
                let found = p.stream_index as usize == stream;
                if found {
                    time = if p.dts != ffi::AV_NOPTS_VALUE { p.dts } else { p.pts };
                }
                ffi::av_packet_unref(packet);
                if found {
                    break;
                }
            }
            ffi::av_packet_free(&mut packet);
            time
        }
    }

    /// the container's kind as ffmpeg names it, such as "mov,mp4,m4a,3gp,3g2,mj2" or "matroska,webm".
    pub fn format_name(&self) -> String {
        unsafe { CStr::from_ptr((*(*self.ctx).iformat).name).to_string_lossy().into_owned() }
    }

    pub fn streams(&self) -> usize {
        unsafe { (*self.ctx).nb_streams as usize }
    }

    /// a value from a stream's metadata, or from the container's with no stream.
    pub fn tag(&self, stream: Option<usize>, key: &str) -> Option<String> {
        let key = CString::new(key).ok()?;
        unsafe {
            let dict = match stream {
                Some(i) => (**(*self.ctx).streams.add(i)).metadata,
                None => (*self.ctx).metadata,
            };
            let e = ffi::av_dict_get(dict, key.as_ptr(), ptr::null(), 0);
            (!e.is_null()).then(|| CStr::from_ptr((*e).value).to_string_lossy().into_owned())
        }
    }

    /// a stream's color tags as h.273 codes: primaries, transfer, matrix, then range (0 not stated, 1 video, 2 full).
    /// after `open` they are the container's; after `probe`, also what the decoder took from the stream or assumed
    pub fn color(&self, stream: usize) -> [i32; 4] {
        unsafe {
            let par = (**(*self.ctx).streams.add(stream)).codecpar;
            [(*par).color_primaries as i32, (*par).color_trc as i32, (*par).color_space as i32, (*par).color_range as i32]
        }
    }

    /// every audio stream, as the container describes it. read before `probe`, which fills a missing length with a guess.
    pub fn audio(&self) -> Vec<Audio> {
        (0..self.streams())
            .filter_map(|i| unsafe {
                let st = *(*self.ctx).streams.add(i);
                let par = (*st).codecpar;
                ((*par).codec_type == AVMediaType::AVMEDIA_TYPE_AUDIO).then(|| Audio {
                    codec_name: CStr::from_ptr(ffi::avcodec_get_name((*par).codec_id)).to_string_lossy().into_owned(),
                    channels: (*par).ch_layout.nb_channels.max(0) as usize,
                    sample_rate: (*par).sample_rate.max(0) as usize,
                    seconds: ((*st).duration != ffi::AV_NOPTS_VALUE).then(|| (*st).duration as f64 * (*st).time_base.num as f64 / (*st).time_base.den as f64),
                })
            })
            .collect()
    }

    /// reads and decodes the first packets until every stream's parameters are filled in. what it fills mixes the
    /// file's own values with the decoder's defaults (adr 0003), so callers take only what they can tell apart.
    pub fn probe(&mut self) -> Result<(), String> {
        match unsafe { ffi::avformat_find_stream_info(self.ctx, ptr::null_mut()) } {
            r if r < 0 => Err(err(r)),
            _ => Ok(()),
        }
    }

    /// after `probe`: bits per sample and the chroma subsampling (as powers of two) of the pixels the decoder puts out.
    pub fn depth(&self, stream: usize) -> Option<(u32, (u32, u32))> {
        unsafe {
            let par = (**(*self.ctx).streams.add(stream)).codecpar;
            // the format is a plain int in the c struct
            let d = ffi::av_pix_fmt_desc_get(std::mem::transmute::<i32, ffi::AVPixelFormat>((*par).format));
            (!d.is_null()).then(|| ((*d).comp[0].depth as u32, ((*d).log2_chroma_w as u32, (*d).log2_chroma_h as u32)))
        }
    }
}

impl Drop for Input {
    fn drop(&mut self) {
        unsafe {
            ffi::avformat_close_input(&mut self.ctx);
            ffi::av_freep(&mut (*self.pb).buffer as *mut *mut u8 as *mut c_void);
            ffi::avio_context_free(&mut self.pb);
            drop(Box::from_raw(self.io));
        }
    }
}

/// one audio stream as the container describes it.
pub struct Audio {
    pub codec_name: String,
    pub channels: usize,
    pub sample_rate: usize,
    /// how long the container says the track is, when it says
    pub seconds: Option<f64>,
}
