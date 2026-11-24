//! the render thread: it owns the gpu and the clip, takes events from the window, and draws. while a clip plays, each
//! turn of the loop is one display refresh. while it stands still, the thread sleeps until something happens.

use super::d3d::{Constants, Gpu, CONSTANTS_HLSL};
use super::ui_paint::UiPainter;
use super::window::{self, Handle};
use crate::event::{Event, Key};
use crate::media::clip::{Clip, Lane};
use crate::media::color;
use crate::media::dec::{Layout, Picture};
use crate::media::hap::Tex;
use crate::media::source::Frame;
use crate::play::Transport;
use crate::ui::{Action, Model, Ui};
use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::Win32::Graphics::Direct3D11::{ID3D11PixelShader, ID3D11ShaderResourceView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::*;

/// every picture shader ends with this. the lane's own code in front of it defines `texel`, one source texel as rgb.
/// at 100% and above every screen pixel is one texel, read by integer address: no filter. below 100% the four nearest
/// texels are converted first and then blended, because stored samples cannot be blended before they are converted.
const MAIN: &str = "
float4 main(float4 pos : SV_Position) : SV_Target {
    float2 at = (pos.xy - origin) * texels_per_pixel;
    if (texels_per_pixel.x <= 1.0) return float4(texel(int2(at)), 1);
    float2 f = at - 0.5;
    int2 p = int2(floor(f));
    float2 w = f - floor(f);
    float3 top = lerp(texel(p), texel(p + int2(1, 0)), w.x);
    float3 bottom = lerp(texel(p + int2(0, 1)), texel(p + int2(1, 1)), w.x);
    return float4(lerp(top, bottom, w.y), 1);
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
float3 texel(int2 p) { return CONVERT(t.Load(int3(clamp(p, int2(0, 0), int2(size) - 1), 0))); }";

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
float3 texel(int2 p) {
    p = clamp(p, int2(0, 0), int2(size) - 1);
    float y = luma(p) * scale_offset.x + scale_offset.y;
    float2 c = chroma(int2(p * chroma_per_luma)) * scale_offset.z + scale_offset.w;
    return float3(y + matrix_.x * c.y, y + matrix_.y * c.x + matrix_.z * c.y, y + matrix_.w * c.x);
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
                    Tex::YCoCgBc3 => "ycocg(v)",
                    // a lone alpha plane is shown as gray
                    Tex::Bc4 => "v.rrr",
                    _ => "v.rgb",
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
        };
        let ps = gpu.pixel_shader(&source).map_err(text)?;
        let transport = Transport::new(clip.timeline.frames(), clip.timeline.fps(), hz);
        Ok(View { clip, transport, ps, painter, constants, shown: None, fault: None, late: 0, unshowable: 0 })
    }

    /// puts the transport's frame into the painter, waiting up to `patience` for it. true when the picture changed.
    fn load(&mut self, gpu: &Gpu, patience: Duration) -> bool {
        let (frame, playing) = (self.transport.frame(), self.transport.playing());
        match &self.clip.lane {
            Lane::Hap(hap) => hap.source.want(frame, playing),
            Lane::Video(video) => video.source.want(frame, playing),
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

    fn draw(&self, gpu: &Gpu) {
        if self.fault.is_some() {
            return;
        }
        let views: Vec<Option<ID3D11ShaderResourceView>> = match &self.painter {
            Painter::Blocks { view, .. } => vec![Some(view.clone())],
            Painter::Planes { planes, .. } => planes.iter().map(|p| Some(p.1.clone())).collect(),
            Painter::Surface { held: Some((_, y, c)), .. } => vec![Some(y.clone()), Some(c.clone())],
            Painter::Surface { held: None, .. } => return,
        };
        // the largest rectangle of the picture's shape that fits the window, centered
        let (w, h) = (self.clip.width as f32, self.clip.height as f32);
        let scale = (gpu.w as f32 / w).min(gpu.h as f32 / h);
        let (dw, dh) = ((w * scale).round(), (h * scale).round());
        gpu.draw(&self.ps, &views, (((gpu.w as f32 - dw) / 2.0).floor(), ((gpu.h as f32 - dh) / 2.0).floor(), dw, dh), (w, h), self.constants);
    }

    fn key(&mut self, key: Key, shift: bool) {
        let last = self.clip.timeline.frames() as i64 - 1;
        match (key, shift) {
            (Key::Space, _) => self.transport.set_playing(!self.transport.playing()),
            (Key::F, false) => self.transport.step(-1),
            (Key::G, false) => self.transport.step(1),
            (Key::Left, false) => self.transport.step(-1),
            (Key::Right, false) => self.transport.step(1),
            (Key::Left, true) => self.transport.step(-10),
            (Key::Right, true) => self.transport.step(10),
            (Key::F, true) | (Key::Home, _) => self.transport.go(0),
            (Key::G, true) | (Key::End, _) => self.transport.go(last),
        }
    }

    /// what the controls show.
    fn model(&self) -> Model<'_> {
        Model { name: &self.clip.name, frame: self.transport.frame(), frames: self.clip.timeline.frames(), playing: self.transport.playing(), rate: self.clip.timeline.rate(), fault: self.fault.as_deref() }
    }

    /// what the window's title says: the taskbar and the tests read it.
    fn title(&self) -> String {
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
        format!("{} · {} / {} · {} × {} · {}{decoded} · {}{state}", c.name, self.transport.frame(), c.timeline.frames(), c.width, c.height, c.codec, c.rate_text())
    }
}

/// puts a newly opened clip in the window, sized to it. a file that could not be opened leaves what was there and
/// says why in the title.
fn show(gpu: &Gpu, window: Handle, clip: Result<Clip, String>, hz: f64, current: &mut Option<View>) {
    match clip.and_then(|clip| View::new(gpu, clip, hz)) {
        Ok(view) => {
            window::fit(window, window::client_for((view.clip.width, view.clip.height)));
            *current = Some(view);
        }
        Err(why) => window::set_title(window, &format!("eyepiece · {why}")),
    }
}

pub fn run(window: Handle, events: Receiver<Event>, first: Option<Result<Clip, String>>) {
    let (w, h) = window::client_size(window);
    // declared before the view on purpose: the view, and with it any decoder writing to this device, goes first
    let mut gpu = match Gpu::new(window.0, w, h) {
        Ok(gpu) => gpu,
        Err(e) => return window::set_title(window, &format!("eyepiece · no graphics device: {e}")),
    };
    let hz = window::refresh_rate();
    let mut ui = Ui::new((w, h), window::scale(window));
    let mut ui_painter = match UiPainter::new(&gpu) {
        Ok(p) => p,
        Err(e) => return window::set_title(window, &format!("eyepiece · the controls could not be set up: {e}")),
    };
    // when the controls want another look with nothing new from the window: they are due to fade
    let mut ui_due: Option<Instant> = None;
    let mut view: Option<View> = None;
    if let Some(clip) = first {
        show(&gpu, window, clip, hz, &mut view);
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
            match event {
                Event::Close => return,
                Event::Resize { w, h } => {
                    if let Err(e) = gpu.resize(w, h) {
                        return window::set_title(window, &format!("eyepiece · the graphics device stopped: {e}"));
                    }
                    ui.resize(w, h);
                }
                Event::Scale(scale) => ui.set_scale(scale),
                Event::PointerMove { x, y } => ui.pointer_move(x, y),
                Event::PointerButton { button, down, x, y } => ui.pointer_button(button, down, x, y),
                Event::PointerLeave => ui.pointer_leave(),
                Event::Open(path) => show(&gpu, window, Clip::open(&path), hz, &mut view),
                Event::Key { key, shift } => {
                    if let Some(v) = &mut view {
                        v.key(key, shift);
                    }
                }
                // one frame a notch. toward the user is forward, as a page scrolls
                Event::Wheel { notches } => {
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
        let controls = view.as_mut().map(|v| {
            let out = ui.run(&v.model());
            for action in &out.actions {
                match action {
                    Action::TogglePlay => v.transport.set_playing(!v.transport.playing()),
                    Action::Step(by) => v.transport.step(*by),
                    Action::Seek(frame) => v.transport.go(*frame as i64),
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
        }

        // a frame that is nearly there is worth a short wait: while playing, half a refresh, since showing the old
        // frame again skips this one for good. standing still, a few milliseconds between looks at the window
        let playing = view.as_ref().is_some_and(|v| v.transport.playing());
        let patience = if playing { Duration::from_secs_f64(0.5 / hz) } else { Duration::from_millis(4) };
        let changed = view.as_mut().is_some_and(|v| v.load(&gpu, patience));
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
                if playing {
                    v.late += missed as u64 + v.waiting() as u64;
                }
                // only a refresh that showed the frame it was due counts toward the next one
                if !v.waiting() {
                    v.transport.tick();
                }
            }
            redraw = false;
            // the title follows the frame while the clip stands still. it is left alone during play: redrawing a
            // title bar makes the compositor miss a refresh now and then
            if !playing {
                let title = view.as_ref().map(|v| v.title());
                if title.is_some() && title != titled {
                    window::set_title(window, title.as_deref().unwrap());
                    titled = title;
                }
            }
        }
        if let Some(out) = &controls {
            ui_painter.free_textures(&out.textures);
        }
        redraw |= again;
    }
}
