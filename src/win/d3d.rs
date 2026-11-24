//! the direct3d 11 renderer: one device, one flip-model swapchain with a 10-bit back buffer, one triangle that covers
//! whatever rectangle the picture is drawn into. everything here runs on the render thread.

use std::ffi::c_void;
use windows::core::{Interface, Result, PCSTR};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1, D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const VS: &str = "
struct O { float4 pos : SV_Position; };
O main(uint id : SV_VertexID) { O o; float2 uv = float2((id << 1) & 2, id & 2); o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1); return o; }";

/// where the picture goes in the window, for the shader: the rectangle's corner in pixels, how many source texels one
/// screen pixel covers, and the picture's size in texels.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Placement {
    pub origin: [f32; 2],
    pub texels_per_pixel: [f32; 2],
    pub size: [f32; 2],
    pub pad: [f32; 2],
}

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    swap: IDXGISwapChain1,
    rtv: Option<ID3D11RenderTargetView>,
    vs: ID3D11VertexShader,
    placement: ID3D11Buffer,
    pub w: u32,
    pub h: u32,
    /// while playing: the number of the first present counted, and the counters at the last look
    /// (presents, the refresh the last one was shown on)
    counting_from: Option<u32>,
    counted: Option<(u32, u32)>,
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
            let buffer = D3D11_BUFFER_DESC { ByteWidth: std::mem::size_of::<Placement>() as u32, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() };
            let mut placement = None;
            device.CreateBuffer(&buffer, None, Some(&mut placement))?;
            let mut gpu = Gpu { device, ctx, swap, rtv: None, vs: vs.unwrap(), placement: placement.unwrap(), w, h, counting_from: None, counted: None };
            gpu.target()?;
            Ok(gpu)
        }
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

    /// rewrites a dynamic texture from ordinary memory: `rows` rows of `row_bytes` (rows of blocks for bc formats).
    /// this is the hap lane's one copy.
    pub fn fill(&self, tex: &ID3D11Texture2D, data: &[u8], rows: usize, row_bytes: usize) -> Result<()> {
        unsafe {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))?;
            let pitch = m.RowPitch as usize;
            let mem = std::slice::from_raw_parts_mut(m.pData as *mut u8, pitch * (rows - 1) + row_bytes);
            if pitch == row_bytes {
                mem.copy_from_slice(&data[..rows * row_bytes]);
            } else {
                for r in 0..rows {
                    mem[r * pitch..r * pitch + row_bytes].copy_from_slice(&data[r * row_bytes..(r + 1) * row_bytes]);
                }
            }
            self.ctx.Unmap(tex, 0);
        }
        Ok(())
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

    /// draws `ps` over the rectangle (x, y, w, h) of the window, reading `srv`. `size` is the picture in texels.
    pub fn draw(&self, ps: &ID3D11PixelShader, srv: &ID3D11ShaderResourceView, rect: (f32, f32, f32, f32), size: (f32, f32)) {
        let (x, y, w, h) = rect;
        let placement = Placement { origin: [x, y], texels_per_pixel: [size.0 / w, size.1 / h], size: [size.0, size.1], pad: [0.0; 2] };
        unsafe {
            self.ctx.UpdateSubresource(&self.placement, 0, None, &placement as *const Placement as *const c_void, 0, 0);
            self.ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: x, TopLeftY: y, Width: w, Height: h, MinDepth: 0.0, MaxDepth: 1.0 }]));
            self.ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.ctx.VSSetShader(&self.vs, None);
            self.ctx.PSSetShader(ps, None);
            self.ctx.PSSetConstantBuffers(0, Some(&[Some(self.placement.clone())]));
            self.ctx.PSSetShaderResources(0, Some(&[Some(srv.clone())]));
            self.ctx.Draw(3, 0);
        }
    }

    /// shows the frame at the next display refresh and waits for it. returns how many refreshes went by without a new
    /// frame since the last present, by the system's own count, when `counting`; pass false for a lone redraw, so the
    /// time spent standing still is never counted as missed.
    pub fn present(&mut self, counting: bool) -> u32 {
        unsafe {
            let _ = self.swap.Present(1, DXGI_PRESENT(0));
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
