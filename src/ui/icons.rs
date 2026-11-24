//! the design's icons, drawn as strokes: each is the 16 x 16 drawing from docs/design/plan.html, line for line.
//! stroke 1.4, square ends, no fill, except where the design fills.

use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke};

#[derive(Clone, Copy, PartialEq)]
pub enum Icon {
    Prev,
    Next,
    Play,
    Pause,
}

/// the stroke the design gives every icon, in the icon's own units
const WIDTH: f32 = 1.4;

/// draws `icon` in the 16 x 16 box at the middle of `rect`, in `color`.
pub fn draw(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let origin = rect.center() - egui::vec2(8.0, 8.0);
    let p = |x: f32, y: f32| Pos2::new(origin.x + x, origin.y + y);
    let stroke = Stroke::new(WIDTH, color);
    // square ends: a line reaches half its width past each end
    let line = |points: &[(f32, f32)]| {
        let mut pts: Vec<Pos2> = points.iter().map(|&(x, y)| p(x, y)).collect();
        let n = pts.len();
        if n >= 2 {
            let (a, b) = (pts[0], pts[1]);
            pts[0] = a - (b - a).normalized() * (WIDTH / 2.0);
            let (y, z) = (pts[n - 2], pts[n - 1]);
            pts[n - 1] = z + (z - y).normalized() * (WIDTH / 2.0);
        }
        painter.add(Shape::line(pts, stroke));
    };
    let fill = |points: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(points.iter().map(|&(x, y)| p(x, y)).collect(), color, Stroke::NONE));
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
    }
}
