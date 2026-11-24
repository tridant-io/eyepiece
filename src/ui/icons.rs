//! the design's icons, drawn as strokes: each is the 16 x 16 drawing from docs/design/plan.html, line for line.
//! stroke 1.4, square ends, no fill, except where the design fills.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Prev,
    Next,
    Play,
    Pause,
    Loop,
    Guides,
    Full,
    Close,
}

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
    }
}
