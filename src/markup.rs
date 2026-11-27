//! markup: shapes and text drawn on a clip (fig. 19). a mark is kept in the file's own pixels, so it stays on the
//! picture through zoom, pan, mirror, flip and rotate, and it belongs to a group, which gives it the range of frames
//! it shows on. a clip's groups are its notes, which `crate::notes` keeps between runs (adr 0010). pure rules, the
//! same on every system: the controls draw what is here.

use crate::view::Look;

/// the inks: red, orange, yellow, green, blue, indigo, violet, then white, black and gray (the owner's ask,
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

/// a group: a range of frames, its out frame kept as a trim's is, and the marks in it. each of them shows on every
/// frame of the range.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub frames: (usize, usize),
    pub marks: Vec<Mark>,
}

impl Group {
    pub fn covers(&self, frame: usize) -> bool {
        (self.frames.0..=self.frames.1).contains(&frame)
    }
}

/// which part of a group's bar on the timebar a drag has hold of; or which of the trim's two markers, its in or out
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Part {
    In,
    Out,
    Whole,
}

/// a group's range as a drag on its bar leaves it: an end follows the pointer to `to` and never passes the other; the
/// whole range moves as far as the pointer went from `from`, its length kept, and stops at the clip's ends. `last` is
/// the clip's last frame.
pub fn dragged(was: (usize, usize), part: Part, from: usize, to: usize, last: usize) -> (usize, usize) {
    match part {
        Part::In => (to.min(was.1), was.1),
        Part::Out => (was.0, to.clamp(was.0, last.max(was.0))),
        Part::Whole => {
            let start = (was.0 + to).saturating_sub(from).min(last.saturating_sub(was.1 - was.0));
            (start, start + was.1 - was.0)
        }
    }
}

/// how many frames a new group lasts: `seconds` of the clip at `fps`, and one frame at least (a still's one)
pub fn lasting(seconds: f32, fps: f64) -> usize {
    ((seconds as f64 * fps).round() as usize).max(1)
}

/// which lane of the timebar each group's bar goes in, so that bars whose ranges overlap do not cover each other: the
/// lowest that is clear of the groups before it, and the last of `most` when none is
pub fn lanes(groups: &[Group], most: usize) -> Vec<usize> {
    let mut lanes: Vec<usize> = Vec::with_capacity(groups.len());
    for (i, group) in groups.iter().enumerate() {
        let taken = |lane: usize| (0..i).any(|j| lanes[j] == lane && groups[j].frames.0 <= group.frames.1 && group.frames.0 <= groups[j].frames.1);
        let lane = (0..most.saturating_sub(1)).find(|lane| !taken(*lane)).unwrap_or(most.saturating_sub(1));
        lanes.push(lane);
    }
    lanes
}

/// the notes on one clip: its groups, the first made first; what they were before each change, the last change last;
/// and what undo took back, the last taken last
#[derive(Clone, Debug, Default)]
pub struct Marks {
    groups: Vec<Group>,
    before: Vec<Vec<Group>>,
    after: Vec<Vec<Group>>,
    /// how many times they have changed: what has been sent to be saved is known by it
    changes: u64,
    /// read from a file that says they were made on another version of the clip's file. a change makes them this
    /// version's
    pub earlier: bool,
}

impl Marks {
    /// notes as they were read
    pub fn of(groups: Vec<Group>, earlier: bool) -> Marks {
        Marks { groups, earlier, ..Marks::default() }
    }

    pub fn groups(&self) -> &[Group] {
        &self.groups
    }

    pub fn changes(&self) -> u64 {
        self.changes
    }

    /// the marks that show on a frame: those of every group whose range holds it, group by group, the first drawn
    /// first
    pub fn on(&self, frame: usize) -> Vec<&Mark> {
        self.groups.iter().filter(|group| group.covers(frame)).flat_map(|group| &group.marks).collect()
    }

    /// where the mark at `index` among those on a frame is kept: its group, and its place in it
    fn place(&self, frame: usize, index: usize) -> Option<(usize, usize)> {
        let covering = self.groups.iter().enumerate().filter(|(_, group)| group.covers(frame));
        covering.flat_map(|(g, group)| (0..group.marks.len()).map(move |m| (g, m))).nth(index)
    }

