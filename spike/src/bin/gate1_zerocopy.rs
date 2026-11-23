//! gate 1: does a hardware-decoded 10-bit hevc frame reach the screen with no cpu copy?
//! usage: gate1_zerocopy <hevc main10 file> [frames]
//! decodes on the gpu with direct3d 11 video on OUR device, then draws by sampling the decoder's own texture where it lies.
//! to check the picture, the same frames are decoded in software and compared: the decoder's planes must match bit for bit,
//! and the drawn pixels must be within one 10-bit code value of the same math done on the cpu.

use spike::dec::Decoder;
use spike::ff::{self, AVPixelFormat, Input, Packet};
use spike::{d3d, win};
use windows::Win32::Graphics::Direct3D11::{D3D11_BIND_DECODER, D3D11_BIND_SHADER_RESOURCE, D3D11_TEXTURE2D_DESC};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_P010, DXGI_FORMAT_R16G16_UNORM, DXGI_FORMAT_R16_UNORM};

/// p010 to rgb, bt.709, video range, 10-bit. integer addressing: no filter anywhere. chroma is the nearest sample.
const PS: &str = "
Texture2DArray<float> ty : register(t0);
Texture2DArray<float2> tc : register(t1);
float4 main(float4 pos : SV_Position) : SV_Target {
    int2 p = int2(pos.xy);
    float y = ty.Load(int4(p, 0, 0)) * (65535.0 / 64.0);
    float2 c = tc.Load(int4(p / 2, 0, 0)) * (65535.0 / 64.0);
    float Y = (y - 64.0) / 876.0, Cb = (c.x - 512.0) / 896.0, Cr = (c.y - 512.0) / 896.0;
    return float4(saturate(float3(Y + 1.5748 * Cr, Y - 0.1873 * Cb - 0.4681 * Cr, Y + 1.8556 * Cb)), 1);
}";

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
    // a 1:1 window on the top-left of the picture, so drawn pixels map to video pixels with no scaling
    let (sw_, sh_) = win::screen_size();
    let (w, h) = (vw.min(sw_ as usize - 200).min(1920) as u32, vh.min(sh_ as usize - 200).min(1080) as u32);
    let hwnd = win::window(100, 100, w as i32, h as i32).expect("window");
    let (device, ctx) = d3d::device().expect("device");
    let gpu = d3d::Gpu::new(device.clone(), ctx.clone(), hwnd, w, h, true).expect("swapchain");
    let ps = gpu.pixel_shader(PS).expect("shader");

    let mut hw = Decoder::open(&hw_in, v.index, Some(&device), 1).expect("hardware decoder");
    let mut sw = Decoder::open(&sw_in, v.index, None, 0).expect("software decoder");
    let (mut hp, mut sp) = (Packet::new(), Packet::new());

    let (mut plane_diffs, mut worst, mut over1, mut compared, mut done) = (0u64, 0i32, 0u64, 0u64, 0usize);
    let mut fmt_ok = true;
    for i in 0..frames {
        if !hw.next(&hw_in, &mut hp, v.index).expect("hw decode") || !sw.next(&sw_in, &mut sp, v.index).expect("sw decode") {
            break;
        }
        done += 1;
        let is_d3d = hw.format() == AVPixelFormat::AV_PIX_FMT_D3D11;
        fmt_ok &= is_d3d && sw.format() == AVPixelFormat::AV_PIX_FMT_YUV420P10LE;
        if !fmt_ok {
            println!("frame {i}: hardware frame format {:?}, software {:?}", hw.format(), sw.format());
            break;
        }
        let (tex, slice) = hw.texture();
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        unsafe { tex.GetDesc(&mut desc) };
        if i == 0 {
            println!(
                "decoder texture: {}x{} array of {}, format {} (P010 = {}), decoder-bound {}, shader-readable {}",
                desc.Width, desc.Height, desc.ArraySize, desc.Format.0, DXGI_FORMAT_P010.0,
                desc.BindFlags & D3D11_BIND_DECODER.0 as u32 != 0, desc.BindFlags & D3D11_BIND_SHADER_RESOURCE.0 as u32 != 0
            );
        }
        // the whole display path: two views onto the decoder's texture, one draw. nothing is copied to system memory.
        let luma = d3d::plane_view(&device, &tex, slice, DXGI_FORMAT_R16_UNORM).expect("luma view");
        let chroma = d3d::plane_view(&device, &tex, slice, DXGI_FORMAT_R16G16_UNORM).expect("chroma view");
        gpu.draw(&ps, &[Some(luma), Some(chroma)]);

        // ---- checking only, from here to the present ----
        let drawn = gpu.read_backbuffer().expect("read back buffer");
        let (_, (hwbytes, pitch)) = unsafe { d3d::read_texture(&device, &ctx, &tex, slice, vw as u32, vh as u32).expect("read decoder texture") };
        let (sy, ys) = sw.plane16(0, vh);
        let (su, us) = sw.plane16(1, vh / 2);
        let (sv, vs) = sw.plane16(2, vh / 2);
        let hw16 = |row: usize, x: usize| u16::from_le_bytes([hwbytes[row * pitch + x * 2], hwbytes[row * pitch + x * 2 + 1]]) >> 6;
        for y in 0..vh {
            for x in 0..vw {
                plane_diffs += (hw16(y, x) != sy[y * ys + x]) as u64;
            }
        }
        for y in 0..vh / 2 {
            for x in 0..vw / 2 {
                plane_diffs += (hw16(vh + y, x * 2) != su[y * us + x]) as u64 + (hw16(vh + y, x * 2 + 1) != sv[y * vs + x]) as u64;
            }
        }
        for y in 0..h as usize {
            for x in 0..w as usize {
                let (yy, cb, cr) = (sy[y * ys + x] as f32, su[(y / 2) * us + x / 2] as f32, sv[(y / 2) * vs + x / 2] as f32);
                let (ly, lb, lr) = ((yy - 64.0) / 876.0, (cb - 512.0) / 896.0, (cr - 512.0) / 896.0);
                let want = [to10(ly + 1.5748 * lr), to10(ly - 0.1873 * lb - 0.4681 * lr), to10(ly + 1.8556 * lb)];
                let p = drawn[y * w as usize + x];
                let got = [(p & 1023) as i32, ((p >> 10) & 1023) as i32, ((p >> 20) & 1023) as i32];
                for c in 0..3 {
                    let d = (want[c] - got[c]).abs();
                    worst = worst.max(d);
                    over1 += (d > 1) as u64;
                    compared += 1;
                }
            }
        }
        let _ = gpu.present(1);
        if !win::pump() {
            break;
        }
    }
    println!("frames checked: {done} of {frames} · picture {vw}x{vh} · window {w}x{h} at 1:1");
    println!("decoder planes, hardware against software: {plane_diffs} samples differ");
    println!("drawn pixels against the same math on the cpu: {compared} values compared, worst difference {worst} code values, {over1} off by more than 1");
    println!("cpu copies on the display path: 0 (the two read-backs above exist only for this check)");
    std::thread::sleep(std::time::Duration::from_millis(400));
    win::close(hwnd);
    let pass = fmt_ok && done == frames && plane_diffs == 0 && over1 == 0;
    std::process::exit(spike::verdict(pass, "the renderer draws from the decoder's own texture, and the picture matches a software decode"));
}
