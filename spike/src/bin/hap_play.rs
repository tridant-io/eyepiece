//! measured alongside: does the hap lane hold one frame per refresh from the file, with a read-ahead thread?
//! usage: hap_play <hap file> [seconds, default 10]
//! one thread reads frames by the offsets in ffmpeg's index, decompresses them in ordinary ram and stays a few frames ahead.
//! the main thread copies once into the gpu texture, draws and presents with vsync on. (with decompression on the main thread
//! as well, 8K60 fell to 55 frames a second: 2.5 ms waiting + 4.7 snappy + 4.8 copy + 6.3 draw and present is over one refresh.) the os's present statistics say whether
//! any refresh went by without a new frame. passes when none did.

use spike::ff::{self, Input};
use spike::hap::{self, Tex};
use spike::{d3d, win};
use std::sync::mpsc;
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

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().cloned() else {
        eprintln!("usage: hap_play <hap file> [seconds]");
        std::process::exit(2);
    };
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let input = Input::open(&path, false).expect("open");
    let v = input.first_video().expect("video");
    let (vw, vh) = (v.width as usize, v.height as usize);
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1.max(1) as f64;
    let index = input.index(v.index);
    let biggest = index.iter().map(|e| e.1).max().unwrap();

    // frame 0, read here once, to learn the texture format
    let own = std::fs::File::open(&path).expect("open");
    let mut first = vec![0u8; index[0].1];
    std::os::windows::fs::FileExt::seek_read(&own, &mut first, index[0].0).expect("read");
    let (tex0, chunks) = { let l = hap::parse(&first).expect("parse"); (l[0].tex, l[0].chunks.len()) };

    // three stages, each on its own thread, each a few frames ahead, buffers handed back for reuse:
    // read (our own handle, frames by offset) -> decompress in ordinary ram -> the main thread, which copies and presents.
    // with reading and decompressing on one thread, the 16-chunk 8K clip managed 51 frames a second.
    let (raw_tx, raw_rx) = mpsc::sync_channel::<(Vec<u8>, usize)>(2);
    let (raw_spare_tx, raw_spare_rx) = mpsc::channel::<Vec<u8>>();
    let (frames_tx, frames_rx) = mpsc::sync_channel::<Vec<u8>>(2);
    let (spare_tx, spare_rx) = mpsc::channel::<Vec<u8>>();
    let reader_path = path.clone();
    let reader_index = index.clone();
    std::thread::spawn(move || {
        let f = std::fs::File::open(&reader_path).expect("open");
        let mut i = 0;
        loop {
            let (pos, size) = reader_index[i % reader_index.len()];
            let mut raw = raw_spare_rx.try_recv().unwrap_or_else(|_| vec![0u8; biggest]);
            std::os::windows::fs::FileExt::seek_read(&f, &mut raw[..size], pos).expect("read");
            if raw_tx.send((raw, size)).is_err() {
                break;
            }
            i += 1;
        }
    });
    std::thread::spawn(move || {
        while let Ok((raw, size)) = raw_rx.recv() {
            let layers = hap::parse(&raw[..size]).expect("parse");
            let mut blocks = spare_rx.try_recv().unwrap_or_default();
            blocks.resize(layers[0].len, 0);
            hap::decompress(&layers[0], &mut blocks, 4).expect("decompress");
            drop(layers);
            let _ = raw_spare_tx.send(raw);
            if frames_tx.send(blocks).is_err() {
                break;
            }
        }
    });

    let (pw, ph) = hap::padded(vw, vh);
    let (row_bytes, rows) = (pw / 4 * tex0.block_bytes(), ph / 4);

    let (sw, sh) = win::screen_size();
    let (w, h) = (vw.min(sw as usize - 200).min(1920) as u32, vh.min(sh as usize - 200).min(1080) as u32);
    let hwnd = win::window(100, 100, w as i32, h as i32).expect("window");
    let (device, ctx) = d3d::device().expect("device");
    let gpu = d3d::Gpu::new(device.clone(), ctx.clone(), hwnd, w, h, false).expect("swapchain");
    let ps = gpu.pixel_shader(if tex0 == Tex::YCoCgBc3 { PS_YCOCG } else { PS_RGB }).expect("shader");
    let format = match tex0 {
        Tex::Bc1 => DXGI_FORMAT_BC1_UNORM,
        Tex::Bc3 | Tex::YCoCgBc3 => DXGI_FORMAT_BC3_UNORM,
        Tex::Bc7 => DXGI_FORMAT_BC7_UNORM,
        Tex::Bc4 => DXGI_FORMAT_BC4_UNORM,
        Tex::Bc6u => DXGI_FORMAT_BC6H_UF16,
        Tex::Bc6s => DXGI_FORMAT_BC6H_SF16,
    };
    let (tex, view) = d3d::dynamic_texture(&device, pw as u32, ph as u32, format).expect("texture");

    let mut samples: Vec<(u32, u32)> = Vec::new(); // present count, the refresh it was shown on
    let (mut t_wait, mut t_copy, mut t_present, mut shown) = (0.0, 0.0, 0.0, 0u32);
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        let t = Instant::now();
        let blocks = frames_rx.recv().expect("frame");
        t_wait += ms(t);
        let t = Instant::now();
        d3d::fill_dynamic(&ctx, &tex, rows, row_bytes, |mem, pitch| {
            if pitch == row_bytes {
                mem.copy_from_slice(&blocks);
            } else {
                for r in 0..rows {
                    mem[r * pitch..r * pitch + row_bytes].copy_from_slice(&blocks[r * row_bytes..(r + 1) * row_bytes]);
                }
            }
        })
        .expect("map");
        let _ = spare_tx.send(blocks);
        t_copy += ms(t);
        let t = Instant::now();
        gpu.draw(&ps, &[Some(view.clone())]);
        let _ = gpu.present(1);
        t_present += ms(t);
        shown += 1;
        if let Some(s) = gpu.frame_stats() {
            if samples.last().map(|l| l.0) != Some(s.PresentCount) {
                samples.push((s.PresentCount, s.PresentRefreshCount));
            }
        }
        if !win::pump() {
            break;
        }
    }
    let elapsed = start.elapsed().as_secs_f64();
    win::close(hwnd);
    // skip the first half second while the queue fills
    let mut missed = 0u32;
    for pair in samples.iter().skip(30).collect::<Vec<_>>().windows(2) {
        let (dp, dr) = (pair[1].0.wrapping_sub(pair[0].0), pair[1].1.wrapping_sub(pair[0].1));
        if dr > dp {
            missed += dr - dp;
        }
    }
    let n = shown as f64;
    let file = std::path::Path::new(&path).file_name().unwrap().to_string_lossy().into_owned();
    println!("{file}: {vw}x{vh} · tag {} · {chunks} chunk(s) · {fps:.0} fps · {:.1} MB a frame · needs {:.0} MB/s", ff::fourcc(v.tag), biggest as f64 / 1048576.0, biggest as f64 / 1048576.0 * fps);
    println!("  {shown} frames presented in {elapsed:.1} s ({:.2} a second), vsync on, read and decompressed ahead on two other threads", n / elapsed);
    println!("  main thread, average ms a frame: waiting for the reader {:.2} · copy to the gpu {:.2} · draw and present call {:.2}", t_wait / n, t_copy / n, t_present / n);
    println!("  refreshes that went by without a new frame, by the os's count: {missed} (of {})", samples.len());
    std::process::exit(spike::verdict(missed == 0 && samples.len() > 120, "one new frame on every refresh, read from the file"));
}
