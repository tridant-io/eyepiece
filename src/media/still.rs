//! one still file into one picture: png, jpeg, tiff, tga, exr or dpx. the whole file goes to ffmpeg's decoder as one
//! packet, and whatever comes back is unpacked into rgba here.

use super::color::{self, Matrix, Stored};
use super::ff::{self, err};
use ffmpeg_sys_next as ffi;
use std::ffi::CStr;
use std::path::Path;
use std::ptr;
use std::sync::OnceLock;

/// how each number of an `Image` is stored
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Samples {
    U8,
    U16,
    F32,
}

/// red, green, blue, alpha, packed, row by row, top row first. `samples` says how each number is stored
/// (native byte order). alpha is 1 when the file has none; `has_alpha` says whether it had one.
pub struct Image {
    pub width: usize,
    pub height: usize,
    pub samples: Samples,
    pub has_alpha: bool,
    /// bits a number in the file: 8, 10, 12 or 16; 16 for half float, 32 for float. 10 and 12-bit numbers are
    /// widened to 16 by repeating their top bits, so `value >> (16 - bits)` is the file's own
    pub bits: u32,
    pub data: Vec<u8>,
    /// what to call the format, as a person would
    pub codec: &'static str,
    /// the frame rate the file states, as its own ratio. only exr states one exactly
    pub rate: Option<(i64, i64)>,
}

struct Format {
    name: &'static str,
    codec: ffi::AVCodecID,
    extensions: &'static [&'static str],
    /// how the file starts, where the format has a fixed start
    signatures: &'static [&'static [u8]],
}

static FORMATS: [Format; 6] = [
    Format { name: "png", codec: ffi::AVCodecID::AV_CODEC_ID_PNG, extensions: &["png"], signatures: &[b"\x89PNG\r\n\x1a\n"] },
    Format { name: "jpeg", codec: ffi::AVCodecID::AV_CODEC_ID_MJPEG, extensions: &["jpg", "jpeg"], signatures: &[b"\xff\xd8\xff"] },
    Format { name: "tiff", codec: ffi::AVCodecID::AV_CODEC_ID_TIFF, extensions: &["tif", "tiff"], signatures: &[b"II*\0", b"MM\0*"] },
    Format { name: "tga", codec: ffi::AVCodecID::AV_CODEC_ID_TARGA, extensions: &["tga"], signatures: &[] },
    Format { name: "exr", codec: ffi::AVCodecID::AV_CODEC_ID_EXR, extensions: &["exr"], signatures: &[b"v/1\x01"] },
    Format { name: "dpx", codec: ffi::AVCodecID::AV_CODEC_ID_DPX, extensions: &["dpx"], signatures: &[b"SDPX", b"XPDS"] },
];

fn by_name(path: &str) -> Option<&'static Format> {
    let ext = Path::new(path).extension()?.to_str()?;
    FORMATS.iter().find(|f| f.extensions.iter().any(|e| e.eq_ignore_ascii_case(ext)))
}

/// true when the file's name is that of a still this module reads.
pub fn is_still(path: &str) -> bool {
    by_name(path).is_some()
}

/// what one decode holds of ffmpeg's, freed together however it ends
struct Work {
    packet: *mut ffi::AVPacket,
    ctx: *mut ffi::AVCodecContext,
    frame: *mut ffi::AVFrame,
}

impl Drop for Work {
    fn drop(&mut self) {
        unsafe {
            ffi::av_frame_free(&mut self.frame);
            ffi::avcodec_free_context(&mut self.ctx);
            ffi::av_packet_free(&mut self.packet);
        }
    }
}

