//! draws the controls with direct3d 11: the triangles egui produces, over the picture, in the same frame.
//! colors arrive premultiplied and gamma-encoded, and are blended as they are: that is what egui lays out for.

use super::d3d::{compile, Gpu};
use std::collections::HashMap;
use std::ffi::c_void;
use windows::core::{Result, PCSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::*;

const VS: &str = "
cbuffer Screen : register(b0) { float2 screen; float2 pad; };
struct I { float2 pos : POSITION; float2 uv : TEXCOORD; float4 color : COLOR; };
struct O { float4 pos : SV_Position; float2 uv : TEXCOORD; float4 color : COLOR; };
O main(I i) { O o; o.pos = float4(i.pos.x / screen.x * 2 - 1, 1 - i.pos.y / screen.y * 2, 0, 1); o.uv = i.uv; o.color = i.color; return o; }";

const PS: &str = "
Texture2D t : register(t0);
SamplerState s : register(s0);
struct O { float4 pos : SV_Position; float2 uv : TEXCOORD; float4 color : COLOR; };
float4 main(O i) : SV_Target { return i.color * t.Sample(s, i.uv); }";

/// one run of triangles that share a texture and a clip rectangle
struct Draw {
    texture: egui::TextureId,
    clip: RECT,
    first_index: u32,
    indices: u32,
    first_vertex: i32,
}

pub struct UiPainter {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    layout: ID3D11InputLayout,
    blend: ID3D11BlendState,
    raster: ID3D11RasterizerState,
    sampler: ID3D11SamplerState,
    screen: ID3D11Buffer,
    /// the vertex and index buffers and how many items each can hold
    vertices: Option<(ID3D11Buffer, usize)>,
    indices: Option<(ID3D11Buffer, usize)>,
    textures: HashMap<egui::TextureId, (ID3D11Texture2D, ID3D11ShaderResourceView)>,
}

fn element(name: &'static std::ffi::CStr, format: DXGI_FORMAT, offset: u32) -> D3D11_INPUT_ELEMENT_DESC {
    D3D11_INPUT_ELEMENT_DESC { SemanticName: PCSTR(name.as_ptr() as *const u8), SemanticIndex: 0, Format: format, InputSlot: 0, AlignedByteOffset: offset, InputSlotClass: D3D11_INPUT_PER_VERTEX_DATA, InstanceDataStepRate: 0 }
}

impl UiPainter {
    pub fn new(gpu: &Gpu) -> Result<UiPainter> {
        let d = &gpu.device;
        unsafe {
            let code = compile(VS, "vs_5_0")?;
            let (mut vs, mut layout) = (None, None);
            d.CreateVertexShader(&code, None, Some(&mut vs))?;
            // egui's vertex: a position and a texture coordinate as two floats each, then four bytes of color
            let elements = [element(c"POSITION", DXGI_FORMAT_R32G32_FLOAT, 0), element(c"TEXCOORD", DXGI_FORMAT_R32G32_FLOAT, 8), element(c"COLOR", DXGI_FORMAT_R8G8B8A8_UNORM, 16)];
            d.CreateInputLayout(&elements, &code, Some(&mut layout))?;

            let mut blend_desc = D3D11_BLEND_DESC::default();
            blend_desc.RenderTarget[0] = D3D11_RENDER_TARGET_BLEND_DESC {
                BlendEnable: true.into(),
                SrcBlend: D3D11_BLEND_ONE,
                DestBlend: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOp: D3D11_BLEND_OP_ADD,
                SrcBlendAlpha: D3D11_BLEND_ONE,
                DestBlendAlpha: D3D11_BLEND_INV_SRC_ALPHA,
                BlendOpAlpha: D3D11_BLEND_OP_ADD,
                RenderTargetWriteMask: D3D11_COLOR_WRITE_ENABLE_ALL.0 as u8,
            };
            let mut blend = None;
            d.CreateBlendState(&blend_desc, Some(&mut blend))?;

            let raster_desc = D3D11_RASTERIZER_DESC { FillMode: D3D11_FILL_SOLID, CullMode: D3D11_CULL_NONE, ScissorEnable: true.into(), DepthClipEnable: true.into(), ..Default::default() };
            let mut raster = None;
            d.CreateRasterizerState(&raster_desc, Some(&mut raster))?;

            let sampler_desc = D3D11_SAMPLER_DESC { Filter: D3D11_FILTER_MIN_MAG_MIP_LINEAR, AddressU: D3D11_TEXTURE_ADDRESS_CLAMP, AddressV: D3D11_TEXTURE_ADDRESS_CLAMP, AddressW: D3D11_TEXTURE_ADDRESS_CLAMP, MaxLOD: f32::MAX, ..Default::default() };
            let mut sampler = None;
            d.CreateSamplerState(&sampler_desc, Some(&mut sampler))?;

            let screen_desc = D3D11_BUFFER_DESC { ByteWidth: 16, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, ..Default::default() };
            let mut screen = None;
            d.CreateBuffer(&screen_desc, None, Some(&mut screen))?;

            Ok(UiPainter {
                vs: vs.unwrap(),
                ps: gpu.pixel_shader(PS)?,
                layout: layout.unwrap(),
                blend: blend.unwrap(),
                raster: raster.unwrap(),
                sampler: sampler.unwrap(),
                screen: screen.unwrap(),
                vertices: None,
                indices: None,
                textures: HashMap::new(),
            })
        }
    }

    /// takes new and changed textures (the font atlas, mostly). call before `paint`.
    pub fn set_textures(&mut self, gpu: &Gpu, delta: &egui::TexturesDelta) -> Result<()> {
        for (id, changes) in &delta.set {
            for change in changes {
                let egui::ImageData::Color(image) = &change.image;
                let [w, h] = image.size;
                // a pixel is four bytes, premultiplied, in the order the texture format wants
                let bytes = unsafe { std::slice::from_raw_parts(image.pixels.as_ptr() as *const u8, image.pixels.len() * 4) };
                match change.pos {
                    None => {
                        let desc = D3D11_TEXTURE2D_DESC {
                            Width: w as u32,
                            Height: h as u32,
                            MipLevels: 1,
                            ArraySize: 1,
                            Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                            Usage: D3D11_USAGE_DEFAULT,
                            BindFlags: D3D11_BIND_SHADER_RESOURCE.0 as u32,
                            ..Default::default()
                        };
                        let init = D3D11_SUBRESOURCE_DATA { pSysMem: bytes.as_ptr() as *const c_void, SysMemPitch: (w * 4) as u32, SysMemSlicePitch: 0 };
                        let (mut tex, mut view) = (None, None);
                        unsafe {
                            gpu.device.CreateTexture2D(&desc, Some(&init), Some(&mut tex))?;
                            let tex: ID3D11Texture2D = tex.unwrap();
                            gpu.device.CreateShaderResourceView(&tex, None, Some(&mut view))?;
                            self.textures.insert(*id, (tex, view.unwrap()));
                        }
                    }
                    Some([x, y]) => {
                        if let Some((tex, _)) = self.textures.get(id) {
                            let region = D3D11_BOX { left: x as u32, top: y as u32, front: 0, right: (x + w) as u32, bottom: (y + h) as u32, back: 1 };
                            unsafe { gpu.ctx.UpdateSubresource(tex, 0, Some(&region), bytes.as_ptr() as *const c_void, (w * 4) as u32, 0) };
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// lets go of textures egui no longer uses. call after `paint`.
    pub fn free_textures(&mut self, delta: &egui::TexturesDelta) {
        for id in &delta.free {
            self.textures.remove(id);
        }
    }

    /// a buffer the cpu rewrites every frame, at least `items` long. kept, and only replaced by a longer one.
    fn buffer(gpu: &Gpu, slot: &mut Option<(ID3D11Buffer, usize)>, items: usize, item_bytes: usize, bind: D3D11_BIND_FLAG) -> Result<ID3D11Buffer> {
        if let Some((buffer, capacity)) = slot {
            if *capacity >= items {
                return Ok(buffer.clone());
            }
        }
        let capacity = items.next_power_of_two().max(1024);
        let desc = D3D11_BUFFER_DESC { ByteWidth: (capacity * item_bytes) as u32, Usage: D3D11_USAGE_DYNAMIC, BindFlags: bind.0 as u32, CPUAccessFlags: D3D11_CPU_ACCESS_WRITE.0 as u32, ..Default::default() };
        let mut buffer = None;
        unsafe { gpu.device.CreateBuffer(&desc, None, Some(&mut buffer))? };
        let buffer = buffer.unwrap();
        *slot = Some((buffer.clone(), capacity));
        Ok(buffer)
    }

    /// draws the controls over what is already in the frame. `scale` is physical pixels for one of egui's units.
    pub fn paint(&mut self, gpu: &Gpu, primitives: &[egui::ClippedPrimitive], scale: f32) -> Result<()> {
        let (mut vertices, mut indices, mut draws): (Vec<egui::epaint::Vertex>, Vec<u32>, Vec<Draw>) = (Vec::new(), Vec::new(), Vec::new());
        for p in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else { continue };
            let clip = RECT {
                left: ((p.clip_rect.min.x * scale).round() as i32).clamp(0, gpu.w as i32),
                top: ((p.clip_rect.min.y * scale).round() as i32).clamp(0, gpu.h as i32),
                right: ((p.clip_rect.max.x * scale).round() as i32).clamp(0, gpu.w as i32),
                bottom: ((p.clip_rect.max.y * scale).round() as i32).clamp(0, gpu.h as i32),
            };
            if mesh.indices.is_empty() || clip.right <= clip.left || clip.bottom <= clip.top {
                continue;
            }
            draws.push(Draw { texture: mesh.texture_id, clip, first_index: indices.len() as u32, indices: mesh.indices.len() as u32, first_vertex: vertices.len() as i32 });
            vertices.extend_from_slice(&mesh.vertices);
            indices.extend_from_slice(&mesh.indices);
        }
        if draws.is_empty() {
            return Ok(());
        }
        let vertex_bytes = std::mem::size_of::<egui::epaint::Vertex>();
        let vb = Self::buffer(gpu, &mut self.vertices, vertices.len(), vertex_bytes, D3D11_BIND_VERTEX_BUFFER)?;
        let ib = Self::buffer(gpu, &mut self.indices, indices.len(), 4, D3D11_BIND_INDEX_BUFFER)?;
        let c = &gpu.ctx;
        unsafe {
            let mut m = D3D11_MAPPED_SUBRESOURCE::default();
            c.Map(&vb, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))?;
            std::ptr::copy_nonoverlapping(vertices.as_ptr() as *const u8, m.pData as *mut u8, vertices.len() * vertex_bytes);
            c.Unmap(&vb, 0);
            c.Map(&ib, 0, D3D11_MAP_WRITE_DISCARD, 0, Some(&mut m))?;
            std::ptr::copy_nonoverlapping(indices.as_ptr() as *const u8, m.pData as *mut u8, indices.len() * 4);
            c.Unmap(&ib, 0);

            let screen = [gpu.w as f32 / scale, gpu.h as f32 / scale, 0.0, 0.0];
            c.UpdateSubresource(&self.screen, 0, None, screen.as_ptr() as *const c_void, 0, 0);
            c.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: gpu.w as f32, Height: gpu.h as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            c.IASetInputLayout(&self.layout);
            c.IASetPrimitiveTopology(D3D_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            c.IASetVertexBuffers(0, 1, Some(&Some(vb)), Some(&(vertex_bytes as u32)), Some(&0));
            c.IASetIndexBuffer(&ib, DXGI_FORMAT_R32_UINT, 0);
            c.VSSetShader(&self.vs, None);
            c.VSSetConstantBuffers(0, Some(&[Some(self.screen.clone())]));
            c.PSSetShader(&self.ps, None);
            c.PSSetSamplers(0, Some(&[Some(self.sampler.clone())]));
            c.OMSetBlendState(&self.blend, None, 0xffff_ffff);
            c.RSSetState(&self.raster);
            for d in &draws {
                let Some((_, view)) = self.textures.get(&d.texture) else { continue };
                c.RSSetScissorRects(Some(&[d.clip]));
                c.PSSetShaderResources(0, Some(&[Some(view.clone())]));
                c.DrawIndexed(d.indices, d.first_index, d.first_vertex);
            }
            // back to what the picture's draw expects: no blending, no clipping, no vertex buffers
            c.OMSetBlendState(None, None, 0xffff_ffff);
            c.RSSetState(None);
            c.IASetInputLayout(None);
        }
        Ok(())
    }
}
