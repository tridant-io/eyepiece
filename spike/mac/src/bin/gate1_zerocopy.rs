//! gate 1 on macos: does a hardware-decoded 10-bit hevc frame reach the renderer with no cpu copy?
//! usage: gate1_zerocopy <hevc main10 file> [frames]
//! decodes with videotoolbox, then draws by sampling the decoder's own surface through metal textures made on it.
//! to check the picture, the same frames are decoded in software and compared: the decoder's planes, read through the
//! gpu, must match bit for bit, and the drawn pixels must be within one 10-bit code value of the same math on the cpu.
//! draws into a texture, not a window: this checks the path, and puts nothing on the screen.

use objc2::rc::Retained;
use objc2_metal::{MTLPixelFormat, MTLTexture, MTLTextureUsage};
use spike::dec::Decoder;
use spike::ff::{self, AVPixelFormat, Input, Packet};
use spike_mac::{read, Gpu, PixelBuffer};
use std::ffi::c_void;

/// one plane, as stored: integer addressing, no filter.
const COPY: &str = r#"
fragment float4 fs(V in [[stage_in]], texture2d<float> t [[texture(0)]]) {
    return t.read(uint2(in.pos.xy));
}
"#;

/// 10-bit biplanar yuv to rgb, bt.709, video range. integer addressing: no filter anywhere. chroma is the nearest sample.
const DRAW: &str = r#"
fragment float4 fs(V in [[stage_in]], texture2d<float> ty [[texture(0)]], texture2d<float> tc [[texture(1)]]) {
    uint2 p = uint2(in.pos.xy);
    float y = ty.read(p).r * (65535.0 / 64.0);
    float2 c = tc.read(p / 2).rg * (65535.0 / 64.0);
    float Y = (y - 64.0) / 876.0, Cb = (c.x - 512.0) / 896.0, Cr = (c.y - 512.0) / 896.0;
    return float4(saturate(float3(Y + 1.5748 * Cr, Y - 0.1873 * Cb - 0.4681 * Cr, Y + 1.8556 * Cb)), 1.0);
}
"#;

