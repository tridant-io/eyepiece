//! the metal renderer: one device, one layer with a 10-bit drawable, one triangle that covers the target and is cut
//! to whatever rectangle the picture is drawn into. videotoolbox decodes into io surfaces, which a texture reads
//! where they are (gate 1).

use crate::media::dec::Hardware;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_core_foundation::CGSize;
use objc2_foundation::NSString;
use objc2_metal::*;
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub type Texture = Retained<ProtocolObject<dyn MTLTexture>>;
pub type Encoder = Retained<ProtocolObject<dyn MTLRenderCommandEncoder>>;

/// what a picture is drawn into: the window's drawable, or a picture of our own (`Gpu::read_pixels`). a metal
/// pipeline is made for one format of target, so every shader is made twice.
pub const TARGETS: [MTLPixelFormat; 2] = [MTLPixelFormat::BGR10A2Unorm, MTLPixelFormat::RGBA16Unorm];

/// one triangle over the whole target. the fragment's position is in the target's pixels, from its top left.
const VERTEX: &str = "
#include <metal_stdlib>
using namespace metal;
struct V { float4 pos [[position]]; };
vertex V vs(uint id [[vertex_id]]) { V o; float2 uv = float2((id << 1) & 2, id & 2); o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1); return o; }";

/// what every picture shader is told. first where the picture goes: the rectangle's corner in the target's pixels, how
/// many source texels one of those pixels covers, and the picture's size in texels. then, for y'cbcr pictures, the
/// color numbers from `media::color` and how much smaller the chroma planes are.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Constants {
    pub origin: [f32; 2],
    pub texels_per_pixel: [f32; 2],
    pub size: [f32; 2],
    pub chroma_per_luma: [f32; 2],
    /// y scale, y offset, chroma scale, chroma offset
    pub scale_offset: [f32; 4],
    /// cr to r, cb to g, cr to g, cb to b
    pub matrix: [f32; 4],
    /// from a texel of the picture as shown to the file's texel (`view::Look::to_source`): a matrix, row by row,
    /// and what is added after it
    pub to_source: [f32; 4],
    pub to_source_add: [f32; 2],
    /// 0 shows the picture; 1, 2, 3 and 4 show red, green, blue or alpha alone
    pub channel: f32,
    /// 1 blends the chroma samples round a pixel, 0 takes the nearest
    pub bilinear: f32,
    /// where a chroma sample sits among its luma samples (`media::dec::Layout::siting`)
    pub siting: [f32; 2],
    /// what a texel of the alpha plane is multiplied by to give alpha
    pub alpha_scale: f32,
    pub unused: f32,
}

/// the metal for `Constants`, shared by every picture shader: `k` in the fragment function.
pub const CONSTANTS_MSL: &str = "struct Constants { float2 origin; float2 texels_per_pixel; float2 size; float2 chroma_per_luma; float4 scale_offset; float4 matrix_; float4 to_source; float2 to_source_add; float channel; float bilinear; float2 siting; float alpha_scale; float unused_; };";

/// a fragment shader with the one vertex shader, made for each kind of target.
pub struct Shader(pub [Retained<ProtocolObject<dyn MTLRenderPipelineState>>; 2]);

/// the pass being drawn into.
struct Pass {
    buffer: Retained<ProtocolObject<dyn MTLCommandBuffer>>,
    encoder: Encoder,
    /// the window's drawable; none inside `read_pixels`
    drawable: Option<Retained<ProtocolObject<dyn CAMetalDrawable>>>,
    /// which of `TARGETS` it is
    target: usize,
    size: (u32, u32),
}

/// when the display showed the frames presented, as the system reports it later, and what that says was missed.
#[derive(Default)]
struct Shown {
    /// the last frame's time on the display, in seconds
    last: f64,
    /// refreshes that went by without a new frame since the count was last taken
    missed: u32,
}

pub struct Gpu {
    pub device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    /// none for a device with no window
    layer: Option<Retained<CAMetalLayer>>,
    pub w: u32,
    pub h: u32,
    /// the window's frame between `clear` and `present`, or a picture of our own inside `read_pixels`
    pass: RefCell<Option<Pass>>,
    shown: Arc<Mutex<Shown>>,
}

/// metal needs no lock of ours around its calls: the decoder writes into surfaces of its own.
pub struct Locked;

fn text(e: Retained<objc2_foundation::NSError>) -> String {
    e.localizedDescription().to_string()
}

