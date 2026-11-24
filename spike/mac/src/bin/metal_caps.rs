//! measured alongside on macos: can this gpu take hap's block-compressed textures as they are stored?
//! usage: metal_caps
//! hap's lane uploads bc1, bc3, bc4, bc5 (ycocg and alpha), bc6h and bc7 blocks without decoding them.
//! not every metal device has these formats; on one that does not, hap would need a decode on the cpu.

use objc2_metal::{MTLDevice, MTLPixelFormat, MTLTextureUsage};
use spike_mac::Gpu;

fn main() {
    let gpu = Gpu::new();
    let supported = gpu.device.supportsBCTextureCompression();
    println!("{}: block-compressed (bc) textures supported: {supported}", gpu.device.name());
    let formats = [
        ("bc1 (hap)", MTLPixelFormat::BC1_RGBA),
        ("bc3 (hap alpha, hap q)", MTLPixelFormat::BC3_RGBA),
        ("bc4 (hap q alpha's alpha plane)", MTLPixelFormat::BC4_RUnorm),
        ("bc5", MTLPixelFormat::BC5_RGUnorm),
        ("bc6h unsigned (hap hdr)", MTLPixelFormat::BC6H_RGBUfloat),
        ("bc7 (hap r)", MTLPixelFormat::BC7_RGBAUnorm),
    ];
    let mut all = supported;
    for (name, format) in formats {
        // an 8K frame: the largest thing the hap lane is asked for
        let ok = supported && gpu.texture(format, 7680, 4320, MTLTextureUsage::ShaderRead).is_some();
        println!("  {name:<34} 7680 x 4320 texture: {}", if ok { "made" } else { "refused" });
        all &= ok;
    }
    std::process::exit(spike::verdict(all, "this gpu takes every hap block format as stored, at 8K"));
}
