//! the design's icons, drawn as strokes: each is the 16 x 16 drawing from docs/design/plan.html, line for line.
//! stroke 1.4, square ends, no fill, except where the design fills.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    First,
    Prev,
    Next,
    Play,
    Pause,
    Loop,
    Guides,
    Info,
    Shot,
    Sound,
    Mute,
    Full,
    Close,
    Pin,
    // the browser's: back and forward along the folders shown, and read again; what is shown of a folder (its
    // column, its cards, or both side by side), and how it is sorted
    Back,
    Forward,
    Refresh,
    List,
    Cards,
    ListAndCards,
    Sort,
    // markup's tools, and undo
    Rectangle,
    Circle,
    Pen,
    Text,
    Undo,
    // the strip's right end (fig. 32): help, and settings
    Help,
    Gear,
}

/// the gear's outline: eight teeth round it, as the design draws it
const GEAR: [(f32, f32); 32] = [
    (12.52, 7.12), (14.25, 7.23), (14.25, 8.77), (12.52, 8.88), (11.81, 10.57), (12.96, 11.88), (11.88, 12.96), (10.57, 11.81),
    (8.88, 12.52), (8.77, 14.25), (7.23, 14.25), (7.12, 12.52), (5.43, 11.81), (4.12, 12.96), (3.04, 11.88), (4.19, 10.57),
    (3.48, 8.88), (1.75, 8.77), (1.75, 7.23), (3.48, 7.12), (4.19, 5.43), (3.04, 4.12), (4.12, 3.04), (5.43, 4.19),
    (7.12, 3.48), (7.23, 1.75), (8.77, 1.75), (8.88, 3.48), (10.57, 4.19), (11.88, 3.04), (12.96, 4.12), (11.81, 5.43),
];

/// the stroke the design gives every icon, in the icon's own units
const WIDTH: f32 = 1.4;

/// draws `icon` at the design's size, 16 x 16, at the middle of `rect`, in `color`.
pub fn draw(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    draw_sized(painter, rect.center(), 16.0, icon, color);
}

