//! what draws a clip's frames, on macos: its shader, and the textures the frame on screen is in.

use super::metal::{Constants, Gpu, Shader, Texture, CONSTANTS_MSL};
use crate::media::clip::{Clip, Lane};
use crate::media::color;
use crate::media::dec::{Layout, Picture};
use crate::media::hap::Tex;
use crate::media::source::Frame;
use crate::media::still::Samples;
use objc2_metal::{MTLPixelFormat, MTLTexture};
use std::sync::Arc;
use std::time::Duration;

/// every picture shader ends with this. the lane's own code in front of it defines `TEXTURES`, the textures the
/// fragment function takes, and `TEXEL(p)`, one of the file's texels as red, green, blue and alpha. `SHOWN(p)` is a
/// texel of the picture as it is shown: mirrored, flipped and turned. at 100% and above every screen pixel is one
/// texel, read by integer address: no filter. from ten screen pixels a texel, a dark line marks each texel's edge.
/// below 100% the four nearest texels are converted first and then blended, because stored samples cannot be blended
/// before they are converted.
const MAIN: &str = "
#define SHOWN(p) TEXEL(int2(floor(float2(dot(k.to_source.xy, float2(p) + 0.5), dot(k.to_source.zw, float2(p) + 0.5)) + k.to_source_add)))
fragment float4 fs(V in [[stage_in]], constant Constants& k [[buffer(0)]], TEXTURES) {
    float2 at = (in.pos.xy - k.origin) * k.texels_per_pixel;
    float4 c;
    if (k.texels_per_pixel.x <= 1.0) {
        c = SHOWN(int2(at));
        float2 into = fract(at) / k.texels_per_pixel;
        if (k.texels_per_pixel.x <= 0.1 && (into.x < 1.0 || into.y < 1.0)) c.rgb *= 0.58;
    } else {
        float2 f = at - 0.5;
        int2 p = int2(floor(f));
        float2 w = f - floor(f);
        float4 top = mix(SHOWN(p), SHOWN(p + int2(1, 0)), w.x);
        float4 bottom = mix(SHOWN(p + int2(0, 1)), SHOWN(p + int2(1, 1)), w.x);
        c = mix(top, bottom, w.y);
    }
    if (k.channel == 1) c.rgb = float3(c.r, 0, 0);
    if (k.channel == 2) c.rgb = float3(0, c.g, 0);
    if (k.channel == 3) c.rgb = float3(0, 0, c.b);
    if (k.channel == 4) c.rgb = float3(c.a);
    return c;
}";

/// one texture holding the whole picture: hap's block-compressed one, or a still's red, green, blue and alpha.
/// `CONVERT` says what is stored. linear light (exr) goes through the srgb curve; what is above 1.0 stops at white.
const ONE_TEXTURE: &str = "
static inline float3 srgb(float3 v) {
    v = max(v, 0.0);
    return mix(v * 12.92, 1.055 * pow(v, float3(1.0 / 2.4)) - 0.055, step(0.0031308, v));
}
static inline float3 ycocg(float4 v) {
    v.xy -= 0.50196078431373;
    float scale = v.z * (255.0 / 8.0) + 1.0;
    float co = v.x / scale, cg = v.y / scale, y = v.w;
    return float3(y + co - cg, y + cg, y - co - cg);
}
static inline float4 texel(texture2d<float> t, constant Constants& k, int2 p) {
    float4 v = t.read(uint2(clamp(p, int2(0, 0), int2(k.size) - 1)));
    return CONVERT(v);
}
#define TEXTURES texture2d<float> t [[texture(0)]]
#define TEXEL(p) texel(t, k, p)";