/// reads a still file whole, through our own shared handle, and decodes it.
pub fn decode(path: &str) -> Result<Image, String> {
    let file = ff::open_shared(path)?;
    let len = file.metadata().map_err(|e| format!("{path}: {e}"))?.len();
    let len = i32::try_from(len).map_err(|_| format!("{path}: too big for one picture"))?;
    unsafe {
        let mut w = Work { packet: ffi::av_packet_alloc(), ctx: ptr::null_mut(), frame: ffi::av_frame_alloc() };
        // av_new_packet adds the zeroed padding ffmpeg's readers may run into past the end
        if w.packet.is_null() || w.frame.is_null() || ffi::av_new_packet(w.packet, len) < 0 {
            return Err("out of memory".into());
        }
        let bytes = std::slice::from_raw_parts_mut((*w.packet).data, len as usize);
        ff::read_at(&file, bytes, 0).map_err(|e| format!("{path}: {e}"))?;
        let format = FORMATS
            .iter()
            .find(|f| f.signatures.iter().any(|s| bytes.starts_with(s)))
            .or_else(|| by_name(path))
            .ok_or_else(|| format!("{path}: not a still this build reads (png, jpeg, tiff, tga, exr, dpx)"))?;
        let codec = ffi::avcodec_find_decoder(format.codec);
        if codec.is_null() {
            return Err(format!("{path}: this build of ffmpeg has no {} decoder", format.name));
        }
        w.ctx = ffi::avcodec_alloc_context3(codec);
        if w.ctx.is_null() {
            return Err("out of memory".into());
        }
        // every core on the one picture where the decoder can split it (exr's blocks). frame threads would only hold it back
        (*w.ctx).thread_count = 0;
        (*w.ctx).thread_type = ffi::FF_THREAD_SLICE;
        let r = ffi::avcodec_open2(w.ctx, codec, ptr::null_mut());
        if r < 0 {
            return Err(format!("{path}: open decoder: {}", err(r)));
        }
        let r = ffi::avcodec_send_packet(w.ctx, w.packet);
        if r < 0 {
            return Err(format!("{path}: {}", err(r)));
        }
        // the end of the stream: a decoder that holds a picture back gives it up now
        ffi::avcodec_send_packet(w.ctx, ptr::null());
        let r = ffi::avcodec_receive_frame(w.ctx, w.frame);
        if r < 0 {
            return Err(format!("{path}: {}", err(r)));
        }
        // exr states its rate as a ratio. dpx states it as a float, which ffmpeg turns into a near ratio: not used
        let r = (*w.ctx).framerate;
        let rate = (format.codec == ffi::AVCodecID::AV_CODEC_ID_EXR && r.num > 0 && r.den > 0).then_some((r.num as i64, r.den as i64));
        let (samples, has_alpha, bits, data) = unpack(&*w.frame).map_err(|e| format!("{path}: {e}"))?;
        let f = &*w.frame;
        Ok(Image { width: f.width as usize, height: f.height as usize, samples, has_alpha, bits, data, codec: format.name, rate })
    }
}

/// a 9 to 16-bit number widened to 16 bits by repeating its top bits: 0 stays 0, the top of the scale becomes 65535.
fn widen(v: u16, bits: u32) -> u16 {
    let v = v as u32;
    ((v << (16 - bits)) | (v >> (2 * bits - 16))) as u16
}

/// a half float (ieee binary16, as exr stores it) as a float: exact, every half has a float of the same value.
fn half(h: u16) -> f32 {
    let (sign, exp, frac) = ((h as u32 & 0x8000) << 16, (h as u32 >> 10) & 0x1f, h as u32 & 0x3ff);
    match exp {
        // zero, and the numbers too small for the usual form: frac * 2^-24
        0 => {
            let v = frac as f32 / 16_777_216.0;
            if sign != 0 { -v } else { v }
        }
        // infinity and not-a-number
        31 => f32::from_bits(sign | 0x7f80_0000 | (frac << 13)),
        _ => f32::from_bits(sign | ((exp + 112) << 23) | (frac << 13)),
    }
}

/// every half float's value, by its bits. a 4K exr frame holds 25 million halves, and looking them up took about half
/// the time of working each out (measured).
fn halves() -> &'static [f32; 1 << 16] {
    static TABLE: OnceLock<Box<[f32; 1 << 16]>> = OnceLock::new();
    TABLE.get_or_init(|| (0..=u16::MAX).map(half).collect::<Box<[f32]>>().try_into().unwrap())
}

