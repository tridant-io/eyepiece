//! the direct3d 11 presenter the gates share: one device, one flip-model swapchain, one full-window triangle.

use std::ffi::c_void;
use windows::core::{Interface, Result, PCSTR};
use windows::Win32::Foundation::{HMODULE, HWND};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

const VS: &str = "
struct O { float4 pos : SV_Position; };
O main(uint id : SV_VertexID) { O o; float2 uv = float2((id << 1) & 2, id & 2); o.pos = float4(uv * float2(2, -2) + float2(-1, 1), 0, 1); return o; }";

pub struct Gpu {
    pub device: ID3D11Device,
    pub ctx: ID3D11DeviceContext,
    pub swap: IDXGISwapChain1,
    pub rtv: ID3D11RenderTargetView,
    pub vs: ID3D11VertexShader,
    pub w: u32,
    pub h: u32,
}

/// shader bytecode, compiled once and then read from a small cache in the temp folder.
/// the player will ship precompiled shaders; the cache stands in for that, so warm runs do not pay for the compiler.
pub fn compile(src: &str, target: &str) -> Result<Vec<u8>> {
    let mut hash = 0xcbf29ce484222325u64;
    for b in src.bytes().chain(target.bytes()) {
        hash = (hash ^ b as u64).wrapping_mul(0x100000001b3);
    }
    let dir = std::env::temp_dir().join("eyepiece-gate-shaders");
    let file = dir.join(format!("{hash:016x}.cso"));
    if let Ok(code) = std::fs::read(&file) {
        return Ok(code);
    }
    let code = compile_now(src, target)?;
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(&file, &code);
    Ok(code)
}

fn compile_now(src: &str, target: &str) -> Result<Vec<u8>> {
    let (mut code, mut errs): (Option<ID3DBlob>, Option<ID3DBlob>) = (None, None);
    let t = std::ffi::CString::new(target).unwrap();
    let r = unsafe { D3DCompile(src.as_ptr() as *const c_void, src.len(), PCSTR::null(), None, None, PCSTR(b"main\0".as_ptr()), PCSTR(t.as_ptr() as *const u8), 0, 0, &mut code, Some(&mut errs)) };
    if let Err(e) = r {
        if let Some(b) = errs {
            let msg = unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) };
            eprintln!("shader error: {}", String::from_utf8_lossy(msg));
        }
        return Err(e);
    }
    let b = code.unwrap();
    Ok(unsafe { std::slice::from_raw_parts(b.GetBufferPointer() as *const u8, b.GetBufferSize()) }.to_vec())
}

/// a hardware device with video support, safe to call from the decoder's threads.
pub fn device() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut device, mut ctx) = (None, None);
    let levels: [D3D_FEATURE_LEVEL; 2] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
    unsafe {
        D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT, Some(&levels), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))?;
    }
    let (device, ctx): (ID3D11Device, ID3D11DeviceContext) = (device.unwrap(), ctx.unwrap());
    unsafe {
        let _ = ctx.cast::<ID3D11Multithread>()?.SetMultithreadProtected(true);
    }
    Ok((device, ctx))
}

