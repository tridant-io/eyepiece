//! measured alongside: the hap lane's real ceiling.
//! usage: hap_speed <hap file> [frames to hold in ram, default 60]
//! reads packets with ffmpeg's demuxer, decompresses snappy chunks on 1..n threads, uploads the blocks as stored,
//! and reports each stage. also draws frame 0 and compares it with ffmpeg's own (cpu) hap decoder.
//! nothing is judged here: it prints numbers.

use spike::dec::Decoder;
use spike::ff::{self, Input, Packet};
use spike::hap::{self, Tex};
use spike::{d3d, win};
use std::time::Instant;
use windows::Win32::Graphics::Dxgi::Common::*;

const PS_YCOCG: &str = "
Texture2D<float4> t : register(t0);
float4 main(float4 pos : SV_Position) : SV_Target {
    float4 v = t.Load(int3(pos.xy, 0)) + float4(-0.50196078431373, -0.50196078431373, 0, 0);
    float scale = v.z * (255.0 / 8.0) + 1.0;
    float co = v.x / scale, cg = v.y / scale, y = v.w;
    return float4(y + co - cg, y + cg, y - co - cg, 1);
}";
const PS_RGB: &str = "Texture2D<float4> t : register(t0); float4 main(float4 pos : SV_Position) : SV_Target { return float4(t.Load(int3(pos.xy, 0)).rgb, 1); }";