/// y'cbcr pictures, from a videotoolbox frame's surface (a luma plane and a plane of chroma pairs) or from three
/// planes in ordinary memory. the numbers come from `media::color`. chroma planes smaller than the picture are
/// brought up to it by blending the four samples round the pixel, each where the file sites it, or by taking the
/// nearest; luma is never blended here.
const YCBCR: &str = "
#ifdef SURFACE
#define TEXTURES texture2d<float> ty [[texture(0)]], texture2d<float> tc [[texture(1)]]
#define PLANES texture2d<float> ty, texture2d<float> tc
#define TEXEL(p) texel(ty, tc, k, p)
#define CHROMA(p) tc.read(uint2(p)).rg
#else
#define TEXTURES texture2d<float> ty [[texture(0)]], texture2d<float> tu [[texture(1)]], texture2d<float> tv [[texture(2)]]
#define PLANES texture2d<float> ty, texture2d<float> tu, texture2d<float> tv
#define TEXEL(p) texel(ty, tu, tv, k, p)
#define CHROMA(p) float2(tu.read(uint2(p)).r, tv.read(uint2(p)).r)
#endif
static inline float4 texel(PLANES, constant Constants& k, int2 p) {
    p = clamp(p, int2(0, 0), int2(k.size) - 1);
    float y = ty.read(uint2(p)).r * k.scale_offset.x + k.scale_offset.y;
    float2 stored;
    if (k.bilinear == 0) {
        stored = CHROMA(int2(float2(p) * k.chroma_per_luma));
    } else {
        // where this pixel falls among the chroma samples, the first of which sits `siting` from the first pixel
        float2 at = (float2(p) - k.siting) * k.chroma_per_luma;
        int2 i = int2(floor(at));
        float2 w = at - floor(at);
        int2 last = int2(ceil(k.size * k.chroma_per_luma)) - 1;
        float2 top = mix(CHROMA(clamp(i, int2(0, 0), last)), CHROMA(clamp(i + int2(1, 0), int2(0, 0), last)), w.x);
        float2 bottom = mix(CHROMA(clamp(i + int2(0, 1), int2(0, 0), last)), CHROMA(clamp(i + int2(1, 1), int2(0, 0), last)), w.x);
        stored = mix(top, bottom, w.y);
    }
    float2 c = stored * k.scale_offset.z + k.scale_offset.w;
    return float4(y + k.matrix_.x * c.y, y + k.matrix_.y * c.x + k.matrix_.z * c.y, y + k.matrix_.w * c.x, 1);
}";

/// where the current frame's samples are, for the shader.
enum Painter {
    /// hap's one texture of blocks, or a still's of red, green, blue and alpha: rewritten every frame
    One(Texture),
    /// software pictures: luma, cb and cr textures, rewritten every frame
    Planes { planes: Vec<Texture>, layout: Layout },
    /// hardware pictures: textures that are the planes of the decoder's own surface. the picture is held with them,
    /// so the decoder does not decode into that surface again while it is on screen
    Surface { held: Option<(Arc<Picture>, Texture, Texture)>, layout: Layout },
}

/// a clip's shader and the frame it draws.
pub struct Paint {
    shader: Shader,
    painter: Painter,
    /// the color half of what the shader is told; the placement half is filled in at each draw
    constants: Constants,
}

impl Paint {
    /// for a clip that has started (`Clip::start`).
    pub fn new(gpu: &Gpu, clip: &Clip) -> Result<Paint, String> {
        let (source, painter, constants) = match &clip.lane {
            Lane::Hap(hap) => {
                let format = match hap.tex {
                    Tex::Bc1 => MTLPixelFormat::BC1_RGBA,
                    Tex::Bc3 | Tex::YCoCgBc3 => MTLPixelFormat::BC3_RGBA,
                    Tex::Bc7 => MTLPixelFormat::BC7_RGBAUnorm,
                    Tex::Bc4 => MTLPixelFormat::BC4_RUnorm,
                    Tex::Bc6u => MTLPixelFormat::BC6H_RGBUfloat,
                    Tex::Bc6s => MTLPixelFormat::BC6H_RGBFloat,
                };
                let convert = match hap.tex {
                    Tex::YCoCgBc3 => "float4(ycocg(v), 1)",
                    // a lone alpha plane is shown as gray
                    Tex::Bc4 => "v.rrrr",
                    _ => "v",
                };
                let tex = gpu.dynamic_texture(hap.padded.0 as u32, hap.padded.1 as u32, format)?;
                (format!("#define CONVERT(v) {convert}\n{CONSTANTS_MSL}\n{ONE_TEXTURE}\n{MAIN}"), Painter::One(tex), Constants::default())
            }
            Lane::Video(video) => {
                let layout = video.layout;
                let c = color::conversion(layout.stored, layout.matrix);
                let constants = Constants {
                    chroma_per_luma: [1.0 / (1 << layout.chroma_shift.0) as f32, 1.0 / (1 << layout.chroma_shift.1) as f32],
                    scale_offset: [c.y_scale, c.y_offset, c.c_scale, c.c_offset],
                    matrix: [c.cr_to_r, c.cb_to_g, c.cr_to_g, c.cb_to_b],
                    bilinear: 1.0,
                    siting: [layout.siting.0, layout.siting.1],
                    ..Default::default()
                };
                if layout.hardware {
                    (format!("#define SURFACE\n{CONSTANTS_MSL}\n{YCBCR}\n{MAIN}"), Painter::Surface { held: None, layout }, constants)
                } else {
                    let format = if layout.stored.texel_bits > 8 { MTLPixelFormat::R16Unorm } else { MTLPixelFormat::R8Unorm };
                    let chroma = (clip.width.div_ceil(1 << layout.chroma_shift.0), clip.height.div_ceil(1 << layout.chroma_shift.1));
                    let planes = [(clip.width, clip.height), chroma, chroma].iter().map(|&(w, h)| gpu.dynamic_texture(w as u32, h as u32, format)).collect::<Result<Vec<_>, _>>()?;
                    (format!("{CONSTANTS_MSL}\n{YCBCR}\n{MAIN}"), Painter::Planes { planes, layout }, constants)
                }
            }
            Lane::Waiting(_) => return Err("the clip did not start".into()),
            Lane::Stills(stills) => {
                let (format, convert) = match stills.samples {
                    Samples::U8 => (MTLPixelFormat::RGBA8Unorm, "v"),
                    Samples::U16 => (MTLPixelFormat::RGBA16Unorm, "v"),
                    // exr holds linear light. what the pixel readout gives for it is what the screen is sent
                    Samples::F32 => (MTLPixelFormat::RGBA32Float, "float4(srgb(v.rgb), v.a)"),
                };
                let tex = gpu.dynamic_texture(clip.width as u32, clip.height as u32, format)?;
                (format!("#define CONVERT(v) {convert}\n{CONSTANTS_MSL}\n{ONE_TEXTURE}\n{MAIN}"), Painter::One(tex), Constants::default())
            }
        };
        let shader = gpu.pixel_shader(&source)?;
        // the picture as the file has it: the look changes these two at each draw
        let constants = Constants { size: [clip.width as f32, clip.height as f32], to_source: [1.0, 0.0, 0.0, 1.0], ..constants };
        Ok(Paint { shader, painter, constants })
    }

