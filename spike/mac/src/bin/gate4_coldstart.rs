//! gate 4 on macos: how long is a cold start, stage by stage?
//! usage: gate4_coldstart <hevc main10 file> [runs]
//! starts itself `runs` times as a fresh process and times each stage from the moment the launch was asked for to the
//! first frame on the display. measured, not judged. the first run is listed apart: it pays for cold caches.
//! this is a bare program, not an app bundle: a launch from finder adds whatever launch services costs.
//! needs the login session (spike/mac/session-run.sh): a window cannot open from an ssh session.

use objc2_metal::MTLPixelFormat;
use objc2_quartz_core::CACurrentMediaTime;
use spike::dec::Decoder;
use spike::ff::{Input, Packet};
use spike_mac::{load, Gpu, PixelBuffer, Window, YUV10_TO_RGB};
use std::process::Command;
use std::sync::mpsc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const STAGES: [&str; 7] = ["process running", "gpu device", "window on screen", "shader compiled", "file open", "first frame decoded", "first frame on the display"];

fn now_ms() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0
}

/// one launch: prints the stage times in milliseconds since `asked` (the parent's clock reading when it launched us).
fn child(asked: f64, path: &str) {
    let mut at = vec![now_ms() - asked];
    let gpu = Gpu::new();
    at.push(now_ms() - asked);
    let input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video stream");
    let opened = now_ms() - asked;
    let window = Window::open(&gpu, (v.width as usize).min(1920), (v.height as usize).min(1080));
    at.push(now_ms() - asked);
    let draw = gpu.pipeline(YUV10_TO_RGB, MTLPixelFormat::BGR10A2Unorm);
    at.push(now_ms() - asked);
    // the file was opened before the window so the window could take its size; reported in the order it is listed
    at.push(opened);
    let mut dec = Decoder::open_videotoolbox(&input, v.index).expect("hardware decoder");
    let mut pkt = Packet::new();
    assert!(dec.next(&input, &mut pkt, v.index).expect("decode"), "no first frame");
    at.push(now_ms() - asked);
    let pb = PixelBuffer(dec.pixel_buffer());
    let (luma, chroma) = (pb.plane(&gpu, 0, MTLPixelFormat::R16Unorm).expect("luma"), pb.plane(&gpu, 1, MTLPixelFormat::RG16Unorm).expect("chroma"));
    // the first presents after a window opens can be dropped by the system: keep presenting until one is shown
    let (tx, rx) = mpsc::channel::<f64>();
    let (mut on_display, mut dropped) = (None, 0usize);
    'presenting: for _ in 0..240 {
        if let Some(drawable) = window.layer.nextDrawable() {
            let tx = tx.clone();
            gpu.present(&draw, &drawable, &[&luma, &chroma], move |t| {
                // the system reports the time on its own clock; this puts it on ours. 0 means never shown.
                let _ = tx.send(if t > 0.0 { now_ms() - (CACurrentMediaTime() - t) * 1000.0 } else { 0.0 });
            });
        }
        window.pump();
        while let Ok(t) = rx.recv_timeout(Duration::from_millis(1)) {
            if t > 0.0 {
                on_display = Some(t);
                break 'presenting;
            }
            dropped += 1;
        }
    }
    at.push(on_display.expect("no frame ever reached the display") - asked);
    window.close();
    println!("{} {dropped}", at.iter().map(|t| format!("{t:.1}")).collect::<Vec<_>>().join(" "));
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("--child") {
        return child(args[1].parse().unwrap(), &args[2]);
    }
    let Some(path) = args.first() else {
        eprintln!("usage: gate4_coldstart <hevc main10 file> [runs]");
        std::process::exit(2);
    };
    let runs: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20);
    let exe = std::env::current_exe().unwrap();
    let load_before = load();
    let mut rows: Vec<Vec<f64>> = Vec::new();
    for _ in 0..runs + 1 {
        let out = Command::new(&exe).args(["--child", &format!("{}", now_ms()), path]).output().expect("launch");
        let line = String::from_utf8_lossy(&out.stdout);
        let row: Vec<f64> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
        if row.len() != STAGES.len() + 1 {
            println!("a launch failed: {}", String::from_utf8_lossy(&out.stderr).lines().last().unwrap_or("no output"));
            std::process::exit(1);
        }
        rows.push(row);
        std::thread::sleep(Duration::from_millis(300));
    }
    println!("{} · {} launches after one first launch · load average {load_before:.1} before, {:.1} after (8 cores)", path.rsplit('/').next().unwrap_or(path), runs, load());
    println!("{:<28} {:>9} {:>9} {:>9}   ms since the launch was asked for", "stage", "first", "p50", "p95");
    for (i, name) in STAGES.iter().enumerate() {
        let mut v: Vec<f64> = rows[1..].iter().map(|r| r[i]).collect();
        let (p50, p95) = spike::p50_p95(&mut v);
        println!("{name:<28} {:>9.1} {p50:>9.1} {p95:>9.1}", rows[0][i]);
    }
    let most = rows.iter().map(|r| r[STAGES.len()] as usize).max().unwrap_or(0);
    println!("presents the system dropped before the first one it showed: at most {most} in any launch");
}
