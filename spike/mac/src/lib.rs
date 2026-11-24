//! metal helpers for the mac gate programs. throwaway.
#![cfg(target_os = "macos")]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::NSString;
use objc2_metal::*;
use std::ffi::c_void;
use std::ptr::NonNull;

pub type Tex = Retained<ProtocolObject<dyn MTLTexture>>;
pub type Pipeline = Retained<ProtocolObject<dyn MTLRenderPipelineState>>;

// one triangle that covers the whole target, no vertex buffer. uv has its origin at the top left, like the picture.
const VERTEX: &str = r#"
#include <metal_stdlib>
using namespace metal;
struct V { float4 pos [[position]]; float2 uv; };
vertex V vs(uint vid [[vertex_id]]) {
    V o;
    float2 p = float2((vid << 1) & 2, vid & 2);
    o.pos = float4(p * 2.0 - 1.0, 0.0, 1.0);
    o.uv = float2(p.x, 1.0 - p.y);
    return o;
}
"#;

pub struct Gpu {
    pub device: Retained<ProtocolObject<dyn MTLDevice>>,
    pub queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
}

impl Default for Gpu {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpu {
    pub fn new() -> Gpu {
        let device = MTLCreateSystemDefaultDevice().expect("a metal device");
        let queue = device.newCommandQueue().expect("a command queue");
        Gpu { device, queue }
    }

    /// a pipeline that draws over the whole target with `fragment`: metal source for a function `fs(V in [[stage_in]], ...)`.
    pub fn pipeline(&self, fragment: &str, target: MTLPixelFormat) -> Pipeline {
        let src = NSString::from_str(&format!("{VERTEX}\n{fragment}"));
        let lib = self.device.newLibraryWithSource_options_error(&src, None).unwrap_or_else(|e| panic!("shader: {}", e.localizedDescription()));
        let desc = MTLRenderPipelineDescriptor::new();
        desc.setVertexFunction(lib.newFunctionWithName(&NSString::from_str("vs")).as_deref());
        desc.setFragmentFunction(lib.newFunctionWithName(&NSString::from_str("fs")).as_deref());
        unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) }.setPixelFormat(target);
        self.device.newRenderPipelineStateWithDescriptor_error(&desc).unwrap_or_else(|e| panic!("pipeline: {}", e.localizedDescription()))
    }

    /// a texture we can draw into, sample and read back. None if the device refuses the format.
    pub fn texture(&self, format: MTLPixelFormat, w: usize, h: usize, usage: MTLTextureUsage) -> Option<Tex> {
        let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, w, h, false) };
        desc.setUsage(usage);
        desc.setStorageMode(MTLStorageMode::Shared);
        self.device.newTextureWithDescriptor(&desc)
    }

    /// draws `pipeline` over `target` with `inputs` bound in order, and waits until the gpu is done.
    pub fn draw(&self, pipeline: &Pipeline, target: &ProtocolObject<dyn MTLTexture>, inputs: &[&ProtocolObject<dyn MTLTexture>]) {
        let pass = MTLRenderPassDescriptor::renderPassDescriptor();
        let att = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        att.setTexture(Some(target));
        att.setLoadAction(MTLLoadAction::Clear);
        att.setStoreAction(MTLStoreAction::Store);
        let cb = self.queue.commandBuffer().expect("a command buffer");
        let enc = cb.renderCommandEncoderWithDescriptor(&pass).expect("an encoder");
        enc.setRenderPipelineState(pipeline);
        for (i, t) in inputs.iter().enumerate() {
            unsafe { enc.setFragmentTexture_atIndex(Some(t), i) };
        }
        unsafe { enc.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3) };
        enc.endEncoding();
        cb.commit();
        cb.waitUntilCompleted();
    }
}

/// a videotoolbox frame's pixel buffer, read through the four corevideo calls the gates need (plain c, declared by hand).
#[derive(Clone, Copy)]
pub struct PixelBuffer(pub *mut c_void);

#[link(name = "CoreVideo", kind = "framework")]
extern "C" {
    fn CVPixelBufferGetPixelFormatType(pb: *mut c_void) -> u32;
    fn CVPixelBufferGetWidthOfPlane(pb: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetHeightOfPlane(pb: *mut c_void, plane: usize) -> usize;
    fn CVPixelBufferGetIOSurface(pb: *mut c_void) -> *mut c_void;
}

impl PixelBuffer {
    /// the four-letter pixel format code, as text
    pub fn format(self) -> String {
        unsafe { CVPixelBufferGetPixelFormatType(self.0) }.to_be_bytes().iter().map(|&b| b as char).collect()
    }

    /// the io surface the decoder wrote into. null if the buffer has none.
    pub fn surface(self) -> *mut c_void {
        unsafe { CVPixelBufferGetIOSurface(self.0) }
    }

    /// a metal texture that IS plane `plane` of the decoder's surface: the same memory, nothing copied.
    pub fn plane(self, gpu: &Gpu, plane: usize, format: MTLPixelFormat) -> Option<Tex> {
        let surface = self.surface();
        if surface.is_null() {
            return None;
        }
        let (w, h) = unsafe { (CVPixelBufferGetWidthOfPlane(self.0, plane), CVPixelBufferGetHeightOfPlane(self.0, plane)) };
        let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, w, h, false) };
        desc.setUsage(MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        gpu.device.newTextureWithDescriptor_iosurface_plane(&desc, unsafe { &*(surface as *const objc2_io_surface::IOSurfaceRef) }, plane)
    }
}

/// the whole of a texture's first level, as bytes.
pub fn read(tex: &ProtocolObject<dyn MTLTexture>, bytes_per_pixel: usize) -> Vec<u8> {
    let (w, h) = (tex.width(), tex.height());
    let mut out = vec![0u8; w * h * bytes_per_pixel];
    let region = MTLRegion { origin: MTLOrigin { x: 0, y: 0, z: 0 }, size: MTLSize { width: w, height: h, depth: 1 } };
    unsafe { tex.getBytes_bytesPerRow_fromRegion_mipmapLevel(NonNull::new(out.as_mut_ptr() as *mut c_void).unwrap(), w * bytes_per_pixel, region, 0) };
    out
}
