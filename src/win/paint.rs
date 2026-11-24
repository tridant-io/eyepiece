//! what draws a clip's frames, on windows: its shader, and the textures the frame on screen is in.

use super::d3d::{Constants, Gpu, CONSTANTS_HLSL};
use crate::media::clip::{Clip, Lane};
use crate::media::color;
use crate::media::dec::{Layout, Picture};
use crate::media::hap::Tex;
use crate::media::source::Frame;
use crate::media::still::Samples;
use std::sync::Arc;
use std::time::Duration;
use windows::Win32::Graphics::Direct3D11::{ID3D11PixelShader, ID3D11ShaderResourceView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::*;

/// every picture shader ends with this. the lane's own code in front of it defines `texel`, one of the file's texels
/// as red, green, blue and alpha. `shown` is a texel of the picture as it is shown: mirrored, flipped and turned.
/// at 100% and above every screen pixel is one texel, read by integer address: no filter. from ten screen pixels a
/// texel, a dark line marks each texel's edge. below 100% the four nearest texels are converted first and then
/// blended, because stored samples cannot be blended before they are converted.
const MAIN: &str = "
float4 shown(int2 p) {
    float2 c = p + 0.5;
    return texel(int2(floor(float2(dot(to_source.xy, c), dot(to_source.zw, c)) + to_source_add)));
}
float4 main(float4 pos : SV_Position) : SV_Target {
    float2 at = (pos.xy - origin) * texels_per_pixel;
    float4 c;
    if (texels_per_pixel.x <= 1.0) {
        c = shown(int2(at));
        float2 into = frac(at) / texels_per_pixel;
        if (texels_per_pixel.x <= 0.1 && (into.x < 1.0 || into.y < 1.0)) c.rgb *= 0.58;
    } else {
        float2 f = at - 0.5;
        int2 p = int2(floor(f));
        float2 w = f - floor(f);
        float4 top = lerp(shown(p), shown(p + int2(1, 0)), w.x);
        float4 bottom = lerp(shown(p + int2(0, 1)), shown(p + int2(1, 1)), w.x);
        c = lerp(top, bottom, w.y);
    }
    if (channel == 1) c.rgb = float3(c.r, 0, 0);
    if (channel == 2) c.rgb = float3(0, c.g, 0);
    if (channel == 3) c.rgb = float3(0, 0, c.b);
    if (channel == 4) c.rgb = c.aaa;
    return c;
}";

/// one texture holding the whole picture: hap's block-compressed one, or a still's red, green, blue and alpha.
/// `CONVERT` says what is stored. linear light (exr) goes through the srgb curve; what is above 1.0 stops at white.
const ONE_TEXTURE: &str = "
Texture2D<float4> t : register(t0);
float3 srgb(float3 v) {
    v = max(v, 0);
    return lerp(v * 12.92, 1.055 * pow(v, 1.0 / 2.4) - 0.055, step(0.0031308, v));
}
float3 ycocg(float4 v) {
    v.xy -= 0.50196078431373;
    float scale = v.z * (255.0 / 8.0) + 1.0;
    float co = v.x / scale, cg = v.y / scale, y = v.w;
    return float3(y + co - cg, y + cg, y - co - cg);
}
float4 texel(int2 p) { return CONVERT(t.Load(int3(clamp(p, int2(0, 0), int2(size) - 1), 0))); }";

/// y'cbcr pictures, from a hardware decoder's surface (luma plane and a plane of chroma pairs, one slice of an array)
/// or from three planes in ordinary memory. the numbers come from `media::color`. chroma is the nearest sample for
/// now; blending it at the file's stated siting is the design's default and is not built yet.
const YCBCR: &str = "
#ifdef SURFACE
Texture2DArray<float> ty : register(t0);
Texture2DArray<float2> tc : register(t1);
float luma(int2 p) { return ty.Load(int4(p, 0, 0)); }
float2 chroma(int2 p) { return tc.Load(int4(p, 0, 0)); }
#else
Texture2D<float> ty : register(t0);
Texture2D<float> tu : register(t1);
Texture2D<float> tv : register(t2);
float luma(int2 p) { return ty.Load(int3(p, 0)); }
float2 chroma(int2 p) { return float2(tu.Load(int3(p, 0)), tv.Load(int3(p, 0))); }
#endif
float4 texel(int2 p) {
    p = clamp(p, int2(0, 0), int2(size) - 1);
    float y = luma(p) * scale_offset.x + scale_offset.y;
    float2 c = chroma(int2(p * chroma_per_luma)) * scale_offset.z + scale_offset.w;
    return float4(y + matrix_.x * c.y, y + matrix_.y * c.x + matrix_.z * c.y, y + matrix_.w * c.x, 1);
}";

/// where the current frame's samples are, for the shader.
enum Painter {
    /// hap: one texture of blocks, rewritten every frame
    Blocks { tex: ID3D11Texture2D, view: ID3D11ShaderResourceView },
    /// stills: one texture of red, green, blue and alpha, rewritten every frame
    Image { tex: ID3D11Texture2D, view: ID3D11ShaderResourceView },
    /// software pictures: luma, cb and cr textures, rewritten every frame
    Planes { planes: Vec<(ID3D11Texture2D, ID3D11ShaderResourceView)>, layout: Layout },
    /// hardware pictures: views onto the decoder's own surface. the picture is held with them, so the decoder does
    /// not decode into that surface again while it is on screen
    Surface { held: Option<(Arc<Picture>, ID3D11ShaderResourceView, ID3D11ShaderResourceView)>, layout: Layout },
}

/// a clip's shader and the frame it draws.
pub struct Paint {
    ps: ID3D11PixelShader,
    painter: Painter,
    /// the color half of what the shader is told; the placement half is filled in at each draw
    constants: Constants,
}

fn text(e: windows::core::Error) -> String {
    e.to_string()
}

impl Paint {
    /// for a clip that has started (`Clip::start`).
    pub fn new(gpu: &Gpu, clip: &Clip) -> Result<Paint, String> {
        let (source, painter, constants) = match &clip.lane {
            Lane::Hap(hap) => {
                let format = match hap.tex {
                    Tex::Bc1 => DXGI_FORMAT_BC1_UNORM,
                    Tex::Bc3 | Tex::YCoCgBc3 => DXGI_FORMAT_BC3_UNORM,
                    Tex::Bc7 => DXGI_FORMAT_BC7_UNORM,
                    Tex::Bc4 => DXGI_FORMAT_BC4_UNORM,
                    Tex::Bc6u => DXGI_FORMAT_BC6H_UF16,
                    Tex::Bc6s => DXGI_FORMAT_BC6H_SF16,
                };
                let convert = match hap.tex {
                    Tex::YCoCgBc3 => "float4(ycocg(v), 1)",
                    // a lone alpha plane is shown as gray
                    Tex::Bc4 => "v.rrrr",
                    _ => "v",
                };
                let (tex, view) = gpu.dynamic_texture(hap.padded.0 as u32, hap.padded.1 as u32, format).map_err(text)?;
                (format!("#define CONVERT(v) {convert}\n{CONSTANTS_HLSL}\n{ONE_TEXTURE}\n{MAIN}"), Painter::Blocks { tex, view }, Constants::default())
            }
            Lane::Video(video) => {
                let layout = video.layout;
                let c = color::conversion(layout.stored, layout.matrix);
                let constants = Constants {
                    chroma_per_luma: [1.0 / (1 << layout.chroma_shift.0) as f32, 1.0 / (1 << layout.chroma_shift.1) as f32],
                    scale_offset: [c.y_scale, c.y_offset, c.c_scale, c.c_offset],
                    matrix: [c.cr_to_r, c.cb_to_g, c.cr_to_g, c.cb_to_b],
                    ..Default::default()
                };
                if layout.hardware {
                    (format!("#define SURFACE\n{CONSTANTS_HLSL}\n{YCBCR}\n{MAIN}"), Painter::Surface { held: None, layout }, constants)
                } else {
                    let format = if layout.stored.texel_bits > 8 { DXGI_FORMAT_R16_UNORM } else { DXGI_FORMAT_R8_UNORM };
                    let chroma = (clip.width.div_ceil(1 << layout.chroma_shift.0), clip.height.div_ceil(1 << layout.chroma_shift.1));
                    let planes = [(clip.width, clip.height), chroma, chroma].iter().map(|&(w, h)| gpu.dynamic_texture(w as u32, h as u32, format).map_err(text)).collect::<Result<Vec<_>, _>>()?;
                    (format!("{CONSTANTS_HLSL}\n{YCBCR}\n{MAIN}"), Painter::Planes { planes, layout }, constants)
                }
            }
            Lane::Waiting(_) => return Err("the clip did not start".into()),
            Lane::Stills(stills) => {
                let (format, convert) = match stills.samples {
                    Samples::U8 => (DXGI_FORMAT_R8G8B8A8_UNORM, "v"),
                    Samples::U16 => (DXGI_FORMAT_R16G16B16A16_UNORM, "v"),
                    // exr holds linear light. what the pixel readout gives for it is what the screen is sent
                    Samples::F32 => (DXGI_FORMAT_R32G32B32A32_FLOAT, "float4(srgb(v.rgb), v.a)"),
                };
                let (tex, view) = gpu.dynamic_texture(clip.width as u32, clip.height as u32, format).map_err(text)?;
                (format!("#define CONVERT(v) {convert}\n{CONSTANTS_HLSL}\n{ONE_TEXTURE}\n{MAIN}"), Painter::Image { tex, view }, Constants::default())
            }
        };
        let ps = gpu.pixel_shader(&source).map_err(text)?;
        // the picture as the file has it: the look changes these two at each draw
        let constants = Constants { size: [clip.width as f32, clip.height as f32], to_source: [1.0, 0.0, 0.0, 1.0], ..constants };
        Ok(Paint { ps, painter, constants })
    }

    /// puts a frame of the clip into the painter, waiting up to `patience` for it. Ok(false) while it is still on
    /// its way.
    pub fn put(&mut self, gpu: &Gpu, clip: &Clip, frame: usize, patience: Duration) -> Result<bool, String> {
        match (&clip.lane, &mut self.painter) {
            (Lane::Hap(hap), Painter::Blocks { tex, .. }) => match hap.source.get(frame, patience) {
                Frame::Ready(blocks) => {
                    let row_bytes = hap.padded.0 / 4 * hap.tex.block_bytes();
                    let _lock = gpu.locked();
                    gpu.fill(tex, &blocks, hap.padded.1 / 4, row_bytes, row_bytes).map(|()| true).map_err(text)
                }
                Frame::Failed(why) => Err(why),
                Frame::Pending => Ok(false),
            },
            (Lane::Stills(stills), Painter::Image { tex, .. }) => match stills.source.get(frame, patience) {
                Frame::Ready(image) => {
                    let row_bytes = image.data.len() / image.height.max(1);
                    let _lock = gpu.locked();
                    gpu.fill(tex, &image.data, image.height, row_bytes, row_bytes).map(|()| true).map_err(text)
                }
                Frame::Failed(why) => Err(why),
                Frame::Pending => Ok(false),
            },
            (Lane::Video(video), painter) => match video.source.get(frame, patience) {
                Frame::Ready(picture) => {
                    let _lock = gpu.locked();
                    match painter {
                        Painter::Surface { held, layout } => {
                            let (texture, slice) = picture.surface();
                            gpu.surface_views(texture, slice, layout.stored.texel_bits > 8).map(|(y, c)| *held = Some((picture, y, c))).map(|()| true).map_err(text)
                        }
                        Painter::Planes { planes, layout } => {
                            let bytes = layout.stored.texel_bits as usize / 8;
                            let widths = [clip.width, clip.width.div_ceil(1 << layout.chroma_shift.0)];
                            (0..3).try_for_each(|i| {
                                let (data, stride, rows) = picture.plane(i, layout.chroma_shift);
                                gpu.fill(&planes[i].0, data, rows, widths[i.min(1)] * bytes, stride)
                            })
                            .map(|()| true)
                            .map_err(text)
                        }
                        Painter::Blocks { .. } | Painter::Image { .. } => Err("the clip and its painter disagree".into()),
                    }
                }
                Frame::Failed(why) => Err(why),
                Frame::Pending => Ok(false),
            },
            _ => Err("the clip and its painter disagree".into()),
        }
    }

    /// the frame in the painter, as the shader reads it. None before the first.
    fn views(&self) -> Option<Vec<Option<ID3D11ShaderResourceView>>> {
        Some(match &self.painter {
            Painter::Blocks { view, .. } | Painter::Image { view, .. } => vec![Some(view.clone())],
            Painter::Planes { planes, .. } => planes.iter().map(|p| Some(p.1.clone())).collect(),
            Painter::Surface { held: Some((_, y, c)), .. } => vec![Some(y.clone()), Some(c.clone())],
            Painter::Surface { held: None, .. } => return None,
        })
    }

    /// true when there is a frame to draw.
    pub fn ready(&self) -> bool {
        self.views().is_some()
    }

    /// draws the frame into `rect` of the target (left, top, width, height), `shown` texels across and down.
    /// `turned` is how the look turns it (`view::Look::to_source`) and the one channel it shows (0 for all); with
    /// none the frame is drawn as the file has it.
    pub fn draw(&self, gpu: &Gpu, rect: (f32, f32, f32, f32), shown: (f32, f32), turned: Option<([f32; 4], [f32; 2], f32)>) {
        let Some(views) = self.views() else { return };
        let constants = match turned {
            Some((to_source, to_source_add, channel)) => Constants { to_source, to_source_add, channel, ..self.constants },
            None => self.constants,
        };
        gpu.draw(&self.ps, &views, rect, shown, constants);
    }
}
