//! the render thread: it owns the gpu and the clip, takes events from the window, and draws. while a clip plays, each
//! turn of the loop is one display refresh. while it stands still, the thread sleeps until something happens.

use super::d3d::{Constants, Gpu, CONSTANTS_HLSL};
use super::ui_paint::UiPainter;
use super::window::{self, Handle};
use crate::event::{Event, Key, Mods};
use crate::media::clip::{Clip, Lane};
use crate::media::color;
use crate::media::dec::{Layout, Picture};
use crate::media::hap::Tex;
use crate::media::meta::Meta;
use crate::media::source::Frame;
use crate::media::still::Samples;
use crate::play::{Loop, Show, Transport};
use crate::settings::{self, Settings};
use crate::shot;
use crate::ui::{Action, Facts, Model, Screen, Shot, Ui};
use crate::view::{Channel, Framing, Guides, Look};
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
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
    /// stills: one texture of red, green, blue and alpha, rewritten every frame. `bits` is the scale pixel values
    /// are given on
    Image { tex: ID3D11Texture2D, view: ID3D11ShaderResourceView, bits: u32 },
    /// software pictures: luma, cb and cr textures, rewritten every frame
    Planes { planes: Vec<(ID3D11Texture2D, ID3D11ShaderResourceView)>, layout: Layout },
    /// hardware pictures: views onto the decoder's own surface. the picture is held with them, so the decoder does
    /// not decode into that surface again while it is on screen
    Surface { held: Option<(Arc<Picture>, ID3D11ShaderResourceView, ID3D11ShaderResourceView)>, layout: Layout },
}

/// a clip on screen.
struct View {
    clip: Clip,
    transport: Transport,
    ps: ID3D11PixelShader,
    painter: Painter,
    /// the color half of what the shader is told; the placement half is filled in at each draw
    constants: Constants,
    /// the frame that is in the painter now
    shown: Option<usize>,
    /// why the current frame cannot be shown, if it cannot
    fault: Option<String>,
    /// refreshes, while playing, on which no new frame was shown when one was due: the frame was not ready, or the
    /// display missed it. the clip runs that much late; no frame is skipped to make it up
    late: u64,
    /// frames that could not be shown since the clip was opened
    unshowable: u64,
    /// where the picture sits in the window: fitted, or zoomed and panned
    framing: Framing,
    /// how it is turned for the screen
    look: Look,
    /// the last pixel read back for the zoom chip: the frame, the pixel, its red, green and blue
    probe: Option<(usize, (usize, usize), [u32; 3])>,
}

fn text(e: windows::core::Error) -> String {
    e.to_string()
}