/// the decoded picture as packed rgba: (samples, has alpha, the file's bits, the bytes). any layout ffmpeg describes
/// as whole bytes per number is read from its own description; the rest is refused by name.
unsafe fn unpack(f: &ffi::AVFrame) -> Result<(Samples, bool, u32, Vec<u8>), String> {
    let desc = ffi::av_pix_fmt_desc_get(std::mem::transmute::<i32, ffi::AVPixelFormat>(f.format));
    if desc.is_null() || f.width <= 0 || f.height <= 0 {
        return Err("the decoder gave no picture".into());
    }
    let desc = &*desc;
    let refused = || format!("pictures stored as {} are not read yet", CStr::from_ptr(desc.name).to_string_lossy());
    let flag = |bit: i32| desc.flags & bit as u64 != 0;
    let comps = &desc.comp[..desc.nb_components as usize];
    let depth = comps[0].depth as u32;
    let odd = ffi::AV_PIX_FMT_FLAG_PAL | ffi::AV_PIX_FMT_FLAG_BITSTREAM | ffi::AV_PIX_FMT_FLAG_HWACCEL | ffi::AV_PIX_FMT_FLAG_BAYER | ffi::AV_PIX_FMT_FLAG_XYZ;
    if flag(odd) || comps.iter().any(|c| c.shift != 0 || c.depth as u32 != depth) {
        return Err(refused());
    }
    let has_alpha = flag(ffi::AV_PIX_FMT_FLAG_ALPHA);
    // the alpha is always the last number; one or two numbers are gray, or gray and alpha
    let alpha = has_alpha.then(|| comps[comps.len() - 1]);
    if comps.len() >= 3 && !flag(ffi::AV_PIX_FMT_FLAG_RGB) {
        return if depth == 8 { Ok((Samples::U8, has_alpha, 8, from_ycbcr(f, desc, alpha))) } else { Err(refused()) };
    }
    let sources = if comps.len() >= 3 { [comps[0], comps[1], comps[2]] } else { [comps[0]; 3] };
    let sources = [Some(sources[0]), Some(sources[1]), Some(sources[2]), alpha];
    let be = flag(ffi::AV_PIX_FMT_FLAG_BE);
    let u16_at = |p: *const u8| {
        let b = [*p, *p.add(1)];
        if be { u16::from_be_bytes(b) } else { u16::from_le_bytes(b) }
    };
    Ok(match (flag(ffi::AV_PIX_FMT_FLAG_FLOAT), depth) {
        (false, 8) => (Samples::U8, has_alpha, 8, pack(f, sources, [255], |p| [*p])),
        (false, 9..=16) => (Samples::U16, has_alpha, depth, pack(f, sources, u16::MAX.to_ne_bytes(), |p| widen(u16_at(p), depth).to_ne_bytes())),
        (true, 16) => {
            let halves = halves();
            (Samples::F32, has_alpha, 16, pack(f, sources, 1f32.to_ne_bytes(), |p| halves[u16_at(p) as usize].to_ne_bytes()))
        }
        (true, 32) => {
            let read = |p: *const u8| {
                let b = [*p, *p.add(1), *p.add(2), *p.add(3)];
                (if be { f32::from_be_bytes(b) } else { f32::from_le_bytes(b) }).to_ne_bytes()
            };
            (Samples::F32, has_alpha, 32, pack(f, sources, 1f32.to_ne_bytes(), read))
        }
        _ => return Err(refused()),
    })
}

/// packs rgba, `N` bytes a number, from where each of red, green, blue and alpha lies in the frame. `read` turns the
/// stored number at a place into the bytes of the packed one; a number with nowhere to come from is `one`.
unsafe fn pack<const N: usize>(f: &ffi::AVFrame, sources: [Option<ffi::AVComponentDescriptor>; 4], one: [u8; N], read: impl Fn(*const u8) -> [u8; N]) -> Vec<u8> {
    let (width, height) = (f.width as usize, f.height as usize);
    let mut out = vec![0u8; width * height * 4 * N];
    for (y, row) in out.chunks_exact_mut(width * 4 * N).enumerate() {
        // one channel at a time, which measured quicker than one pixel at a time
        for (channel, source) in sources.iter().enumerate() {
            let source = source.map(|c| (f.data[c.plane as usize].offset(y as isize * f.linesize[c.plane as usize] as isize + c.offset as isize) as *const u8, c.step as usize));
            for x in 0..width {
                let at = (x * 4 + channel) * N;
                row[at..at + N].copy_from_slice(&match source {
                    Some((start, step)) => read(start.add(x * step)),
                    None => one,
                });
            }
        }
    }
    out
}