    /// puts a frame of the clip into the painter, waiting up to `patience` for it. Ok(false) while it is still on
    /// its way.
    pub fn put(&mut self, gpu: &Gpu, clip: &Clip, frame: usize, patience: Duration) -> Result<bool, String> {
        match (&clip.lane, &mut self.painter) {
            (Lane::Hap(hap), Painter::One(tex)) => match hap.source.get(frame, patience) {
                Frame::Ready(blocks) => {
                    let row_bytes = hap.padded.0 / 4 * hap.tex.block_bytes();
                    gpu.fill(tex, &blocks, hap.padded.1 / 4, row_bytes, row_bytes).map(|()| true)
                }
                Frame::Failed(why) => Err(why),
                Frame::Pending => Ok(false),
            },
            (Lane::Stills(stills), Painter::One(tex)) => match stills.source.get(frame, patience) {
                Frame::Ready(image) => {
                    let row_bytes = image.data.len() / image.height.max(1);
                    gpu.fill(tex, &image.data, image.height, row_bytes, row_bytes).map(|()| true)
                }
                Frame::Failed(why) => Err(why),
                Frame::Pending => Ok(false),
            },
            (Lane::Video(video), painter) => match video.source.get(frame, patience) {
                Frame::Ready(picture) => match painter {
                    Painter::Surface { held, layout } => {
                        let (buffer, _) = picture.surface();
                        gpu.surface_planes(buffer, layout.stored.texel_bits > 8).map(|(y, c)| *held = Some((picture, y, c))).map(|()| true)
                    }
                    Painter::Planes { planes, layout } => {
                        let bytes = layout.stored.texel_bits as usize / 8;
                        let widths = [clip.width, clip.width.div_ceil(1 << layout.chroma_shift.0)];
                        (0..3).try_for_each(|i| {
                            let (data, stride, rows) = picture.plane(i, layout.chroma_shift);
                            gpu.fill(&planes[i], data, rows, widths[i.min(1)] * bytes, stride)
                        })
                        .map(|()| true)
                    }
                    Painter::One(_) => Err("the clip and its painter disagree".into()),
                },
                Frame::Failed(why) => Err(why),
                Frame::Pending => Ok(false),
            },
            _ => Err("the clip and its painter disagree".into()),
        }
    }

    /// the frame in the painter, as the shader reads it. None before the first.
    fn textures(&self) -> Option<Vec<&Texture>> {
        Some(match &self.painter {
            Painter::One(tex) => vec![tex],
            Painter::Planes { planes, .. } => planes.iter().collect(),
            Painter::Surface { held: Some((_, y, c)), .. } => vec![y, c],
            Painter::Surface { held: None, .. } => return None,
        })
    }

    /// how chroma that is smaller than the picture is brought up to it: blended from the samples round each
    /// pixel, which is how a painter starts, or taken from the nearest.
    pub fn set_bilinear(&mut self, on: bool) {
        self.constants.bilinear = on as u8 as f32;
    }

    /// true when there is a frame to draw.
    pub fn ready(&self) -> bool {
        self.textures().is_some()
    }