impl View {
    fn new(gpu: &Gpu, mut clip: Clip, hz: f64) -> Result<View, String> {
        // a clip ffmpeg decodes starts here, now that there is a device for a hardware decoder to write into
        clip.start(Some(gpu.hardware()))?;
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
                let (format, convert, bits) = match stills.samples {
                    Samples::U8 => (DXGI_FORMAT_R8G8B8A8_UNORM, "v", stills.bits),
                    Samples::U16 => (DXGI_FORMAT_R16G16B16A16_UNORM, "v", stills.bits),
                    // exr holds linear light. what the pixel readout gives for it is what the screen is sent
                    Samples::F32 => (DXGI_FORMAT_R32G32B32A32_FLOAT, "float4(srgb(v.rgb), v.a)", 16),
                };
                let (tex, view) = gpu.dynamic_texture(clip.width as u32, clip.height as u32, format).map_err(text)?;
                (format!("#define CONVERT(v) {convert}\n{CONSTANTS_HLSL}\n{ONE_TEXTURE}\n{MAIN}"), Painter::Image { tex, view, bits }, Constants::default())
            }
        };
        let ps = gpu.pixel_shader(&source).map_err(text)?;
        // the picture as the file has it: the look changes these two at each draw
        let constants = Constants { size: [clip.width as f32, clip.height as f32], to_source: [1.0, 0.0, 0.0, 1.0], ..constants };
        let mut transport = Transport::new(clip.timeline.frames(), clip.timeline.fps(), hz);
        // a sequence opened by one of its files opens on that file's frame
        transport.go(clip.first_frame as i64);
        let framing = Framing::new((clip.width, clip.height));
        Ok(View { clip, transport, ps, painter, constants, shown: None, fault: None, late: 0, unshowable: 0, framing, look: Look::default(), probe: None })
    }

    /// puts the transport's frame into the painter, waiting up to `patience` for it. true when the picture changed.
    fn load(&mut self, gpu: &Gpu, patience: Duration) -> bool {
        let (frame, want) = (self.transport.frame(), self.transport.ahead());
        match &self.clip.lane {
            Lane::Hap(hap) => hap.source.want(&want),
            Lane::Video(video) => video.source.want(&want),
            Lane::Stills(stills) => stills.source.want(&want),
            Lane::Waiting(_) => {}
        }
        if self.shown == Some(frame) {
            return false;
        }
        // the frame, into the painter. Ok(false) while it is still on its way
        let loaded: Result<bool, String> = match (&self.clip.lane, &mut self.painter) {
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
                            let widths = [self.clip.width, self.clip.width.div_ceil(1 << layout.chroma_shift.0)];
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
        };
        match loaded {
            Ok(false) => false,
            result => {
                self.fault = result.err();
                self.unshowable += self.fault.is_some() as u64;
                self.shown = Some(frame);
                true
            }
        }
    }

    /// true while the frame the transport is on has not reached the painter.
    fn waiting(&self) -> bool {
        self.shown != Some(self.transport.frame())
    }

    /// the frame in the painter, as the shader reads it. None when there is nothing to draw.
    fn views(&self) -> Option<Vec<Option<ID3D11ShaderResourceView>>> {
        if self.fault.is_some() {
            return None;
        }
        Some(match &self.painter {
            Painter::Blocks { view, .. } | Painter::Image { view, .. } => vec![Some(view.clone())],
            Painter::Planes { planes, .. } => planes.iter().map(|p| Some(p.1.clone())).collect(),
            Painter::Surface { held: Some((_, y, c)), .. } => vec![Some(y.clone()), Some(c.clone())],
            Painter::Surface { held: None, .. } => return None,
        })
    }

    fn size(&self) -> (usize, usize) {
        (self.clip.width, self.clip.height)
    }

    /// the picture, turned as the look has it, where the framing puts it. nothing during the pause on black.
    fn draw(&self, gpu: &Gpu) {
        let (Some(views), Show::Frame(_)) = (self.views(), self.transport.show()) else { return };
        let (to_source, to_source_add) = self.look.to_source(self.size());
        let channel = match self.look.channel {
            Channel::Rgb => 0.0,
            Channel::Red => 1.0,
            Channel::Green => 2.0,
            Channel::Blue => 3.0,
            Channel::Alpha => 4.0,
        };
        let shown = self.look.shown(self.size());
        gpu.draw(&self.ps, &views, self.framing.rect((gpu.w as f32, gpu.h as f32)), (shown.0 as f32, shown.1 as f32), Constants { to_source, to_source_add, channel, ..self.constants });
    }

    /// the frame in the painter, drawn `w` x `h` into a picture of our own and read back (`Gpu::read_pixels`): the
    /// file's own frame as it is stored, whatever the window shows.
    fn read(&self, gpu: &Gpu, w: u32, h: u32) -> Option<Vec<u16>> {
        let views = self.views()?;
        let size = (self.clip.width as f32, self.clip.height as f32);
        let _lock = gpu.locked();
        gpu.read_pixels(w, h, |gpu| gpu.draw(&self.ps, &views, (0.0, 0.0, w as f32, h as f32), size, self.constants)).ok()
    }

    /// the window's picture as it is shown, zoom, pan and look included, without the controls.
    fn read_view(&self, gpu: &Gpu) -> Option<Vec<u16>> {
        self.views()?;
        let _lock = gpu.locked();
        gpu.read_pixels(gpu.w, gpu.h, |gpu| self.draw(gpu)).ok()
    }

    /// how the clip is decoded, in the metadata panel's words.
    fn decoded(&self) -> &'static str {
        match &self.painter {
            Painter::Blocks { .. } => "blocks to the gpu as stored",
            Painter::Surface { .. } => "hardware, drawn from the decoder's own surface",
            Painter::Planes { .. } | Painter::Image { .. } => "software",
        }
    }

    /// bits a sample: the top of the scale pixel values are given on.
    fn depth(&self) -> u32 {
        match &self.painter {
            Painter::Blocks { .. } => 8,
            Painter::Image { bits, .. } => *bits,
            Painter::Planes { layout, .. } | Painter::Surface { layout, .. } => layout.stored.bits,
        }
    }

    /// reads the pixel back from the frame in the painter, unless that is the one already read: the picture is drawn
    /// at its own size into one pixel of our own, never read from what the window shows. the numbers are the red,
    /// green and blue the screen is sent, on the scale of the file's bit depth. true when there is a new value.
    fn probe(&mut self, gpu: &Gpu, pixel: Option<(usize, usize)>) -> bool {
        let wanted = self.shown.zip(pixel);
        if wanted == self.probe.map(|(frame, pixel, _)| (frame, pixel)) {
            return false;
        }
        self.probe = None;
        let (Some((frame, pixel)), Some(views)) = (wanted, self.views()) else { return false };
        let _lock = gpu.locked();
        let size = (self.clip.width as f32, self.clip.height as f32);
        let read = gpu.read_pixels(1, 1, |gpu| gpu.draw(&self.ps, &views, (-(pixel.0 as f32), -(pixel.1 as f32), size.0, size.1), size, self.constants));
        let Ok(read) = read else { return false };
        let top = ((1u32 << self.depth()) - 1) as f32;
        self.probe = Some((frame, pixel, [0, 1, 2].map(|i| (read[i] as f32 / 65535.0 * top).round() as u32)));
        true
    }

    /// turns the picture another way. it then starts fitted again (prototype `orient`); one channel alone leaves
    /// the zoom as it is.
    fn set_look(&mut self, look: Look) {
        if (look.mirror, look.flip, look.turns) != (self.look.mirror, self.look.flip, self.look.turns) {
            self.framing = Framing::new(look.shown(self.size()));
        }
        self.look = look;
    }

    /// a key, for the clip. the place is the pointer's when it is over the picture. true when the key changed the view.
    fn key(&mut self, key: Key, mods: Mods, at: Option<(f32, f32)>, window: (f32, f32)) -> bool {
        let arrow = matches!(key, Key::Left | Key::Right | Key::Up | Key::Down);
        // control and alt make another command of a key; the ones built so far are alt with the arrows
        if mods.ctrl || (mods.alt && !arrow) {
            return false;
        }
        let last = self.clip.timeline.frames() as i64 - 1;
        match (key, mods.shift) {
            // an eighth of the window a press
            _ if mods.alt => {
                let (x, y) = match key {
                    Key::Left => (1.0, 0.0),
                    Key::Right => (-1.0, 0.0),
                    Key::Up => (0.0, 1.0),
                    _ => (0.0, -1.0),
                };
                self.framing.pan((x * window.0 / 8.0, y * window.1 / 8.0), window);
                return true;
            }
            (Key::Space, _) => self.transport.set_playing(!self.transport.playing()),
            (Key::F, false) => self.transport.step(-1),
            (Key::G, false) => self.transport.step(1),
            (Key::Left, false) => self.transport.step(-1),
            (Key::Right, false) => self.transport.step(1),
            (Key::Left, true) => self.transport.step(-10),
            (Key::Right, true) => self.transport.step(10),
            (Key::F, true) | (Key::Home, _) => self.transport.go(0),
            (Key::G, true) | (Key::End, _) => self.transport.go(last),
            (Key::J, _) => self.transport.shuttle(-1),
            (Key::K, _) => self.transport.shuttle_stop(),
            (Key::L, _) => self.transport.shuttle(1),
            // - and = zoom out and in one stop, about the pointer (the owner's keys); with shift they step the speed
            (Key::Minus | Key::Equals, false) => {
                self.framing.step(if key == Key::Equals { 1 } else { -1 }, at, window);
                return true;
            }
            (Key::Minus, true) => self.transport.speed_step(-1),
            (Key::Equals, true) => self.transport.speed_step(1),
            (Key::Backspace, _) => self.transport.speed_reset(),
            (Key::H, _) => self.set_look(Look { mirror: !self.look.mirror, ..self.look }),
            (Key::V, _) => self.set_look(Look { flip: !self.look.flip, ..self.look }),
            (Key::Q, _) => self.set_look(Look { turns: (self.look.turns + 1) % 4, ..self.look }),
            (Key::Digit0 | Key::Digit1 | Key::Digit2, _) => {
                let zoom = match key {
                    Key::Digit0 => None,
                    Key::Digit1 => Some(1.0),
                    _ => Some(2.0),
                };
                self.framing.set_zoom(zoom, at, window);
                return true;
            }
            _ => {}
        }
        false
    }

    /// what the controls show. the pixel is the file's pixel under the pointer, when the zoom chip is to name it.
    fn model<'a>(&'a self, window: (f32, f32), pixel: Option<(usize, usize)>, settings: &Settings, facts: Facts<'a>, shots_to: &'a str) -> Model<'a> {
        Model {
            facts,
            decoded: self.decoded(),
            shots_to,
            frame: self.transport.frame(),
            frames: self.clip.timeline.frames(),
            playing: self.transport.playing(),
            rate: self.clip.timeline.rate(),
            fault: self.fault.as_deref(),
            gaps: &self.clip.missing,
            linear: matches!(&self.clip.lane, Lane::Stills(stills) if stills.samples == Samples::F32),
            looping: self.transport.loop_mode(),
            seam: self.transport.seam_place(),
            seam_seconds: settings.loop_analysis_seconds as f64,
            speed: self.transport.speed(),
            shuttle: self.transport.shuttle_rate(),
            zoom_label: self.framing.label(window),
            zoom: self.framing.scale(window),
            zoomed: self.framing.zoomed(),
            exact: self.framing.exact(window),
            overflows: self.framing.overflows(window),
            pixel,
            value: self.probe.filter(|(_, at, _)| Some(*at) == pixel).map(|(.., value)| value),
            picture: self.framing.rect(window),
            shown_size: self.look.shown(self.size()),
            look: self.look,
            guides: guides(settings),
        }
    }

    /// does what the controls asked of the clip. the tabs' and the window's own are not for it.
    fn act(&mut self, action: &Action, window: (f32, f32), settings: &mut Settings) {
        match action {
            Action::TogglePlay => self.transport.set_playing(!self.transport.playing()),
            Action::Step(by) => self.transport.step(*by),
            Action::Seek(frame) => self.transport.go(*frame as i64),
            Action::SeekPlace(place) => self.transport.go_place(*place),
            Action::Loop(mode) => {
                settings.loop_file = if *mode == Loop::Off { settings::Loop::Off } else { settings::Loop::One };
                self.transport.set_loop(*mode);
            }
            Action::Seam(seconds) => {
                self.transport.set_seam(*seconds);
                // the transport holds the seconds to what it takes: keep what it kept
                if let Some(kept) = self.transport.seam() {
                    settings.loop_analysis_seconds = kept as f32;
                }
            }
            Action::Speed(speed) => self.transport.set_speed(*speed),
            Action::Zoom(zoom) => self.framing.set_zoom(*zoom, None, window),
            Action::Pan(by) => self.framing.pan(*by, window),
            Action::Look(look) => self.set_look(*look),
            Action::NewTab | Action::SelectTab(_) | Action::CloseTab(_) | Action::Open(_) | Action::PlayFolder(_) | Action::Browse(_) | Action::Shot(_) | Action::Copy(_) | Action::Guides(_) | Action::Fullscreen | Action::MoveWindow | Action::Minimize | Action::ToggleMaximize | Action::Close => {}
        }
    }

    /// what the window's title says: the taskbar and the tests read it.
    fn title(&self, window: (f32, f32), settings: &Settings) -> String {
        let c = &self.clip;
        // the panel will have to admit whether the gpu or the cpu decodes; until then the title does
        let decoded = match &self.painter {
            Painter::Blocks { .. } | Painter::Image { .. } => "",
            Painter::Surface { .. } => ", hardware",
            Painter::Planes { .. } => ", software",
        };
        // what went wrong stays in the title: late refreshes, frames that could not be shown, and the one on screen
        let mut state = String::new();
        if self.late > 0 {
            state += &format!(" · {} late", self.late);
        }
        if self.unshowable > 0 {
            state += &format!(" · {} could not be shown", self.unshowable);
        }
        if let Some(why) = &self.fault {
            state += &format!(" · this frame cannot be shown: {why}");
        }
        // the number is where the transport is; until that frame is in the painter the picture is still the old one
        if self.waiting() {
            state += " · reading";
        }
        // what is not as a clip opens: loop, speed, the look, the guides, the zoom
        if self.transport.seam().is_some() {
            state += " · loop analysis";
        } else if self.transport.loop_mode() == Loop::Off {
            state += " · loop off";
        }
        if self.transport.speed() != 1.0 {
            state += &format!(" · {}×", self.transport.speed());
        }
        if let Some(look) = self.look.label() {
            state += &format!(" · {look}");
        }
        for (on, name) in [(settings.grid, "grid"), (settings.center, "center")] {
            if on {
                state += &format!(" · {name}");
            }
        }
        if self.framing.zoomed() {
            state += &format!(" · {}", self.framing.label(window));
        }
        if let Some((_, (x, y), [r, g, b])) = self.probe {
            state += &format!(" · x {x} y {y} · {r} {g} {b}");
        }
        format!("{} · {} / {} · {} × {} · {}{decoded} · {}{state}", c.name, self.transport.frame(), c.timeline.frames(), c.width, c.height, c.codec, c.rate_text())
    }
}

