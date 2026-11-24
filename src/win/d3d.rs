//! the direct3d 11 renderer: one device, one flip-model swapchain with a 10-bit back buffer, one triangle that covers
//! whatever rectangle the picture is drawn into. the device is shared with ffmpeg's hardware decoder, which writes
//! its frames into surfaces on it from the decoding thread; both sides take the device's own lock around their calls.

use crate::media::dec::Hardware;
use std::ffi::c_void;
use windows::core::{Interface, Result, PCSTR};
use windows::Win32::Foundation::{DXGI_STATUS_OCCLUDED, HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_SRV_DIMENSION_TEXTURE2DARRAY, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const VS: &str = "
struct O { float4 pos : SV_Position; };
O main(uint id : SV_VertexID) { O o; float2 uv = float2((id << 1) & 2, id & 2); o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1); return o; }";

/// what every picture shader is told. first where the picture goes: the rectangle's corner in window pixels, how many
/// source texels one window pixel covers, and the picture's size in texels. then, for y'cbcr pictures, the color
/// numbers from `media::color` and how much smaller the chroma planes are.
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
}

/// the hlsl for `Constants`, shared by every picture shader.
pub const CONSTANTS_HLSL: &str = "cbuffer Constants : register(b0) { float2 origin; float2 texels_per_pixel; float2 size; float2 chroma_per_luma; float4 scale_offset; float4 matrix_; };";

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    /// the device's own lock, recursive, which ffmpeg's decoder takes too
    multithread: ID3D11Multithread,
    swap: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    vs: ID3D11VertexShader,
    constants: ID3D11Buffer,
    pub w: u32,
    pub h: u32,
    /// while playing: the number of the first present counted, and the counters at the last look
    /// (presents, the refresh the last one was shown on)
    counting_from: Option<u32>,
    counted: Option<(u32, u32)>,
}

/// holds the device's lock until dropped.
pub struct Locked<'a>(&'a ID3D11Multithread);

impl Drop for Locked<'_> {
    fn drop(&mut self) {
        unsafe { self.0.Leave() };
    }
}

unsafe extern "C" fn lock(multithread: *mut c_void) {
    if let Some(m) = ID3D11Multithread::from_raw_borrowed(&multithread) {
        m.Enter();
    }
}

unsafe extern "C" fn unlock(multithread: *mut c_void) {
    if let Some(m) = ID3D11Multithread::from_raw_borrowed(&multithread) {
        m.Leave();
    }
}

/// compiled shaders are kept in the os cache folder, in our own folder there, one small file each.
fn shader_cache() -> Option<std::path::PathBuf> {
    Some(std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA")?).join("eyepiece").join("cache").join("shaders"))
}

/// shader bytecode, compiled on first use and read from the cache after that, so a start does not pay for the compiler.
fn compile(src: &str, target: &str) -> Result<Vec<u8>> {
    let mut hash = 0xcbf29ce484222325u64;
    for b in src.bytes().chain(target.bytes()) {
        hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let file = shader_cache().map(|d| d.join(format!("{hash:016x}.cso")));
    if let Some(code) = file.as_ref().and_then(|f| std::fs::read(f).ok()) {
        return Ok(code);
    }
    let (mut code, mut errs): (Option<ID3DBlob>, Option<ID3DBlob>) = (None, None);
    let t = std::ffi::CString::new(target).unwrap();
    let r = unsafe { D3DCompile(src.as_ptr() as *const c_void, src.len(), PCSTR::null(), None, None, PCSTR(c"main".as_ptr() as *const u8), PCSTR(t.as_ptr() as *const u8), 0, 0, &mut code, Some(&mut errs)) };
    if let Err(e) = r {
        if let Some(b) = errs {
            let msg = unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) };
            eprintln!("shader error: {}", String::from_utf8_lossy(msg));
        }
        return Err(e);
    }
    let b = code.unwrap();
    let code = unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }.to_vec();
    if let Some(f) = file {
        let _ = std::fs::create_dir_all(f.parent().unwrap());
        let _ = std::fs::write(&f, &code);
    }
    Ok(code)
}