fn dxgi(t: Tex) -> DXGI_FORMAT {
    match t {
        Tex::Bc1 => DXGI_FORMAT_BC1_UNORM,
        Tex::Bc3 | Tex::YCoCgBc3 => DXGI_FORMAT_BC3_UNORM,
        Tex::Bc7 => DXGI_FORMAT_BC7_UNORM,
        Tex::Bc4 => DXGI_FORMAT_BC4_UNORM,
        Tex::Bc6u => DXGI_FORMAT_BC6H_UF16,
        Tex::Bc6s => DXGI_FORMAT_BC6H_SF16,
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: hap_speed <hap file> [frames to hold in ram]");
        std::process::exit(2);
    };
    let hold: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    println!("ffmpeg {} · {cores} logical cores", ff::version());

    // 1. the demuxer alone: how fast packets come off the disk
    let input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video");
    let (vw, vh) = (v.width as usize, v.height as usize);
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1.max(1) as f64;
    let mut pkt = Packet::new();
    let t = Instant::now();
    let (mut bytes, mut n, mut held): (u64, usize, Vec<Vec<u8>>) = (0, 0, Vec::new());
    while pkt.read(&input).expect("read") {
        if pkt.stream() != v.index {
            continue;
        }
        bytes += pkt.data().len() as u64;
        n += 1;
        if held.len() < hold {
            held.push(pkt.data().to_vec());
        }
    }
    let secs = t.elapsed().as_secs_f64();
    let per_frame = bytes as f64 / n as f64 / 1048576.0;
    println!("{path}\n  {vw}x{vh} · tag {} · {n} frames at {fps:.2} fps · {per_frame:.1} MB a frame on disk · needs {:.0} MB/s to play", ff::fourcc(v.tag), per_frame * fps);
    println!("  read through the demuxer: {:.0} MB/s ({:.0} frames/s). the file may already be in the os cache.", bytes as f64 / 1048576.0 / secs, n as f64 / secs);

    // 2. the structure of a frame
    let layers = hap::parse(&held[0]).expect("parse");
    for l in &layers {
        println!("  texture {:?} · {} chunk(s) · {:.1} MB decompressed · {:.2}x snappy", l.tex, l.chunks.len(), l.len as f64 / 1048576.0, l.len as f64 / held[0].len() as f64);
    }
    let (pw, ph) = hap::padded(vw, vh);
    let main_layer_len = layers[0].len;
    let expect = pw / 4 * (ph / 4) * layers[0].tex.block_bytes();
    if main_layer_len != expect {
        println!("  texture size {main_layer_len} is not the {expect} expected for {pw}x{ph}");
    }

    // 3. snappy alone, by thread count, into ordinary memory
    let mut buf = vec![0u8; layers.iter().map(|l| l.len).max().unwrap()];
    let mut counts = vec![1, 2, 4, 8, 16, cores];
    counts.retain(|&c| c <= cores);
    counts.dedup();
    for &threads in &counts {
        let t = Instant::now();
        for f in &held {
            for l in hap::parse(f).expect("parse") {
                hap::decompress(&l, &mut buf, threads).expect("decompress");
            }
        }
        let ms = t.elapsed().as_secs_f64() * 1000.0 / held.len() as f64;
        println!("  snappy, {threads:>2} thread(s): {ms:6.2} ms a frame ({:.0} frames/s)", 1000.0 / ms);
    }

    // 4. the whole lane from ram: decompress, upload, draw, present without waiting for vsync
    let (sw, sh) = win::screen_size();
    let (w, h) = (vw.min(sw as usize - 200).min(1920) as u32, vh.min(sh as usize - 200).min(1080) as u32);
    let hwnd = win::window(100, 100, w as i32, h as i32).expect("window");
    let (device, ctx) = d3d::device().expect("device");
    let gpu = d3d::Gpu::new(device.clone(), ctx.clone(), hwnd, w, h, false).expect("swapchain");
    let tex0 = layers[0].tex;
    let ps = gpu.pixel_shader(if tex0 == Tex::YCoCgBc3 { PS_YCOCG } else { PS_RGB }).expect("shader");
    let (tex, view) = d3d::dynamic_texture(&device, pw as u32, ph as u32, dxgi(tex0)).expect("texture");
    let row_bytes = pw / 4 * tex0.block_bytes();
    let rows = ph / 4;
    // `direct` decompresses straight into the gpu's mapped memory. otherwise: decompress in ordinary ram, then one copy.
    let mut lane = |frame: &[u8], threads: usize, direct: bool| {
        let l = &hap::parse(frame).expect("parse")[0];
        if !direct {
            hap::decompress(l, &mut buf, threads).expect("decompress");
        }
        d3d::fill_dynamic(&ctx, &tex, rows, row_bytes, |mem, pitch| {
            if direct && pitch == row_bytes {
                hap::decompress(l, mem, threads).expect("decompress");
                return;
            }
            if direct {
                hap::decompress(l, &mut buf, threads).expect("decompress");
            }
            if pitch == row_bytes {
                mem.copy_from_slice(&buf[..rows * row_bytes]);
            } else {
                // the driver pads its rows
                for r in 0..rows {
                    mem[r * pitch..r * pitch + row_bytes].copy_from_slice(&buf[r * row_bytes..(r + 1) * row_bytes]);
                }
            }
        })
        .expect("map");
        gpu.draw(&ps, &[Some(view.clone())]);
    };

    // frame 0, checked against ffmpeg's own decoder
    lane(&held[0], cores.min(8), false);
    let drawn = gpu.read_backbuffer().expect("read back");
    let _ = gpu.present(1);
    win::pump();
    let ref_in = Input::open(path, false).expect("open");
    match Decoder::open(&ref_in, v.index, None, 0) {
        Ok(mut dec) => {
            let mut rp = Packet::new();
            if dec.next(&ref_in, &mut rp, v.index).unwrap_or(false) {
                let (px, stride) = dec.plane8(0, vh);
                let (mut worst, mut sum, mut count) = (0i32, 0u64, 0u64);
                for y in 0..h as usize {
                    for x in 0..w as usize {
                        let p = drawn[y * w as usize + x];
                        let got = [(p >> 16) & 255, (p >> 8) & 255, p & 255];
                        for c in 0..3 {
                            let d = (got[c] as i32 - px[y * stride + x * 4 + c] as i32).abs();
                            worst = worst.max(d);
                            sum += d as u64;
                            count += 1;
                        }
                    }
                }
                println!("  frame 0 on the gpu against ffmpeg's cpu decoder, 8-bit: worst difference {worst}, mean {:.3}", sum as f64 / count as f64);
            }
        }
        Err(e) => println!("  ffmpeg cannot decode this one for comparison: {e}"),
    }

    for &(threads, direct) in &[(1usize, false), (4, false), (8, false), (4, true)] {
        let t = Instant::now();
        let mut shown = 0;
        'run: for _ in 0..(if direct { 1 } else { 3 }) {
            for f in held.iter().take(if direct { 20 } else { held.len() }) {
                lane(f, threads, direct);
                let _ = gpu.present(0);
                shown += 1;
                if !win::pump() {
                    break 'run;
                }
            }
        }
        unsafe { ctx.Flush() };
        let ms = t.elapsed().as_secs_f64() * 1000.0 / shown as f64;
        println!("  whole lane from ram, {threads:>2} thread(s), {}: {ms:6.2} ms a frame ({:.0} frames/s)", if direct { "decompressed straight into gpu memory" } else { "decompressed in ram, then one copy" }, 1000.0 / ms);
    }

    // 5a. reading frames ourselves: ffmpeg only supplies the index (where each frame is and how big)
    let index = input.index(v.index);
    let own = std::fs::File::open(path).expect("open");
    let mut fbuf = vec![0u8; index.iter().map(|e| e.1).max().unwrap_or(0)];
    let t = Instant::now();
    let mut got = 0u64;
    for &(pos, size) in &index {
        std::os::windows::fs::FileExt::seek_read(&own, &mut fbuf[..size], pos).expect("read");
        got += size as u64;
    }
    let secs = t.elapsed().as_secs_f64();
    println!("  read by index with our own handle: {:.0} MB/s ({:.0} frames/s), {} index entries", got as f64 / 1048576.0 / secs, index.len() as f64 / secs, index.len());
    let t = Instant::now();
    let mut shown = 0;
    for &(pos, size) in &index {
        std::os::windows::fs::FileExt::seek_read(&own, &mut fbuf[..size], pos).expect("read");
        lane(&fbuf[..size], 4, false);
        let _ = gpu.present(0);
        shown += 1;
        if !win::pump() {
            break;
        }
    }
    unsafe { ctx.Flush() };
    let ms = t.elapsed().as_secs_f64() * 1000.0 / shown.max(1) as f64;
    println!("  whole lane from the file, our own reads, 4 threads: {ms:6.2} ms a frame ({:.0} frames/s) over {shown} frames", 1000.0 / ms);

    // 5. the whole lane from the file
    let mut input = Input::open(path, false).expect("open");
    let t = Instant::now();
    let mut shown = 0;
    let _ = input.seek(v.index, 0);
    while pkt.read(&input).expect("read") {
        if pkt.stream() != v.index {
            continue;
        }
        lane(pkt.data(), 4, false);
        let _ = gpu.present(0);
        shown += 1;
        if !win::pump() {
            break;
        }
    }
    unsafe { ctx.Flush() };
    let ms = t.elapsed().as_secs_f64() * 1000.0 / shown.max(1) as f64;
    println!("  whole lane from the file, through ffmpeg's reader, 4 threads: {ms:6.2} ms a frame ({:.0} frames/s) over {shown} frames", 1000.0 / ms);
    win::close(hwnd);
}