/// one tab of the window.
struct Tab {
    /// the file it was asked to show, as given
    path: Option<String>,
    /// its clip, when the file opened
    view: Option<View>,
    /// why it did not
    error: Option<String>,
    /// the browser is laid over what the tab holds
    browsing: bool,
    /// the file's facts for the metadata panel, read in the background: on their way, here, or why not
    facts: Option<Loaded<Meta>>,
}

/// something read in the background: on its way, here, or not to be had
enum Loaded<T> {
    Reading(Receiver<Result<T, String>>),
    Ready(T),
    Failed(String),
}

impl<T> Loaded<T> {
    /// takes what has arrived, if anything has.
    fn poll(&mut self) {
        if let Loaded::Reading(arriving) = self {
            match arriving.try_recv() {
                Ok(Ok(it)) => *self = Loaded::Ready(it),
                Ok(Err(why)) => *self = Loaded::Failed(why),
                Err(TryRecvError::Disconnected) => *self = Loaded::Failed("the reader stopped".into()),
                Err(TryRecvError::Empty) => {}
            }
        }
    }
}

/// a tab that was closed: its file, the frame it was on and how the picture sat, for bringing it back
type Closed = (String, usize, Framing, Look);

impl Tab {
    fn empty() -> Tab {
        Tab { path: None, view: None, error: None, browsing: false, facts: None }
    }