impl Gpu {
    /// a swapchain on `hwnd`. `ten_bit` asks for a 10-bit back buffer.
    pub fn new(device: ID3D11Device, ctx: ID3D11DeviceContext, hwnd: HWND, w: u32, h: u32, ten_bit: bool) -> Result<Gpu> {
        unsafe {
            let factory: IDXGIFactory2 = CreateDXGIFactory1()?;
            let desc = DXGI_SWAP_CHAIN_DESC1 {
                Width: w,
                Height: h,
                Format: if ten_bit { DXGI_FORMAT_R10G10B10A2_UNORM } else { DXGI_FORMAT_B8G8R8A8_UNORM },
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
            let back: ID3D11Texture2D = swap.GetBuffer(0)?;
            let mut rtv = None;
            device.CreateRenderTargetView(&back, None, Some(&mut rtv))?;
            let mut vs = None;
            device.CreateVertexShader(&compile(VS, "vs_5_0")?, None, Some(&mut vs))?;
            Ok(Gpu { device, ctx, swap, rtv: rtv.unwrap(), vs: vs.unwrap(), w, h })
        }
    }

    pub fn pixel_shader(&self, hlsl: &str) -> Result<ID3D11PixelShader> {
        let mut ps = None;
        unsafe { self.device.CreatePixelShader(&compile(hlsl, "ps_5_0")?, None, Some(&mut ps))? };
        Ok(ps.unwrap())
    }

    /// draws the full-window triangle with `ps`, reading `srvs`.
    pub fn draw(&self, ps: &ID3D11PixelShader, srvs: &[Option<ID3D11ShaderResourceView>]) {
        unsafe {
            // a flip-model swapchain unbinds its target at every present
            self.ctx.OMSetRenderTargets(Some(&[Some(self.rtv.clone())]), None);
            self.ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: self.w as f32, Height: self.h as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            self.ctx.IASetPrimitiveTopology(windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            self.ctx.VSSetShader(&self.vs, None);
            self.ctx.PSSetShader(ps, None);
            self.ctx.PSSetShaderResources(0, Some(srvs));
            self.ctx.Draw(3, 0);
            // let go of the decoder's texture so it can be decoded into again
            let none: Vec<Option<ID3D11ShaderResourceView>> = vec![None; srvs.len()];
            self.ctx.PSSetShaderResources(0, Some(&none));
        }
    }

    pub fn present(&self, sync_interval: u32) -> windows::core::HRESULT {
        unsafe { self.swap.Present(sync_interval, DXGI_PRESENT(0)) }
    }

    pub fn frame_stats(&self) -> Option<DXGI_FRAME_STATISTICS> {
        let mut s = DXGI_FRAME_STATISTICS::default();
        unsafe { self.swap.GetFrameStatistics(&mut s).ok().map(|_| s) }
    }

    /// the back buffer as packed pixels, row by row. for checking only: the player never reads the screen back.
    pub fn read_backbuffer(&self) -> Result<Vec<u32>> {
        unsafe {
            let back: ID3D11Texture2D = self.swap.GetBuffer(0)?;
            read_texture(&self.device, &self.ctx, &back, 0, self.w, self.h).map(|(px, _)| px)
        }
    }
}

/// copies one subresource of a texture to system memory. returns 32-bit words per row for packed formats,
/// and the raw bytes with their row pitch for planar ones.
pub unsafe fn read_texture(device: &ID3D11Device, ctx: &ID3D11DeviceContext, tex: &ID3D11Texture2D, subresource: u32, w: u32, h: u32) -> Result<(Vec<u32>, (Vec<u8>, usize))> {
    let mut desc = D3D11_TEXTURE2D_DESC::default();
    tex.GetDesc(&mut desc);
    let planar = desc.Format == DXGI_FORMAT_P010 || desc.Format == DXGI_FORMAT_NV12;
    desc.Width = w;
    desc.Height = h;
    desc.ArraySize = 1;
    desc.MipLevels = 1;
    desc.Usage = D3D11_USAGE_STAGING;
    desc.BindFlags = 0;
    desc.CPUAccessFlags = D3D11_CPU_ACCESS_READ.0 as u32;
    desc.MiscFlags = 0;
    let mut st = None;
    device.CreateTexture2D(&desc, None, Some(&mut st))?;
    let st: ID3D11Texture2D = st.unwrap();
    ctx.CopySubresourceRegion(&st, 0, 0, 0, 0, tex, subresource, None);
    let mut m = D3D11_MAPPED_SUBRESOURCE::default();
    ctx.Map(&st, 0, D3D11_MAP_READ, 0, Some(&mut m))?;
    let pitch = m.RowPitch as usize;
    let out = if planar {
        // luma rows, then the interleaved chroma rows at half height
        let bytes = std::slice::from_raw_parts(m.pData as *const u8, pitch * (h as usize + h as usize / 2)).to_vec();
        (Vec::new(), (bytes, pitch))
    } else {
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            px.extend_from_slice(std::slice::from_raw_parts((m.pData as *const u8).add(y * pitch) as *const u32, w as usize));
        }
        (px, (Vec::new(), pitch))
    };
    ctx.Unmap(&st, 0);
    Ok(out)
}

/// a view of one plane of one slice of a decoder's texture array, so a shader can read it where it lies.
pub fn plane_view(device: &ID3D11Device, tex: &ID3D11Texture2D, slice: u32, format: DXGI_FORMAT) -> Result<ID3D11ShaderResourceView> {
    let desc = D3D11_SHADER_RESOURCE_VIEW_DESC {
        Format: format,
        ViewDimension: windows::Win32::Graphics::Direct3D::D3D11_SRV_DIMENSION_TEXTURE2DARRAY,
        Anonymous: D3D11_SHADER_RESOURCE_VIEW_DESC_0 { Texture2DArray: D3D11_TEX2D_ARRAY_SRV { MostDetailedMip: 0, MipLevels: 1, FirstArraySlice: slice, ArraySize: 1 } },
    };
    let mut v = None;
    unsafe { device.CreateShaderResourceView(tex, Some(&desc), Some(&mut v))? };
    Ok(v.unwrap())
}

/// a texture filled once from system memory, and a view of it. `pitch` is bytes per row (per row of blocks for bc formats).
pub fn texture_with_data(device: &ID3D11Device, w: u32, h: u32, format: DXGI_FORMAT, data: &[u8], pitch: u32) -> Result<(ID3D11Texture2D, ID3D11ShaderResourceView)> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: w,
        Height: h,
        MipLevels: 1,
        ArraySize: 1,
        Format: format,
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_IMMUTABLE,
        BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
        ..Default::default()
    };
    let init = D3D11_SUBRESOURCE_DATA { pSysMem: data.as_ptr() as *const c_void, SysMemPitch: pitch, SysMemSlicePitch: 0 };
    let (mut tex, mut view) = (None, None);
    unsafe {
        device.CreateTexture2D(&desc, Some(&init), Some(&mut tex))?;
        let tex: ID3D11Texture2D = tex.unwrap();
        device.CreateShaderResourceView(&tex, None, Some(&mut view))?;
        Ok((tex, view.unwrap()))
    }
}

