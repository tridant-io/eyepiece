//! gate 2: can we learn when a frame reached the screen?
//! usage: gate2_present <hevc file> [seconds per part, default 20]
//! part 1 shows every frame for exactly one refresh (hardware decode, drawn from the decoder's texture) and reads the
//! os's present statistics after each one: do they advance once per refresh, how steady is the measured refresh rate,
//! and did any refresh go by without our frame?
//! part 2 paces the same clip by the clock instead, as a clip with sound would be, and counts repeated and skipped frames
//! against what the difference between clip rate and display rate predicts.
//! nothing here is confirmed from outside the app; that needs a capture tool or a camera.

use spike::dec::Decoder;
use spike::ff::{Input, Packet};
use spike::{d3d, win};
use windows::Win32::Graphics::Direct3D11::ID3D11ShaderResourceView;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_FORMAT_R16G16_UNORM, DXGI_FORMAT_R16_UNORM, DXGI_FORMAT_R8G8_UNORM, DXGI_FORMAT_R8_UNORM};
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

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

fn qpc() -> i64 {
    let mut t = 0;
    unsafe { QueryPerformanceCounter(&mut t).unwrap() };
    t
}

struct Player {
    input: Input,
    dec: Decoder,
    pkt: Packet,
    stream: usize,
    loops: u32,
}

