//! where the picture sits in the window (fitted, or zoomed about a point), how it is turned for the screen, and the
//! lines drawn over it. pure arithmetic, following the design's prototype rule for rule. sizes are physical pixels:
//! at 100% one video pixel is one pixel of the display.

/// which of the picture's channels are shown.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Channel {
    #[default]
    Rgb,
    Red,
    Green,
    Blue,
    Alpha,
}

/// how the picture is turned for the screen: mirrored, flipped, turned by quarters, one channel alone. it changes
/// the view, never the file.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Look {
    pub channel: Channel,
    pub mirror: bool,
    pub flip: bool,
    /// quarter turns clockwise, 0 to 3
    pub turns: u8,
}

impl Look {
    /// the look a file asks for (`ff::Input::turn`): `degrees` clockwise, mirrored first when `mirror`. None when
    /// that is not a quarter turn, which is shown as stored.
    pub fn upright(degrees: f64, mirror: bool) -> Option<Look> {
        let quarters = (degrees / 90.0).round();
        // within a degree, as ffmpeg's own player takes it
        ((degrees - quarters * 90.0).abs() < 1.0).then(|| Look { mirror, turns: quarters.rem_euclid(4.0) as u8, ..Look::default() })
    }

    /// this look with `then` on top: the file's own turn, then the user's, whose mirror and flip are of the picture
    /// as this look shows it. the channel is `then`'s.
    pub fn then(self, then: Look) -> Look {
        // a picture turned a quarter is mirrored, as shown, by flipping it before the turn
        let (mirror, flip) = if self.turns % 2 == 1 { (then.flip, then.mirror) } else { (then.mirror, then.flip) };
        Look { channel: then.channel, mirror: self.mirror != mirror, flip: self.flip != flip, turns: (self.turns + then.turns) % 4 }
    }

    /// the picture's size as shown: a quarter turn swaps its sides.
    pub fn shown(&self, size: (usize, usize)) -> (usize, usize) {
        if self.turns % 2 == 1 {
            (size.1, size.0)
        } else {
            size
        }
    }

    /// from a place in the picture as shown to the place in the file's picture: a 2 x 2 matrix, row by row, and what
    /// is added after it. the file's picture is mirrored and flipped first and turned after (prototype `ofr`), so
    /// this undoes the turn and then the mirror and flip.
    pub fn to_source(self, size: (usize, usize)) -> ([f32; 4], [f32; 2]) {
        let turn_back: [f32; 4] = match self.turns % 4 {
            0 => [1.0, 0.0, 0.0, 1.0],
            1 => [0.0, 1.0, -1.0, 0.0],
            2 => [-1.0, 0.0, 0.0, -1.0],
            _ => [0.0, -1.0, 1.0, 0.0],
        };
        let (x, y) = (if self.mirror { -1.0 } else { 1.0 }, if self.flip { -1.0 } else { 1.0 });
        let m = [turn_back[0] * x, turn_back[1] * x, turn_back[2] * y, turn_back[3] * y];
        // middles map to middles
        let shown = self.shown(size);
        let (from, to) = ((shown.0 as f32 / 2.0, shown.1 as f32 / 2.0), (size.0 as f32 / 2.0, size.1 as f32 / 2.0));
        (m, [to.0 - (m[0] * from.0 + m[1] * from.1), to.1 - (m[2] * from.0 + m[3] * from.1)])
    }

    /// the file's pixel behind a pixel of the picture as shown.
    pub fn source_pixel(&self, shown: (usize, usize), size: (usize, usize)) -> (usize, usize) {
        let (m, add) = self.to_source(size);
        let (x, y) = (shown.0 as f32 + 0.5, shown.1 as f32 + 0.5);
        ((m[0] * x + m[1] * y + add[0]).floor() as usize, (m[2] * x + m[3] * y + add[1]).floor() as usize)
    }

    /// what the chip says, when the picture is not shown as the file has it: "mirrored · rotated 90° · red only".
    pub fn label(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.mirror {
            parts.push("mirrored".to_string());
        }
        if self.flip {
            parts.push("flipped".to_string());
        }
        if !self.turns.is_multiple_of(4) {
            parts.push(format!("rotated {}°", self.turns as u32 % 4 * 90));
        }
        match self.channel {
            Channel::Rgb => {}
            Channel::Red => parts.push("red only".to_string()),
            Channel::Green => parts.push("green only".to_string()),
            Channel::Blue => parts.push("blue only".to_string()),
            Channel::Alpha => parts.push("alpha only".to_string()),
        }
        (!parts.is_empty()).then(|| parts.join(" · "))
    }
}

