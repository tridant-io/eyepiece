//! the render thread: it owns the gpu and the clip, takes events from the window, and draws. while a clip plays, each
//! turn of the loop is one display refresh. while it stands still, the thread sleeps until something happens.

use super::d3d::Gpu;
use super::window::{self, Handle};
use crate::event::{Event, Key};
use crate::media::clip::Clip;
use crate::media::hap::Tex;
use crate::media::hap_source::Frame;
use crate::play::Transport;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Duration;
use windows::Win32::Graphics::Direct3D11::{ID3D11PixelShader, ID3D11ShaderResourceView, ID3D11Texture2D};
use windows::Win32::Graphics::Dxgi::Common::*;

/// reads one texel and turns it into rgb, per hap variant. `CONVERT` is defined in front of this.
/// at 100% and above every screen pixel is one texel, read by integer address: no filter. below 100% the four nearest
/// texels are converted first and then blended, because scaled ycocg cannot be blended before it is converted.
const PICTURE: &str = "
Texture2D<float4> t : register(t0);
cbuffer Placement : register(b0) { float2 origin; float2 texels_per_pixel; float2 size; float2 pad; };
float3 ycocg(float4 v) {
    v.xy -= 0.50196078431373;
    float scale = v.z * (255.0 / 8.0) + 1.0;
    float co = v.x / scale, cg = v.y / scale, y = v.w;
    return float3(y + co - cg, y + cg, y - co - cg);
}
float3 texel(int2 p) { return CONVERT(t.Load(int3(clamp(p, int2(0, 0), int2(size) - 1), 0))); }
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

/// a clip on screen: its transport and the texture its current frame is copied into.
struct View {
    clip: Clip,
    transport: Transport,
    tex: ID3D11Texture2D,
    srv: ID3D11ShaderResourceView,
    ps: ID3D11PixelShader,
    /// the frame that is in the texture now
    shown: Option<usize>,
    /// why the current frame cannot be shown, if it cannot
    fault: Option<String>,
    /// refreshes, while playing, on which no new frame was shown when one was due: the frame was not ready, or the
    /// display missed it. the clip runs that much late; no frame is skipped to make it up
    late: u64,
}

impl View {
    fn new(gpu: &Gpu, clip: Clip, hz: f64) -> Result<View, String> {
        let format = match clip.hap.tex {
            Tex::Bc1 => DXGI_FORMAT_BC1_UNORM,
            Tex::Bc3 | Tex::YCoCgBc3 => DXGI_FORMAT_BC3_UNORM,
            Tex::Bc7 => DXGI_FORMAT_BC7_UNORM,
            Tex::Bc4 => DXGI_FORMAT_BC4_UNORM,
            Tex::Bc6u => DXGI_FORMAT_BC6H_UF16,
            Tex::Bc6s => DXGI_FORMAT_BC6H_SF16,
        };
        let convert = match clip.hap.tex {
            Tex::YCoCgBc3 => "ycocg(v)",
            // a lone alpha plane is shown as gray
            Tex::Bc4 => "v.rrr",
            _ => "v.rgb",
        };
        let ps = gpu.pixel_shader(&format!("#define CONVERT(v) {convert}\n{PICTURE}")).map_err(|e| e.to_string())?;
        let (tex, srv) = gpu.dynamic_texture(clip.hap.padded.0 as u32, clip.hap.padded.1 as u32, format).map_err(|e| e.to_string())?;
        let transport = Transport::new(clip.timeline.frames(), clip.timeline.fps(), hz);
        Ok(View { clip, transport, tex, srv, ps, shown: None, fault: None, late: 0 })
    }

    /// puts the transport's frame into the texture, waiting up to `patience` for it. true when the picture changed.
    fn load(&mut self, gpu: &Gpu, patience: Duration) -> bool {
        let frame = self.transport.frame();
        self.clip.hap.want(frame, self.transport.playing());
        if self.shown == Some(frame) {
            return false;
        }
        match self.clip.hap.get(frame, patience) {
            Frame::Ready(blocks) => {
                let (pw, ph) = self.clip.hap.padded;
                let fault = gpu.fill(&self.tex, &blocks, ph / 4, pw / 4 * self.clip.hap.tex.block_bytes()).err().map(|e| e.to_string());
                self.fault = fault;
                self.shown = Some(frame);
                true
            }
            Frame::Failed(why) => {
                self.fault = Some(why);
                self.shown = Some(frame);
                true
            }
            Frame::Pending => false,
        }
    }

    /// true while the frame the transport is on has not reached the texture.
    fn waiting(&self) -> bool {
        self.shown != Some(self.transport.frame())
    }

    fn draw(&self, gpu: &Gpu) {
        if self.fault.is_some() {
            return;
        }
        // the largest rectangle of the picture's shape that fits the window, centered
        let (w, h) = (self.clip.width as f32, self.clip.height as f32);
        let scale = (gpu.w as f32 / w).min(gpu.h as f32 / h);
        let (dw, dh) = ((w * scale).round(), (h * scale).round());
        gpu.draw(&self.ps, &self.srv, (((gpu.w as f32 - dw) / 2.0).floor(), ((gpu.h as f32 - dh) / 2.0).floor(), dw, dh), (w, h));
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

    /// what the title bar says until the control row exists.
    fn title(&self) -> String {
        let c = &self.clip;
        let state = match &self.fault {
            Some(why) => format!(" · this frame cannot be shown: {why}"),
            None if self.late > 0 => format!(" · {} late", self.late),
            None => String::new(),
        };
        format!("{} · {} / {} · {} × {} · {} · {}{state}", c.name, self.transport.frame(), c.timeline.frames(), c.width, c.height, c.codec, c.rate_text())
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
    let mut gpu = match Gpu::new(window.0, w, h) {
        Ok(gpu) => gpu,
        Err(e) => return window::set_title(window, &format!("eyepiece · no graphics device: {e}")),
    };
    let hz = window::refresh_rate();
    let mut view: Option<View> = None;
    if let Some(clip) = first {
        show(&gpu, window, clip, hz, &mut view);
    }
    let (mut redraw, mut titled) = (true, None);
    loop {
        // with nothing moving and nothing owed, sleep until the window says something
        let playing = view.as_ref().is_some_and(|v| v.transport.playing());
        let waiting = view.as_ref().is_some_and(|v| v.waiting());
        let mut next = if playing || waiting || redraw {
            match events.try_recv() {
                Ok(e) => Some(e),
                Err(TryRecvError::Empty) => None,
                Err(TryRecvError::Disconnected) => return,
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
                }
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

        // a frame that is nearly there is worth a short wait: while playing, half a refresh, since showing the old
        // frame again skips this one for good. standing still, a few milliseconds between looks at the window
        let playing = view.as_ref().is_some_and(|v| v.transport.playing());
        let patience = if playing { Duration::from_secs_f64(0.5 / hz) } else { Duration::from_millis(4) };
        let changed = view.as_mut().is_some_and(|v| v.load(&gpu, patience));
        if redraw || changed || playing {
            gpu.clear();
            if let Some(v) = &view {
                v.draw(&gpu);
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
    }
}
