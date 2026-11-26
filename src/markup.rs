//! markup: shapes and text drawn on one frame of one clip (fig. 19). a mark is kept in the file's own pixels, so it
//! stays on the picture through zoom, pan, mirror, flip and rotate. marks last until their tab closes; the saved image
//! is the record. pure rules, the same on every system: the controls draw what is here.

use crate::view::Look;
use std::collections::BTreeMap;

/// the four inks, in the design's order: red, yellow, cyan, white
pub const INKS: [[u8; 3]; 4] = [[0xff, 0x3b, 0x30], [0xff, 0xd6, 0x0a], [0x32, 0xd3, 0xff], [0xff, 0xff, 0xff]];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tool {
    #[default]
    Rectangle,
    Circle,
    Pen,
    Text,
}

/// a place in the file's picture, in its pixels: (0, 0) is its top left corner, (width, height) its bottom right
pub type Point = (f32, f32);

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    /// two opposite corners
    Rectangle(Point, Point),
    /// the corners of the box it fills
    Circle(Point, Point),
    Pen(Vec<Point>),
    /// where the text's first letter starts, at its top
    Text(Point, String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub shape: Shape,
    pub ink: [u8; 3],
}

impl Mark {
    /// a mark begun with `tool` at `at`, before the pointer has moved
    pub fn start(tool: Tool, at: Point, ink: [u8; 3]) -> Mark {
        let shape = match tool {
            Tool::Rectangle => Shape::Rectangle(at, at),
            Tool::Circle => Shape::Circle(at, at),
            Tool::Pen => Shape::Pen(vec![at]),
            Tool::Text => Shape::Text(at, String::new()),
        };
        Mark { shape, ink }
    }

    /// the pointer, still down, has come to `at`: a shape's far corner follows it, a pen's line runs on to it.
    pub fn reach(&mut self, at: Point) {
        match &mut self.shape {
            Shape::Rectangle(_, far) | Shape::Circle(_, far) => *far = at,
            Shape::Pen(points) if points.last() != Some(&at) => points.push(at),
            Shape::Pen(_) | Shape::Text(..) => {}
        }
    }

    /// too small to have been meant, as a click leaves it: a pen of fewer than three points, a shape whose corners are
    /// less than 6 of the design's pixels apart across and down together (`scale` of them to a pixel of the file), or
    /// text that is only spaces. the prototype counts 6 of the file's pixels, which at 16× would drop a box drawn round
    /// a single pixel.
    pub fn tiny(&self, scale: f32) -> bool {
        match &self.shape {
            Shape::Pen(points) => points.len() < 3,
            Shape::Rectangle(a, b) | Shape::Circle(a, b) => ((b.0 - a.0).abs() + (b.1 - a.1).abs()) * scale < 6.0,
            Shape::Text(_, text) => text.trim().is_empty(),
        }
    }
}

/// the far corner of a square from `from` toward `to`: the larger of the two extents, both ways, each the way the
/// drag went. shift keeps a box square and a circle round with it, in the window's pixels, so it is square as seen.
pub fn square(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let side = dx.abs().max(dy.abs());
    (from.0 + side * dx.signum(), from.1 + side * dy.signum())
}

/// the marks of one clip, by frame
#[derive(Clone, Debug, Default)]
pub struct Marks(BTreeMap<usize, Vec<Mark>>);

impl Marks {
    /// the marks on a frame, the first drawn first
    pub fn on(&self, frame: usize) -> &[Mark] {
        self.0.get(&frame).map_or(&[], |marks| marks)
    }

    pub fn add(&mut self, frame: usize, mark: Mark) {
        self.0.entry(frame).or_default().push(mark);
    }

    /// takes back the last mark drawn on a frame. false when it had none.
    pub fn undo(&mut self, frame: usize) -> bool {
        let Some(marks) = self.0.get_mut(&frame) else { return false };
        marks.pop();
        if marks.is_empty() {
            self.0.remove(&frame);
        }
        true
    }

    /// takes every mark off a frame.
    pub fn clear(&mut self, frame: usize) {
        self.0.remove(&frame);
    }

    /// the frames that carry marks, in order: the timebar puts a tick at each.
    pub fn frames(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.keys().copied()
    }
}

/// where the file's picture is in a window, so a mark can be put on it and found again: the picture as shown (left,
/// top, width, height, as `Framing::rect` gives it), how it is turned (`Look`), and the file's own size.
pub struct Placing {
    pub picture: (f32, f32, f32, f32),
    pub look: Look,
    pub size: (usize, usize),
}

impl Placing {
    /// the place in the file's picture under a point of the window.
    pub fn to_file(&self, at: (f32, f32)) -> Point {
        let (x, y, w, h) = self.picture;
        let shown = self.look.shown(self.size);
        let s = ((at.0 - x) * shown.0 as f32 / w, (at.1 - y) * shown.1 as f32 / h);
        let (m, add) = self.look.to_source(self.size);
        (m[0] * s.0 + m[1] * s.1 + add[0], m[2] * s.0 + m[3] * s.1 + add[1])
    }

