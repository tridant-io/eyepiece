//! gate 4, follow-up: creating the direct3d device is most of a cold start. can it be made cheaper?
//! usage: devtime            runs every variant five times, each in a fresh process, and prints the median
//!        devtime <variant>  one creation, timed
//! a fresh process each time matters: once the driver is loaded in a process, a second device is cheap.

use std::process::Command;
use std::time::Instant;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIAdapter, IDXGIFactory1};

const VARIANTS: [(&str, &str); 7] = [
    ("default", "default adapter, bgra + video flags, levels 11.1 and 11.0 (what the gates use)"),
    ("noflags", "default adapter, no flags"),
    ("video", "default adapter, video flag only"),
    ("level110", "default adapter, no flags, level 11.0 only"),
    ("adapter", "adapter picked by us through dxgi first, no flags"),
    ("nothreads", "default adapter, no flags, driver worker threads off"),
    ("warp", "the software rasterizer, for comparison"),
];

fn one(variant: &str) {
    let levels2 = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    let levels1 = [D3D_FEATURE_LEVEL_11_0];
    let (mut device, mut ctx) = (None, None);
    let mut pre = 0.0;
    let t = Instant::now();
    let r = unsafe {
        match variant {
            "noflags" => D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), Some(&levels2), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)),
            "video" => D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_VIDEO_SUPPORT, Some(&levels2), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)),
            "level110" => D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), Some(&levels1), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)),
            "adapter" => {
                let f: IDXGIFactory1 = CreateDXGIFactory1().expect("factory");
                let a: IDXGIAdapter = f.EnumAdapters(0).expect("adapter");
                pre = t.elapsed().as_secs_f64() * 1000.0;
                D3D11CreateDevice(&a, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), Some(&levels2), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))
            }
            "nothreads" => D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_PREVENT_INTERNAL_THREADING_OPTIMIZATIONS, Some(&levels2), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)),
            "warp" => D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), Some(&levels2), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)),
            _ => D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, Some(&levels2), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)),
        }
    };
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    // a second device in the same process, to show what the driver load alone costs
    let (mut d2, mut c2): (Option<ID3D11Device>, Option<ID3D11DeviceContext>) = (None, None);
    let t2 = Instant::now();
    let _ = unsafe { D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), Some(&levels2), D3D11_SDK_VERSION, Some(&mut d2), None, Some(&mut c2)) };
    let again = t2.elapsed().as_secs_f64() * 1000.0;
    println!("MS {ms:.1} {pre:.1} {again:.1} {}", if r.is_ok() { "ok" } else { "failed" });
}

fn main() {
    if let Some(v) = std::env::args().nth(1) {
        one(&v);
        return;
    }
    let exe = std::env::current_exe().unwrap();
    println!("{:<10} {:>9} {:>9}  what", "variant", "median ms", "2nd device");
    for (name, what) in VARIANTS {
        let (mut times, mut again) = (Vec::new(), Vec::new());
        for _ in 0..5 {
            let out = Command::new(&exe).arg(name).output().expect("launch");
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(l) = text.lines().find(|l| l.starts_with("MS")) {
                let p: Vec<&str> = l.split_whitespace().collect();
                times.push(p[1].parse::<f64>().unwrap_or(0.0));
                again.push(p[3].parse::<f64>().unwrap_or(0.0));
            }
            std::thread::sleep(std::time::Duration::from_millis(150));
        }
        let (m, _) = spike::p50_p95(&mut times);
        let (a, _) = spike::p50_p95(&mut again);
        println!("{name:<10} {m:>9.1} {a:>9.1}  {what}");
    }
}
