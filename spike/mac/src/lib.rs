//! metal, window and videotoolbox helpers for the mac gate programs. throwaway.
#![cfg(target_os = "macos")]

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSEventMask, NSScreen, NSWindow, NSWindowStyleMask};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSDate, NSDefaultRunLoopMode, NSPoint, NSRect, NSSize, NSString};
use objc2_metal::*;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
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

/// 10-bit biplanar yuv to rgb, bt.709, video range. integer addressing: no filter anywhere. chroma is the nearest sample.
pub const YUV10_TO_RGB: &str = r#"
fragment float4 fs(V in [[stage_in]], texture2d<float> ty [[texture(0)]], texture2d<float> tc [[texture(1)]]) {
    uint2 p = uint2(in.pos.xy);
    float y = ty.read(p).r * (65535.0 / 64.0);
    float2 c = tc.read(p / 2).rg * (65535.0 / 64.0);
    float Y = (y - 64.0) / 876.0, Cb = (c.x - 512.0) / 896.0, Cr = (c.y - 512.0) / 896.0;
    return float4(saturate(float3(Y + 1.5748 * Cr, Y - 0.1873 * Cb - 0.4681 * Cr, Y + 1.8556 * Cb)), 1.0);
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

    /// one draw of `pipeline` over `target` with `inputs` bound in order, encoded and not yet sent.
    fn encode(&self, pipeline: &Pipeline, target: &ProtocolObject<dyn MTLTexture>, inputs: &[&ProtocolObject<dyn MTLTexture>]) -> Retained<ProtocolObject<dyn MTLCommandBuffer>> {
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
        cb
    }

    /// draws into a texture and waits until the gpu is done.
    pub fn draw(&self, pipeline: &Pipeline, target: &ProtocolObject<dyn MTLTexture>, inputs: &[&ProtocolObject<dyn MTLTexture>]) {
        let cb = self.encode(pipeline, target, inputs);
        cb.commit();
        cb.waitUntilCompleted();
    }

    /// draws into a window's drawable and presents it. `on_glass` is called later, on another thread, with the host
    /// time in seconds at which the frame reached the display (0 if it never did).
    pub fn present(&self, pipeline: &Pipeline, drawable: &ProtocolObject<dyn CAMetalDrawable>, inputs: &[&ProtocolObject<dyn MTLTexture>], on_glass: impl Fn(f64) + 'static) {
        let cb = self.encode(pipeline, &drawable.texture(), inputs);
        let block = block2::RcBlock::new(move |d: NonNull<ProtocolObject<dyn MTLDrawable>>| on_glass(unsafe { d.as_ref() }.presentedTime()));
        let d: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(drawable);
        unsafe { d.addPresentedHandler(block2::RcBlock::as_ptr(&block)) };
        cb.presentDrawable(d);
        cb.commit();
    }
}

/// a plain window holding one metal layer: 10-bit, synced to the display.
pub struct Window {
    app: Retained<NSApplication>,
    window: Retained<NSWindow>,
    pub layer: Retained<CAMetalLayer>,
    /// the display's nominal refresh rate
    pub refresh: f64,
}

impl Window {
    /// `w` x `h` are pixels, not points. the window is on screen when this returns.
    pub fn open(gpu: &Gpu, w: usize, h: usize) -> Window {
        let mtm = MainThreadMarker::new().expect("the main thread");
        let app = NSApplication::sharedApplication(mtm);
        // no dock icon: this is a test window
        app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
        app.finishLaunching();
        let screen = NSScreen::mainScreen(mtm).expect("a screen");
        let scale = screen.backingScaleFactor();
        let rect = NSRect::new(NSPoint::new(80.0, 80.0), NSSize::new(w as f64 / scale, h as f64 / scale));
        let window = unsafe { NSWindow::initWithContentRect_styleMask_backing_defer(mtm.alloc(), rect, NSWindowStyleMask::Titled, NSBackingStoreType::Buffered, false) };
        unsafe { window.setReleasedWhenClosed(false) };
        let layer = CAMetalLayer::new();
        layer.setDevice(Some(&gpu.device));
        layer.setPixelFormat(MTLPixelFormat::BGR10A2Unorm);
        layer.setDrawableSize(CGSize { width: w as f64, height: h as f64 });
        let view = window.contentView().expect("a content view");
        view.setLayer(Some(&layer));
        view.setWantsLayer(true);
        window.orderFrontRegardless();
        let win = Window { app, window, layer, refresh: screen.maximumFramesPerSecond() as f64 };
        win.pump();
        win
    }

    /// lets the window system deliver whatever is waiting. call it between frames.
    pub fn pump(&self) {
        while let Some(e) = self.app.nextEventMatchingMask_untilDate_inMode_dequeue(NSEventMask::Any, Some(&NSDate::distantPast()), unsafe { NSDefaultRunLoopMode }, true) {
            self.app.sendEvent(&e);
        }
    }

    pub fn close(&self) {
        self.window.close();
        self.pump();
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

extern "C" {
    fn getloadavg(avg: *mut f64, n: i32) -> i32;
}

/// the machine's one-minute load average: how busy it was while a timing ran. about 0 to 1 per core is idle to full.
pub fn load() -> f64 {
    let mut avg = [0f64; 3];
    unsafe { getloadavg(avg.as_mut_ptr(), 3) };
    avg[0]
}