/// a texture meant to be rewritten every frame, and a view of it.
pub fn dynamic_texture(device: &ID3D11Device, w: u32, h: u32, format: DXGI_FORMAT) -> Result<(ID3D11Texture2D, ID3D11ShaderResourceView)> {
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
        device.CreateTexture2D(&desc, None, Some(&mut tex))?;
        let tex: ID3D11Texture2D = tex.unwrap();
        device.CreateShaderResourceView(&tex, None, Some(&mut view))?;
        Ok((tex, view.unwrap()))
    }
}

/// maps a dynamic texture for a full rewrite and hands the caller the gpu's own memory to fill.
/// `rows` is the number of rows (rows of blocks for bc formats) and `row_bytes` what one row holds.
/// when the driver's pitch equals `row_bytes` the caller gets one contiguous slice, so data can be decompressed straight into it.
pub fn fill_dynamic(ctx: &ID3D11DeviceContext, tex: &ID3D11Texture2D, rows: usize, row_bytes: usize, fill: impl FnOnce(&mut [u8], usize)) -> Result<()> {
    unsafe {
        let mut m = D3D11_MAPPED_SUBRESOURCE::default();
        ctx.Map(tex, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))?;
        let pitch = m.RowPitch as usize;
        let len = pitch * (rows - 1) + row_bytes;
        fill(std::slice::from_raw_parts_mut(m.pData as *mut u8, len), pitch);
        ctx.Unmap(tex, 0);
    }
    Ok(())
}

