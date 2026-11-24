//! gate 3 on macos: does 10-bit survive to the swapchain?
//! usage: gate3_ramp
//! draws a 1024-step ramp into a 10-bit drawable of a metal layer and reads it back. passes when every step is exact.
//! the layer is never put on screen: this is about the numbers that reach the swapchain, not what the panel shows.

use objc2_core_foundation::CGSize;
use objc2_metal::{MTLPixelFormat, MTLTexture};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use spike_mac::{read, Gpu};

const RAMP: &str = r#"
fragment float4 fs(V in [[stage_in]]) {
    float v = floor(in.pos.x) / 1023.0;
    return float4(v, v, v, 1.0);
}
"#;

fn main() {
    let gpu = Gpu::new();
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&gpu.device));
    layer.setPixelFormat(MTLPixelFormat::BGR10A2Unorm);
    layer.setFramebufferOnly(false);
    layer.setDrawableSize(CGSize { width: 1024.0, height: 16.0 });
    let drawable = layer.nextDrawable().expect("the layer gave no drawable");
    let tex = drawable.texture();
    println!("drawable {} x {}, pixel format {:?}", tex.width(), tex.height(), tex.pixelFormat());
    gpu.draw(&gpu.pipeline(RAMP, MTLPixelFormat::BGR10A2Unorm), &tex, &[]);
    let px = read(&tex, 4);
    let (mut wrong, mut worst) = (0usize, 0i32);
    for (i, word) in px.chunks_exact(4).enumerate() {
        let (x, word) = ((i % 1024) as i32, u32::from_le_bytes(word.try_into().unwrap()));
        // blue in the low ten bits, then green, then red, then two bits of alpha
        for shift in [0, 10, 20] {
            let got = ((word >> shift) & 0x3ff) as i32;
            if got != x {
                wrong += 1;
                worst = worst.max((got - x).abs());
            }
        }
    }
    println!("1024 steps x 16 rows x 3 channels read back: {wrong} wrong, worst error {worst} code value(s)");
    std::process::exit(spike::verdict(wrong == 0, "every 10-bit code value drawn into the swapchain's back buffer reads back exact"));
}