impl Gpu {
    /// a hardware device and a swapchain on `hwnd`, `w` x `h` pixels.
    pub fn new(hwnd: HWND, w: u32, h: u32) -> Result<Gpu> {
        let (mut device, mut ctx) = (None, None);
        let levels: [D3D_FEATURE_LEVEL; 2] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
        unsafe {
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, Some(&levels), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))?;
            let (device, ctx): (ID3D11Device, ID3D11DeviceContext) = (device.unwrap(), ctx.unwrap());
            // the decoder uses this device from its own thread
            let multithread: ID3D11Multithread = ctx.cast()?;
            let _ = multithread.SetMultithreadProtected(true);
            // one frame may wait for the display, not the default three. with three, a present returns at once until
            // the queue is full: play starts with a burst of frames the reader cannot have ready, a step shows
            // three refreshes late, and the queue stutters as it settles
            device.cast::<IDXGIDevice1>()?.SetMaximumFrameLatency(1)?;
            let factory: IDXGIFactory2 = CreateDXGIFactory1()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: DXGI_FORMAT_R10G10B10A2_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                BufferUsage: DXGI_USAGE_RENDER_TARGET_OUTPUT,
                BufferCount: 2,
                Scaling: DXGI_SCALING_STRETCH,
                SwapEffect: DXGI_SWAP_EFFECT_FLIP_DISCARD,
                AlphaMode: DXGI_ALPHA_MODE_IGNORE,
                ..Default::default()
            };
            let swap = factory.CreateSwapChainForHwnd(&device, hwnd, &desc, None, None)?;
            let _ = factory.MakeWindowAssociation(hwnd, DXGI_MWA_NO_ALT_ENTER);
            let mut vs = None;
            device.CreateVertexShader(&compile(VS, "vs_5_0")?, None, Some(&mut vs))?;
            let buffer = D3D11_BUFFER_DESC { ByteWidth: std::mem::size_of::<Constants>() as u32, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() };
            let mut constants = None;
            device.CreateBuffer(&buffer, None, Some(&mut constants))?;
            let mut gpu = Gpu { device, ctx, multithread, swap, rtv: None, vs: vs.unwrap(), constants: constants.unwrap(), w, h, counting_from: None, counted: None };
            gpu.target()?;
            Ok(gpu)
        }
    }

    /// what a hardware decoder needs to write its frames onto this device. the decoder must be gone before the device.
    pub fn hardware(&self) -> Hardware {
        Hardware { device: self.device.as_raw(), lock, unlock, lock_ctx: self.multithread.as_raw() }
    }

    /// takes the device's lock for a run of calls, so the decoder's calls do not land in the middle of them.
    /// not to be held across a present: that waits for the display, and the decoder would wait with it.
    pub fn locked(&self) -> Locked<'_> {
        unsafe { self.multithread.Enter() };
        Locked(&self.multithread)
    }

    fn target(&mut self) -> Result<()> {
        unsafe {
            let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            self.device.CreateRenderTargetView(&back, None, Some(&mut self.rtv))
        }
    }

    /// the window's picture area changed size.
    pub fn resize(&mut self, w: u32, h: u32) -> Result<()> {
        if (w, h) == (self.w, self.h) {
            return Ok(());
        }
        // the back buffers cannot be resized while anything still points at them
        self.rtv = None;
        unsafe {
            self.ctx.OMSetRenderTargets(None, None);
            self.swap.ResizeBuffers(0, w, h, DXGI_FORMAT_UNKNOWN, DXGI_SWAP_CHAIN_FLAG(0))?;
        }
        (self.w, self.h) = (w, h);
        self.target()
    }

    pub fn pixel_shader(&self, hlsl: &str) -> Result<ID3D11PixelShader> {
        let mut ps = None;
        unsafe { self.device.CreatePixelShader(&compile(hlsl, "ps_5_0")?, None, Some(&mut ps))? };
        Ok(ps.unwrap())
    }

    /// a texture meant to be rewritten every frame, and a view of it.
    pub fn dynamic_texture(&self, w: u32, h: u32, format: DXGI_FORMAT) -> Result<(ID3D11Texture2D, ID3D11ShaderResourceView)> {
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: format,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DYNAMIC,
            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
            CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32,
            ..Default::default()
        };
        let (mut tex, mut view) = (None, None);
        unsafe {
            self.device.CreateTexture2D(&desc, None, Some(&mut tex))?;
            let tex: ID3D11Texture2D = tex.unwrap();
            self.device.CreateShaderResourceView(&tex, None, Some(&mut view))?;
            Ok((tex, view.unwrap()))
        }
    }

    /// rewrites a dynamic texture from ordinary memory: `rows` rows of `row_bytes` (rows of blocks for bc formats),
    /// which lie `stride` bytes apart in `data`. this is the one copy a frame from ordinary memory costs.
    pub fn fill(&self, tex: &ID3D11Texture2D, data: &[u8], rows: usize, row_bytes: usize, stride: usize) -> Result<()> {
        unsafe {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))?;
            let pitch = m.RowPitch as usize;
            let mem = std::slice::from_raw_parts_mut(m.pData as *mut u8, pitch * (rows - 1) + row_bytes);
            if pitch == row_bytes && stride == row_bytes {
                mem.copy_from_slice(&data[..rows * row_bytes]);
            } else {
                for r in 0..rows {
                    mem[r * pitch..r * pitch + row_bytes].copy_from_slice(&data[r * stride..r * stride + row_bytes]);
                }
            }
            self.ctx.Unmap(tex, 0);
        }
        Ok(())
    }

    /// two views onto one slice of a hardware decoder's texture array, so a shader reads the frame where it was
    /// decoded: the luma plane, and the plane that holds both chroma samples side by side. `texture` is the decoder's
    /// ID3D11Texture2D, borrowed. `deep` is true for more than 8 bits a sample.
    pub fn surface_views(&self, texture: *mut c_void, slice: u32, deep: bool) -> Result<(ID3D11ShaderResourceView, ID3D11ShaderResourceView)> {
        let view = |format: DXGI_FORMAT| -> Result<ID3D11ShaderResourceView> {
            let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
                Format: format,
                ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2DARRAY,
                Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 { Texture2DArray: D3D11_TEX2D_ARRAY_SRV { MostDetailedMip: 0, MipLevels: 1, FirstArraySlice: slice, ArraySize: 1 } },
            };
            let mut v = None;
            unsafe {
                let texture = ID3D11Texture2D::from_raw_borrowed(&texture).ok_or_else(windows::core::Error::empty)?;
                self.device.CreateShaderResourceView(texture, Some(&desc), Some(&mut v))?;
            }
            Ok(v.unwrap())
        };
        Ok(if deep { (view(DXGI_FORMAT_R16_UNORM)?, view(DXGI_FORMAT_R16G16_UNORM)?) } else { (view(DXGI_FORMAT_R8_UNORM)?, view(DXGI_FORMAT_R8G8_UNORM)?) })
    }

    /// starts a frame: the whole window black.
    pub fn clear(&self) {
        unsafe {
            // a flip-model swapchain unbinds its target at every present
            self.ctx.OMSetRenderTargets(Some(std::slice::from_ref(&self.rtv)), None);
            if let Some(rtv) = &self.rtv {
                self.ctx.ClearRenderTargetView(rtv, &[0.0, 0.0, 0.0, 1.0]);
            }
        }
    }

    /// draws `ps` over the rectangle (x, y, w, h) of the window, reading `views`. the placement fields of `constants`
    /// are filled in here; `size` is the picture in texels.
    pub fn draw(&self, ps: &ID3D11PixelShader, views: &[Option<ID3D11ShaderResourceView>], rect: (f32, f32, f32, f32), size: (f32, f32), mut constants: Constants) {
        let (x, y, w, h) = rect;
        (constants.origin, constants.texels_per_pixel, constants.size) = ([x, y], [size.0 / w, size.1 / h], [size.0, size.1]);
        unsafe {
            self.ctx.UpdateSubresource(&self.constants, 0, None, &constants as *const Constants as *const c_void, 0, 0);
            self.ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: x, TopLeftY: y, Width: w, Height: h, MinDepth: 0.0, MaxDepth: 1.0 }]));
            self.ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.ctx.VSSetShader(&self.vs, None);
            self.ctx.PSSetShader(ps, None);
            self.ctx.PSSetConstantBuffers(0, Some(&[Some(self.constants.clone())]));
            self.ctx.PSSetShaderResources(0, Some(views));
            self.ctx.Draw(3, 0);
            // let go of the views: a decoder's surface that is still bound cannot be decoded into again
            let none: Vec<Option<ID3D11ShaderResourceView>> = vec![None; views.len()];
            self.ctx.PSSetShaderResources(0, Some(&none));
        }
    }

    /// shows the frame at the next display refresh and waits for it. returns how many refreshes went by without a new
    /// frame since the last present, by the system's own count, when `counting`; pass false for a lone redraw, so the
    /// time spent standing still is never counted as missed.
    /// a window that cannot be seen (minimized, or fully covered) has no refresh to wait for: the wait is then
    /// `refresh` of ordinary sleep, so a clip that is playing keeps its pace and the loop does not spin.
    pub fn present(&mut self, counting: bool, refresh: std::time::Duration) -> u32 {
        if unsafe { self.swap.Present(1, DXGI_PRESENT(0)) } == DXGI_STATUS_OCCLUDED {
            std::thread::sleep(refresh);
            (self.counting_from, self.counted) = (None, None);
            return 0;
        }
        if !counting {
            (self.counting_from, self.counted) = (None, None);
            return 0;
        }
        let Ok(made) = (unsafe { self.swap.GetLastPresentCount() }) else { return 0 };
        let from = *self.counting_from.get_or_insert(made);
        let mut s = DXGI_FRAME_STATISTICS::default();
        // the statistics describe a present a few refreshes back. one from before the count began would charge
        // the whole time spent standing still as missed
        if unsafe { self.swap.GetFrameStatistics(&mut s) }.is_err() || s.PresentCount < from {
            return 0;
        }
        let now = (s.PresentCount, s.PresentRefreshCount);
        let missed = match self.counted {
            Some(was) if now.0 != was.0 => now.1.wrapping_sub(was.1).saturating_sub(now.0.wrapping_sub(was.0)),
            _ => 0,
        };
        if self.counted.map(|was| was.0) != Some(now.0) {
            self.counted = Some(now);
        }
        missed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::clip::{Clip, Lane};
    use crate::media::dec::Picture;
    use crate::media::source::Frame;
    use std::time::Duration;

    fn media(name: &str) -> String {
        format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))
    }

    /// a hardware device with no window: what the decoder needs, and enough to read its surfaces back.
    fn device() -> (ID3D11Device, ID3D11DeviceContext, ID3D11Multithread) {
        let (mut device, mut ctx) = (None, None);
        unsafe {
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_VIDEO_SUPPORT, Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)).expect("a hardware device");
            let (device, ctx): (ID3D11Device, ID3D11DeviceContext) = (device.unwrap(), ctx.unwrap());
            let multithread: ID3D11Multithread = ctx.cast().unwrap();
            let _ = multithread.SetMultithreadProtected(true);
            (device, ctx, multithread)
        }
    }

    /// a picture's luma as sample values, whichever side decoded it: a surface is copied out of the gpu to be read.
    fn luma(picture: &Picture, device: &ID3D11Device, ctx: &ID3D11DeviceContext, w: usize, h: usize) -> Vec<u16> {
        let layout = picture.layout().expect("layout");
        let deep = layout.stored.texel_bits > 8;
        let sample = |row: &[u8], x: usize| if deep { u16::from_le_bytes([row[x * 2], row[x * 2 + 1]]) >> if layout.stored.top_aligned { 16 - layout.stored.bits } else { 0 } } else { row[x] as u16 };
        if !layout.hardware {
            let (data, stride, _) = picture.plane(0, layout.chroma_shift);
            return (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| sample(&data[y * stride..], x)).collect();
        }
        unsafe {
            let (texture, slice) = picture.surface();
            let texture = ID3D11Texture2D::from_raw_borrowed(&texture).expect("surface");
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            texture.GetDesc(&mut desc);
            (desc.ArraySize, desc.Usage, desc.BindFlags, desc.CPUAccessFlags, desc.MiscFlags) = (1, D3D11_USAGE_STAGING, 0, D3D11_CPU_ACCESS_READ.0 as u32, 0);
            let mut copy = None;
            device.CreateTexture2D(&desc, None, Some(&mut copy)).expect("staging texture");
            let copy: ID3D11Texture2D = copy.unwrap();
            ctx.CopySubresourceRegion(&copy, 0, 0, 0, 0, texture, slice, None);
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            ctx.Map(&copy, 0, D3D11_MAP_READ, 0, Some(&mut m)).expect("map");
            let rows = std::slice::from_raw_parts(m.pData as *const u8, m.RowPitch as usize * h);
            let out = (0..h).flat_map(|y| (0..w).map(move |x| (y, x))).map(|(y, x)| sample(&rows[y * m.RowPitch as usize..], x)).collect();
            ctx.Unmap(&copy, 0);
            out
        }
    }

    /// the frames of a clip in `order`, as luma. with `hardware`, the gpu must take the stream.
    fn frames(name: &str, order: &[usize], hardware: Option<Hardware>, device: &ID3D11Device, ctx: &ID3D11DeviceContext) -> Vec<Vec<u16>> {
        let mut clip = Clip::open(&media(name)).expect("open");
        clip.start(hardware).expect("start");
        let Lane::Video(video) = &clip.lane else { panic!("not the video lane") };
        assert_eq!(video.layout.hardware, hardware.is_some(), "{name}: the gpu on this machine did not take this stream");
        order
            .iter()
            .map(|&frame| {
                video.source.want(frame, false);
                match video.source.get(frame, Duration::from_secs(20)) {
                    Frame::Ready(picture) => luma(&picture, device, ctx, clip.width, clip.height),
                    Frame::Pending => panic!("{name}: frame {frame} never arrived"),
                    Frame::Failed(why) => panic!("{name}: frame {frame}: {why}"),
                }
            })
            .collect()
    }

    /// the hardware decoder, asked for frames in any order, returns exactly the frames the software decoder returns
    /// straight through: the same samples, bit for bit. this is gate 1's check and the jump check in one, on the
    /// path the player really uses for h.264 and hevc. needs a gpu with a video decoder.
    #[test]
    fn the_hardware_decoder_returns_the_right_frames_in_any_order() {
        let (device, ctx, multithread) = device();
        let hardware = Hardware { device: device.as_raw(), lock, unlock, lock_ctx: multithread.as_raw() };
        for name in ["h264_320.mp4", "hevc10_320.mp4"] {
            let straight: Vec<usize> = (0..24).collect();
            let software = frames(name, &straight, None, &device, &ctx);
            assert!((1..24).all(|i| software[i] != software[i - 1]), "{name}: the test clip's frames should all differ");
            let order = [0, 13, 5, 23, 22, 21, 8, 7, 16, 15, 1, 23, 0, 9];
            let jumped = frames(name, &order, Some(hardware), &device, &ctx);
            for (frame, picture) in order.iter().zip(&jumped) {
                assert!(*picture == software[*frame], "{name}: frame {frame} from the hardware decoder, reached by a jump, is not the software decoder's frame {frame}");
            }
        }
    }
}