    /// the group of the mark at `index` among those on a frame
    pub fn group_of(&self, frame: usize, index: usize) -> Option<usize> {
        self.place(frame, index).map(|(group, _)| group)
    }

    /// the group a mark drawn on `frame` goes into: the one selected, when its range holds the frame; else the newest
    /// whose range does. none starts a group of its own
    pub fn home(&self, frame: usize, selected: Option<usize>) -> Option<usize> {
        let covers = |group: &usize| self.groups.get(*group).is_some_and(|g| g.covers(frame));
        selected.filter(covers).or_else(|| self.groups.iter().rposition(|g| g.covers(frame)))
    }

    /// a change to the groups, which undo can take back. a group left with no marks goes. what undo took back before
    /// can no longer be brought back
    fn change_to(&mut self, mut groups: Vec<Group>) {
        groups.retain(|group| !group.marks.is_empty());
        if groups != self.groups {
            self.before.push(std::mem::replace(&mut self.groups, groups));
            self.after.clear();
            (self.changes, self.earlier) = (self.changes + 1, false);
        }
    }

    /// a mark drawn on `frame`: it joins the group `home` names, or starts one there that lasts `lasting` frames, and
    /// no further than the last of the clip's `frames`
    pub fn add(&mut self, frame: usize, selected: Option<usize>, mark: Mark, lasting: usize, frames: usize) {
        let mut groups = self.groups.clone();
        match self.home(frame, selected) {
            Some(group) => groups[group].marks.push(mark),
            None => groups.push(Group { frames: (frame, (frame + lasting.max(1) - 1).min(frames.saturating_sub(1)).max(frame)), marks: vec![mark] }),
        }
        self.change_to(groups);
    }

    /// the mark at `index` among those on a frame made `to` (moved, scaled, another ink, its text typed again), or
    /// taken off
    pub fn change(&mut self, frame: usize, index: usize, to: Option<Mark>) {
        let Some((group, at)) = self.place(frame, index) else { return };
        let mut groups = self.groups.clone();
        match to {
            Some(mark) => groups[group].marks[at] = mark,
            None => _ = groups[group].marks.remove(at),
        }
        self.change_to(groups);
    }

    /// the mark at `index` among those on a frame gets a range of its own: it moves into a new group with the range
    /// it had, to be set from there. one alone in its group has its own already
    pub fn own_range(&mut self, frame: usize, index: usize) {
        let Some((group, at)) = self.place(frame, index).filter(|(group, _)| self.groups[*group].marks.len() > 1) else { return };
        let mut groups = self.groups.clone();
        let mark = groups[group].marks.remove(at);
        groups.push(Group { frames: groups[group].frames, marks: vec![mark] });
        self.change_to(groups);
    }

    /// a group's range set, as a drag on its bar leaves it: never backwards, and no further than the last of the
    /// clip's `frames`
    pub fn set_range(&mut self, group: usize, (first, last): (usize, usize), frames: usize) {
        if group >= self.groups.len() {
            return;
        }
        let mut groups = self.groups.clone();
        let last = last.min(frames.saturating_sub(1));
        groups[group].frames = (first.min(last), last);
        self.change_to(groups);
    }

    /// a group's in (or, with `out`, its out) set at `frame`, as the trim's keys set its points: the other end comes
    /// along when it would be left behind, so a group is never backwards. returns its range then
    pub fn set_end(&mut self, group: usize, out: bool, frame: usize) -> Option<(usize, usize)> {
        let was = self.groups.get(group)?.frames;
        let frames = if out { (was.0.min(frame), frame) } else { (frame, was.1.max(frame)) };
        self.set_range(group, frames, usize::MAX);
        Some(frames)
    }

    /// takes back the last change: a mark drawn, changed or taken off, a range set, a clear. false when there was none.
    pub fn undo(&mut self) -> bool {
        let Some(was) = self.before.pop() else { return false };
        self.after.push(std::mem::replace(&mut self.groups, was));
        self.changes += 1;
        true
    }

