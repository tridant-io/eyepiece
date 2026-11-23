//! gate 4: how long is a cold start, stage by stage?
//! usage: gate4_coldstart <file> [runs, default 20]     launches itself that many times and reports each stage
//!        gate4_coldstart --child <file>                one launch: open, show the first frame, print one STAGES line
//! every time is milliseconds since the os created the process, so dll loading before main() is counted.
//! the order is the plan's: gpu on its own thread from the first instant, file header, window at the clip's shape,
//! first frame decoded while the gpu is still starting, then upload and present.
//! measured, not judged. the plan's target for a warm launch on a local hap or prores file is 250 ms.

use spike::dec::Decoder;
use spike::ff::{self, AVPixelFormat, Input, Packet};
use spike::hap::{self, Tex};
use spike::win::ms_since_process_start as now;
use spike::{d3d, win};
use std::process::Command;
use windows::Win32::Graphics::Direct3D11::{ID3D11Device, ID3D11DeviceContext, ID3D11ShaderResourceView};
use windows::Win32::Graphics::Dwm::DwmFlush;
use windows::Win32::Graphics::Dxgi::Common::*;

struct SendGpu(ID3D11Device, ID3D11DeviceContext);
unsafe impl Send for SendGpu {}

/// a pixel shader that maps window pixels onto picture pixels by nearest sample, then converts.
fn shader(kind: &str, sx: f64, sy: f64) -> String {
    let head = format!("static const float2 S = float2({sx:.9}, {sy:.9});\n");
    head.clone() + match kind {
        "ycocg" => "Texture2D<float4> t : register(t0);
float4 main(float4 pos : SV_Position) : SV_Target {
    float4 v = t.Load(int3(pos.xy * S, 0)) + float4(-0.50196078431373, -0.50196078431373, 0, 0);
    float scale = v.z * (255.0 / 8.0) + 1.0; float co = v.x / scale, cg = v.y / scale, y = v.w;
    return float4(y + co - cg, y + cg, y - co - cg, 1);
}",
        "rgb" => "Texture2D<float4> t : register(t0); float4 main(float4 pos : SV_Position) : SV_Target { return float4(t.Load(int3(pos.xy * S, 0)).rgb, 1); }",
        // planar 4:2:2 10-bit or 4:4:4 12-bit in 16-bit textures, bt.709 video range
        "p422_10" | "p444_12" => {
            return head
                + &format!(
                    "Texture2D<float> ty : register(t0); Texture2D<float> tu : register(t1); Texture2D<float> tv : register(t2);
float4 main(float4 pos : SV_Position) : SV_Target {{
    int2 p = int2(pos.xy * S); int2 c = int2(p.x {}, p.y);
    float m = {};
    float Y = (ty.Load(int3(p, 0)) * 65535.0 - 64.0 * m) / (876.0 * m), Cb = (tu.Load(int3(c, 0)) * 65535.0 - 512.0 * m) / (896.0 * m), Cr = (tv.Load(int3(c, 0)) * 65535.0 - 512.0 * m) / (896.0 * m);
    return float4(saturate(float3(Y + 1.5748 * Cr, Y - 0.1873 * Cb - 0.4681 * Cr, Y + 1.8556 * Cb)), 1);
}}",
                    if kind == "p422_10" { "/ 2" } else { "" },
                    if kind == "p422_10" { "1.0" } else { "4.0" }
                )
        }
        // the hardware decoder's texture: p010 or nv12
        _ => "Texture2DArray<float> ty : register(t0); Texture2DArray<float2> tc : register(t1);
float4 main(float4 pos : SV_Position) : SV_Target {
    int2 p = int2(pos.xy * S);
    float y = ty.Load(int4(p, 0, 0)) * (65535.0 / 64.0); float2 c = tc.Load(int4(p / 2, 0, 0)) * (65535.0 / 64.0);
    float Y = (y - 64.0) / 876.0, Cb = (c.x - 512.0) / 896.0, Cr = (c.y - 512.0) / 896.0;
    return float4(saturate(float3(Y + 1.5748 * Cr, Y - 0.1873 * Cb - 0.4681 * Cr, Y + 1.8556 * Cb)), 1);
}",
    }
}

fn bc_format(t: Tex) -> DXGI_FORMAT {
    match t {
        Tex::Bc1 => DXGI_FORMAT_BC1_UNORM,
        Tex::Bc3 | Tex::YCoCgBc3 => DXGI_FORMAT_BC3_UNORM,
        Tex::Bc7 => DXGI_FORMAT_BC7_UNORM,
        Tex::Bc4 => DXGI_FORMAT_BC4_UNORM,
        Tex::Bc6u => DXGI_FORMAT_BC6H_UF16,
        Tex::Bc6s => DXGI_FORMAT_BC6H_SF16,
    }
}