    /// the point of the window over a place in the file's picture.
    pub fn to_window(&self, p: Point) -> (f32, f32) {
        let (x, y, w, h) = self.picture;
        let shown = self.look.shown(self.size);
        let (m, add) = self.look.to_source(self.size);
        // the look only turns, mirrors and flips: its matrix is undone by the same matrix turned over
        let d = (p.0 - add[0], p.1 - add[1]);
        let s = (m[0] * d.0 + m[2] * d.1, m[1] * d.0 + m[3] * d.1);
        (x + s.0 * w / shown.0 as f32, y + s.1 * h / shown.1 as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::{Channel, Framing};

    const RED: [u8; 3] = INKS[0];

    #[test]
    fn marks_sit_on_their_frame_and_the_last_drawn_is_taken_back_first() {
        let mut marks = Marks::default();
        let mark = |x: f32| Mark::start(Tool::Rectangle, (x, x), RED);
        marks.add(5, mark(1.0));
        marks.add(5, mark(2.0));
        marks.add(9, mark(3.0));
        assert_eq!(marks.on(5), [mark(1.0), mark(2.0)]);
        assert!(marks.on(6).is_empty());
        assert_eq!(marks.frames().collect::<Vec<_>>(), [5, 9]);
        assert!(marks.undo(5));
        assert_eq!(marks.on(5), [mark(1.0)]);
        assert!(marks.undo(5) && marks.on(5).is_empty());
        // a frame with no marks left carries no tick
        assert_eq!(marks.frames().collect::<Vec<_>>(), [9]);
        assert!(!marks.undo(5), "nothing to take back");
        marks.clear(9);
        assert_eq!(marks.frames().count(), 0);
    }

    #[test]
    fn a_shape_follows_the_pointer_and_a_click_is_no_mark() {
        let mut rect = Mark::start(Tool::Rectangle, (10.0, 10.0), RED);
        assert!(rect.tiny(1.0));
        rect.reach((14.0, 12.0));
        assert_eq!(rect.shape, Shape::Rectangle((10.0, 10.0), (14.0, 12.0)));
        // 6 across and down together on screen is a mark: at 1:1 that is 6 of the file's pixels, at 16× under one
        assert!(!rect.tiny(1.0) && !rect.tiny(16.0) && rect.tiny(0.5));
        let mut pen = Mark::start(Tool::Pen, (0.0, 0.0), RED);
        pen.reach((0.0, 0.0));
        pen.reach((1.0, 0.0));
        assert!(pen.tiny(1.0), "two points");
        pen.reach((2.0, 1.0));
        assert_eq!(pen.shape, Shape::Pen(vec![(0.0, 0.0), (1.0, 0.0), (2.0, 1.0)]));
        assert!(!pen.tiny(1.0));
        assert!(Mark { shape: Shape::Text((0.0, 0.0), "  ".into()), ink: RED }.tiny(1.0));
    }

    /// every look: the four turns, each mirrored, flipped, both or neither
    fn looks() -> Vec<Look> {
        (0..16).map(|i| Look { channel: Channel::Rgb, mirror: i & 1 != 0, flip: i & 2 != 0, turns: (i >> 2) as u8 }).collect()
    }

    /// a point of the window is put in the file's pixel that the zoom chip names there, through every look and through
    /// zoom and pan, and comes back to the same point of the window.
    #[test]
    fn a_point_of_the_window_is_the_files_pixel_under_it_through_zoom_and_the_look() {
        let (size, window) = ((64, 32), (640.0, 360.0));
        let mut checked = 0;
        for look in looks() {
            let mut framing = Framing::new(look.shown(size));
            for step in 0..3 {
                if step == 1 {
                    framing.set_zoom(Some(16.0), Some((200.0, 120.0)), window);
                }
                if step == 2 {
                    framing.pan((-90.0, 40.0), window);
                }
                let place = Placing { picture: framing.rect(window), look, size };
                for at in [(321.3, 179.6), (101.0, 33.0), (600.5, 300.25), (322.0, 10.0)] {
                    let Some(shown) = framing.pixel_at(at, window) else { continue };
                    let file = place.to_file(at);
                    assert_eq!((file.0.floor() as usize, file.1.floor() as usize), look.source_pixel(shown, size), "{look:?} step {step} at {at:?}");
                    let back = place.to_window(file);
                    assert!((back.0 - at.0).abs() < 1e-3 && (back.1 - at.1).abs() < 1e-3, "{look:?}: {at:?} came back as {back:?}");
                    checked += 1;
                }
            }
        }
        // a point off the picture names no pixel; most are on it
        assert!(checked > 16 * 3 * 2, "only {checked} points were on the picture");
    }

    /// a mark put down in one view is over the same pixel of the file in another: zoomed in about another point,
    /// panned, mirrored and turned.
    #[test]
    fn a_mark_stays_on_its_pixel_when_the_view_changes() {
        let (size, window) = ((64, 32), (640.0, 360.0));
        let plain = Framing::new(size);
        let drawn = Placing { picture: plain.rect(window), look: Look::default(), size }.to_file((253.0, 144.0));
        let pixel = (drawn.0.floor() as usize, drawn.1.floor() as usize);
        for look in looks() {
            // fitted, then 24× about where the mark is, then pushed aside
            let mut framing = Framing::new(look.shown(size));
            let before = Placing { picture: framing.rect(window), look, size }.to_window(drawn);
            framing.set_zoom(Some(24.0), Some(before), window);
            framing.pan((30.0, -20.0), window);
            let at = Placing { picture: framing.rect(window), look, size }.to_window(drawn);
            let shown = framing.pixel_at(at, window).unwrap_or_else(|| panic!("{look:?}: the mark is off the picture at {at:?}"));
            assert_eq!(look.source_pixel(shown, size), pixel, "{look:?}: the mark is over another pixel");
        }
    }
}