    /// brings back the last change undo took back. false when there was none.
    pub fn redo(&mut self) -> bool {
        let Some(next) = self.after.pop() else { return false };
        self.before.push(std::mem::replace(&mut self.groups, next));
        self.changes += 1;
        true
    }

    /// takes off the group selected, or with none every group that shows on a frame.
    pub fn clear(&mut self, frame: usize, selected: Option<usize>) {
        let selected = selected.filter(|group| *group < self.groups.len());
        let kept = |(i, group): &(usize, &Group)| selected.map_or(!group.covers(frame), |s| s != *i);
        self.change_to(self.groups.iter().enumerate().filter(kept).map(|(_, group)| group.clone()).collect());
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

    /// a mark as the tests draw it
    fn mark(x: f32) -> Mark {
        Mark::start(Tool::Rectangle, (x, x), RED)
    }

    /// the marks on a frame, as their own copies
    fn on(marks: &Marks, frame: usize) -> Vec<Mark> {
        marks.on(frame).into_iter().cloned().collect()
    }

    /// a mark drawn on a frame no group covers starts a group there that lasts the settings' seconds (3 by default:
    /// 75 frames of a clip at 25 a second, in 55 and out 129 as a trim counts) and no further than the clip's last
    /// frame; a still's group is its one frame. the mark shows on every frame of the range and on no other.
    #[test]
    fn a_mark_starts_a_group_that_lasts_three_seconds_and_no_further_than_the_clip() {
        let seconds = crate::settings::Settings::default().note_seconds;
        assert_eq!((seconds, lasting(seconds, 25.0), lasting(seconds, 24000.0 / 1001.0), lasting(0.5, 60.0)), (3.0, 75, 72, 30));
        assert_eq!(lasting(seconds, 0.0), 1, "a still has no rate, and one frame");
        let mut marks = Marks::default();
        marks.add(55, None, mark(1.0), 75, 150);
        assert_eq!(marks.groups(), [Group { frames: (55, 129), marks: vec![mark(1.0)] }]);
        assert_eq!((on(&marks, 54), on(&marks, 55), on(&marks, 129), on(&marks, 130)), (vec![], vec![mark(1.0)], vec![mark(1.0)], vec![]));
        marks.add(140, None, mark(2.0), 75, 150);
        assert_eq!(marks.groups()[1].frames, (140, 149), "cut at the clip's last frame");
        let mut still = Marks::default();
        still.add(0, None, mark(1.0), lasting(seconds, 0.0), 1);
        assert_eq!(still.groups()[0].frames, (0, 0));
    }

    /// a mark drawn on a frame a group covers joins it, the newest of several; with a group selected it joins that
    /// one, when its range holds the frame; and only on a frame no group covers does it start one.
    #[test]
    fn a_mark_joins_the_group_that_covers_its_frame_the_newest_or_the_one_selected() {
        let mut marks = Marks::default();
        marks.add(10, None, mark(1.0), 20, 100);
        marks.add(40, None, mark(2.0), 20, 100);
        marks.set_range(1, (20, 59), 100);
        assert_eq!(marks.groups().iter().map(|g| g.frames).collect::<Vec<_>>(), [(10, 29), (20, 59)]);
        assert_eq!((marks.home(15, None), marks.home(25, None), marks.home(25, Some(0)), marks.home(45, Some(0)), marks.home(70, Some(0))), (Some(0), Some(1), Some(0), Some(1), None));
        marks.add(25, None, mark(3.0), 20, 100);
        assert_eq!(marks.groups()[1].marks, [mark(2.0), mark(3.0)], "the newest that covers it");
        marks.add(25, Some(0), mark(4.0), 20, 100);
        assert_eq!(marks.groups()[0].marks, [mark(1.0), mark(4.0)], "the one selected");
        marks.add(45, Some(0), mark(5.0), 20, 100);
        assert_eq!(marks.groups()[1].marks.len(), 3, "the one selected does not hold the frame: the one that does");
        marks.add(70, Some(0), mark(6.0), 20, 100);
        assert_eq!((marks.groups().len(), marks.groups()[2].frames), (3, (70, 89)), "no group covers it: a new one");
        // the marks on a frame are every covering group's, group by group
        assert_eq!(on(&marks, 25), [mark(1.0), mark(4.0), mark(2.0), mark(3.0), mark(5.0)]);
        assert_eq!((marks.group_of(25, 1), marks.group_of(25, 2), marks.group_of(25, 5)), (Some(0), Some(1), None));
    }

    /// a drag on a group's bar: an end follows the pointer and stops at the other end, the out at the clip's last
    /// frame too; the middle moves the whole range as far as the pointer went, its length kept, between the clip's
    /// ends.
    #[test]
    fn a_groups_end_dragged_resizes_it_and_its_middle_moves_it() {
        let was = (55, 129);
        assert_eq!(dragged(was, Part::In, 55, 40, 149), (40, 129));
        assert_eq!(dragged(was, Part::In, 55, 140, 149), (129, 129), "the in never passes the out");
        assert_eq!(dragged(was, Part::Out, 129, 100, 149), (55, 100));
        assert_eq!(dragged(was, Part::Out, 129, 20, 149), (55, 55), "nor the out the in");
        assert_eq!(dragged(was, Part::Out, 129, 400, 149), (55, 149), "nor the clip's last frame");
        assert_eq!(dragged(was, Part::Whole, 90, 100, 149), (65, 139));
        assert_eq!(dragged(was, Part::Whole, 90, 20, 149), (0, 74), "it stops at the clip's first frame");
        assert_eq!(dragged(was, Part::Whole, 90, 149, 149), (75, 149), "and at its last");
        let mut marks = Marks::default();
        marks.add(55, None, mark(1.0), 75, 150);
        marks.set_range(0, (65, 400), 150);
        assert_eq!(marks.groups()[0].frames, (65, 149));
        assert!(marks.undo() && marks.groups()[0].frames == (55, 129), "a range set is a step undo takes back");
    }

    /// a group's in and out set at the frame on screen, as shift with [ and ] set a trim's: the other end comes along
    /// when it would be left behind.
    #[test]
    fn a_groups_in_and_out_are_set_at_a_frame_and_it_is_never_backwards() {
        let mut marks = Marks::default();
        marks.add(55, None, mark(1.0), 75, 150);
        assert_eq!(marks.set_end(0, false, 60), Some((60, 129)));
        assert_eq!(marks.set_end(0, true, 100), Some((60, 100)));
        assert_eq!(marks.set_end(0, false, 120), Some((120, 120)), "an in past the out takes the out along");
        assert_eq!(marks.set_end(0, true, 30), Some((30, 30)), "an out before the in takes the in along");
        assert_eq!((marks.groups()[0].frames, marks.set_end(3, true, 30)), ((30, 30), None));
    }

    /// "own range" moves a mark into a new group with the range it had, which is then set by itself; a mark alone in
    /// its group has its own already. a group left with no marks goes.
    #[test]
    fn a_mark_gets_a_range_of_its_own_in_a_new_group() {
        let mut marks = Marks::default();
        marks.add(10, None, mark(1.0), 20, 100);
        marks.add(12, None, mark(2.0), 20, 100);
        marks.own_range(12, 1);
        assert_eq!(marks.groups(), [Group { frames: (10, 29), marks: vec![mark(1.0)] }, Group { frames: (10, 29), marks: vec![mark(2.0)] }]);
        marks.own_range(12, 1);
        assert_eq!(marks.groups().len(), 2, "alone in its group: nothing to do");
        marks.set_range(1, (40, 50), 100);
        assert_eq!((on(&marks, 12), on(&marks, 45)), (vec![mark(1.0)], vec![mark(2.0)]));
        marks.change(45, 0, None);
        assert_eq!(marks.groups().len(), 1, "its last mark taken off, the group goes");
    }

    /// bars whose ranges overlap go in lanes, the lowest that is clear; past the lanes there are, in the last.
    #[test]
    fn groups_that_overlap_take_lanes() {
        let group = |frames| Group { frames, marks: vec![mark(1.0)] };
        let groups = [group((0, 10)), group((5, 20)), group((11, 30)), group((8, 9)), group((40, 50))];
        assert_eq!(lanes(&groups, 3), [0, 1, 0, 2, 0]);
        assert_eq!(lanes(&groups, 2), [0, 1, 0, 1, 0]);
    }

    /// every change to a clip's notes is a step undo takes back, the last first, and redo brings back: a mark drawn,
    /// changed, taken off, a clear. a change that changes nothing, or names no mark, is no step. a new change ends
    /// what could be brought back. each step counts as a change, so it is saved.
    #[test]
    fn undo_takes_back_each_change_the_last_first_and_redo_brings_it_back() {
        let mut marks = Marks::default();
        let (a, b, c) = (mark(1.0), Mark { shape: Shape::Text((5.0, 5.0), "b".into(), 1.0), ink: RED }, mark(3.0));
        let white = Mark { ink: INKS[7], ..b.clone() };
        marks.add(0, None, a.clone(), 1, 100);
        marks.add(0, None, b.clone(), 1, 100);
        marks.add(4, None, a.clone(), 1, 100);
        marks.change(0, 1, Some(white.clone()));
        marks.change(0, 1, Some(white.clone()));
        marks.change(0, 0, None);
        marks.change(0, 7, None);
        marks.clear(0, None);
        assert!(on(&marks, 0).is_empty() && !marks.redo(), "nothing taken back yet");
        assert_eq!(marks.changes(), 6);
        for was in [vec![white.clone()], vec![a.clone(), white.clone()], vec![a.clone(), b.clone()]] {
            assert!(marks.undo());
            assert_eq!((on(&marks, 0), on(&marks, 4)), (was, vec![a.clone()]));
        }
        assert!(marks.undo() && on(&marks, 4).is_empty(), "the mark on another frame was drawn before those changes");
        assert!(marks.undo() && marks.undo() && marks.groups().is_empty() && !marks.undo(), "nothing more to take back");
        assert!(marks.redo() && marks.redo());
        assert_eq!(on(&marks, 0), [a.clone(), b.clone()]);
        marks.add(0, None, c.clone(), 1, 100);
        assert!(!marks.redo(), "a new change ends what could be brought back");
        assert_eq!(on(&marks, 0), [a.clone(), b.clone(), c.clone()]);
        marks.clear(0, None);
        assert!(marks.undo() && on(&marks, 0).len() == 3 && marks.redo() && on(&marks, 0).is_empty(), "a clear is brought back too");
        assert_eq!(marks.changes(), 18, "undo and redo are changes too: what is saved follows them");
    }

    /// clear takes off every group that shows on the frame on screen, or with a group selected that group alone;
    /// notes read from a file made on an earlier version of the clip's file are this version's once they change.
    #[test]
    fn clear_takes_the_groups_on_a_frame_or_the_one_selected() {
        let group = |frames, x| Group { frames, marks: vec![mark(x)] };
        let mut marks = Marks::of(vec![group((0, 10), 1.0), group((5, 20), 2.0), group((30, 40), 3.0)], true);
        assert!(marks.earlier && marks.changes() == 0);
        marks.clear(7, Some(2));
        assert_eq!(marks.groups(), [group((0, 10), 1.0), group((5, 20), 2.0)]);
        assert!(!marks.earlier);
        marks.clear(7, None);
        assert!(marks.groups().is_empty());
    }

    /// the inks are the rainbow, then white, black and gray (the owner's ask, 2026-11-25): ten, the design's red,
    /// yellow and white as they were, its cyan a blue. no two are near: at least 60 apart in red, green and blue
    /// together.
    #[test]
    fn the_inks_are_the_rainbow_then_white_black_and_gray_each_apart() {
        let hex = |rgb: [u8; 3]| format!("{:02x}{:02x}{:02x}", rgb[0], rgb[1], rgb[2]);
        assert_eq!(INKS.iter().map(|ink| hex(*ink)).collect::<Vec<_>>(), ["ff3b30", "ff9500", "ffd60a", "34c759", "1e90ff", "4b3cf0", "c74dff", "ffffff", "000000", "8e8e93"]);
        for (i, a) in INKS.iter().enumerate() {
            for b in &INKS[i + 1..] {
                let apart = a.iter().zip(b).map(|(x, y)| (*x as f32 - *y as f32).powi(2)).sum::<f32>().sqrt();
                assert!(apart > 60.0, "{} and {} are {apart:.0} apart", hex(*a), hex(*b));
            }
        }
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
