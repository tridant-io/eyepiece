//! markup: shapes and text drawn on one frame of one clip (fig. 19). a mark is kept in the file's own pixels, so it
//! stays on the picture through zoom, pan, mirror, flip and rotate. marks last until their tab closes; the saved image
//! is the record. pure rules, the same on every system: the controls draw what is here.

use crate::view::Look;
use std::collections::BTreeMap;

/// the inks: red, orange, yellow, green, blue, indigo, violet, then white, black and grey (the owner's ask,
/// 2026-11-25). the design's red, yellow and white are as it has them, the rest from the same family of bright system
/// colors; its cyan is a blue now, which stands apart from indigo where cyan stood near it in name only. a mark keeps
/// its ink as red, green and blue, so one drawn in cyan before stays cyan
pub const INKS: [[u8; 3]; 10] = [
    [0xff, 0x3b, 0x30],
    [0xff, 0x95, 0x00],
    [0xff, 0xd6, 0x0a],
    [0x34, 0xc7, 0x59],
    [0x1e, 0x90, 0xff],
    [0x4b, 0x3c, 0xf0],
    [0xc7, 0x4d, 0xff],
    [0xff, 0xff, 0xff],
    [0x00, 0x00, 0x00],
    [0x8e, 0x8e, 0x93],
];

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Tool {
    /// takes hold of a mark drawn already, to move it or scale it
    Select,
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
    /// where the text's first letter starts, at its top; the text; and its size, 1 for the design's
    Text(Point, String, f32),
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
            // the select tool draws nothing, and starts no mark
            Tool::Text | Tool::Select => Shape::Text(at, String::new(), 1.0),
        };
        Mark { shape, ink }
    }

    /// the mark with each of its points put through `f`: how it is moved and scaled. a text's corner goes through it
    /// and its size stays.
    pub fn through(&self, f: impl Fn(Point) -> Point) -> Mark {
        let shape = match &self.shape {
            Shape::Rectangle(a, b) => Shape::Rectangle(f(*a), f(*b)),
            Shape::Circle(a, b) => Shape::Circle(f(*a), f(*b)),
            Shape::Pen(points) => Shape::Pen(points.iter().map(|p| f(*p)).collect()),
            Shape::Text(at, text, size) => Shape::Text(f(*at), text.clone(), *size),
        };
        Mark { shape, ink: self.ink }
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
            Shape::Text(_, text, _) => text.trim().is_empty(),
        }
    }
}

/// the larger of the two, both ways, each keeping its own sign
fn even((a, b): (f32, f32)) -> (f32, f32) {
    let most = a.abs().max(b.abs());
    (most * a.signum(), most * b.signum())
}

/// the far corner of a square from `from` toward `to`: the larger of the two extents, both ways, each the way the
/// drag went. shift keeps a box square and a circle round with it, in the window's pixels, so it is square as seen.
pub fn square(from: (f32, f32), to: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = even((to.0 - from.0, to.1 - from.1));
    (from.0 + dx, from.1 + dy)
}

/// the scale about `about` that takes `from` to `to`, each way on its own: a corner of a mark's bounds dragged. with
/// `keep` (shift) the two are kept in proportion as `square` keeps a box: the larger, each the way it went. a way the
/// bounds have no extent along stays as it is.
pub fn scale_by(about: (f32, f32), from: (f32, f32), to: (f32, f32), keep: bool) -> (f32, f32) {
    let way = |about: f32, from: f32, to: f32| if (from - about).abs() > f32::EPSILON { (to - about) / (from - about) } else { 1.0 };
    let by = (way(about.0, from.0, to.0), way(about.1, from.1, to.1));
    if keep { even(by) } else { by }
}

/// at the strongest a pen's line is smoothed over a bell this wide (its sigma), in the design's pixels on screen
const SMOOTHEST: f32 = 12.0;

