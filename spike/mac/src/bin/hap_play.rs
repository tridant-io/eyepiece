//! measured alongside on macos: does the hap lane hold one frame per refresh from the file?
//! usage: hap_play <hap file> [seconds, default 10]
//! the shared read-ahead reads frames by the offsets in ffmpeg's index and decompresses them in ordinary ram. this
//! thread copies once into a block-compressed texture, draws and presents, paced by the display. the system says
//! when each frame reached the display. passes when, after the first half second, every frame did and no refresh
//! went by without a new one.
//! needs the login session (spike/mac/session-run.sh): a window cannot open from an ssh session.

use objc2_metal::{MTLPixelFormat, MTLTextureUsage};
use spike::ff::{self, Input};
use spike::hap::{self, Tex};
use spike_mac::{hap_format, load, upload_blocks, Gpu, Window, RGB_AS_STORED, YCOCG_TO_RGB};
use std::sync::{Arc, Mutex};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first().cloned() else {
        eprintln!("usage: hap_play <hap file> [seconds]");
        std::process::exit(2);
    };
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10.0);
    let load_before = load();
    let input = Input::open(&path, false).expect("open");
    let v = input.first_video().expect("video");
    let (vw, vh) = (v.width as usize, v.height as usize);
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1.max(1) as f64;
    let index = input.index(v.index);
    let biggest = index.iter().map(|e| e.1).max().unwrap();

    // frame 0, read here once, to learn the texture format
    let own = std::fs::File::open(&path).expect("open");
    let mut first = vec![0u8; index[0].1];
    hap::read_at(&own, &mut first, index[0].0).expect("read");
    let (tex0, chunks) = {
        let l = hap::parse(&first).expect("parse");
        (l[0].tex, l[0].chunks.len())
    };
    let (frames_rx, spare_tx) = hap::read_ahead(&path, index.clone(), 4);

    let gpu = Gpu::new();
    let window = Window::open(&gpu, vw.min(1920), vh.min(1080));
    let draw = gpu.pipeline(if tex0 == Tex::YCoCgBc3 { YCOCG_TO_RGB } else { RGB_AS_STORED }, MTLPixelFormat::BGR10A2Unorm);
    let (pw, ph) = hap::padded(vw, vh);
    // three textures in turn, so a frame is never written into one the gpu may still be drawing from
    let ring: Vec<_> = (0..3).map(|_| gpu.texture(hap_format(tex0), pw, ph, MTLTextureUsage::ShaderRead).expect("this gpu refused the block format")).collect();

    let glass: Arc<Mutex<Vec<(usize, f64)>>> = Arc::new(Mutex::new(Vec::new()));
    let (mut t_wait, mut t_copy, mut t_present, mut shown) = (0.0, 0.0, 0.0, 0usize);
    let ms = |t: Instant| t.elapsed().as_secs_f64() * 1000.0;
    let start = Instant::now();
    while start.elapsed().as_secs_f64() < seconds {
        let t = Instant::now();
        let blocks = frames_rx.recv().expect("frame");
        t_wait += ms(t);
        let t = Instant::now();
        let tex = &ring[shown % 3];
        upload_blocks(tex, &blocks, tex0.block_bytes());
        let _ = spare_tx.send(blocks);
        t_copy += ms(t);
        let t = Instant::now();
        // waits here until the display has a free back buffer: this is what paces the loop to the refresh
        let Some(drawable) = window.layer.nextDrawable() else { continue };
        let (sink, k) = (glass.clone(), shown);
        gpu.present(&draw, &drawable, &[tex], move |at| sink.lock().unwrap().push((k, at)));
        t_present += ms(t);
        shown += 1;
        window.pump();
    }
    let elapsed = start.elapsed().as_secs_f64();
    std::thread::sleep(std::time::Duration::from_millis(400));
    window.pump();
    window.close();

    // skip the first half second: the queue fills, and the system drops the first presents after a window opens
    let mut got: Vec<(usize, f64)> = glass.lock().unwrap().iter().copied().filter(|g| g.0 >= 30).collect();
    got.sort_by_key(|g| g.0);
    let never = got.iter().filter(|g| g.1 == 0.0).count();
    let times: Vec<f64> = got.iter().map(|g| g.1).filter(|&t| t > 0.0).collect();
    let gaps: Vec<f64> = times.windows(2).map(|p| p[1] - p[0]).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let period = sorted.get(sorted.len() / 2).copied().unwrap_or(1.0);
    let missed: usize = gaps.iter().map(|g| ((g / period).round() as usize).saturating_sub(1)).sum();

    let n = shown as f64;
    let file = path.rsplit('/').next().unwrap_or(&path);
    println!("{file}: {vw}x{vh} · tag {} · {chunks} chunk(s) · {fps:.0} fps · {:.1} MB a frame · needs {:.0} MB/s", ff::fourcc(v.tag), biggest as f64 / 1048576.0, biggest as f64 / 1048576.0 * fps);
    println!("  load average: {load_before:.1} before, {:.1} after (8 cores; over 8 means the machine was busy with other work)", load());
    println!("  {shown} frames presented in {elapsed:.1} s ({:.2} a second), paced by the display, read and decompressed ahead on two other threads", n / elapsed);
    println!("  this thread, average ms a frame: waiting for the reader {:.2} · copy to the gpu {:.2} · draw and present call {:.2}", t_wait / n, t_copy / n, t_present / n);
    println!("  after the first half second: {} frames reached the display at {:.4} Hz · never reached it: {never} · refreshes without a new frame: {missed}", times.len(), 1.0 / period);
    std::process::exit(spike::verdict(never == 0 && missed == 0 && times.len() > 120, "one new frame on every refresh, read from the file"));
}
