//! measured alongside on macos: the hap lane from ram, step by step, and whether the picture is right.
//! usage: hap_lane <hap file> [frames, default 60]
//! loads frames into ram, then times each step with nothing else in the way: snappy by thread count, the one copy
//! into a block-compressed texture, and the whole lane (decompress, copy, draw off screen, wait for the gpu).
//! where ffmpeg can decode the clip itself (hap, hap alpha, hap q), frame 0 as the gpu draws it is compared with
//! ffmpeg's. prints numbers; nothing is judged, and nothing is put on the screen.

use objc2_metal::{MTLPixelFormat, MTLTextureUsage};
use spike::dec::Decoder;
use spike::ff::{self, AVPixelFormat, Input, Packet};
use spike::hap::{self, Tex};
use spike_mac::{hap_format, load, read, upload_blocks, Gpu, RGB_AS_STORED, YCOCG_TO_RGB};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: hap_lane <hap file> [frames]");
        std::process::exit(2);
    };
    let input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video");
    let (vw, vh) = (v.width as usize, v.height as usize);
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1.max(1) as f64;
    let index = input.index(v.index);
    let n = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(60).min(index.len());
    let f = std::fs::File::open(path).expect("open");
    let frames: Vec<Vec<u8>> = index[..n]
        .iter()
        .map(|&(pos, size)| {
            let mut b = vec![0u8; size];
            hap::read_at(&f, &mut b, pos).expect("read");
            b
        })
        .collect();
    let (tex0, chunks, len) = {
        let l = hap::parse(&frames[0]).expect("parse");
        (l[0].tex, l[0].chunks.len(), l[0].len)
    };
    let on_disk = frames.iter().map(Vec::len).sum::<usize>() as f64 / n as f64 / 1048576.0;
    let file = path.rsplit('/').next().unwrap_or(path);
    println!("{file}\n  {vw}x{vh} · tag {} · {n} frames at {fps:.2} fps · {on_disk:.1} MB a frame on disk · needs {:.0} MB/s to play", ff::fourcc(v.tag), on_disk * fps);
    println!("  texture {tex0:?} · {chunks} chunk(s) · {:.1} MB decompressed · load average {:.1} (8 cores)", len as f64 / 1048576.0, load());
    let per_frame = |t: Instant| t.elapsed().as_secs_f64() * 1000.0 / n as f64;

    let mut blocks = vec![0u8; len];
    for threads in [1, 4, 8] {
        let t = Instant::now();
        for fr in &frames {
            hap::decompress(&hap::parse(fr).unwrap()[0], &mut blocks, threads).expect("decompress");
        }
        let ms = per_frame(t);
        println!("  snappy, {threads} thread(s): {ms:6.2} ms a frame ({:.0} frames/s)", 1000.0 / ms);
    }

    let gpu = Gpu::new();
    let (pw, ph) = hap::padded(vw, vh);
    // three textures in turn, so a frame is never written into one the gpu may still be drawing from
    let ring: Vec<_> = (0..3).map(|_| gpu.texture(hap_format(tex0), pw, ph, MTLTextureUsage::ShaderRead).expect("this gpu refused the block format")).collect();
    let target = gpu.texture(MTLPixelFormat::RGBA8Unorm, vw, vh, MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead).expect("target");
    let draw = gpu.pipeline(if tex0 == Tex::YCoCgBc3 { YCOCG_TO_RGB } else { RGB_AS_STORED }, MTLPixelFormat::RGBA8Unorm);

    let t = Instant::now();
    for i in 0..n {
        upload_blocks(&ring[i % 3], &blocks, tex0.block_bytes());
    }
    println!("  the one copy into the texture: {:6.2} ms a frame", per_frame(t));

    let t = Instant::now();
    for (i, fr) in frames.iter().enumerate() {
        hap::decompress(&hap::parse(fr).unwrap()[0], &mut blocks, 4).expect("decompress");
        upload_blocks(&ring[i % 3], &blocks, tex0.block_bytes());
        gpu.draw(&draw, &target, &[&ring[i % 3]]);
    }
    let ms = per_frame(t);
    println!("  whole lane from ram, 4 threads, drawn at full size off screen: {ms:6.2} ms a frame ({:.0} frames/s) · load average {:.1}", 1000.0 / ms, load());

    // is the picture right? only where ffmpeg has a decoder to compare with
    let Ok(mut reference) = Decoder::open_software(&input, v.index, 0) else {
        println!("  picture not checked: ffmpeg has no decoder for this variant");
        return;
    };
    let mut pkt = Packet::new();
    if !reference.next(&input, &mut pkt, v.index).unwrap_or(false) || ![AVPixelFormat::AV_PIX_FMT_RGB0, AVPixelFormat::AV_PIX_FMT_RGBA].contains(&reference.format()) {
        println!("  picture not checked: ffmpeg gave no rgb frame");
        return;
    }
    hap::decompress(&hap::parse(&frames[0]).unwrap()[0], &mut blocks, 4).expect("decompress");
    upload_blocks(&ring[0], &blocks, tex0.block_bytes());
    gpu.draw(&draw, &target, &[&ring[0]]);
    let ours = read(&target, 4);
    let (theirs, stride) = reference.plane8(0, vh);
    let (mut worst, mut sum) = (0i32, 0u64);
    for y in 0..vh {
        for x in 0..vw {
            for c in 0..3 {
                let d = (ours[(y * vw + x) * 4 + c] as i32 - theirs[y * stride + x * 4 + c] as i32).abs();
                worst = worst.max(d);
                sum += d as u64;
            }
        }
    }
    println!("  frame 0 as the gpu draws it against ffmpeg's own decode: worst difference {worst} of 255, mean {:.3}", sum as f64 / (vw * vh * 3) as f64);
}
