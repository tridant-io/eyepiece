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
use crate::media::source::Frame;
use crate::play::{Loop, Show, Transport};
use crate::settings::{self, Settings};
use crate::ui::{Action, Model, Ui};
use crate::view::{Channel, Framing, Guides, Look};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
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

/// hap: one block-compressed texture. `CONVERT` says what the variant stores.
const HAP: &str = "
Texture2D<float4> t : register(t0);
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
                (format!("#define CONVERT(v) {convert}\n{CONSTANTS_HLSL}\n{HAP}\n{MAIN}"), Painter::Blocks { tex, view }, Constants::default())
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
            Lane::Stills(_) => return Err("stills are not drawn yet".into()),
        };
        let ps = gpu.pixel_shader(&source).map_err(text)?;
        // the picture as the file has it: the look changes these two at each draw
        let constants = Constants { size: [clip.width as f32, clip.height as f32], to_source: [1.0, 0.0, 0.0, 1.0], ..constants };
        let transport = Transport::new(clip.timeline.frames(), clip.timeline.fps(), hz);
        let framing = Framing::new((clip.width, clip.height));
        Ok(View { clip, transport, ps, painter, constants, shown: None, fault: None, late: 0, unshowable: 0, framing, look: Look::default(), probe: None })
    }

    /// puts the transport's frame into the painter, waiting up to `patience` for it. true when the picture changed.
    fn load(&mut self, gpu: &Gpu, patience: Duration) -> bool {
        let (frame, want) = (self.transport.frame(), self.transport.ahead());
        match &self.clip.lane {
            Lane::Hap(hap) => hap.source.want(&want),
            Lane::Video(video) => video.source.want(&want),
            Lane::Waiting(_) | Lane::Stills(_) => {}
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
                        Painter::Blocks { .. } => Err("the clip and its painter disagree".into()),
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
            Painter::Blocks { view, .. } => vec![Some(view.clone())],
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

    /// bits a sample: the top of the scale pixel values are given on.
    fn depth(&self) -> u32 {
        match &self.painter {
            Painter::Blocks { .. } => 8,
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
            (Key::Minus, _) => self.transport.speed_step(-1),
            (Key::Equals, _) => self.transport.speed_step(1),
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
    fn model(&self, window: (f32, f32), pixel: Option<(usize, usize)>, settings: &Settings, fullscreen: bool) -> Model<'_> {
        Model {
            name: &self.clip.name,
            frame: self.transport.frame(),
            frames: self.clip.timeline.frames(),
            playing: self.transport.playing(),
            rate: self.clip.timeline.rate(),
            fault: self.fault.as_deref(),
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
            fullscreen,
        }
    }

    /// what the window's title says: the taskbar and the tests read it.
    fn title(&self, window: (f32, f32), settings: &Settings) -> String {
        let c = &self.clip;
        // the panel will have to admit whether the gpu or the cpu decodes; until then the title does
        let decoded = match &self.painter {
            Painter::Blocks { .. } => "",
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

/// puts a newly opened clip in the window, sized to it. a file that could not be opened leaves what was there and
/// says why in the title.
fn show(gpu: &Gpu, window: Handle, clip: Result<Clip, String>, hz: f64, settings: &Settings, current: &mut Option<View>) {
    match clip.and_then(|clip| View::new(gpu, clip, hz)) {
        Ok(mut view) => {
            window::fit(window, window::client_for((view.clip.width, view.clip.height)));
            view.transport.set_loop(if settings.loop_file == settings::Loop::Off { Loop::Off } else { Loop::One });
            *current = Some(view);
        }
        Err(why) => window::set_title(window, &format!("eyepiece · {why}")),
    }
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
pub fn run(window: Handle, events: Receiver<Event>, first: Option<Result<Clip, String>>) {
    let mut settings = Settings::load();
    play(window, events, first, &mut settings);
    if let Err(e) = settings.save() {
        eprintln!("eyepiece: the settings were not saved: {e}");
    }
}

fn play(window: Handle, events: Receiver<Event>, first: Option<Result<Clip, String>>, settings: &mut Settings) {
    let (w, h) = window::client_size(window);
    // declared before the view on purpose: the view, and with it any decoder writing to this device, goes first
    let mut gpu = match Gpu::new(window.0, w, h) {
        Ok(gpu) => gpu,
        Err(e) => return window::set_title(window, &format!("eyepiece · no graphics device: {e}")),
    };
    let hz = window::refresh_rate();
    let mut ui = Ui::new((w, h), window::scale(window), settings.timecode);
    let mut ui_painter = match UiPainter::new(&gpu) {
        Ok(p) => p,
        Err(e) => return window::set_title(window, &format!("eyepiece · the controls could not be set up: {e}")),
    };
    // when the controls want another look with nothing new from the window: they are due to fade
    let mut ui_due: Option<Instant> = None;
    let mut view: Option<View> = None;
    // where the pointer is, in physical pixels, while it is in the window; and physical pixels for one of the design's
    let (mut pointer, mut scale) = (None, window::scale(window));
    // the picture area fills a display; the pointer is shown (it hides in fullscreen while the controls are away)
    let (mut fullscreen, mut pointer_shown) = (false, true);
    if let Some(clip) = first {
        show(&gpu, window, clip, hz, settings, &mut view);
    }
    let (mut redraw, mut titled) = (true, None);
    loop {
        // with nothing moving and nothing owed, sleep until the window says something or the controls are due
        let playing = view.as_ref().is_some_and(|v| v.transport.playing());
        let waiting = view.as_ref().is_some_and(|v| v.waiting());
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
                Event::Open(path) => show(&gpu, window, Clip::open(&path), hz, settings, &mut view),
                // a field being typed into takes the keys; escape closes it and its menu
                Event::Key { key, mods } if ui.typing() => {
                    ui.key(key, mods);
                    if key == Key::Escape {
                        ui.escape();
                    }
                }
                // escape leaves fullscreen first, then closes a menu
                Event::Key { key: Key::Escape, .. } => {
                    if fullscreen {
                        fullscreen = false;
                        window::set_fullscreen(window, false, false);
                    } else {
                        ui.escape();
                    }
                }
                // the keys that are not one clip's: fullscreen (with shift, on the next display), the readout, the
                // guides, the loop mode and loop analysis, which are kept between runs
                Event::Key { key, mods } if !mods.ctrl && !mods.alt && matches!(key, Key::P | Key::T | Key::D | Key::C | Key::R | Key::A) => match key {
                    Key::P => {
                        fullscreen = !fullscreen || mods.shift;
                        window::set_fullscreen(window, fullscreen, mods.shift);
                    }
                    Key::T => ui.toggle_timecode(),
                    Key::D => settings.grid = !settings.grid,
                    Key::C => settings.center = !settings.center,
                    Key::R => {
                        // off, one, and round again; from loop analysis, back to plain looping
                        settings.loop_file = if settings.loop_file == settings::Loop::Off || view.as_ref().is_some_and(|v| v.transport.seam().is_some()) { settings::Loop::One } else { settings::Loop::Off };
                        if let Some(v) = &mut view {
                            v.transport.set_loop(if settings.loop_file == settings::Loop::Off { Loop::Off } else { Loop::One });
                        }
                    }
                    _ => {
                        if let Some(v) = &mut view {
                            v.transport.set_seam(v.transport.seam().is_none().then_some(settings.loop_analysis_seconds as f64));
                        }
                    }
                },
                Event::Key { key, mods } => {
                    if view.as_mut().is_some_and(|v| v.key(key, mods, over_picture(pointer, size, scale), size)) {
                        ui.view_changed();
                    }
                }
                Event::Char(c) => {
                    if ui.typing() {
                        ui.char(c);
                    }
                }
                // with shift, one stop of zoom a notch, about the pointer: away from the user is in
                Event::Wheel { notches, mods } if mods.shift => {
                    if let Some(v) = &mut view {
                        (0..notches.abs()).for_each(|_| v.framing.step(notches.signum(), pointer, size));
                        ui.view_changed();
                    }
                }
                // one frame a notch. toward the user is forward, as a page scrolls
                Event::Wheel { notches, .. } => {
                    if let Some(v) = &mut view {
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
        // the pixel the zoom chip names: the one under the pointer, while a zoom is set
        let pixel_of = |v: &View| over_picture(pointer, size, scale).filter(|_| v.framing.zoomed()).and_then(|p| v.framing.pixel_at(p, size)).map(|shown| v.look.source_pixel(shown, v.size()));
        let controls = view.as_mut().map(|v| {
            let out = ui.run(&v.model(size, pixel_of(v), settings, fullscreen));
            settings.timecode = ui.timecode();
            for action in &out.actions {
                match action {
                    Action::TogglePlay => v.transport.set_playing(!v.transport.playing()),
                    Action::Step(by) => v.transport.step(*by),
                    Action::Seek(frame) => v.transport.go(*frame as i64),
                    Action::SeekPlace(place) => v.transport.go_place(*place),
                    Action::Loop(mode) => {
                        settings.loop_file = if *mode == Loop::Off { settings::Loop::Off } else { settings::Loop::One };
                        v.transport.set_loop(*mode);
                    }
                    Action::Seam(seconds) => {
                        v.transport.set_seam(*seconds);
                        // the transport holds the seconds to what it takes: keep what it kept
                        if let Some(kept) = v.transport.seam() {
                            settings.loop_analysis_seconds = kept as f32;
                        }
                    }
                    Action::Speed(speed) => v.transport.set_speed(*speed),
                    Action::Zoom(zoom) => v.framing.set_zoom(*zoom, None, size),
                    Action::ToggleFit(at) => v.framing.toggle_fit(Some(*at), size),
                    Action::Pan(by) => v.framing.pan(*by, size),
                    Action::Look(look) => v.set_look(*look),
                    Action::Guides(to) => (settings.grid, settings.center, settings.tile) = (to.grid, to.center, to.tile),
                    Action::Fullscreen => {
                        fullscreen = !fullscreen;
                        window::set_fullscreen(window, fullscreen, false);
                    }
                    Action::MoveWindow => window::begin_move(window),
                    Action::Minimize => window::minimize(window),
                    Action::ToggleMaximize => window::toggle_maximize(window),
                    Action::Close => window::close(window),
                }
            }
            out
        });
        // they are drawn again when they ask to be: at once while something of theirs is moving, later when a fade
        // is due, and once more after a click so they show what it did
        let mut again = false;
        ui_due = None;
        if let Some(out) = &controls {
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
            again |= !out.actions.is_empty();
            // fullscreen with the controls away is the frame and nothing else: no pointer either
            if pointer_shown != (!fullscreen || out.shown) {
                pointer_shown = !pointer_shown;
                window::show_pointer(window, pointer_shown);
            }
        }

        // a frame that is nearly there is worth a short wait: while playing, half a refresh, since showing the old
        // frame again skips this one for good. standing still, a few milliseconds between looks at the window
        let playing = view.as_ref().is_some_and(|v| v.transport.playing());
        let patience = if playing { Duration::from_secs_f64(0.5 / hz) } else { Duration::from_millis(4) };
        let changed = view.as_mut().is_some_and(|v| v.load(&gpu, patience));
        // the chip was laid out before this frame arrived: a new value under the pointer is one more look
        again |= view.as_mut().is_some_and(|v| {
            let pixel = pixel_of(v);
            v.probe(&gpu, pixel)
        });
        if redraw || changed || playing {
            {
                // the decoder shares this device: its calls stay out of the middle of ours. the lock is let go before
                // the present, which waits for the display
                let _lock = gpu.locked();
                gpu.clear();
                if let Some(v) = &view {
                    v.draw(&gpu);
                }
                if let Some(out) = &controls {
                    if let Err(e) = ui_painter.paint(&gpu, &out.primitives, out.scale) {
                        return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
                    }
                }
            }
            let missed = gpu.present(playing, Duration::from_secs_f64(1.0 / hz));
            if let Some(v) = &mut view {
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
                let title = view.as_ref().map(|v| v.title(size, settings));
                if title.is_some() && title != titled {
                    window::set_title(window, title.as_deref().unwrap());
                    titled = title;
                }
            }
        }
        if let Some(mut out) = controls {
            ui_painter.free_textures(&out.textures);
            // both halves are done with: egui checks that no change is dropped unseen
            out.textures.clear();
        }
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
