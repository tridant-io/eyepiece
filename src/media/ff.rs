//! ffmpeg, behind small safe wrappers. it reads containers and decodes; it never opens a file itself.

use ffmpeg_sys_next as ffi;
use std::ffi::{c_int, c_void, CStr, CString};
use std::fs::File;
use std::ptr;

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

/// one stored sample of a stream: where it lies in the file, its decode time in the stream's units, and whether
/// decoding can start at it.
#[derive(Clone, Copy)]
pub struct Sample {
    pub pos: u64,
    pub size: usize,
    pub time: i64,
    pub key: bool,
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

    /// every stored sample of a stream, in file order, from the container's own table. the header alone gives this.
    pub fn samples(&self, stream: usize) -> Vec<Sample> {
        unsafe {
            let st = *(*self.ctx).streams.add(stream);
            let n = ffi::avformat_index_get_entries_count(st);
            (0..n)
                .filter_map(|i| {
                    let e = ffi::avformat_index_get_entry(st, i);
                    (!e.is_null()).then(|| Sample { pos: (*e).pos as u64, size: (*e).size() as usize, time: (*e).timestamp, key: (*e).flags() & ffi::AVINDEX_KEYFRAME != 0 })
                })
                .collect()
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