fn to10(v: f32) -> i32 {
    (v.clamp(0.0, 1.0) * 1023.0 + 0.5).floor() as i32
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gate1_zerocopy <hevc main10 file> [frames]");
        std::process::exit(2);
    };
    let frames: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(8);
    println!("ffmpeg {}", ff::version());

    let hw_in = Input::open(path, false).expect("open");
    let sw_in = Input::open(path, false).expect("open");
    let v = hw_in.first_video().expect("video stream");
    let (vw, vh) = (v.width as usize, v.height as usize);
    // the top-left of the picture at 1:1, so drawn pixels map to video pixels with no scaling
    let (w, h) = (vw.min(1920), vh.min(1080));

    let gpu = Gpu::new();
    let target_usage = MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead;
    let drawn_tex = gpu.texture(MTLPixelFormat::BGR10A2Unorm, w, h, target_usage).expect("target");
    let luma_out = gpu.texture(MTLPixelFormat::R16Unorm, vw, vh, target_usage).expect("target");
    let chroma_out = gpu.texture(MTLPixelFormat::RG16Unorm, vw / 2, vh / 2, target_usage).expect("target");
    let draw = gpu.pipeline(DRAW, MTLPixelFormat::BGR10A2Unorm);
    let copy_luma = gpu.pipeline(COPY, MTLPixelFormat::R16Unorm);
    let copy_chroma = gpu.pipeline(COPY, MTLPixelFormat::RG16Unorm);

    let mut hw = Decoder::open_videotoolbox(&hw_in, v.index).expect("hardware decoder");
    let mut sw = Decoder::open_software(&sw_in, v.index, 0).expect("software decoder");
    let (mut hp, mut sp) = (Packet::new(), Packet::new());

    let (mut plane_diffs, mut worst, mut over1, mut compared, mut done) = (0u64, 0i32, 0u64, 0u64, 0usize);
    let (mut fmt_ok, mut same_memory) = (true, true);
    for i in 0..frames {
        if !hw.next(&hw_in, &mut hp, v.index).expect("hw decode") || !sw.next(&sw_in, &mut sp, v.index).expect("sw decode") {
            break;
        }
        done += 1;
        fmt_ok &= hw.format() == AVPixelFormat::AV_PIX_FMT_VIDEOTOOLBOX && sw.format() == AVPixelFormat::AV_PIX_FMT_YUV420P10LE;
        if !fmt_ok {
            println!("frame {i}: hardware frame format {:?}, software {:?}", hw.format(), sw.format());
            break;
        }
        let pb = PixelBuffer(hw.pixel_buffer());
        // the whole display path: two textures made on the decoder's surface, one draw. nothing is copied to system memory.
        let (Some(luma), Some(chroma)) = (pb.plane(&gpu, 0, MTLPixelFormat::R16Unorm), pb.plane(&gpu, 1, MTLPixelFormat::RG16Unorm)) else {
            println!("frame {i}: the decoder's buffer (format {}) could not be wrapped as metal textures", pb.format());
            fmt_ok = false;
            break;
        };
        gpu.draw(&draw, &drawn_tex, &[&luma, &chroma]);

        // ---- checking only, from here to the end of the loop ----
        let on_surface = |t: &spike_mac::Tex| t.iosurface().is_some_and(|s| Retained::as_ptr(&s) as *const c_void == pb.surface() as *const c_void);
        same_memory &= on_surface(&luma) && on_surface(&chroma);
        if i == 0 {
            println!(
                "decoder buffer: pixel format '{}', luma {}x{}, chroma {}x{}, textures on the decoder's own surface: {}",
                pb.format(), luma.width(), luma.height(), chroma.width(), chroma.height(), same_memory
            );
        }
        gpu.draw(&copy_luma, &luma_out, &[&luma]);
        gpu.draw(&copy_chroma, &chroma_out, &[&chroma]);
        let (hy, hc, drawn) = (read(&luma_out, 2), read(&chroma_out, 4), read(&drawn_tex, 4));
        let at16 = |b: &[u8], i: usize| u16::from_le_bytes([b[i * 2], b[i * 2 + 1]]) >> 6;
        let (sy, ys) = sw.plane16(0, vh);
        let (su, us) = sw.plane16(1, vh / 2);
        let (sv, vs) = sw.plane16(2, vh / 2);
        for y in 0..vh {
            for x in 0..vw {
                plane_diffs += (at16(&hy, y * vw + x) != sy[y * ys + x]) as u64;
            }
        }
        for y in 0..vh / 2 {
            for x in 0..vw / 2 {
                let k = (y * (vw / 2) + x) * 2;
                plane_diffs += (at16(&hc, k) != su[y * us + x]) as u64 + (at16(&hc, k + 1) != sv[y * vs + x]) as u64;
            }
        }
        for y in 0..h {
            for x in 0..w {
                let (yy, cb, cr) = (sy[y * ys + x] as f32, su[(y / 2) * us + x / 2] as f32, sv[(y / 2) * vs + x / 2] as f32);
                let (ly, lb, lr) = ((yy - 64.0) / 876.0, (cb - 512.0) / 896.0, (cr - 512.0) / 896.0);
                let want = [to10(ly + 1.5748 * lr), to10(ly - 0.1873 * lb - 0.4681 * lr), to10(ly + 1.8556 * lb)];
                let k = (y * w + x) * 4;
                let p = u32::from_le_bytes([drawn[k], drawn[k + 1], drawn[k + 2], drawn[k + 3]]);
                // red sits in bits 20 to 29 of this format, blue in the low ten
                let got = [((p >> 20) & 1023) as i32, ((p >> 10) & 1023) as i32, (p & 1023) as i32];
                for c in 0..3 {
                    let d = (want[c] - got[c]).abs();
                    worst = worst.max(d);
                    over1 += (d > 1) as u64;
                    compared += 1;
                }
            }
        }
    }
    println!("frames checked: {done} of {frames} · picture {vw}x{vh} · drawn {w}x{h} at 1:1");
    println!("decoder planes read through the gpu, hardware against software: {plane_diffs} samples differ");
    println!("drawn pixels against the same math on the cpu: {compared} values compared, worst difference {worst} code values, {over1} off by more than 1");
    println!("cpu copies on the display path: 0 (the read-backs above exist only for this check)");
    let pass = fmt_ok && same_memory && done == frames && plane_diffs == 0 && over1 == 0;
    std::process::exit(spike::verdict(pass, "the renderer draws from the decoder's own surface, and the picture matches a software decode"));
}