impl Player {
    /// decodes the next frame, starting the file again at its end.
    fn next(&mut self) -> bool {
        for _ in 0..2 {
            if self.dec.next(&self.input, &mut self.pkt, self.stream).expect("decode") {
                return true;
            }
            self.input.seek(self.stream, 0).expect("seek");
            self.dec.flush();
            self.loops += 1;
        }
        false
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(path) = args.first() else {
        eprintln!("usage: gate2_present <hevc file> [seconds per part]");
        std::process::exit(2);
    };
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(20.0);
    let mut freq = 0;
    unsafe { QueryPerformanceFrequency(&mut freq).unwrap() };
    let freq = freq as f64;

    let input = Input::open(path, false).expect("open");
    let v = input.first_video().expect("video");
    let fps = v.avg_rate.0 as f64 / v.avg_rate.1.max(1) as f64;
    let (sw, sh) = win::screen_size();
    let (w, h) = ((v.width as usize).min(sw as usize - 200).min(1920) as u32, (v.height as usize).min(sh as usize - 200).min(1080) as u32);
    let hwnd = win::window(100, 100, w as i32, h as i32).expect("window");
    let (device, ctx) = d3d::device().expect("device");
    let gpu = d3d::Gpu::new(device.clone(), ctx, hwnd, w, h, true).expect("swapchain");
    let ps = gpu.pixel_shader(PS).expect("shader");
    let dec = Decoder::open(&input, v.index, Some(&device), 1).expect("hardware decoder");
    let mut p = Player { input, dec, pkt: Packet::new(), stream: v.index, loops: 0 };

    let views = |p: &Player| -> [Option<ID3D11ShaderResourceView>; 2] {
        let (tex, slice) = p.dec.texture();
        let mut d = Default::default();
        unsafe { tex.GetDesc(&mut d) };
        let (fy, fc) = if d.Format == DXGI_FORMAT_NV12 { (DXGI_FORMAT_R8_UNORM, DXGI_FORMAT_R8G8_UNORM) } else { (DXGI_FORMAT_R16_UNORM, DXGI_FORMAT_R16G16_UNORM) };
        [Some(d3d::plane_view(&device, &tex, slice, fy).expect("view")), Some(d3d::plane_view(&device, &tex, slice, fc).expect("view"))]
    };

    // ---- part 1: every frame, one refresh each ----
    let mut samples: Vec<(u32, u32, u32, i64)> = Vec::new(); // present count, present refresh, sync refresh, sync qpc
    let (mut no_stats, mut presented) = (0u32, 0u32);
    let start = qpc();
    let mut alive = true;
    while alive && (qpc() - start) as f64 / freq < seconds {
        if !p.next() {
            break;
        }
        let vs = views(&p);
        gpu.draw(&ps, &vs);
        let _ = gpu.present(1);
        presented += 1;
        match gpu.frame_stats() {
            Some(s) => {
                if samples.last().map(|l| l.0) != Some(s.PresentCount) {
                    samples.push((s.PresentCount, s.PresentRefreshCount, s.SyncRefreshCount, s.SyncQPCTime));
                }
            }
            None => no_stats += 1,
        }
        alive = win::pump();
    }
    let elapsed1 = (qpc() - start) as f64 / freq;
    // the refresh rate, from the os's own vsync timestamps
    let hz = |a: &(u32, u32, u32, i64), b: &(u32, u32, u32, i64)| (b.2.wrapping_sub(a.2)) as f64 / ((b.3 - a.3) as f64 / freq);
    let (mut refresh, mut lo, mut hi, mut missed, mut steady) = (0.0, f64::MAX, f64::MIN, 0u32, true);
    if samples.len() > 120 {
        let s = &samples[30..]; // let the first half second settle
        refresh = hz(&s[0], &s[s.len() - 1]);
        // one-second windows
        let mut a = 0;
        for b in 1..s.len() {
            if (s[b].3 - s[a].3) as f64 / freq >= 1.0 {
                let x = hz(&s[a], &s[b]);
                lo = lo.min(x);
                hi = hi.max(x);
                a = b;
            }
        }
        for pair in s.windows(2) {
            let (dp, dr) = (pair[1].0.wrapping_sub(pair[0].0), pair[1].1.wrapping_sub(pair[0].1));
            if dr > dp {
                missed += dr - dp;
            }
        }
        steady = hi - lo <= 0.01;
    }
    println!("part 1, every frame for one refresh: {presented} presents in {elapsed1:.1} s, clip looped {} time(s)", p.loops);
    println!("  present statistics: {} distinct readings, {no_stats} calls with none available", samples.len());
    println!("  measured refresh rate {refresh:.4} Hz. over one-second windows: {lo:.4} to {hi:.4} Hz (spread {:.4})", hi - lo);
    println!("  refreshes that went by without a new frame: {missed}");

    // ---- part 2: the clip keeps its own rate, as a clip with sound must. which frame is due at each refresh? ----
    // 2a asks the wall clock at the moment of the call. 2b counts the display's refreshes, which the statistics let us trust.
    let mut results = Vec::new();
    for by_refresh in [false, true] {
        let (mut repeats, mut skips, mut shown_idx, mut decoded_idx, mut presents2) = (0u32, 0u32, -1i64, -1i64, 0u32);
        let mut vs: [Option<ID3D11ShaderResourceView>; 2] = [None, None];
        let before = gpu.frame_stats();
        let start = qpc();
        while alive && (qpc() - start) as f64 / freq < seconds {
            let want = if by_refresh { (presents2 as f64 * fps / refresh).floor() as i64 } else { ((qpc() - start) as f64 / freq * fps).floor() as i64 };
            if want > shown_idx {
                // decode up to the wanted frame; the ones in between are never shown
                while decoded_idx < want {
                    if !p.next() {
                        break;
                    }
                    decoded_idx += 1;
                }
                if shown_idx >= 0 && want > shown_idx + 1 {
                    skips += (want - shown_idx - 1) as u32;
                }
                shown_idx = want;
                vs = views(&p);
            } else {
                repeats += 1;
            }
            gpu.draw(&ps, &vs);
            let _ = gpu.present(1);
            presents2 += 1;
            alive = win::pump();
        }
        let elapsed2 = (qpc() - start) as f64 / freq;
        let after = gpu.frame_stats();
        // did the display really advance one refresh per present, by the os's count?
        let os_refreshes = match (before, after) {
            (Some(a), Some(b)) => b.PresentRefreshCount.wrapping_sub(a.PresentRefreshCount) as i64,
            _ => -1,
        };
        let expected = (fps - refresh).abs() * elapsed2;
        println!("part 2{}, {}: clip {fps:.4} fps on a {refresh:.4} Hz display for {elapsed2:.1} s", if by_refresh { "b" } else { "a" }, if by_refresh { "frame chosen by counting refreshes" } else { "frame chosen by the wall clock at the call" });
        println!("  repeated frames {repeats}, skipped frames {skips}. the rate difference predicts {expected:.1}. presents {presents2}, refreshes by the os's count {os_refreshes}");
        results.push(((repeats + skips) as f64, expected, presents2 as i64, os_refreshes));
    }
    println!("  confirmed from outside the app: no. that needs a present-capture tool run as administrator, or a camera.");
    win::close(hwnd);
    let (events, expected, presents2, os_refreshes) = results[1];
    let pass = samples.len() > 120 && steady && missed == 0 && (events - expected).abs() <= 1.0 && (presents2 - os_refreshes).abs() <= 2;
    std::process::exit(spike::verdict(pass, "present statistics advance once per refresh, the refresh rate is steady to 0.01 Hz, no refresh was missed, and pacing on the refresh count gives exactly the predicted cadence"));
}
