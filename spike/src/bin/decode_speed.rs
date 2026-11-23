//! measured alongside: software decode throughput, and what one lone frame step costs.
//! usage: decode_speed <file> [frames for the step test, default 12]
//! part 1 decodes the whole clip in software on all cores and reports frames per second.
//! part 2 jumps to frames spread through the clip and times decoding just that frame from a standing start,
//! which is what stepping to an arbitrary frame costs: seek to the key frame at or before it, then decode up to it.
//! nothing is judged here: it prints numbers.

use spike::dec::Decoder;
use spike::ff::{self, Input, Packet};
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: decode_speed <file> [frames for the step test]");
        std::process::exit(2);
    };
    let steps: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(12);
    let mut input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video");
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1.max(1) as f64;
    let file = std::path::Path::new(path).file_name().unwrap().to_string_lossy().into_owned();

    // part 1: straight through
    let mut dec = Decoder::open(&input, v.index, None, 0).expect("decoder");
    let mut pkt = Packet::new();
    let t = Instant::now();
    let mut n = 0u32;
    let mut pts = Vec::new();
    let mut fmt = None;
    while dec.next(&input, &mut pkt, v.index).expect("decode") {
        fmt.get_or_insert(dec.format());
        pts.push(dec.pts());
        n += 1;
    }
    let secs = t.elapsed().as_secs_f64();
    println!("{file}\n  {}x{} · {} · tag {} · {fps:.2} fps · decoded as {:?}", v.width, v.height, v.codec_name, ff::fourcc(v.tag), fmt);
    println!("  straight through, all cores: {n} frames in {secs:.2} s = {:.1} frames/s ({:.2}x real time)", n as f64 / secs, n as f64 / secs / fps);

    pts.sort();
    for stepping in [false, true] {
        let mut dec = if stepping { Decoder::open_stepping(&input, v.index).expect("decoder") } else { Decoder::open(&input, v.index, None, 0).expect("decoder") };
    // part 2: lone steps to frames spread through the clip
    let mut times = Vec::new();
    let mut worst_run = 0u32;
    for i in 0..steps {
        let target = pts[(pts.len() - 1) * (i + 1) / (steps + 1)];
        let t = Instant::now();
        input.seek(v.index, target).expect("seek");
        dec.flush();
        let mut run = 0u32;
        loop {
            if !dec.next(&input, &mut pkt, v.index).expect("decode") {
                break;
            }
            run += 1;
            if dec.pts() >= target {
                break;
            }
        }
        times.push(t.elapsed().as_secs_f64() * 1000.0);
        worst_run = worst_run.max(run);
    }
    let (p50, p95) = spike::p50_p95(&mut times);
    println!("  one lone step to an arbitrary frame, {}: p50 {p50:.1} ms, p95 {p95:.1} ms, worst case decoded {worst_run} frame(s) to get there", if stepping { "slice threads" } else { "frame threads" });
    }
}