    /// a tab for a file: its clip, or why there is none.
    fn open(gpu: &Gpu, path: String, clip: Result<Clip, String>, hz: f64, settings: &Settings, wake: &Sender<Event>) -> Tab {
        // the facts are read apart from the clip: they stay available for a file that does not play
        let (reader, arriving) = mpsc::channel();
        let (file, wake) = (path.clone(), wake.clone());
        std::thread::spawn(move || {
            let _ = reader.send(Meta::read(&file));
            let _ = wake.send(Event::Wake);
        });
        let view = clip.and_then(|clip| View::new(gpu, clip, hz)).map(|mut view| {
            view.transport.set_loop(if settings.loop_file == settings::Loop::Off { Loop::Off } else { Loop::One });
            view
        });
        let (view, error) = match view {
            Ok(view) => (Some(view), None),
            Err(why) => (None, Some(why)),
        };
        Tab { path: Some(path), view, error, browsing: false, facts: Some(Loaded::Reading(arriving)) }
    }

    /// what the strip calls it.
    fn name(&self) -> &str {
        match (&self.view, &self.path) {
            (Some(view), _) => &view.clip.name,
            (None, Some(path)) => Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path),
            (None, None) => "new tab",
        }
    }

    /// nothing has been opened in it.
    fn is_empty(&self) -> bool {
        self.view.is_none() && self.error.is_none()
    }

    /// the browser is what this tab shows: asked for over its clip, or because there is nothing else to show.
    fn browsing(&self) -> bool {
        self.browsing || self.is_empty()
    }
}

/// the tabs of the window, of which one is up. there is always at least one.
struct Tabs {
    all: Vec<Tab>,
    up: usize,
    closed: Vec<Closed>,
    /// the display's refresh rate, which every clip opened is paced to
    hz: f64,
}

impl Tabs {
    fn tab(&mut self) -> &mut Tab {
        &mut self.all[self.up]
    }

    /// the clip on screen, when a clip is what is on screen.
    fn view(&mut self) -> Option<&mut View> {
        let tab = self.tab();
        if tab.browsing() {
            None
        } else {
            tab.view.as_mut()
        }
    }

    /// puts another tab up. the one left behind stops where it is. two clips of one size and length change places on
    /// the same frame, zoom and pan, which makes two tabs a flip comparison.
    fn select(&mut self, to: usize) {
        if to == self.up || to >= self.all.len() {
            return;
        }
        let carried = self.tab().view.as_mut().map(|v| {
            v.transport.set_playing(false);
            (v.size(), v.clip.timeline.frames(), v.transport.frame(), v.framing.clone(), v.look)
        });
        self.up = to;
        if let (Some((size, frames, frame, framing, look)), Some(v)) = (carried, self.tab().view.as_mut()) {
            if (v.size(), v.clip.timeline.frames()) == (size, frames) {
                v.transport.go(frame as i64);
                (v.framing, v.look) = (framing, look);
            }
        }
    }

    /// a new tab with nothing in it: the browser.
    fn new_tab(&mut self) {
        self.all.push(Tab::empty());
        self.select(self.all.len() - 1);
    }