impl Gpu {
    /// the default device, drawing into the window's layer, `w` x `h` pixels.
    pub fn new(window: super::window::Handle, w: u32, h: u32) -> Result<Gpu, String> {
        let layer = unsafe { Retained::retain(window.0 as *mut CAMetalLayer) }.ok_or("the window has no layer")?;
        let mut gpu = Gpu::offscreen()?;
        layer.setDevice(Some(&gpu.device));
        layer.setPixelFormat(TARGETS[0]);
        // a frame may wait for the display behind one other, not two: a step shows a refresh sooner
        layer.setMaximumDrawableCount(2);
        layer.setDrawableSize(CGSize { width: w as f64, height: h as f64 });
        (gpu.layer, gpu.w, gpu.h) = (Some(layer), w, h);
        Ok(gpu)
    }

    /// the default device with no window: it draws only into pictures of its own (`read_pixels`).
    pub fn offscreen() -> Result<Gpu, String> {
        let device = MTLCreateSystemDefaultDevice().ok_or("no metal device")?;
        let queue = device.newCommandQueue().ok_or("no command queue")?;
        Ok(Gpu { device, queue, layer: None, w: 0, h: 0, pass: RefCell::new(None), shown: Arc::default() })
    }

    /// what a hardware decoder needs of the renderer: nothing, here.
    pub fn hardware(&self) -> Hardware {
        Hardware
    }

    pub fn locked(&self) -> Locked {
        Locked
    }