/// draws `icon` in a box `size` on a side, around `middle`.
pub fn draw_sized(painter: &Painter, middle: Pos2, size: f32, icon: Icon, color: Color32) {
    let unit = size / 16.0;
    let p = |x: f32, y: f32| Pos2::new(middle.x + (x - 8.0) * unit, middle.y + (y - 8.0) * unit);
    let stroke = Stroke::new(WIDTH * unit, color);
    // square ends: a line reaches half its width past each end
    let line = |points: &[(f32, f32)]| {
        let mut pts: Vec<Pos2> = points.iter().map(|&(x, y)| p(x, y)).collect();
        let n = pts.len();
        if n >= 2 {
            let (a, b) = (pts[0], pts[1]);
            pts[0] = a - (b - a).normalized() * (stroke.width / 2.0);
            let (y, z) = (pts[n - 2], pts[n - 1]);
            pts[n - 1] = z + (z - y).normalized() * (stroke.width / 2.0);
        }
        painter.add(Shape::line(pts, stroke));
    };
    let fill = |points: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(points.iter().map(|&(x, y)| p(x, y)).collect(), color, Stroke::NONE));
    };
    // a quarter of a circle as short lines, from one angle to another in degrees, clockwise on screen
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<(f32, f32)> {
        (0..=6).map(|i| (from + (to - from) * i as f32 / 6.0).to_radians()).map(|a| (cx + r * a.cos(), cy + r * a.sin())).collect()
    };
    match icon {
        // not in the design: previous frame's bar with a second chevron, its ink centered in the box (the owner's
        // word, 2026-11-25)
        Icon::First => {
            line(&[(3.0, 3.5), (3.0, 12.5)]);
            line(&[(8.5, 3.5), (4.3, 8.0), (8.5, 12.5)]);
            line(&[(13.0, 3.5), (8.8, 8.0), (13.0, 12.5)]);
        }
        Icon::Prev => {
            line(&[(5.0, 3.5), (5.0, 12.5)]);
            line(&[(11.5, 3.5), (6.8, 8.0), (11.5, 12.5)]);
        }
        Icon::Next => {
            line(&[(11.0, 3.5), (11.0, 12.5)]);
            line(&[(4.5, 3.5), (9.2, 8.0), (4.5, 12.5)]);
        }
        Icon::Play => fill(&[(5.0, 3.0), (5.0, 13.0), (13.0, 8.0)]),
        Icon::Pause => {
            fill(&[(4.5, 3.0), (7.1, 3.0), (7.1, 13.0), (4.5, 13.0)]);
            fill(&[(8.9, 3.0), (11.5, 3.0), (11.5, 13.0), (8.9, 13.0)]);
        }
        Icon::Loop => {
            line(&[&[(3.0, 7.5)][..], &arc(5.5, 7.0, 2.5, 180.0, 270.0), &[(13.0, 4.5)]].concat());
            line(&[(11.0, 2.5), (13.0, 4.5), (11.0, 6.5)]);
            line(&[&[(13.0, 8.5)][..], &arc(10.5, 9.0, 2.5, 0.0, 90.0), &[(3.0, 11.5)]].concat());
            line(&[(5.0, 13.5), (3.0, 11.5), (5.0, 9.5)]);
        }
        Icon::Guides => {
            painter.rect_stroke(Rect::from_min_max(p(2.5, 2.5), p(13.5, 13.5)), 0.0, stroke, egui::StrokeKind::Middle);
            painter.line_segment([p(8.0, 2.5), p(8.0, 13.5)], stroke);
            painter.line_segment([p(2.5, 8.0), p(13.5, 8.0)], stroke);
            painter.circle_stroke(p(8.0, 8.0), 2.7 * unit, stroke);
        }
        Icon::Info => {
            // an i in a ring: its dot stands clear of its stem (as one line the two read as a bar, the owner saw)
            painter.circle_stroke(p(8.0, 8.0), 5.7 * unit, stroke);
            painter.circle_filled(p(8.0, 5.0), 0.95 * unit, color);
            painter.line_segment([p(8.0, 7.4), p(8.0, 11.2)], stroke);
        }
        Icon::Shot => {
            painter.add(Shape::closed_line([(2.5, 5.5), (4.7, 5.5), (5.7, 4.0), (10.3, 4.0), (11.3, 5.5), (13.5, 5.5), (13.5, 12.5), (2.5, 12.5)].iter().map(|&(x, y)| p(x, y)).collect(), stroke));
            painter.circle_stroke(p(8.0, 8.9), 2.2 * unit, stroke);
        }
        Icon::Sound | Icon::Mute => {
            painter.add(Shape::closed_line([(2.5, 6.3), (4.7, 6.3), (8.0, 3.7), (8.0, 12.3), (4.7, 9.7), (2.5, 9.7)].iter().map(|&(x, y)| p(x, y)).collect(), stroke));
            if icon == Icon::Sound {
                line(&arc(8.524, 8.0, 2.6, -43.8, 43.8));
                line(&arc(8.334, 8.0, 5.0, -42.8, 42.8));
            } else {
                line(&[(10.6, 6.3), (13.8, 9.7)]);
                line(&[(13.8, 6.3), (10.6, 9.7)]);
            }
        }
        Icon::Full => {
            line(&[(2.8, 6.0), (2.8, 2.8), (6.0, 2.8)]);
            line(&[(10.0, 2.8), (13.2, 2.8), (13.2, 6.0)]);
            line(&[(13.2, 10.0), (13.2, 13.2), (10.0, 13.2)]);
            line(&[(6.0, 13.2), (2.8, 13.2), (2.8, 10.0)]);
        }
        Icon::Close => {
            line(&[(4.5, 4.5), (11.5, 11.5)]);
            line(&[(11.5, 4.5), (4.5, 11.5)]);
        }
        Icon::Rectangle => {
            painter.rect_stroke(Rect::from_min_max(p(3.0, 4.5), p(13.0, 11.5)), 0.0, stroke, egui::StrokeKind::Middle);
        }
        Icon::Pin => {
            // a pushpin, upright: its head, its collar, its needle
            line(&[(5.5, 3.0), (10.5, 3.0)]);
            line(&[(6.5, 3.0), (6.5, 7.4), (4.4, 9.6), (11.6, 9.6), (9.5, 7.4), (9.5, 3.0)]);
            line(&[(8.0, 9.6), (8.0, 13.6)]);
        }
        Icon::Circle => {
            painter.circle_stroke(p(8.0, 8.0), 5.0 * unit, stroke);
        }
        Icon::Pen => {
            painter.add(Shape::closed_line([(3.0, 13.0), (3.8, 10.0), (11.0, 2.8), (13.2, 5.0), (6.0, 12.2)].iter().map(|&(x, y)| p(x, y)).collect(), stroke));
        }
        Icon::Text => {
            line(&[(4.0, 4.5), (12.0, 4.5)]);
            line(&[(8.0, 4.5), (8.0, 12.5)]);
        }
        Icon::Back => {
            line(&[(12.5, 8.0), (4.0, 8.0)]);
            line(&[(7.5, 4.2), (3.7, 8.0), (7.5, 11.8)]);
        }
        Icon::Forward => {
            line(&[(3.5, 8.0), (12.0, 8.0)]);
            line(&[(8.5, 4.2), (12.3, 8.0), (8.5, 11.8)]);
        }
        // three quarters of a ring, round by the bottom and the left to the top right, where its head points on round
        Icon::Refresh => {
            painter.add(Shape::line([arc(8.0, 8.4, 4.8, 20.0, 110.0), arc(8.0, 8.4, 4.8, 110.0, 200.0), arc(8.0, 8.4, 4.8, 200.0, 290.0)].concat().iter().map(|&(x, y)| p(x, y)).collect(), stroke));
            fill(&[(10.43, 1.73), (12.46, 4.92), (8.85, 6.05)]);
        }
        // rows: a mark and a line each
        Icon::List => {
            for y in [4.0, 8.0, 12.0] {
                fill(&[(2.5, y - 1.0), (4.5, y - 1.0), (4.5, y + 1.0), (2.5, y + 1.0)]);
                line(&[(7.0, y), (13.0, y)]);
            }
        }
        Icon::Cards => {
            for (x, y) in [(2.5, 2.5), (9.0, 2.5), (2.5, 9.0), (9.0, 9.0)] {
                painter.rect_stroke(Rect::from_min_max(p(x, y), p(x + 4.5, y + 4.5)), 0.0, stroke, egui::StrokeKind::Middle);
            }
        }
        // short rows at the left, two cards stacked at the right
        Icon::ListAndCards => {
            for y in [4.0, 8.0, 12.0] {
                line(&[(2.5, y), (5.5, y)]);
            }
            for y in [2.5, 9.0] {
                painter.rect_stroke(Rect::from_min_max(p(8.5, y), p(13.5, y + 4.5)), 0.0, stroke, egui::StrokeKind::Middle);
            }
        }
        // an arrow up beside an arrow down
        Icon::Sort => {
            line(&[(5.0, 13.0), (5.0, 3.0)]);
            line(&[(2.8, 5.0), (5.0, 2.7), (7.2, 5.0)]);
            line(&[(11.0, 3.0), (11.0, 13.0)]);
            line(&[(8.8, 11.0), (11.0, 13.3), (13.2, 11.0)]);
        }
        Icon::Undo => {
            line(&[(5.5, 4.0), (3.0, 6.5), (5.5, 9.0)]);
            line(&[&[(3.5, 6.5)][..], &arc(9.0, 9.7, 3.2, -90.0, 90.0), &[(6.5, 12.9)]].concat());
        }
        // a question mark: its hook two thirds of a ring round (8, 6.14), from the left over the top, then a curve
        // into its stem; its dot drawn as the info icon's
        Icon::Help => {
            let hook = [arc(8.0, 6.14, 2.3, 178.4, 258.4), arc(8.0, 6.14, 2.3, 258.4, 338.4), arc(8.0, 6.14, 2.3, 338.4, 418.6)].concat();
            let into = (1..=6).map(|i| {
                let (t, u) = (i as f32 / 6.0, 1.0 - i as f32 / 6.0);
                let at = |a: f32, b: f32, c: f32, d: f32| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
                (at(9.2, 8.5, 8.0, 8.0), at(8.1, 8.5, 9.0, 9.9))
            });
            line(&hook.into_iter().chain(into).chain([(8.0, 10.4)]).collect::<Vec<_>>());
            painter.circle_filled(p(8.0, 12.75), 0.95 * unit, color);
        }
        Icon::Gear => {
            painter.add(Shape::closed_line(GEAR.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
            painter.circle_stroke(p(8.0, 8.0), 2.1 * unit, stroke);
        }
    }
}