/// a pen's points as drawn, smoothed by `strength`, 0 (off) to 1: each point the mean of the line round it, weighted
/// as a bell by how far along the line it is on screen (`on_screen` puts a point there, in the design's pixels). the
/// line between the points counts, not the points alone, so a quick stroke's few points and a slow one's many are
/// smoothed alike. the line taken round a point reaches as far behind it as ahead and no farther than the nearer end,
/// so the first and last points stay where the pointer went down and came up, and a short line keeps its length.
pub fn smooth(points: &[Point], strength: f32, on_screen: impl Fn(Point) -> (f32, f32)) -> Vec<Point> {
    let sigma = strength.clamp(0.0, 1.0) * SMOOTHEST;
    let n = points.len();
    if sigma <= 0.0 || n < 3 {
        return points.to_vec();
    }
    let seen: Vec<(f32, f32)> = points.iter().map(|p| on_screen(*p)).collect();
    let mut along = vec![0.0f32; n];
    for i in 1..n {
        along[i] = along[i - 1] + (seen[i].0 - seen[i - 1].0).hypot(seen[i].1 - seen[i - 1].1);
    }
    let total = along[n - 1];
    // the place on the line drawn `s` along it, between the points either side
    let at = |s: f32| {
        let j = along.partition_point(|&a| a < s).clamp(1, n - 1);
        let (a, b) = (points[j - 1], points[j]);
        let t = if along[j] > along[j - 1] { (s - along[j - 1]) / (along[j] - along[j - 1]) } else { 0.0 };
        match t {
            _ if t <= 0.0 => a,
            _ if t >= 1.0 => b,
            _ => (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t),
        }
    };
    // the bell taken in steps of a quarter of its sigma, out to three sigmas
    let step = sigma / 4.0;
    (0..n)
        .map(|i| {
            let steps = ((3.0 * sigma).min(along[i]).min(total - along[i]) / step) as i32;
            let (mut x, mut y, mut sum) = (0.0, 0.0, 0.0);
            for k in -steps..=steps {
                let w = (-0.5 * (k as f32 / 4.0).powi(2)).exp();
                let p = at(along[i] + k as f32 * step);
                (x, y, sum) = (x + w * p.0, y + w * p.1, sum + w);
            }
            (x / sum, y / sum)
        })
        .collect()
}

/// the marks of one clip, by frame
#[derive(Clone, Debug, Default)]
pub struct Marks(BTreeMap<usize, Frame>);

/// one frame's marks, what they were before each change to them, the last change last, and what undo took back, the
/// last taken last
#[derive(Clone, Debug, Default)]
struct Frame {
    marks: Vec<Mark>,
    before: Vec<Vec<Mark>>,
    after: Vec<Vec<Mark>>,
}

impl Marks {
    /// the marks on a frame, the first drawn first
    pub fn on(&self, frame: usize) -> &[Mark] {
        self.0.get(&frame).map_or(&[], |f| &f.marks)
    }

    /// a change to a frame's marks, which undo can take back. what undo took back before can no longer be brought back
    fn change_to(&mut self, frame: usize, marks: Vec<Mark>) {
        let f = self.0.entry(frame).or_default();
        if marks != f.marks {
            f.before.push(std::mem::replace(&mut f.marks, marks));
            f.after.clear();
        }
    }

    pub fn add(&mut self, frame: usize, mark: Mark) {
        let mut marks = self.on(frame).to_vec();
        marks.push(mark);
        self.change_to(frame, marks);
    }

    /// the mark at `index` on a frame made `to` (moved, scaled, another ink, its text typed again), or taken off
    pub fn change(&mut self, frame: usize, index: usize, to: Option<Mark>) {
        let mut marks = self.on(frame).to_vec();
        if index >= marks.len() {
            return;
        }
        match to {
            Some(mark) => marks[index] = mark,
            None => _ = marks.remove(index),
        }
        self.change_to(frame, marks);
    }

    /// takes back the last change to a frame's marks: a mark drawn, changed or taken off, or the frame cleared. false
    /// when there was none.
    pub fn undo(&mut self, frame: usize) -> bool {
        let Some(f) = self.0.get_mut(&frame) else { return false };
        let Some(was) = f.before.pop() else { return false };
        f.after.push(std::mem::replace(&mut f.marks, was));
        true
    }

    /// brings back the last change undo took back from a frame's marks. false when there was none.
    pub fn redo(&mut self, frame: usize) -> bool {
        let Some(f) = self.0.get_mut(&frame) else { return false };
        let Some(next) = f.after.pop() else { return false };
        f.before.push(std::mem::replace(&mut f.marks, next));
        true
    }

    /// takes every mark off a frame.
    pub fn clear(&mut self, frame: usize) {
        self.change_to(frame, Vec::new());
    }

    /// the frames that carry marks, in order: the timebar puts a tick at each.
    pub fn frames(&self) -> impl Iterator<Item = usize> + '_ {
        self.0.iter().filter(|(_, f)| !f.marks.is_empty()).map(|(frame, _)| *frame)
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