    /// opens a file. from the browser it takes the place of what the tab that is up holds; dropped on the window or
    /// handed over by another launch it gets a tab of its own, and never replaces what is being looked at. the
    /// window grows to a bigger clip and never shrinks.
    fn open(&mut self, gpu: &Gpu, window: Handle, path: String, settings: &Settings, wake: &Sender<Event>, replace: bool) {
        let clip = Clip::open(&path);
        let tab = Tab::open(gpu, path, clip, self.hz, settings, wake);
        if let Some(view) = &tab.view {
            window::fit(window, window::client_for((view.clip.width, view.clip.height)));
        }
        if replace || self.tab().is_empty() {
            *self.tab() = tab;
        } else {
            self.all.push(tab);
            self.select(self.all.len() - 1);
        }
    }

    /// closes a tab, remembering it. closing the last one leaves an empty tab: the window stays.
    fn close(&mut self, at: usize) {
        if at >= self.all.len() {
            return;
        }
        let tab = self.all.remove(at);
        if let (Some(path), Some(view)) = (tab.path, tab.view) {
            self.closed.push((path, view.transport.frame(), view.framing.clone(), view.look));
        }
        if self.all.is_empty() {
            self.all.push(Tab::empty());
        }
        if at < self.up || self.up >= self.all.len() {
            self.up = self.up.saturating_sub(1);
        }
    }

    /// brings back the tab closed last, on its frame, zoom and pan.
    fn reopen(&mut self, gpu: &Gpu, window: Handle, settings: &Settings, wake: &Sender<Event>) {
        let Some((path, frame, framing, look)) = self.closed.pop() else { return };
        if !self.tab().is_empty() {
            self.new_tab();
        }
        self.open(gpu, window, path, settings, wake, true);
        if let Some(v) = &mut self.tab().view {
            v.transport.go(frame as i64);
            (v.framing, v.look) = (framing, look);
        }
    }
}

/// takes a screenshot of the clip on screen: the file's frame at full size, the view as shown, or the frame to the
/// clipboard. files are written in the background, and a toast says what was written.
fn shoot(v: &View, gpu: &Gpu, what: Shot, settings: &Settings, wake: &Sender<Event>) {
    let (w, h) = (v.clip.width as u32, v.clip.height as u32);
    let (pixels, size, stem) = match what {
        Shot::Frame | Shot::Copy => (v.read(gpu, w, h), (w, h), shot::stem(&v.clip.name, v.transport.frame())),
        Shot::View => (v.read_view(gpu), (gpu.w, gpu.h), shot::stem(&v.clip.name, v.transport.frame()) + ".view"),
    };
    let Some(pixels) = pixels else {
        let _ = wake.send(Event::Toast("there is no picture to save".into()));
        return;
    };
    if what == Shot::Copy {
        let said = if window::copy_picture(size.0, size.1, &pixels) { format!("frame copied · {} × {}, 8 bits a channel", size.0, size.1) } else { "the clipboard could not be had".to_string() };
        let _ = wake.send(Event::Toast(said));
        return;
    }
    let (deep, folder, wake) = (v.depth() > 8, shots_folder(settings), wake.clone());
    let detail = if what == Shot::View { "as shown".to_string() } else { format!("{} × {}", size.0, size.1) };
    std::thread::spawn(move || {
        let said = match shot::save(&pixels, size.0, size.1, deep, &folder, &stem) {
            Ok(file) => format!("saved to {} · {} · {detail}", shots_to(&folder), file.file_name().map_or_else(|| file.display().to_string(), |n| n.to_string_lossy().into_owned())),
            Err(why) => format!("not saved · {why}"),
        };
        let _ = wake.send(Event::Toast(said));
    });
}

/// where screenshots go: the folder the settings name, or pictures / eyepiece.
fn shots_folder(settings: &Settings) -> std::path::PathBuf {
    settings.screenshot_folder.clone().unwrap_or_else(shot::folder)
}

/// a folder as the controls name it: its last two parts, "pictures / eyepiece".
fn shots_to(folder: &Path) -> String {
    let parts: Vec<String> = folder.components().rev().take(2).map(|c| c.as_os_str().to_string_lossy().to_lowercase()).collect();
    parts.into_iter().rev().collect::<Vec<_>>().join(" / ")
}

/// the guides as the settings have them.
fn guides(settings: &Settings) -> Guides {
    Guides { grid: settings.grid, center: settings.center, tile: settings.tile, color: settings.guide_color }
}

/// the pointer, when it is over the picture: between the strip and the bar, as in the design's prototype.
fn over_picture(pointer: Option<(f32, f32)>, window: (f32, f32), scale: f32) -> Option<(f32, f32)> {
    pointer.filter(|p| p.1 > 34.0 * scale && p.1 < window.1 - 52.0 * scale)
}

/// runs the player until its window closes. what is kept between runs is read first and written back after.
/// `first` is the file given at the start, with its clip already opened or the reason it did not.
pub fn run(window: Handle, events: Receiver<Event>, wake: Sender<Event>, first: Option<(String, Result<Clip, String>)>) {
    let mut settings = Settings::load();
    play(window, events, wake, first, &mut settings);
    if let Err(e) = settings.save() {
        eprintln!("eyepiece: the settings were not saved: {e}");
    }
}