/// 8-bit y'cbcr, as jpeg stores it (jfif): full range and bt.601, whatever the frame is tagged. chroma is read at the
/// nearest sample, as for video.
unsafe fn from_ycbcr(f: &ffi::AVFrame, desc: &ffi::AVPixFmtDescriptor, alpha: Option<ffi::AVComponentDescriptor>) -> Vec<u8> {
    let c = color::conversion(Stored { bits: 8, texel_bits: 8, top_aligned: false, full_range: true }, Matrix::Bt601);
    let (width, height) = (f.width as usize, f.height as usize);
    let at = |comp: ffi::AVComponentDescriptor, x: usize, y: usize| {
        let plane = comp.plane as usize;
        *f.data[plane].offset(y as isize * f.linesize[plane] as isize + (comp.offset as usize + x * comp.step as usize) as isize)
    };
    let (sx, sy) = (desc.log2_chroma_w as u32, desc.log2_chroma_h as u32);
    let mut out = vec![0u8; width * height * 4];
    for (y, row) in out.chunks_exact_mut(width * 4).enumerate() {
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let luma = at(desc.comp[0], x, y) as f32 / 255.0 * c.y_scale + c.y_offset;
            let cb = at(desc.comp[1], x >> sx, y >> sy) as f32 / 255.0 * c.c_scale + c.c_offset;
            let cr = at(desc.comp[2], x >> sx, y >> sy) as f32 / 255.0 * c.c_scale + c.c_offset;
            let rgb = [luma + c.cr_to_r * cr, luma + c.cb_to_g * cb + c.cr_to_g * cr, luma + c.cb_to_b * cb];
            for (to, v) in pixel.iter_mut().zip(rgb) {
                // rounded by adding a half: a float cast to an integer saturates, so below 0 is 0 and above 255 is 255
                *to = (v * 255.0 + 0.5) as u8;
            }
            pixel[3] = alpha.map_or(255, |a| at(a, x, y));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(name: &str) -> String {
        format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    /// pixel (x, y) as its four numbers, whatever they are stored as.
    fn px(image: &Image, x: usize, y: usize) -> [f64; 4] {
        let size = match image.samples {
            Samples::U8 => 1,
            Samples::U16 => 2,
            Samples::F32 => 4,
        };
        let at = (y * image.width + x) * 4 * size;
        std::array::from_fn(|i| {
            let b = &image.data[at + i * size..][..size];
            match image.samples {
                Samples::U8 => b[0] as f64,
                Samples::U16 => u16::from_ne_bytes([b[0], b[1]]) as f64,
                Samples::F32 => f32::from_ne_bytes([b[0], b[1], b[2], b[3]]) as f64,
            }
        })
    }

    #[test]
    fn the_pinned_ffmpeg_has_every_decoder() {
        let missing: Vec<&str> = FORMATS.iter().filter(|f| unsafe { ffi::avcodec_find_decoder(f.codec) }.is_null()).map(|f| f.name).collect();
        assert!(missing.is_empty(), "no decoder for {missing:?}");
    }

    /// each test still is a pattern of known numbers (tests/media/make.ps1). pixels (3, 5) and (15, 0) are read: a
    /// picture upside down, mirrored or with its channels swapped gives other numbers.
    #[test]
    fn every_format_decodes_to_its_size_depth_and_numbers() {
        let w10 = |v| widen(v, 10) as f64;
        let cases = [
            ("still_8bit.png", "png", Samples::U8, false, 8, [48.0, 80.0, 128.0, 255.0], [240.0, 0.0, 128.0, 255.0]),
            ("still.tga", "tga", Samples::U8, true, 8, [48.0, 80.0, 128.0, 63.0], [240.0, 0.0, 128.0, 255.0]),
            ("still_16bit.png", "png", Samples::U16, false, 16, [12293.0, 20483.0, 1234.0, 65535.0], [61440.0, 15.0, 1234.0, 65535.0]),
            ("still_16bit.tif", "tiff", Samples::U16, true, 16, [12293.0, 20483.0, 1234.0, 65532.0], [61440.0, 15.0, 1234.0, 65520.0]),
            ("still_10bit.dpx", "dpx", Samples::U16, false, 10, [w10(197), w10(320), w10(777), 65535.0], [w10(960), 0.0, w10(777), 65535.0]),
            ("still_gray.png", "png", Samples::U16, false, 16, [12293.0, 12293.0, 12293.0, 65535.0], [61440.0, 61440.0, 61440.0, 65535.0]),
            // half floats, and a blue above 1.0 that nothing may clip
            ("still_half.exr", "exr", Samples::F32, false, 16, [0.375, 0.3125, 2.5, 1.0], [1.875, 0.0, 2.5, 1.0]),
            // exr stores color premultiplied by alpha, and ffmpeg's encoder did so: the numbers come back as stored
            ("still_float.exr", "exr", Samples::F32, true, 32, [0.1875, 0.15625, 1.25, 0.5], [0.9375, 0.0, 1.25, 0.5]),
        ];
        for (name, codec, samples, has_alpha, bits, at_3_5, at_15_0) in cases {
            let image = decode(&media(name)).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!((image.width, image.height, image.codec, image.samples, image.has_alpha, image.bits), (16, 16, codec, samples, has_alpha, bits), "{name}");
            assert_eq!(image.data.len(), 16 * 16 * 4 * [1, 2, 4][samples as usize], "{name}");
            assert_eq!((px(&image, 3, 5), px(&image, 15, 0)), (at_3_5, at_15_0), "{name}");
        }
    }

    #[test]
    fn jpeg_comes_back_close_to_its_color() {
        let image = decode(&media("still.jpg")).expect("decode");
        assert_eq!((image.codec, image.samples, image.has_alpha, image.bits), ("jpeg", Samples::U8, false, 8));
        for (x, y) in [(0, 0), (8, 8), (15, 15)] {
            let got = px(&image, x, y);
            assert!(got.iter().zip([51.0, 102.0, 153.0, 255.0]).all(|(g, e)| (g - e).abs() <= 3.0), "({x}, {y}): {got:?}");
        }
    }

    #[test]
    fn exr_states_its_rate_and_the_rest_do_not() {
        assert_eq!(decode(&media("still_float.exr")).unwrap().rate, Some((30000, 1001)));
        // made at ffmpeg's default of 25 a second
        assert_eq!(decode(&media("still_half.exr")).unwrap().rate, Some((25, 1)));
        assert_eq!(decode(&media("still_10bit.dpx")).unwrap().rate, None);
        assert_eq!(decode(&media("still_8bit.png")).unwrap().rate, None);
    }

    #[test]
    fn widening_keeps_both_ends_and_shifts_back() {
        for bits in [10, 12, 16] {
            let top = ((1u32 << bits) - 1) as u16;
            assert_eq!((widen(0, bits), widen(top, bits)), (0, 65535));
            assert!((0..=top).all(|v| widen(v, bits) >> (16 - bits) == v));
        }
    }

    #[test]
    fn halves_are_exact() {
        assert_eq!([half(0x0000), half(0x3c00), half(0xc000), half(0x4100), half(0x7bff)], [0.0, 1.0, -2.0, 2.5, 65504.0]);
        // the smallest number below the usual form, and infinity
        assert_eq!((half(0x0001), half(0x7c00)), (2f32.powi(-24), f32::INFINITY));
        assert!(half(0x7e00).is_nan());
    }

    #[test]
    fn what_is_not_a_still_is_an_error() {
        let e = decode(&media("hap_64.frame0.rgb")).err().expect("an error");
        assert!(e.ends_with("not a still this build reads (png, jpeg, tiff, tga, exr, dpx)"), "{e}");
        assert!(decode(&media("no such file.png")).is_err());
        assert!(is_still("a.EXR") && is_still("b.jpeg") && !is_still("c.mov") && !is_still("png"));
    }
}