    /// `mark` changed on screen: each of its points taken to the window, through `f` there, and back to the file. a
    /// move or a scale made with the pointer is so in the window's pixels, however the picture is turned and whatever
    /// shape its pixels are drawn.
    pub fn on_screen(&self, mark: &Mark, f: impl Fn((f32, f32)) -> (f32, f32)) -> Mark {
        mark.through(|p| self.to_file(f(self.to_window(p))))
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

    /// the inks are the rainbow, then white, black and grey (the owner's ask, 2026-11-25): ten, the design's red,
    /// yellow and white as they were, its cyan a blue. no two are near: at least 60 apart in red, green and blue
    /// together.
    #[test]
    fn the_inks_are_the_rainbow_then_white_black_and_grey_each_apart() {
        let hex = |rgb: [u8; 3]| format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        assert_eq!(INKS.iter().map(|ink| hex(*ink)).collect::<Vec<_>>(), ["ff3b30", "ff9500", "ffd60a", "34c759", "1e90ff", "4b3cf0", "c74dff", "ffffff", "000000", "8e8e93"]);
        for (i, a) in INKS.iter().enumerate() {
            for b in &INKS[i + 1..] {
                let apart = a.iter().zip(b).map(|(x, y)| (*x as f32 - *y as f32).powi(2)).sum::<f32>().sqrt();
                assert!(apart > 60.0, "{} and {} are {apart:.0} apart", hex(*a), hex(*b));
            }
        }
    }

    /// redo brings back what undo took back, a step at a time, and a new change ends what could be brought back; one
    /// frame's steps are its own.
    #[test]
    fn redo_brings_back_what_undo_took_and_a_new_change_ends_it() {
        let mut marks = Marks::default();
        let mark = |x: f32| Mark::start(Tool::Rectangle, (x, x), RED);
        let (a, b, c) = (mark(1.0), mark(2.0), mark(3.0));
        let white = Mark { ink: INKS[7], ..b.clone() };
        marks.add(0, a.clone());
        marks.add(0, b.clone());
        marks.change(0, 1, Some(white.clone()));
        marks.add(4, c.clone());
        assert!(!marks.redo(0), "nothing taken back yet");
        assert!(marks.undo(0) && marks.undo(0));
        assert_eq!(marks.on(0), [a.clone()]);
        assert!(marks.redo(0));
        assert_eq!(marks.on(0), [a.clone(), b.clone()]);
        assert!(marks.redo(0));
        assert_eq!(marks.on(0), [a.clone(), white.clone()]);
        assert!(!marks.redo(0), "all brought back");
        assert!(marks.undo(0) && marks.undo(0));
        marks.add(0, c.clone());
        assert!(!marks.redo(0), "a new change ends what could be brought back");
        assert_eq!(marks.on(0), [a.clone(), c.clone()]);
        marks.clear(0);
        assert!(marks.undo(0) && marks.on(0) == [a.clone(), c.clone()]);
        assert!(marks.redo(0) && marks.on(0).is_empty(), "a clear is brought back too");
        assert!(marks.undo(4) && marks.on(4).is_empty() && marks.redo(4));
        assert_eq!(marks.on(4), [c]);
    }

    /// every change to a frame's marks is a step undo takes back, the last first: a mark drawn, changed, taken off,
    /// or the frame cleared. a change that changes nothing, or names no mark, is no step; another frame is left alone.
    #[test]
    fn undo_takes_back_each_change_to_a_frame_the_last_first() {
        let mut marks = Marks::default();
        let a = Mark::start(Tool::Rectangle, (1.0, 1.0), RED);
        let b = Mark { shape: Shape::Text((5.0, 5.0), "b".into(), 1.0), ink: RED };
        let white = Mark { ink: INKS[3], ..b.clone() };
        marks.add(0, a.clone());
        marks.add(0, b.clone());
        marks.add(4, a.clone());
        marks.change(0, 1, Some(white.clone()));
        marks.change(0, 1, Some(white.clone()));
        marks.change(0, 0, None);
        marks.change(0, 7, None);
        marks.clear(0);
        assert!(marks.on(0).is_empty());
        assert_eq!(marks.frames().collect::<Vec<_>>(), [4], "a cleared frame carries no tick");
        for was in [vec![white.clone()], vec![a.clone(), white.clone()], vec![a.clone(), b.clone()], vec![a.clone()], vec![]] {
            assert!(marks.undo(0));
            assert_eq!(marks.on(0), was);
        }
        assert!(!marks.undo(0), "nothing more to take back");
        assert_eq!(marks.on(4), [a]);
    }

    /// a move and a scale made on screen are so in the window's pixels, whatever the turn and the pixels' shape as
    /// drawn: with the file turned a quarter and its pixels drawn twice as wide as high, a box moved 30 across and 12
    /// down moves so on screen and by the turned, unevenly scaled amounts in the file, and one made twice as wide on
    /// screen is twice as tall in the file. text keeps its size through both. a corner dragged scales each way on its
    /// own, or with shift both by the larger, each the way it went.
    #[test]
    fn a_mark_moved_or_scaled_on_screen_is_so_on_screen_and_turned_in_the_file() {
        let close = |a: f32, b: f32| (a - b).abs() < 1e-3;
        let place = Placing { picture: (100.0, 20.0, 320.0, 320.0), look: Look { turns: 1, ..Look::default() }, size: (64, 32) };
        let corners = |mark: &Mark| match mark.shape {
            Shape::Rectangle(a, b) => (a, b),
            _ => panic!("{mark:?}"),
        };
        let boxed = Mark { shape: Shape::Rectangle((10.0, 5.0), (20.0, 15.0)), ink: RED };
        let (a, b) = corners(&boxed);
        let moved = place.on_screen(&boxed, |p| (p.0 + 30.0, p.1 + 12.0));
        let (ma, mb) = corners(&moved);
        for (was, now) in [(a, ma), (b, mb)] {
            let (w, n) = (place.to_window(was), place.to_window(now));
            assert!(close(n.0 - w.0, 30.0) && close(n.1 - w.1, 12.0), "on screen: {w:?} to {n:?}");
            // a pixel as shown is 10 across and 5 down: 3 and 2.4 of them, across the file's height and along its width
            assert!(close((now.0 - was.0).abs(), 2.4) && close((now.1 - was.1).abs(), 3.0), "in the file: {was:?} to {now:?}");
        }
        let about = place.to_window(a);
        let (wa, wb) = corners(&place.on_screen(&boxed, |p| (about.0 + (p.0 - about.0) * 2.0, p.1)));
        assert!(close((wb.0 - wa.0).abs(), (b.0 - a.0).abs()) && close((wb.1 - wa.1).abs(), 2.0 * (b.1 - a.1).abs()), "{wa:?} {wb:?}");
        let text = Mark { shape: Shape::Text((3.0, 4.0), "x".into(), 1.5), ink: RED };
        assert!(matches!(place.on_screen(&text, |p| (p.0 * 2.0, p.1 + 30.0)).shape, Shape::Text(_, _, size) if size == 1.5));
        assert_eq!(scale_by((0.0, 0.0), (10.0, 20.0), (30.0, 10.0), false), (3.0, 0.5));
        assert_eq!(scale_by((0.0, 0.0), (10.0, 20.0), (30.0, 10.0), true), (3.0, 3.0));
        assert_eq!(scale_by((0.0, 0.0), (10.0, 20.0), (-30.0, 10.0), true), (-3.0, 3.0));
        assert_eq!(scale_by((5.0, 5.0), (15.0, 5.0), (25.0, 9.0), false), (2.0, 1.0), "no extent down: as it was");
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
        assert!(Mark { shape: Shape::Text((0.0, 0.0), "  ".into(), 1.0), ink: RED }.tiny(1.0));
    }

    /// a pen's line is smoothed (the owner's ask, 2026-11-25). off, its points are as drawn. on, a hand's jitter goes
    /// and its first and last points hold; the default is gentle, keeping most of a wave a person drew on purpose,
    /// where the strongest takes most of it; and a line of three points keeps its ends.
    #[test]
    fn a_pens_line_is_smoothed_gently_and_keeps_its_ends() {
        let as_is = |p: Point| p;
        // a jitter of 1.5 each side every 2 across, and a wave 6 high and 40 long drawn a point every 3 across
        let jitter: Vec<Point> = (0..40).map(|i| (i as f32 * 2.0, if i % 2 == 0 { -1.5 } else { 1.5 })).collect();
        let wave: Vec<Point> = (0..41).map(|i| (i as f32 * 3.0, 6.0 * (i as f32 * 3.0 * std::f32::consts::TAU / 40.0).sin())).collect();
        // how far the line strays across, away from its ends
        let stray = |points: &[Point]| points[8..points.len() - 8].iter().map(|p| p.1.abs()).fold(0.0, f32::max);
        assert_eq!(smooth(&jitter, 0.0, as_is), jitter, "off");
        let gentle = crate::settings::Settings::default().pen_smoothing;
        assert!(gentle > 0.0 && gentle < 0.5, "the default is gentle: {gentle}");
        for line in [&jitter, &wave] {
            for strength in [gentle, 1.0] {
                let smoothed = smooth(line, strength, as_is);
                assert_eq!((smoothed[0], smoothed[smoothed.len() - 1]), (line[0], line[line.len() - 1]), "the ends hold at {strength}");
                assert!(stray(&smoothed) < stray(line), "at {strength} the line strays {} where it strayed {}", stray(&smoothed), stray(line));
            }
        }
        assert!(stray(&smooth(&jitter, gentle, as_is)) < 0.3 * stray(&jitter), "the default takes a hand's jitter");
        assert!(stray(&smooth(&wave, gentle, as_is)) > 0.7 * stray(&wave), "and keeps most of a wave drawn on purpose");
        assert!(stray(&smooth(&wave, 1.0, as_is)) < 0.5 * stray(&wave), "the strongest takes most of it");
        let short = [(0.0, 0.0), (1.0, 1.0), (2.0, 0.0)];
        let smoothed = smooth(&short, 1.0, as_is);
        assert_eq!((smoothed[0], smoothed[2]), (short[0], short[2]), "a short line keeps its length");
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