fn play(window: Handle, events: Receiver<Event>, wake: Sender<Event>, first: Option<(String, Result<Clip, String>)>, settings: &mut Settings) {
    let (w, h) = window::client_size(window);
    // declared before the tabs on purpose: a clip, and with it any decoder writing to this device, goes first
    let mut gpu = match Gpu::new(window.0, w, h) {
        Ok(gpu) => gpu,
        Err(e) => return window::set_title(window, &format!("eyepiece · no graphics device: {e}")),
    };
    let hz = window::refresh_rate();
    let mut ui = Ui::new((w, h), window::scale(window), settings.timecode, settings.browser_folder.clone());
    let mut ui_painter = match UiPainter::new(&gpu) {
        Ok(p) => p,
        Err(e) => return window::set_title(window, &format!("eyepiece · the controls could not be set up: {e}")),
    };
    // when the controls want another look with nothing new from the window: they are due to fade
    let mut ui_due: Option<Instant> = None;
    let first = match first {
        Some((path, clip)) => Tab::open(&gpu, path, clip, hz, settings, &wake),
        None => Tab::empty(),
    };
    // what just happened, said in the window until then; and where screenshots go, as the menu names it
    let mut toast: Option<(String, Instant)> = None;
    let shots_label = shots_to(&shots_folder(settings));
    let mut tabs = Tabs { all: vec![first], up: 0, closed: Vec::new(), hz };
    // where the pointer is, in physical pixels, while it is in the window; and physical pixels for one of the design's
    let (mut pointer, mut scale) = (None, window::scale(window));
    // the picture area fills a display; the pointer is shown (it hides in fullscreen while the controls are away)
    let (mut fullscreen, mut pointer_shown) = (false, true);
    let (mut redraw, mut titled) = (true, None);
    // the last key was a command, not typing: the character windows makes of the same press is then not typed
    // anywhere (N opens a new tab; its "n" must not pick a file in that tab's browser)
    let mut command = false;
    loop {
        // with nothing moving and nothing owed, sleep until the window says something or the controls are due
        let playing = tabs.view().is_some_and(|v| v.transport.playing());
        let waiting = tabs.view().is_some_and(|v| v.waiting());
        let mut next = if playing || waiting || redraw {
            match events.try_recv() {
                Ok(e) => Some(e),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
            }
        } else if let Some(due) = ui_due {
            match events.recv_timeout(due.saturating_duration_since(Instant::now())) {
                Ok(e) => Some(e),
                Err(RecvTimeoutError::Timeout) => {
                    redraw = true;
                    None
                }
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match events.recv() {
                Ok(e) => Some(e),
                Err(_) => return,
            }
        };
        while let Some(event) = next {
            let size = (gpu.w as f32, gpu.h as f32);
            let browsing = tabs.tab().browsing();
            if let Event::Key { key, mods } = &event {
                let for_browser = browsing && !mods.ctrl && !mods.alt && !matches!(key, Key::BracketLeft | Key::BracketRight);
                command = !(ui.typing() || for_browser);
            }
            match event {
                Event::Close => return,
                Event::Resize { w, h } => {
                    if let Err(e) = gpu.resize(w, h) {
                        return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
                    }
                    ui.resize(w, h);
                }
                Event::Scale(to) => {
                    scale = to;
                    ui.set_scale(to);
                }
                Event::PointerMove { x, y } => {
                    pointer = Some((x, y));
                    ui.pointer_move(x, y);
                }
                Event::PointerButton { button, down, x, y } => ui.pointer_button(button, down, x, y),
                Event::PointerLeave => {
                    pointer = None;
                    ui.pointer_leave();
                }
                Event::Open(path) => tabs.open(&gpu, window, path, settings, &wake, false),
                Event::Toast(said) => toast = Some((said, Instant::now() + Duration::from_millis(2600))),
                // what was read in the background is taken up below, with everything else
                Event::Wake => {}
                // a field being typed into takes the keys; escape closes it and its menu
                Event::Key { key, mods } if ui.typing() => {
                    ui.key(key, mods);
                    if key == Key::Escape {
                        ui.escape();
                    }
                }
                // tabs. the brackets go round them; control + W closes the one that is up and control + shift + T
                // brings the last one closed back
                Event::Key { key: key @ (Key::BracketLeft | Key::BracketRight), mods } if !mods.ctrl && !mods.alt => {
                    let count = tabs.all.len();
                    tabs.select((tabs.up + if key == Key::BracketLeft { count - 1 } else { 1 }) % count);
                }
                Event::Key { key: Key::W, mods } if mods.ctrl => tabs.close(tabs.up),
                Event::Key { key: Key::T, mods } if mods.ctrl && mods.shift => tabs.reopen(&gpu, window, settings, &wake),
                // control + C copies the frame
                Event::Key { key: Key::C, mods } if mods.ctrl && !browsing => {
                    if let Some(v) = tabs.view() {
                        shoot(v, &gpu, Shot::Copy, settings, &wake);
                    }
                }
                // escape leaves fullscreen first; then it is the browser's, which goes back to the clip, or closes a menu
                Event::Key { key: Key::Escape, .. } => {
                    if fullscreen {
                        fullscreen = false;
                        window::set_fullscreen(window, false, false);
                    } else if browsing {
                        ui.browser_key(Key::Escape, Mods::default());
                    } else {
                        ui.escape();
                    }
                }
                // in the browser the keys move about it and letters select by name
                Event::Key { key, mods } if browsing && !mods.ctrl && !mods.alt => ui.browser_key(key, mods),
                // only the character of the command's own press: a key the player has no use for sends no key, only
                // its character, which must still reach the browser
                Event::Char(_) if command => command = false,
                Event::Char(c) if browsing => ui.browser_char(c),
                // the keys that are not one clip's: a new tab, the browser over this one, fullscreen (with shift, on
                // the next display), the readout, the guides, the loop mode and loop analysis
                Event::Key { key, mods } if !mods.ctrl && !mods.alt && matches!(key, Key::N | Key::O | Key::P | Key::T | Key::D | Key::C | Key::R | Key::A | Key::S | Key::I) => match key {
                    Key::N => tabs.new_tab(),
                    // a screenshot: the file's frame at full size; with shift, the view as shown
                    Key::S => {
                        if let Some(v) = tabs.view() {
                            shoot(v, &gpu, if mods.shift { Shot::View } else { Shot::Frame }, settings, &wake);
                        }
                    }
                    Key::I => ui.toggle_facts(),
                    Key::O => {
                        let tab = tabs.tab();
                        tab.browsing = true;
                        if let Some(v) = &mut tab.view {
                            v.transport.set_playing(false);
                        }
                    }
                    Key::P => {
                        fullscreen = !fullscreen || mods.shift;
                        window::set_fullscreen(window, fullscreen, mods.shift);
                    }
                    Key::T => ui.toggle_timecode(),
                    Key::D => settings.grid = !settings.grid,
                    Key::C => settings.center = !settings.center,
                    Key::R => {
                        // off, one, and round again; from loop analysis, back to plain looping
                        settings.loop_file = if settings.loop_file == settings::Loop::Off || tabs.view().is_some_and(|v| v.transport.seam().is_some()) { settings::Loop::One } else { settings::Loop::Off };
                        if let Some(v) = tabs.view() {
                            v.transport.set_loop(if settings.loop_file == settings::Loop::Off { Loop::Off } else { Loop::One });
                        }
                    }
                    _ => {
                        if let Some(v) = tabs.view() {
                            v.transport.set_seam(v.transport.seam().is_none().then_some(settings.loop_analysis_seconds as f64));
                        }
                    }
                },
                Event::Key { key, mods } => {
                    if tabs.view().is_some_and(|v| v.key(key, mods, over_picture(pointer, size, scale), size)) {
                        ui.view_changed();
                    }
                }
                Event::Char(c) => {
                    if ui.typing() {
                        ui.char(c);
                    }
                }
                Event::Wheel { notches, .. } if browsing => ui.browser_wheel(notches),
                // over the metadata panel the wheel turns its list
                Event::Wheel { notches, .. } if ui.wheel(notches) => {}
                // with shift, one stop of zoom a notch, about the pointer: away from the user is in
                Event::Wheel { notches, mods } if mods.shift => {
                    if let Some(v) = tabs.view() {
                        (0..notches.abs()).for_each(|_| v.framing.step(notches.signum(), pointer, size));
                        ui.view_changed();
                    }
                }
                // one frame a notch. toward the user is forward, as a page scrolls
                Event::Wheel { notches, .. } => {
                    if let Some(v) = tabs.view() {
                        v.transport.step(-notches as i64);
                    }
                }
            }
            redraw = true;
            next = events.try_recv().ok();
        }

        // the controls: laid out against where the clip is now. what they ask for is done before the frame is
        // fetched, so a click and its picture are one refresh apart
        let size = (gpu.w as f32, gpu.h as f32);
        // the pixel the zoom chip names: the file's pixel under the pointer, while a zoom is set
        let pixel_of = |v: &View| over_picture(pointer, size, scale).filter(|_| v.framing.zoomed()).and_then(|p| v.framing.pixel_at(p, size)).map(|shown| v.look.source_pixel(shown, v.size()));
        if let Some(facts) = &mut tabs.tab().facts {
            facts.poll();
        }
        if toast.as_ref().is_some_and(|(_, until)| Instant::now() >= *until) {
            toast = None;
        }
        let out = {
            let tab = &tabs.all[tabs.up];
            let facts = match &tab.facts {
                Some(Loaded::Ready(meta)) => Facts::Ready(meta),
                Some(Loaded::Failed(why)) => Facts::Failed(why),
                Some(Loaded::Reading(_)) | None => Facts::Reading,
            };
            let screen = Screen {
                tabs: tabs.all.iter().map(Tab::name).collect(),
                current: tabs.up,
                clip: tab.view.as_ref().map(|v| v.model(size, pixel_of(v), settings, facts, &shots_label)),
                error: tab.error.as_deref(),
                browsing: tab.browsing(),
                fullscreen,
                toast: toast.as_ref().map(|(said, _)| said.as_str()),
            };
            ui.run(&screen)
        };
        settings.timecode = ui.timecode();
        if settings.browser_folder.as_deref() != Some(ui.browser_folder()) {
            settings.browser_folder = Some(ui.browser_folder().to_path_buf());
        }
        for action in &out.actions {
            match action {
                Action::NewTab => tabs.new_tab(),
                Action::SelectTab(to) => tabs.select(*to),
                Action::CloseTab(at) => tabs.close(*at),
                Action::Open(path) => tabs.open(&gpu, window, path.to_string_lossy().into_owned(), settings, &wake, true),
                Action::Shot(what) => {
                    if let Some(v) = tabs.view() {
                        shoot(v, &gpu, *what, settings, &wake);
                    }
                }
                Action::Copy(text) => {
                    let said = if window::copy_text(text) { "metadata copied" } else { "the clipboard could not be had" };
                    toast = Some((said.to_string(), Instant::now() + Duration::from_millis(2600)));
                }
                Action::Browse(on) => tabs.tab().browsing = *on,
                // folders do not play yet: until they do, the browser's shift + enter and play folder do nothing
                Action::PlayFolder(_) => {}
                Action::Guides(to) => (settings.grid, settings.center, settings.tile) = (to.grid, to.center, to.tile),
                Action::Fullscreen => {
                    fullscreen = !fullscreen;
                    window::set_fullscreen(window, fullscreen, false);
                }
                Action::MoveWindow => window::begin_move(window),
                Action::Minimize => window::minimize(window),
                Action::ToggleMaximize => window::toggle_maximize(window),
                Action::Close => window::close(window),
                // the rest are for the clip on screen
                clip => {
                    if let Some(v) = tabs.view() {
                        v.act(clip, size, settings);
                    }
                }
            }
        }
        // the controls are drawn again when they ask to be: at once while something of theirs is moving, later when
        // a fade is due, and once more after a click so they show what it did
        let mut again = !out.actions.is_empty();
        ui_due = None;
        let set = {
            let _lock = gpu.locked();
            ui_painter.set_textures(&gpu, &out.textures)
        };
        if let Err(e) = set {
            return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
        }
        match out.again {
            Some(wait) if wait.is_zero() => again = true,
            Some(wait) => ui_due = Some(Instant::now() + wait),
            None => {}
        }
        // a toast is taken away when its time is up, even if nothing else happens
        if let Some((_, until)) = &toast {
            ui_due = Some(ui_due.map_or(*until, |due| due.min(*until)));
        }
        // fullscreen with the controls away is the frame and nothing else: no pointer either
        if pointer_shown != (!fullscreen || out.shown) {
            pointer_shown = !pointer_shown;
            window::show_pointer(window, pointer_shown);
        }

        // a frame that is nearly there is worth a short wait: while playing, half a refresh, since showing the old
        // frame again skips this one for good. standing still, a few milliseconds between looks at the window
        let playing = tabs.view().is_some_and(|v| v.transport.playing());
        let patience = if playing { Duration::from_secs_f64(0.5 / hz) } else { Duration::from_millis(4) };
        let changed = tabs.view().is_some_and(|v| v.load(&gpu, patience));
        // the chip was laid out before this frame arrived: a new value under the pointer is one more look
        again |= tabs.view().is_some_and(|v| {
            let pixel = pixel_of(v);
            v.probe(&gpu, pixel)
        });
        if redraw || changed || playing {
            {
                // the decoder shares this device: its calls stay out of the middle of ours. the lock is let go before
                // the present, which waits for the display
                let _lock = gpu.locked();
                gpu.clear();
                if let Some(v) = tabs.view() {
                    v.draw(&gpu);
                }
                if let Err(e) = ui_painter.paint(&gpu, &out.primitives, out.scale) {
                    return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
                }
            }
            let missed = gpu.present(playing, Duration::from_secs_f64(1.0 / hz));
            if let Some(v) = tabs.view() {
                // the pause on black between rounds of loop analysis shows no frame: nothing to be late with
                let black = v.transport.show() == Show::Black;
                if playing && !black {
                    v.late += missed as u64 + v.waiting() as u64;
                }
                // only a refresh that showed what it was due counts toward the next one
                if black || !v.waiting() {
                    v.transport.tick();
                }
                // play that ended by itself (looping off, at the clip's end) is one more look, for the controls and
                // the title
                again |= playing && !v.transport.playing();
            }
            redraw = false;
            // the title follows the frame while the clip stands still. it is left alone during play: redrawing a
            // title bar makes the compositor miss a refresh now and then
            if !playing {
                let tab = &tabs.all[tabs.up];
                let title = match (&tab.view, &tab.error) {
                    (Some(v), _) => v.title(size, settings) + if tab.browsing { " · browser" } else { "" },
                    (None, Some(why)) => format!("eyepiece · {why}"),
                    (None, None) => "eyepiece".to_string(),
                };
                if titled.as_ref() != Some(&title) {
                    window::set_title(window, &title);
                    titled = Some(title);
                }
            }
        }
        ui_painter.free_textures(&out.textures);
        // both halves are done with: egui checks that no change is dropped unseen
        let mut textures = out.textures;
        textures.clear();
        redraw |= again;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use windows::core::w;
    use windows::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WINDOW_EX_STYLE, WS_POPUP};

    /// a test clip with its first frame in the painter, on a window that is never shown. needs a gpu.
    fn view(name: &str) -> (Gpu, View) {
        let hwnd = unsafe { CreateWindowExW(WINDOW_EX_STYLE(0), w!("STATIC"), w!(""), WS_POPUP, 0, 0, 64, 64, None, None, None, None) }.expect("a window");
        let gpu = Gpu::new(hwnd, 64, 64).expect("a graphics device");
        let clip = Clip::open(&format!("{}/tests/media/{name}", env!("CARGO_MANIFEST_DIR"))).expect("open");
        let mut view = View::new(&gpu, clip, 60.0).expect("view");
        assert!(view.load(&gpu, Duration::from_secs(20)), "the first frame arrived");
        (gpu, view)
    }

    /// the pixel readout gives the file's pixel, whatever the window shows: each one read back matches ffmpeg's
    /// decode of the same hap frame, within the few code values by which two bc1 decoders round.
    #[test]
    fn a_pixel_read_back_is_the_files_pixel() {
        let (gpu, mut view) = view("hap_64.mov");
        // zoomed and pushed aside: the reading must not care
        view.framing.set_zoom(Some(8.0), Some((3.0, 60.0)), (64.0, 64.0));
        let referee = std::fs::read(format!("{}/tests/media/hap_64.frame0.rgb", env!("CARGO_MANIFEST_DIR"))).expect("referee");
        // every seventh pixel each way: the four corners are among them
        for (x, y) in (0..64).step_by(7).flat_map(|y| (0..64).step_by(7).map(move |x| (x, y))) {
            assert!(view.probe(&gpu, Some((x, y))), "pixel {x},{y} was read");
            let (_, at, value) = view.probe.expect("a value");
            assert_eq!(at, (x, y));
            let wanted = &referee[(y * 64 + x) * 3..][..3];
            let worst = (0..3).map(|c| value[c].abs_diff(wanted[c] as u32)).max().unwrap();
            assert!(worst <= 3, "pixel {x},{y}: read {value:?}, ffmpeg decodes {wanted:?}");
        }
        // the same pixel of the same frame is not read twice; with no pixel asked for, nothing is kept
        assert!(!view.probe(&gpu, Some((63, 63))));
        assert!(!view.probe(&gpu, None) && view.probe.is_none());
    }
}