/// the lines drawn over the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Guides {
    pub grid: bool,
    /// center lines and circle
    pub center: bool,
    /// the grid's tile, in video pixels
    pub tile: (u32, u32),
    pub color: [u8; 3],
}

/// the stops shift + wheel walks through, with "fit" slotted in wherever it falls.
pub const LADDER: [f32; 14] = [0.125, 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 64.0];
const NEAR: f32 = 1e-4;

#[derive(Clone)]
pub struct Framing {
    /// the picture's size in its own pixels
    video: (f32, f32),
    /// None is fit; otherwise how many display pixels one video pixel takes
    zoom: Option<f32>,
    /// the video point at the middle of the window
    center: (f32, f32),
}

impl Framing {
    pub fn new(video: (usize, usize)) -> Framing {
        let video = (video.0.max(1) as f32, video.1.max(1) as f32);
        Framing { video, zoom: None, center: (video.0 / 2.0, video.1 / 2.0) }
    }

    /// the scale that shows the whole picture, as large as the window allows.
    pub fn fit_scale(&self, window: (f32, f32)) -> f32 {
        (window.0 / self.video.0).min(window.1 / self.video.1)
    }

    pub fn scale(&self, window: (f32, f32)) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit_scale(window))
    }

    /// true when a zoom is set, rather than fit.
    pub fn zoomed(&self) -> bool {
        self.zoom.is_some()
    }

    /// true when the picture is larger than the window either way: a drag on it then moves the picture, not the
    /// window.
    pub fn overflows(&self, window: (f32, f32)) -> bool {
        let s = self.scale(window);
        self.video.0 * s > window.0 + 0.5 || self.video.1 * s > window.1 + 0.5
    }

    /// where the middle of the window may be: a picture no larger than the window stays in the middle of it, and a
    /// larger one moves until its edge meets the window's and no further.
    fn held(&self, center: (f32, f32), window: (f32, f32)) -> (f32, f32) {
        let s = self.scale(window);
        let axis = |center: f32, video: f32, window: f32| {
            let half = window / (2.0 * s);
            if video * s <= window + 0.5 {
                video / 2.0
            } else {
                center.clamp(half, video - half)
            }
        };
        (axis(center.0, self.video.0, window.0), axis(center.1, self.video.1, window.1))
    }

    /// where the picture is drawn in the window: left, top, width, height. on whole display pixels, so that at a
    /// whole multiple every video pixel is a clean square.
    pub fn rect(&self, window: (f32, f32)) -> (f32, f32, f32, f32) {
        let s = self.scale(window);
        let (w, h) = ((self.video.0 * s).round(), (self.video.1 * s).round());
        // held again here: the window may have changed size since the picture was last moved
        let center = self.held(self.center, window);
        ((window.0 / 2.0 - center.0 * s).round(), (window.1 / 2.0 - center.1 * s).round(), w, h)
    }

    /// the video pixel under a point of the window, when there is one.
    pub fn pixel_at(&self, at: (f32, f32), window: (f32, f32)) -> Option<(usize, usize)> {
        let (x, y, w, h) = self.rect(window);
        let s = self.scale(window);
        let (vx, vy) = (((at.0 - x) / s).floor(), ((at.1 - y) / s).floor());
        (at.0 >= x && at.1 >= y && at.0 < x + w && at.1 < y + h && vx < self.video.0 && vy < self.video.1).then_some((vx as usize, vy as usize))
    }

    /// sets the zoom (None for fit), keeping the video point under `at` where it is; with no point, the middle.
    pub fn set_zoom(&mut self, zoom: Option<f32>, at: Option<(f32, f32)>, window: (f32, f32)) {
        let (s0, fit) = (self.scale(window), self.fit_scale(window));
        // the point's distance from the middle of the window, and the video point there
        let (ax, ay) = at.map_or((0.0, 0.0), |p| (p.0 - window.0 / 2.0, p.1 - window.1 / 2.0));
        let under = (self.center.0 + ax / s0, self.center.1 + ay / s0);
        self.zoom = zoom.map(|z| z.clamp(LADDER[0], LADDER[LADDER.len() - 1])).filter(|z| (z - fit).abs() > NEAR);
        self.center = match self.zoom {
            Some(s1) => self.held((under.0 - ax / s1, under.1 - ay / s1), window),
            // fitted, the picture sits in the middle
            None => (self.video.0 / 2.0, self.video.1 / 2.0),
        }
    }

    /// one stop along the ladder, in (+1) or out (-1), about `at`.
    pub fn step(&mut self, direction: i32, at: Option<(f32, f32)>, window: (f32, f32)) {
        let (s, fit) = (self.scale(window), self.fit_scale(window));
        let mut stops: Vec<f32> = LADDER.iter().copied().chain([fit]).collect();
        stops.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let next = if direction > 0 { stops.iter().find(|x| **x > s * 1.001) } else { stops.iter().rev().find(|x| **x < s * 0.999) };
        if let Some(&next) = next {
            self.set_zoom(Some(next).filter(|n| (n - fit).abs() > NEAR), at, window);
        }
    }

    /// moves the picture with the pointer, by this many display pixels.
    pub fn pan(&mut self, by: (f32, f32), window: (f32, f32)) {
        let s = self.scale(window);
        self.center = self.held((self.center.0 - by.0 / s, self.center.1 - by.1 / s), window);
    }

    /// what the bar says: "fit 43.2%", or "200%".
    pub fn label(&self, window: (f32, f32)) -> String {
        let s = self.scale(window);
        let percent = if s >= 1.0 { format!("{}", (s * 100.0).round()) } else { format!("{}", (s * 1000.0).round() / 10.0) };
        format!("{}{percent}%", if self.zoom.is_none() { "fit " } else { "" })
    }

    /// true when every video pixel is a whole number of display pixels: nothing is resampled.
    pub fn exact(&self, window: (f32, f32)) -> bool {
        let s = self.scale(window);
        s >= 1.0 && (s - s.round()).abs() < NEAR
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WINDOW: (f32, f32) = (1920.0, 1080.0);

    #[test]
    fn a_new_picture_is_fitted_and_centered() {
        let f = Framing::new((3840, 2160));
        assert_eq!(f.scale(WINDOW), 0.5);
        assert_eq!(f.rect(WINDOW), (0.0, 0.0, 1920.0, 1080.0));
        assert_eq!(f.label(WINDOW), "fit 50%");
        assert!(!f.zoomed());
        // a picture of another shape is letterboxed
        let f = Framing::new((1000, 1000));
        assert_eq!(f.rect(WINDOW), (420.0, 0.0, 1080.0, 1080.0));
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let mut f = Framing::new((3840, 2160));
        let at = (480.0, 270.0);
        let before = f.pixel_at(at, WINDOW).unwrap();
        f.set_zoom(Some(2.0), Some(at), WINDOW);
        assert_eq!(f.pixel_at(at, WINDOW).unwrap(), before);
        assert_eq!(f.label(WINDOW), "200%");
        assert!(f.exact(WINDOW) && f.zoomed());
        f.set_zoom(Some(8.0), Some(at), WINDOW);
        assert_eq!(f.pixel_at(at, WINDOW).unwrap(), before);
    }

    #[test]
    fn at_one_to_one_a_video_pixel_is_a_display_pixel() {
        let mut f = Framing::new((3840, 2160));
        f.set_zoom(Some(1.0), None, WINDOW);
        let (x, y, w, h) = f.rect(WINDOW);
        assert_eq!((x, y, w, h), (-960.0, -540.0, 3840.0, 2160.0));
        assert_eq!(f.pixel_at((0.0, 0.0), WINDOW), Some((960, 540)));
        assert_eq!(f.pixel_at((1.0, 0.0), WINDOW), Some((961, 540)));
    }

    #[test]
    fn the_ladder_passes_through_fit() {
        let mut f = Framing::new((3840, 2160));
        // fit is 50%, which is on the ladder: the next stop in is 100%
        f.step(1, None, WINDOW);
        assert_eq!(f.label(WINDOW), "100%");
        f.step(-1, None, WINDOW);
        assert_eq!(f.label(WINDOW), "fit 50%");
        f.step(-1, None, WINDOW);
        assert_eq!(f.label(WINDOW), "25%");
        // a fit that is not on the ladder is a stop of its own
        let mut f = Framing::new((1000, 1000));
        f.set_zoom(Some(1.0), None, WINDOW);
        f.step(1, None, WINDOW);
        assert_eq!(f.label(WINDOW), "fit 108%");
        f.step(1, None, WINDOW);
        assert_eq!(f.label(WINDOW), "200%");
    }

    #[test]
    fn panning_moves_the_picture_with_the_pointer_and_stops_at_its_edges() {
        let mut f = Framing::new((3840, 2160));
        f.pan((50.0, 50.0), WINDOW);
        assert_eq!(f.rect(WINDOW), (0.0, 0.0, 1920.0, 1080.0), "a fitted picture does not pan");
        f.set_zoom(Some(2.0), None, WINDOW);
        let (x, y, ..) = f.rect(WINDOW);
        f.pan((100.0, -40.0), WINDOW);
        assert_eq!((f.rect(WINDOW).0, f.rect(WINDOW).1), (x + 100.0, y - 40.0));
        f.pan((1e6, 1e6), WINDOW);
        // pushed as far as it goes: the picture's corner meets the window's
        assert_eq!((f.rect(WINDOW).0, f.rect(WINDOW).1), (0.0, 0.0));
        assert!(f.overflows(WINDOW));
        // a picture smaller than the window stays in the middle of it, zoomed or not
        let mut f = Framing::new((640, 360));
        f.set_zoom(Some(2.0), Some((10.0, 10.0)), WINDOW);
        f.pan((300.0, 300.0), WINDOW);
        assert_eq!(f.rect(WINDOW), (320.0, 180.0, 1280.0, 720.0));
        assert!(f.zoomed() && !f.overflows(WINDOW));
    }

    #[test]
    fn a_turned_picture_reads_the_right_pixel_of_the_file() {
        let size = (4, 2);
        let straight = Look::default();
        assert_eq!((straight.shown(size), straight.source_pixel((3, 1), size), straight.label()), (size, (3, 1), None));
        // a quarter turn clockwise: the file's top left is shown at the top right
        let turned = Look { turns: 1, ..Look::default() };
        assert_eq!(turned.shown(size), (2, 4));
        assert_eq!(turned.source_pixel((1, 0), size), (0, 0));
        assert_eq!(turned.source_pixel((0, 3), size), (3, 1));
        // mirrored: left and right change places
        let mirrored = Look { mirror: true, ..Look::default() };
        assert_eq!(mirrored.source_pixel((0, 1), size), (3, 1));
        // mirrored first, then turned: the file's top right is shown at the top right
        let both = Look { mirror: true, turns: 1, ..Look::default() };
        assert_eq!(both.source_pixel((1, 0), size), (3, 0));
        assert_eq!(Look { flip: true, turns: 3, channel: Channel::Red, ..Look::default() }.label().unwrap(), "flipped · rotated 270° · red only");
    }

    /// a file's turn is taken to the nearest quarter within a degree, counted clockwise; anything else is no quarter
    #[test]
    fn a_files_angle_is_a_quarter_turn_or_none() {
        let turns = |degrees: f64| Look::upright(degrees, false).map(|look| look.turns);
        assert_eq!([0.0, 90.0, 180.0, 270.0, -90.0, 360.0, 90.6, 269.4].map(turns), [0, 1, 2, 3, 3, 0, 1, 3].map(Some));
        assert_eq!([45.0, 91.5, f64::NAN].map(turns), [None; 3]);
        assert_eq!(Look::upright(180.0, true), Some(Look { mirror: true, turns: 2, ..Look::default() }));
    }

    /// the user's look on top of the file's own turn reads the same pixel of the file as the two one after the other,
    /// for every turn a file can ask for and every look the user can choose; the user's channel is the one shown
    #[test]
    fn the_users_look_on_top_of_the_files_turn_reads_the_same_pixel() {
        let looks: Vec<Look> = (0..16).map(|i| Look { channel: Channel::Rgb, mirror: i & 1 != 0, flip: i & 2 != 0, turns: (i >> 2) as u8 }).collect();
        let size = (5, 3);
        for file in looks.iter().filter(|l| !l.flip) {
            let upright = file.shown(size);
            for user in &looks {
                let both = file.then(Look { channel: Channel::Red, ..*user });
                assert_eq!((both.shown(size), both.channel), (user.shown(upright), Channel::Red));
                for shown in (0..both.shown(size).0).flat_map(|x| (0..both.shown(size).1).map(move |y| (x, y))) {
                    assert_eq!(both.source_pixel(shown, size), file.source_pixel(user.source_pixel(shown, upright), size), "{file:?} then {user:?} at {shown:?}");
                }
            }
        }
    }

    #[test]
    fn labels_below_one_keep_a_decimal() {
        let f = Framing::new((1920, 1080));
        assert_eq!(f.label((680.0, 383.0)), "fit 35.4%");
        assert!(!f.exact((680.0, 383.0)));
    }
}