    /// the window's picture area changed size.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<(), String> {
        if let Some(layer) = &self.layer {
            layer.setDrawableSize(CGSize { width: w as f64, height: h as f64 });
        }
        (self.w, self.h) = (w, h);
        Ok(())
    }

    /// `fragment` is metal source for a function `fs(V in [[stage_in]], ...)`.
    pub fn pixel_shader(&self, fragment: &str) -> Result<Shader, String> {
        self.shader(VERTEX, fragment, false)
    }

    /// a pipeline of `vertex` and `fragment` (functions `vs` and `fs`) for each kind of target. `blended` lays what
    /// is drawn over what is there, taking its color as already multiplied by its alpha.
    pub fn shader(&self, vertex: &str, fragment: &str, blended: bool) -> Result<Shader, String> {
        let source = NSString::from_str(&format!("{vertex}\n{fragment}"));
        let library = self.device.newLibraryWithSource_options_error(&source, None).map_err(text)?;
        let make = |target: MTLPixelFormat| {
            let desc = MTLRenderPipelineDescriptor::new();
            desc.setVertexFunction(library.newFunctionWithName(&NSString::from_str("vs")).as_deref());
            desc.setFragmentFunction(library.newFunctionWithName(&NSString::from_str("fs")).as_deref());
            let color = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
            color.setPixelFormat(target);
            if blended {
                color.setBlendingEnabled(true);
                color.setSourceRGBBlendFactor(MTLBlendFactor::One);
                color.setSourceAlphaBlendFactor(MTLBlendFactor::One);
                color.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
                color.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
            }
            self.device.newRenderPipelineStateWithDescriptor_error(&desc).map_err(text)
        };
        Ok(Shader([make(TARGETS[0])?, make(TARGETS[1])?]))
    }

    /// a texture meant to be rewritten every frame.
    pub fn dynamic_texture(&self, w: u32, h: u32, format: MTLPixelFormat) -> Result<Texture, String> {
        self.texture(w, h, format, MTLTextureUsage::ShaderRead)
    }

    fn texture(&self, w: u32, h: u32, format: MTLPixelFormat, usage: MTLTextureUsage) -> Result<Texture, String> {
        let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, w as usize, h as usize, false) };
        desc.setUsage(usage);
        // memory the cpu and the gpu share, as apple's own chips have it: written and read back with no copy between
        desc.setStorageMode(MTLStorageMode::Shared);
        self.device.newTextureWithDescriptor(&desc).ok_or_else(|| format!("no {w} x {h} texture"))
    }

    /// rewrites a texture from ordinary memory: `rows` rows of `row_bytes` (rows of blocks for bc formats), which lie
    /// `stride` bytes apart in `data`. this is the one copy a frame from ordinary memory costs.
    pub fn fill(&self, tex: &Texture, data: &[u8], rows: usize, row_bytes: usize, stride: usize) -> Result<(), String> {
        if data.len() < stride * (rows - 1) + row_bytes {
            return Err("the frame is shorter than its picture".into());
        }
        let region = MTLRegion { origin: MTLOrigin { x: 0, y: 0, z: 0 }, size: MTLSize { width: tex.width(), height: tex.height(), depth: 1 } };
        unsafe { tex.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, NonNull::new(data.as_ptr() as *mut c_void).unwrap(), stride) };
        Ok(())
    }

    /// two textures that are the planes of a videotoolbox frame's io surface, so a shader reads the frame where it
    /// was decoded: luma, and the plane that holds both chroma samples side by side. `buffer` is the frame's
    /// CVPixelBuffer, borrowed. `deep` is true for more than 8 bits a sample.
    pub fn surface_planes(&self, buffer: *mut c_void, deep: bool) -> Result<(Texture, Texture), String> {
        #[link(name = "CoreVideo", kind = "framework")]
        extern "C" {
            fn CVPixelBufferGetWidthOfPlane(buffer: *mut c_void, plane: usize) -> usize;
            fn CVPixelBufferGetHeightOfPlane(buffer: *mut c_void, plane: usize) -> usize;
            fn CVPixelBufferGetIOSurface(buffer: *mut c_void) -> *mut c_void;
        }
        let surface = unsafe { CVPixelBufferGetIOSurface(buffer) };
        if surface.is_null() {
            return Err("the decoder's frame has no surface".into());
        }
        let plane = |plane: usize, format: MTLPixelFormat| {
            let (w, h) = unsafe { (CVPixelBufferGetWidthOfPlane(buffer, plane), CVPixelBufferGetHeightOfPlane(buffer, plane)) };
            let desc = unsafe { MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(format, w, h, false) };
            desc.setUsage(MTLTextureUsage::ShaderRead);
            desc.setStorageMode(MTLStorageMode::Shared);
            self.device.newTextureWithDescriptor_iosurface_plane(&desc, unsafe { &*(surface as *const objc2_io_surface::IOSurfaceRef) }, plane).ok_or("no texture on the decoder's surface")
        };
        Ok(if deep { (plane(0, MTLPixelFormat::R16Unorm)?, plane(1, MTLPixelFormat::RG16Unorm)?) } else { (plane(0, MTLPixelFormat::R8Unorm)?, plane(1, MTLPixelFormat::RG8Unorm)?) })
    }

    /// starts a pass that draws into `target`, cleared to black.
    fn begin(&self, target: &ProtocolObject<dyn MTLTexture>) -> Option<(Retained<ProtocolObject<dyn MTLCommandBuffer>>, Encoder)> {
        let pass = MTLRenderPassDescriptor::renderPassDescriptor();
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        color.setTexture(Some(target));
        color.setLoadAction(MTLLoadAction::Clear);
        color.setClearColor(MTLClearColor { red: 0.0, green: 0.0, blue: 0.0, alpha: 1.0 });
        color.setStoreAction(MTLStoreAction::Store);
        let buffer = self.queue.commandBuffer()?;
        let encoder = buffer.renderCommandEncoderWithDescriptor(&pass)?;
        Some((buffer, encoder))
    }

    /// draws into a picture of our own instead of the window, and hands back its pixels: four 16-bit numbers each
    /// (red, green, blue, alpha; 65535 is 1.0), row by row. this is how a pixel's value and a screenshot are taken
    /// from the source picture, at its own size, never from what the window happens to show.
    /// `draw` does the drawing, with `Gpu::draw`. call it between frames.
    pub fn read_pixels(&self, w: u32, h: u32, draw: impl FnOnce(&Gpu)) -> Result<Vec<u16>, String> {
        let target = self.texture(w, h, TARGETS[1], MTLTextureUsage::RenderTarget)?;
        let (buffer, encoder) = self.begin(&target).ok_or("no pass")?;
        let outer = self.pass.replace(Some(Pass { buffer, encoder, drawable: None, target: 1, size: (w, h) }));
        draw(self);
        let Some(pass) = self.pass.replace(outer) else { return Err("the pass was taken".into()) };
        pass.encoder.endEncoding();
        pass.buffer.commit();
        pass.buffer.waitUntilCompleted();
        let mut out = vec![0u16; (w * h * 4) as usize];
        let region = MTLRegion { origin: MTLOrigin { x: 0, y: 0, z: 0 }, size: MTLSize { width: w as usize, height: h as usize, depth: 1 } };
        unsafe { target.getBytes_bytesPerRow_fromRegion_mipmapLevel(NonNull::new(out.as_mut_ptr() as *mut c_void).unwrap(), w as usize * 8, region, 0) };
        Ok(out)
    }

    /// starts a frame: the whole window black. waits here until the display has a back buffer free, which is what
    /// paces play to the refresh. with none in a second (a window that cannot be seen), nothing is drawn.
    pub fn clear(&self) {
        let Some(drawable) = self.layer.as_ref().and_then(|layer| layer.nextDrawable()) else { return };
        let Some((buffer, encoder)) = self.begin(&drawable.texture()) else { return };
        self.pass.replace(Some(Pass { buffer, encoder, drawable: Some(drawable), target: 0, size: (self.w, self.h) }));
    }

    /// the pass being drawn into, for the painter of the controls: its encoder, which of `TARGETS` it is, its size.
    pub fn pass(&self) -> Option<(Encoder, usize, (u32, u32))> {
        self.pass.borrow().as_ref().map(|pass| (pass.encoder.clone(), pass.target, pass.size))
    }

    /// draws `shader` over the rectangle (x, y, w, h) of the target, reading `textures`. `shown` is the picture's
    /// size in texels as it is shown (a quarter turn swaps its sides); where it goes is filled into `constants` here.
    pub fn draw(&self, shader: &Shader, textures: &[&Texture], rect: (f32, f32, f32, f32), shown: (f32, f32), mut constants: Constants) {
        let Some((encoder, target, size)) = self.pass() else { return };
        let (x, y, w, h) = rect;
        (constants.origin, constants.texels_per_pixel) = ([x, y], [shown.0 / w, shown.1 / h]);
        // the triangle covers the target and is cut to the rectangle: a cut must lie inside the target, and the
        // shader places texels from the uncut rectangle
        let (left, top) = (x.max(0.0).floor(), y.max(0.0).floor());
        let (right, bottom) = ((x + w).min(size.0 as f32).ceil(), (y + h).min(size.1 as f32).ceil());
        if right <= left || bottom <= top {
            return;
        }
        encoder.setRenderPipelineState(&shader.0[target]);
        encoder.setViewport(MTLViewport { originX: 0.0, originY: 0.0, width: size.0 as f64, height: size.1 as f64, znear: 0.0, zfar: 1.0 });
        encoder.setScissorRect(MTLScissorRect { x: left as usize, y: top as usize, width: (right - left) as usize, height: (bottom - top) as usize });
        unsafe {
            encoder.setFragmentBytes_length_atIndex(NonNull::from(&constants).cast(), std::mem::size_of::<Constants>(), 0);
            for (i, texture) in textures.iter().enumerate() {
                encoder.setFragmentTexture_atIndex(Some(texture), i);
            }
            encoder.drawPrimitives_vertexStart_vertexCount(MTLPrimitiveType::Triangle, 0, 3);
        }
    }

    /// sends the frame to the display, which shows it at a refresh. returns how many refreshes went by without a new
    /// frame since the last present that asked, by the times the system reports frames reached the display, when
    /// `counting`; pass false for a lone redraw, so the time spent standing still is never counted as missed.
    /// a window that cannot be seen has no drawable: the wait is then `refresh` of ordinary sleep, so a clip that is
    /// playing keeps its pace and the loop does not spin.
    pub fn present(&mut self, counting: bool, refresh: Duration) -> u32 {
        let Some(pass) = self.pass.take() else {
            std::thread::sleep(refresh);
            *self.shown.lock().unwrap() = Shown::default();
            return 0;
        };
        pass.encoder.endEncoding();
        if let Some(drawable) = &pass.drawable {
            let (shown, period) = (self.shown.clone(), refresh.as_secs_f64());
            // called later, on a thread of the system's, with when the frame reached the display: 0 if it never did
            let reached = block2::RcBlock::new(move |drawable: NonNull<ProtocolObject<dyn MTLDrawable>>| {
                let at = unsafe { drawable.as_ref() }.presentedTime();
                let mut shown = shown.lock().unwrap();
                if at > 0.0 {
                    if shown.last > 0.0 && period > 0.0 {
                        shown.missed += (((at - shown.last) / period).round() as u32).saturating_sub(1);
                    }
                    shown.last = at;
                }
            });
            let drawable: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(&**drawable);
            unsafe { drawable.addPresentedHandler(block2::RcBlock::as_ptr(&reached)) };
            pass.buffer.presentDrawable(drawable);
        }
        pass.buffer.commit();
        let mut shown = self.shown.lock().unwrap();
        if !counting {
            *shown = Shown::default();
            return 0;
        }
        std::mem::take(&mut shown.missed)
    }
}
