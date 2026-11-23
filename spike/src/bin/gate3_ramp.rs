//! gate 3: does 10-bit survive to the swapchain?
//! draws a 1024-step gray ramp into a 10-bit back buffer and reads it back. passes when every code value is exact.
//! this proves the path as far as the swapchain. what the cable and the panel do after that is outside any app.

use spike::{d3d, win};

const PS: &str = "float4 main(float4 pos : SV_Position) : SV_Target { float v = min(floor(pos.x), 1023.0) / 1023.0; return float4(v, v, v, 1); }";

fn main() {
    let (w, h) = (1024u32, 64u32);
    let hwnd = win::window(100, 100, w as i32, h as i32).expect("window");
    let (device, ctx) = d3d::device().expect("device");
    let gpu = d3d::Gpu::new(device, ctx, hwnd, w, h, true).expect("10-bit swapchain");
    let ps = gpu.pixel_shader(PS).expect("shader");
    gpu.draw(&ps, &[]);
    let px = gpu.read_backbuffer().expect("readback");
    let _ = gpu.present(1);
    win::pump();
    let (mut wrong, mut first) = (0usize, None);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let p = px[y * w as usize + x];
            let (r, g, b) = (p & 1023, (p >> 10) & 1023, (p >> 20) & 1023);
            if r != x as u32 || g != x as u32 || b != x as u32 {
                wrong += 1;
                first.get_or_insert((x, y, r, g, b));
            }
        }
    }
    println!("back buffer: R10G10B10A2, {w} x {h}. distinct levels drawn: 1024. pixels with a wrong code value: {wrong}");
    if let Some((x, y, r, g, b)) = first {
        println!("first mismatch at ({x}, {y}): wanted {x}, got {r} {g} {b}");
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    win::close(hwnd);
    std::process::exit(spike::verdict(wrong == 0, "every one of 1024 code values reads back exact from the 10-bit back buffer"));
}
