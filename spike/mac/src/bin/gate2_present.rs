//! gate 2 on macos: can we learn when a frame reached the screen?
//! usage: gate2_present <hevc main10 file> [seconds]
//! plays the clip in a window, one present per refresh, drawing from the decoder's own surface, and asks the system
//! when each frame reached the display. the frame for each present is chosen by counting presents, never by the clock.
//! passes when every present comes back with a time, the times advance one refresh at a time with none missed, and
//! the measured refresh rate holds to 0.01 Hz between the two halves of the run.
//! needs the login session (spike/mac/session-run.sh): a window cannot open from an ssh session.

use objc2_metal::MTLPixelFormat;
use spike::dec::Decoder;
use spike::ff::{Input, Packet};
use spike_mac::{load, Gpu, PixelBuffer, Window, YUV10_TO_RGB};
use std::sync::{Arc, Mutex};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gate2_present <hevc main10 file> [seconds]");
        std::process::exit(2);
    };
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let load_before = load();

    let mut input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video stream");
    let (w, h) = ((v.width as usize).min(1920), (v.height as usize).min(1080));
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1 as f64;
    let gpu = Gpu::new();
    let window = Window::open(&gpu, w, h);
    let draw = gpu.pipeline(YUV10_TO_RGB, MTLPixelFormat::BGR10A2Unorm);
    let mut dec = Decoder::open_videotoolbox(&input, v.index).expect("hardware decoder");
    let mut pkt = Packet::new();

    let presents = (seconds * window.refresh) as usize;
    let glass: Arc<Mutex<Vec<(usize, f64)>>> = Arc::new(Mutex::new(Vec::with_capacity(presents)));
    // frames decoded so far, counting across loops of the clip; the one on hand is number `decoded - 1`
    let (mut decoded, mut repeats, mut no_drawable) = (0usize, 0usize, 0usize);
    for k in 0..presents {
        // which frame belongs on this refresh: a count, not a clock
        let want = (k as f64 * fps / window.refresh).floor() as usize;
        if decoded > 0 && want == decoded - 1 {
            repeats += 1;
        }
        while decoded <= want {
            if dec.next(&input, &mut pkt, v.index).expect("decode") {
                decoded += 1;
            } else {
                // end of the clip: go round again
                input.seek(v.index, 0).expect("seek");
                dec.flush();
            }
        }
        let pb = PixelBuffer(dec.pixel_buffer());
        let (luma, chroma) = (pb.plane(&gpu, 0, MTLPixelFormat::R16Unorm).expect("luma"), pb.plane(&gpu, 1, MTLPixelFormat::RG16Unorm).expect("chroma"));
        // waits here until the display has a free back buffer: this is what paces the loop to the refresh
        let Some(drawable) = window.layer.nextDrawable() else {
            no_drawable += 1;
            continue;
        };
        let sink = glass.clone();
        gpu.present(&draw, &drawable, &[&luma, &chroma], move |t| sink.lock().unwrap().push((k, t)));
        window.pump();
    }
    std::thread::sleep(std::time::Duration::from_millis(400));
    window.pump();
    window.close();

    let mut got = glass.lock().unwrap().clone();
    got.sort_by_key(|g| g.0);
    let never = got.iter().filter(|g| g.1 == 0.0).count();
    let times: Vec<f64> = got.iter().map(|g| g.1).filter(|&t| t > 0.0).collect();
    let mut gaps: Vec<f64> = times.windows(2).map(|p| p[1] - p[0]).collect();
    let mut sorted = gaps.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let period = sorted.get(sorted.len() / 2).copied().unwrap_or(0.0);
    // a gap of two periods is one refresh that went by without a new frame
    let missed: usize = gaps.iter().map(|g| ((g / period).round() as usize).saturating_sub(1)).sum();
    let doubled = gaps.iter().filter(|&&g| g < period * 0.5).count();
    gaps.retain(|&g| (g / period).round() as usize == 1);
    let mean = |s: &[f64]| s.iter().sum::<f64>() / s.len().max(1) as f64;
    let (first, second) = gaps.split_at(gaps.len() / 2);
    let (hz1, hz2) = (1.0 / mean(first), 1.0 / mean(second));

    println!("{} · {:.2} fps · window {w}x{h} · display nominally {:.0} Hz", path.rsplit('/').next().unwrap_or(path), fps, window.refresh);
    println!("load average: {load_before:.1} before, {:.1} after (8 cores; over 8 means the machine was busy with other work)", load());
    println!("presents sent: {presents} · came back with a time: {} · never reached the display: {never} · no back buffer in time: {no_drawable}", got.len() - never);
    println!("refresh period from those times: {:.4} ms = {:.4} Hz · first half {hz1:.4} Hz, second half {hz2:.4} Hz", period * 1000.0, 1.0 / period);
    println!("refreshes that went by without a new frame: {missed} · two frames inside one refresh: {doubled}");
    let dropped: Vec<usize> = got.iter().filter(|g| g.1 == 0.0).map(|g| g.0).collect();
    if !dropped.is_empty() {
        println!("presents that never reached the display, by number: {:?}", &dropped[..dropped.len().min(24)]);
    }
    println!("frames shown twice on purpose, {:.2} fps on this display: {repeats} (predicted {:.1})", fps, presents as f64 * (1.0 - fps / window.refresh));
    let pass = got.len() == presents && never == 0 && no_drawable == 0 && missed == 0 && (hz1 - hz2).abs() < 0.01;
    std::process::exit(spike::verdict(pass, "every present reports when it reached the display, one refresh apart, at a steady rate"));
}
