//! draws the controls with metal: the triangles egui produces, over the picture, in the same pass.
//! colors arrive premultiplied and gamma-encoded, and are blended as they are: that is what egui lays out for.

use super::metal::{Gpu, Shader, Texture};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_metal::*;
use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;

/// egui's vertex as its memory has it: a position and a texture coordinate as two floats each, then four bytes of
/// color. the shader reads it out of the buffer by its number, so no layout is described.
const VERTEX: &str = "
#include <metal_stdlib>
using namespace metal;
struct I { packed_float2 pos; packed_float2 uv; uchar r, g, b, a; };
struct V { float4 pos [[position]]; float2 uv; float4 color; };
vertex V vs(uint id [[vertex_id]], const device I* vertices [[buffer(0)]], constant float2& screen [[buffer(1)]]) {
    I i = vertices[id];
    V o;
    o.pos = float4(i.pos.x / screen.x * 2 - 1, 1 - i.pos.y / screen.y * 2, 0, 1);
    o.uv = i.uv;
    o.color = float4(i.r, i.g, i.b, i.a) / 255.0;
    return o;
}";

const FRAGMENT: &str = "
fragment float4 fs(V in [[stage_in]], texture2d<float> t [[texture(0)]], sampler s [[sampler(0)]]) { return in.color * t.sample(s, in.uv); }";

/// one run of triangles that share a texture and a clip rectangle
struct Draw {
    texture: egui::TextureId,
    clip: MTLScissorRect,
    first_index: usize,
    indices: usize,
    first_vertex: isize,
}

pub struct UiPainter {
    shader: Shader,
    sampler: Retained<ProtocolObject<dyn MTLSamplerState>>,
    textures: HashMap<egui::TextureId, Texture>,
}

impl UiPainter {
    pub fn new(gpu: &Gpu) -> Result<UiPainter, String> {
        let desc = MTLSamplerDescriptor::new();
        desc.setMinFilter(MTLSamplerMinMagFilter::Linear);
        desc.setMagFilter(MTLSamplerMinMagFilter::Linear);
        let sampler = gpu.device.newSamplerStateWithDescriptor(&desc).ok_or("no sampler")?;
        Ok(UiPainter { shader: gpu.shader(VERTEX, FRAGMENT, true)?, sampler, textures: HashMap::new() })
    }

    /// takes new and changed textures (the font atlas, mostly). call before `paint`.
    pub fn set_textures(&mut self, gpu: &Gpu, delta: &egui::TexturesDelta) -> Result<(), String> {
        for (id, changes) in &delta.set {
            for change in changes {
                let egui::ImageData::Color(image) = &change.image;
                let [w, h] = image.size;
                // a pixel is four bytes, premultiplied, in the order the texture format wants
                let bytes = NonNull::new(image.pixels.as_ptr() as *mut c_void).ok_or("an empty texture")?;
                let (x, y) = match change.pos {
                    None => {
                        self.textures.insert(*id, gpu.dynamic_texture(w as u32, h as u32, MTLPixelFormat::RGBA8Unorm)?);
                        (0, 0)
                    }
                    Some([x, y]) => (x, y),
                };
                if let Some(tex) = self.textures.get(id) {
                    let region = MTLRegion { origin: MTLOrigin { x, y, z: 0 }, size: MTLSize { width: w, height: h, depth: 1 } };
                    unsafe { tex.replaceRegion_mipmapLevel_withBytes_bytesPerRow(region, 0, bytes, w * 4) };
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

    /// draws the controls over what is already in the pass. `scale` is physical pixels for one of egui's units, and
    /// `(w, h)` the size of what is drawn into: the window, or a picture of our own (`Gpu::read_pixels`).
    pub fn paint(&mut self, gpu: &Gpu, primitives: &[egui::ClippedPrimitive], scale: f32, (w, h): (u32, u32)) -> Result<(), String> {
        let Some((encoder, target, _)) = gpu.pass() else { return Ok(()) };
        let (mut vertices, mut indices, mut draws): (Vec<egui::epaint::Vertex>, Vec<u32>, Vec<Draw>) = (Vec::new(), Vec::new(), Vec::new());
        for p in primitives {
            let egui::epaint::Primitive::Mesh(mesh) = &p.primitive else { continue };
            let edge = |at: f32, most: u32| ((at * scale).round() as i64).clamp(0, most as i64) as usize;
            let (left, top, right, bottom) = (edge(p.clip_rect.min.x, w), edge(p.clip_rect.min.y, h), edge(p.clip_rect.max.x, w), edge(p.clip_rect.max.y, h));
            if mesh.indices.is_empty() || right <= left || bottom <= top {
                continue;
            }
            let clip = MTLScissorRect { x: left, y: top, width: right - left, height: bottom - top };
            draws.push(Draw { texture: mesh.texture_id, clip, first_index: indices.len(), indices: mesh.indices.len(), first_vertex: vertices.len() as isize });
            vertices.extend_from_slice(&mesh.vertices);
            indices.extend_from_slice(&mesh.indices);
        }
        if draws.is_empty() {
            return Ok(());
        }
        // buffers of their own for every pass: the gpu reads them after this returns, and the next pass's triangles
        // must not land in them meanwhile. the pass keeps them until it is done
        let buffer = |bytes: *const u8, len: usize| unsafe { gpu.device.newBufferWithBytes_length_options(NonNull::new(bytes as *mut c_void).unwrap(), len, MTLResourceOptions::StorageModeShared) }.ok_or("no buffer for the controls");
        let vb = buffer(vertices.as_ptr() as *const u8, vertices.len() * std::mem::size_of::<egui::epaint::Vertex>())?;
        let ib = buffer(indices.as_ptr() as *const u8, indices.len() * 4)?;
        let screen = [w as f32 / scale, h as f32 / scale];
        encoder.setRenderPipelineState(&self.shader.0[target]);
        encoder.setViewport(MTLViewport { originX: 0.0, originY: 0.0, width: w as f64, height: h as f64, znear: 0.0, zfar: 1.0 });
        unsafe {
            encoder.setVertexBuffer_offset_atIndex(Some(&vb), 0, 0);
            encoder.setVertexBytes_length_atIndex(NonNull::from(&screen).cast(), 8, 1);
            encoder.setFragmentSamplerState_atIndex(Some(&self.sampler), 0);
            for d in &draws {
                let Some(texture) = self.textures.get(&d.texture) else { continue };
                encoder.setScissorRect(d.clip);
                encoder.setFragmentTexture_atIndex(Some(texture), 0);
                encoder.drawIndexedPrimitives_indexCount_indexType_indexBuffer_indexBufferOffset_instanceCount_baseVertex_baseInstance(MTLPrimitiveType::Triangle, d.indices, MTLIndexType::UInt32, &ib, d.first_index * 4, 1, d.first_vertex, 0);
            }
        }
        Ok(())
    }
}