    /// draws the frame into `rect` of the target (left, top, width, height), `shown` texels across and down.
    /// `turned` is how the look turns it (`view::Look::to_source`) and the one channel it shows (0 for all); with
    /// none the frame is drawn as the file has it.
    pub fn draw(&self, gpu: &Gpu, rect: (f32, f32, f32, f32), shown: (f32, f32), turned: Option<([f32; 4], [f32; 2], f32)>) {
        let Some(textures) = self.textures() else { return };
        let constants = match turned {
            Some((to_source, to_source_add, channel)) => Constants { to_source, to_source_add, channel, ..self.constants },
            None => self.constants,
        };
        gpu.draw(&self.shader, &textures, rect, shown, constants);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a test clip's first frame, drawn into a picture of our own with no window, and read back.
    fn frame(name: &str) -> (usize, usize, Vec<u16>) {
        let gpu = Gpu::offscreen().expect("a metal device");
        let mut clip = Clip::open(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))).expect("open");
        clip.start(Some(gpu.hardware())).expect("start");
        let mut paint = Paint::new(&gpu, &clip).expect("a painter");
        assert_eq!(paint.put(&gpu, &clip, 0, Duration::from_secs(20)), Ok(true), "{name}: the first frame");
        let (w, h) = (clip.width, clip.height);
        let read = gpu.read_pixels(w as u32, h as u32, |gpu| paint.draw(gpu, (0.0, 0.0, w as f32, h as f32), (w as f32, h as f32), None)).expect("read back");
        (w, h, read)
    }

    /// every pixel of a hap frame read back matches ffmpeg's decode of it, within the few code values by which two
    /// bc1 decoders round.
    #[test]
    fn a_hap_frame_is_drawn_as_the_file_has_it() {
        let (w, h, read) = frame("hap_64.mov");
        let referee = std::fs::read(format!("{}/tests/media/hap_64.frame0.rgb", env!("CARGO_MANIFEST_DIR"))).expect("referee");
        assert_eq!((w, h, read.len(), referee.len()), (64, 64, 64 * 64 * 4, 64 * 64 * 3));
        let worst = (0..64 * 64).flat_map(|i| (0..3).map(move |c| (i, c))).map(|(i, c)| ((read[i * 4 + c] as f32 / 257.0).round() as i32 - referee[i * 3 + c] as i32).abs()).max().unwrap();
        assert!(worst <= 3, "the worst pixel is {worst} code values off");
    }

    /// every lane draws: a frame comes back that is not one flat color, opaque where the lane draws no alpha.
    #[test]
    fn every_lane_draws_its_first_frame() {
        for name in ["hapq_64.mov", "hapalpha_64.mov", "prores_64.mov", "prores4444_64.mov", "h264_64.mp4", "hevc10_64.mp4", "h264_422_64.mp4", "still_8bit.png", "still_16bit.png", "still_half.exr", "vp9_64.webm"] {
            let (w, h, read) = frame(name);
            let first: [u16; 3] = [read[0], read[1], read[2]];
            let varied = (0..w * h).any(|i| (0..3).any(|c| read[i * 4 + c].abs_diff(first[c]) > 2000));
            assert!(varied, "{name}: {w} x {h}, every pixel near {first:?}");
        }
        for name in ["hapq_64.mov", "prores_64.mov", "h264_64.mp4", "hevc10_64.mp4"] {
            let (_, _, read) = frame(name);
            assert!(read.chunks(4).all(|p| p[3] == 65535), "{name}: not opaque");
        }
    }

    /// the same frame through videotoolbox and through ffmpeg's software decoder: the two pictures agree, for 4:2:0
    /// in 8 and 10 bits and for the 4:2:2 that videotoolbox takes on apple's own chips.
    #[test]
    fn the_hardware_decoder_and_the_software_one_draw_the_same_picture() {
        for name in ["h264_64.mp4", "hevc10_64.mp4", "h264_422_64.mp4"] {
            let hardware = frame(name).2;
            let gpu = Gpu::offscreen().expect("a metal device");
            let mut clip = Clip::open(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))).expect("open");
            clip.start(None).expect("start");
            let mut paint = Paint::new(&gpu, &clip).expect("a painter");
            assert_eq!(paint.put(&gpu, &clip, 0, Duration::from_secs(20)), Ok(true));
            let software = gpu.read_pixels(64, 64, |gpu| paint.draw(gpu, (0.0, 0.0, 64.0, 64.0), (64.0, 64.0), None)).expect("read back");
            let worst = hardware.iter().zip(&software).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
            assert!(worst <= 2 * 257, "{name}: the worst sample is {} code values of 8 bits off", worst / 257);
        }
    }
}