fn child(path: &str, cpu_first: bool) {
    let t_main = now();
    // for a cpu-painted first frame the software device has to exist before the hardware one starts (they serialize)
    let warp = if cpu_first { Some(d3d::warp().expect("software device")) } else { None };
    let t_warp = now();
    // the gpu starts on its own thread before anything else
    let gpu_thread = std::thread::spawn(|| {
        let (d, c) = d3d::device().expect("device");
        (SendGpu(d, c), now())
    });

    // the file's header: enough for the shape of the window
    let input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video");
    let (vw, vh) = (v.width as usize, v.height as usize);
    let t_open = now();

    // the window, at the clip's shape, fitted to 90% of the screen
    win::dpi_aware();
    let (sw, sh) = win::screen_size();
    let fit = (0.9 * sw as f64 / vw as f64).min(0.9 * sh as f64 / vh as f64).min(1.0);
    let (w, h) = ((vw as f64 * fit).round() as u32, (vh as f64 * fit).round() as u32);
    let hwnd = win::window((sw - w as i32) / 2, (sh - h as i32) / 2, w as i32, h as i32).expect("window");
    win::pump();
    let t_window = now();

    // the first frame, by the lane its codec takes
    let tag = ff::fourcc(v.tag);
    let is_hap = tag.starts_with("Hap");
    let hardware = !is_hap && matches!(v.codec_name.as_str(), "hevc" | "h264");
    let mut pkt = Packet::new();
    let (mut hap_blocks, mut hap_tex) = (Vec::new(), Tex::Bc1);
    let mut sw_dec: Option<Decoder> = None;
    let mut t_decoded = 0.0;
    if is_hap {
        // ffmpeg supplies only where the frame is; we read it ourselves and decompress in ordinary ram
        let (pos, size) = input.index(v.index)[0];
        let mut raw = vec![0u8; size];
        let f = std::fs::File::open(path).expect("open");
        std::os::windows::fs::FileExt::seek_read(&f, &mut raw, pos).expect("read");
        let layers = hap::parse(&raw).expect("hap");
        hap_tex = layers[0].tex;
        hap_blocks = vec![0u8; layers[0].len];
        hap::decompress(&layers[0], &mut hap_blocks, 4).expect("decompress");
        t_decoded = now();
    } else if !hardware {
        let mut d = Decoder::open(&input, v.index, None, 0).expect("software decoder");
        assert!(d.next(&input, &mut pkt, v.index).expect("decode"), "no frame");
        sw_dec = Some(d);
        t_decoded = now();
    }

    // optional: paint this first frame with the software rasterizer now, instead of waiting for the hardware device
    let (mut t_cpu, mut cpu_parts) = (0.0, [0.0f64; 3]);
    if cpu_first && (is_hap || sw_dec.is_some()) {
        let (sx, sy) = (vw as f64 / w as f64, vh as f64 / h as f64);
        let planes: Vec<d3d::CpuPlane> = if is_hap {
            let (pw, ph) = hap::padded(vw, vh);
            vec![d3d::CpuPlane { w: pw as u32, h: ph as u32, format: bc_format(hap_tex), data: &hap_blocks, pitch: (pw / 4 * hap_tex.block_bytes()) as u32 }]
        } else {
            let d = sw_dec.as_ref().unwrap();
            let cw = if d.format() == AVPixelFormat::AV_PIX_FMT_YUV422P10LE { vw / 2 } else { vw };
            [(0usize, vw), (1, cw), (2, cw)].iter().map(|&(i, pw)| { let (px, stride) = d.plane8(i, vh); d3d::CpuPlane { w: pw as u32, h: vh as u32, format: DXGI_FORMAT_R16_UNORM, data: px, pitch: stride as u32 } }).collect()
        };
        let kind = if is_hap { if hap_tex == Tex::YCoCgBc3 { "ycocg" } else { "rgb" } } else if sw_dec.as_ref().unwrap().format() == AVPixelFormat::AV_PIX_FMT_YUV422P10LE { "p422_10" } else { "p444_12" };
        cpu_parts = d3d::cpu_first_frame(warp.as_ref().unwrap(), hwnd, w, h, &shader(kind, sx, sy), &planes).expect("cpu first frame");
        t_cpu = now();
    }

    let (SendGpu(device, ctx), t_device) = gpu_thread.join().expect("gpu thread");
    let t_joined = now();
    let mut hw_dec: Option<Decoder> = None;
    if hardware {
        let mut d = Decoder::open(&input, v.index, Some(&device), 1).expect("hardware decoder");
        assert!(d.next(&input, &mut pkt, v.index).expect("decode"), "no frame");
        hw_dec = Some(d);
        t_decoded = now();
    }

    let gpu = d3d::Gpu::new(device.clone(), ctx, hwnd, w, h, true).expect("swapchain");
    let (sx, sy) = (vw as f64 / w as f64, vh as f64 / h as f64);
    let t_swap = now();

    // upload, or for the hardware lane just two views
    let (kind, views): (&str, Vec<Option<ID3D11ShaderResourceView>>) = if is_hap {
        let (pw, ph) = hap::padded(vw, vh);
        let (_t, view) = d3d::texture_with_data(&device, pw as u32, ph as u32, bc_format(hap_tex), &hap_blocks, (pw / 4 * hap_tex.block_bytes()) as u32).expect("texture");
        (if hap_tex == Tex::YCoCgBc3 { "ycocg" } else { "rgb" }, vec![Some(view)])
    } else if let Some(d) = &sw_dec {
        let (kind, cw) = match d.format() {
            AVPixelFormat::AV_PIX_FMT_YUV422P10LE => ("p422_10", vw / 2),
            AVPixelFormat::AV_PIX_FMT_YUV444P12LE => ("p444_12", vw),
            other => panic!("software lane: unhandled pixel format {other:?}"),
        };
        let mut views = Vec::new();
        for (i, pw) in [(0usize, vw), (1, cw), (2, cw)] {
            let (px, stride) = d.plane8(i, vh);
            views.push(Some(d3d::texture_with_data(&device, pw as u32, vh as u32, DXGI_FORMAT_R16_UNORM, px, stride as u32).expect("plane").1));
        }
        (kind, views)
    } else {
        let d = hw_dec.as_ref().unwrap();
        let (tex, slice) = d.texture();
        let mut desc = Default::default();
        unsafe { tex.GetDesc(&mut desc) };
        let (fy, fc) = if desc.Format == DXGI_FORMAT_NV12 { (DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_UNORM) } else { (DXGI_FORMAT_R16_UNORM, DXGI_FORMAT_R16G16_UNORM) };
        ("hw", vec![Some(d3d::plane_view(&device, &tex, slice, fy).expect("view")), Some(d3d::plane_view(&device, &tex, slice, fc).expect("view"))])
    };
    let ps = gpu.pixel_shader(&shader(kind, sx, sy)).expect("shader");
    let t_upload = now();

    gpu.draw(&ps, &views);
    let _ = gpu.present(0);
    let t_present = now();
    unsafe {
        let _ = DwmFlush();
    }
    let t_visible = now();
    println!(
        "STAGES warp={t_warp:.1} main={t_main:.1} open={t_open:.1} window={t_window:.1} decoded={t_decoded:.1} device={t_device:.1} joined={t_joined:.1} swapchain={t_swap:.1} upload={t_upload:.1} present={t_present:.1} visible={t_visible:.1} cpufirst={t_cpu:.1} cpudev={:.1} cpudraw={:.1} cpublit={:.1} lane={}",
        cpu_parts[0], cpu_parts[1], cpu_parts[2],
        if is_hap { "hap" } else if hardware { "hardware" } else { "software" }
    );
    win::pump();
    std::thread::sleep(std::time::Duration::from_millis(150));
    win::close(hwnd);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        child(&args[1], args.iter().any(|a| a == "--cpu-first"));
        return;
    }
    let Some(path) = args.first() else {
        eprintln!("usage: gate4_coldstart <file> [runs]");
        std::process::exit(2);
    };
    let runs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let cpu_first = args.iter().any(|a| a == "--cpu-first");
    let exe = std::env::current_exe().unwrap();
    let names = ["main", "open", "window", "decoded", "cpufirst", "device", "joined", "swapchain", "upload", "present", "visible", "cpudev", "cpudraw", "cpublit"];
    let mut rows: Vec<Vec<f64>> = Vec::new();
    let mut lane = String::new();
    for _ in 0..runs {
        let out = Command::new(&exe).args(if cpu_first { vec!["--child", path.as_str(), "--cpu-first"] } else { vec!["--child", path.as_str()] }).output().expect("launch");
        let text = String::from_utf8_lossy(&out.stdout);
        let Some(line) = text.lines().find(|l| l.starts_with("STAGES")) else {
            eprintln!("a launch failed: {}", String::from_utf8_lossy(&out.stderr));
            continue;
        };
        let get = |k: &str| line.split_whitespace().find_map(|p| p.strip_prefix(&format!("{k}="))).unwrap_or("0").to_string();
        rows.push(names.iter().map(|n| get(n).parse().unwrap_or(0.0)).collect());
        lane = get("lane");
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    if rows.len() < 3 {
        std::process::exit(spike::verdict(false, "too few launches succeeded"));
    }
    let file = std::path::Path::new(path).file_name().unwrap().to_string_lossy();
    println!("{file} · {lane} lane · {} launches, ms since the os created the process", rows.len());
    println!("{:<34} {:>9} {:>9} {:>9}", "reached", "first run", "p50", "p95");
    let labels = ["main() entered (exe and dlls loaded)", "file header read", "window on screen", "first frame decoded", "FIRST FRAME PAINTED BY THE CPU", "gpu device ready (own thread)", "gpu thread joined", "swapchain made", "frame uploaded, shader ready", "present() returned", "next composition (frame visible)", "  (cpu paint: shaders)", "  (cpu paint: draw + read back)", "  (cpu paint: gdi blit)"];
    for (i, label) in labels.iter().enumerate() {
        let mut later: Vec<f64> = rows[1..].iter().map(|r| r[i]).collect();
        let (p50, p95) = spike::p50_p95(&mut later);
        println!("{label:<34} {:>9.1} {p50:>9.1} {p95:>9.1}", rows[0][i]);
    }
    println!("measured, not judged. the plan's target for a warm launch is 250 ms to the first frame.");
}
