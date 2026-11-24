//! where the picture sits in the window: fitted, or zoomed about a point. pure arithmetic, following the design's
//! prototype rule for rule. sizes are physical pixels: at 100% one video pixel is one pixel of the display.

/// the stops shift + wheel walks through, with "fit" slotted in wherever it falls.
pub const LADDER: [f32; 14] = [0.125, 0.25, 0.5, 1.0, 2.0, 3.0, 4.0, 6.0, 8.0, 12.0, 16.0, 24.0, 32.0, 64.0];
const NEAR: f32 = 1e-4;

pub struct Framing {
    /// the picture's size in its own pixels
    video: (f32, f32),
    /// None is fit; otherwise how many display pixels one video pixel takes
    zoom: Option<f32>,
    /// the video point at the middle of the window
    center: (f32, f32),
    /// the zoom and place left by the last return to fit, for double click to go back to
    last: Option<(f32, (f32, f32))>,
}

impl Framing {
    pub fn new(video: (usize, usize)) -> Framing {
        let video = (video.0.max(1) as f32, video.1.max(1) as f32);
        Framing { video, zoom: None, center: (video.0 / 2.0, video.1 / 2.0), last: None }
    }

    /// the scale that shows the whole picture, as large as the window allows.
    pub fn fit_scale(&self, window: (f32, f32)) -> f32 {
        (window.0 / self.video.0).min(window.1 / self.video.1)
    }

    pub fn scale(&self, window: (f32, f32)) -> f32 {
        self.zoom.unwrap_or_else(|| self.fit_scale(window))
    }

    /// true when a zoom is set, rather than fit: a drag on the picture then pans it instead of moving the window.
    pub fn zoomed(&self) -> bool {
        self.zoom.is_some()
    }

    /// where the picture is drawn in the window: left, top, width, height. on whole display pixels, so that at a
    /// whole multiple every video pixel is a clean square.
    pub fn rect(&self, window: (f32, f32)) -> (f32, f32, f32, f32) {
        let s = self.scale(window);
        let (w, h) = ((self.video.0 * s).round(), (self.video.1 * s).round());
        ((window.0 / 2.0 - self.center.0 * s).round(), (window.1 / 2.0 - self.center.1 * s).round(), w, h)
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
        if zoom.is_none() {
            if let Some(z) = self.zoom {
                self.last = Some((z, self.center));
            }
        }
        // the point's distance from the middle of the window, and the video point there
        let (ax, ay) = at.map_or((0.0, 0.0), |p| (p.0 - window.0 / 2.0, p.1 - window.1 / 2.0));
        let under = (self.center.0 + ax / s0, self.center.1 + ay / s0);
        self.zoom = zoom.map(|z| z.clamp(LADDER[0], LADDER[LADDER.len() - 1])).filter(|z| (z - fit).abs() > NEAR);
        match self.zoom {
            Some(s1) => {
                self.center = (under.0 - ax / s1, under.1 - ay / s1);
                self.keep_in_reach();
            }
            // fitted, the picture sits in the middle
            None => self.center = (self.video.0 / 2.0, self.video.1 / 2.0),
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

    /// the double click: from a zoom back to fit; from fit back to the last zoom and place; with none to go back
    /// to, 1:1 about `at`.
    pub fn toggle_fit(&mut self, at: Option<(f32, f32)>, window: (f32, f32)) {
        if self.zoom.is_some() {
            self.set_zoom(None, None, window);
        } else if let Some((zoom, center)) = self.last {
            (self.zoom, self.center) = (Some(zoom), center);
        } else {
            self.set_zoom(Some(1.0), at, window);
        }
    }

    /// moves the picture with the pointer, by this many display pixels.
    pub fn pan(&mut self, by: (f32, f32), window: (f32, f32)) {
        if self.zoom.is_some() {
            let s = self.scale(window);
            self.center = (self.center.0 - by.0 / s, self.center.1 - by.1 / s);
            self.keep_in_reach();
        }
    }

    /// the picture can be pushed around until its edge reaches the middle of the window, and no further.
    fn keep_in_reach(&mut self) {
        self.center = (self.center.0.clamp(0.0, self.video.0), self.center.1.clamp(0.0, self.video.1));
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
    fn double_click_goes_between_fit_and_the_last_zoom() {
        let mut f = Framing::new((3840, 2160));
        // nothing to go back to: 1:1 about the pointer
        f.toggle_fit(Some((100.0, 100.0)), WINDOW);
        assert_eq!(f.label(WINDOW), "100%");
        f.pan((-300.0, -200.0), WINDOW);
        let placed = f.rect(WINDOW);
        f.toggle_fit(None, WINDOW);
        assert_eq!(f.label(WINDOW), "fit 50%");
        f.toggle_fit(None, WINDOW);
        assert_eq!(f.rect(WINDOW), placed);
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
        // pushed as far as it goes: the picture's corner sits at the middle of the window
        assert_eq!((f.rect(WINDOW).0, f.rect(WINDOW).1), (960.0, 540.0));
    }

    #[test]
    fn labels_below_one_keep_a_decimal() {
        let f = Framing::new((1920, 1080));
        assert_eq!(f.label((680.0, 383.0)), "fit 35.4%");
        assert!(!f.exact((680.0, 383.0)));
    }
}