/// the software rasterizer as a device. about 10 ms to create, against about 160 for the hardware one.
pub fn warp() -> Result<(ID3D11Device, ID3D11DeviceContext)> {
    let (mut device, mut ctx) = (None, None);
    unsafe {
        D3D11CreateDevice(None, windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_WARP, HMODULE::default(), D3D11_CREATE_DEVICE_FLAG(0), Some(&[D3D_FEATURE_LEVEL_11_0]), D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx))?;
    }
    Ok((device.unwrap(), ctx.unwrap()))
}

pub struct CpuPlane<'a> {
    pub w: u32,
    pub h: u32,
    pub format: DXGI_FORMAT,
    pub data: &'a [u8],
    pub pitch: u32,
}

/// paints one frame into `hwnd` without the hardware device, which takes far longer to create than everything else in a launch.
/// the software rasterizer runs the same pixel shader offscreen, and gdi puts the result in the window.
/// `warp` must be made with `warp()` BEFORE the hardware device starts: device creation is serialized inside direct3d,
/// so a software device asked for while the hardware one is being made waits for it, which defeats the purpose.
/// returns the milliseconds spent on (shaders, draw and read back, the gdi blit).
pub fn cpu_first_frame(warp: &(ID3D11Device, ID3D11DeviceContext), hwnd: HWND, w: u32, h: u32, ps_src: &str, planes: &[CpuPlane]) -> Result<[f64; 3]> {
    use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
    use windows::Win32::Graphics::Gdi::{GetDC, ReleaseDC, SetDIBitsToDevice, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS};
    let t0 = std::time::Instant::now();
    unsafe {
        let (device, ctx) = (warp.0.clone(), warp.1.clone());
        let (mut vs, mut ps) = (None, None);
        device.CreateVertexShader(&compile(VS, "vs_5_0")?, None, Some(&mut vs))?;
        device.CreatePixelShader(&compile(ps_src, "ps_5_0")?, None, Some(&mut ps))?;
        let t1 = std::time::Instant::now();
        let mut views = Vec::new();
        for p in planes {
            views.push(Some(texture_with_data(&device, p.w, p.h, p.format, p.data, p.pitch)?.1));
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: w,
            Height: h,
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            ..Default::default()
        };
        let (mut target, mut rtv) = (None, None);
        device.CreateTexture2D(&desc, None, Some(&mut target))?;
        let target: ID3D11Texture2D = target.unwrap();
        device.CreateRenderTargetView(&target, None, Some(&mut rtv))?;
        ctx.OMSetRenderTargets(Some(&[rtv]), None);
        ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w as f32, Height: h as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
        ctx.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
        ctx.VSSetShader(&vs.unwrap(), None);
        ctx.PSSetShader(&ps.unwrap(), None);
        ctx.PSSetShaderResources(0, Some(&views));
        ctx.Draw(3, 0);
        let (px, _) = read_texture(&device, &ctx, &target, 0, w, h)?;
        let t2 = std::time::Instant::now();
        let info = BITMAPINFO { bmiHeader: BITMAPINFOHEADER { biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32, biWidth: w as i32, biHeight: -(h as i32), biPlanes: 1, biBitCount: 32, biCompression: BI_RGB.0, ..Default::default() }, ..Default::default() };
        let dc = GetDC(Some(hwnd));
        SetDIBitsToDevice(dc, 0, 0, w, h, 0, 0, 0, h, px.as_ptr() as *const c_void, &info, DIB_RGB_COLORS);
        ReleaseDC(Some(hwnd), dc);
        let t3 = std::time::Instant::now();
        Ok([(t1 - t0).as_secs_f64() * 1000.0, (t2 - t1).as_secs_f64() * 1000.0, (t3 - t2).as_secs_f64() * 1000.0])
    }
}
