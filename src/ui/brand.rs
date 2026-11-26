//! the tridant star and wordmark, and the program's icon, as textures the controls draw tinted and turned. egui draws
//! no svg and no crate is added for it, so the brand kit's own files (assets/brand) are read by a small rasterizer of
//! ours: the absolute M, L, H, V, C and Z the files use, filled by nonzero winding at 4 x 4 samples a pixel, with the
//! star's stroke, which the kit draws together with its fill.

use egui::emath::Rot2;
use egui::epaint::Mesh;
use egui::{Color32, Pos2, Rect, TextureHandle, Vec2};
use std::sync::LazyLock;

static STAR: LazyLock<Drawing> = LazyLock::new(|| Drawing::parse(include_str!("../../assets/brand/tridant-mark.svg")));
/// the star of tridant.io's favicon, bolder than the kit's at the strip's size
static MARK: LazyLock<Drawing> = LazyLock::new(|| Drawing::parse(include_str!("../../assets/brand/tridant-favicon-star.svg")));
static WORDMARK: LazyLock<Drawing> = LazyLock::new(|| Drawing::parse(include_str!("../../assets/brand/tridant-wordmark.svg")));
/// samples a pixel takes along each side
const SAMPLES: usize = 4;
/// short lines a cubic is drawn with
const CURVE: usize = 16;
/// the star's own middle in its file's units, which it turns about (its view box is not centered on it), and the
/// square round it the turning star is drawn in: the 92 of the card's box (fig. 32) and 2 more each side, so no tip
/// meets the texture's edge as it turns
const MIDDLE: Pos2 = Pos2::new(50.0, 50.0);
const TURNING: f32 = 96.0;
/// how much of the card's 40 the turning star's texture takes
pub const TURNING_SIZE: f32 = 40.0 * TURNING / 92.0;

/// what is drawn as a texture
#[derive(Clone, Copy, PartialEq)]
pub enum Art {
    /// the favicon's star in its view box: the strip's mark, about's small star
    Star,
    /// the star round its own middle, for the card's star that turns
    Turning,
    Wordmark,
    /// the program's icon, assets/icon/eyepiece.png
    Icon,
}

/// the textures made so far, one for each art and size in pixels
#[derive(Default)]
pub struct Pictures(Vec<(Art, [usize; 2], TextureHandle)>);

impl Pictures {
    /// `art` as a texture `size` pixels, made the first time it is asked for at that size.
    pub fn get(&mut self, ctx: &egui::Context, art: Art, size: [usize; 2]) -> egui::TextureId {
        if let Some((.., made)) = self.0.iter().find(|(a, s, _)| *a == art && *s == size) {
            return made.id();
        }
        let image = match art {
            Art::Star => alpha(size, &MARK.raster(MARK.view, size)),
            Art::Turning => alpha(size, &STAR.raster(Rect::from_center_size(MIDDLE, Vec2::splat(TURNING)), size)),
            Art::Wordmark => alpha(size, &WORDMARK.raster(WORDMARK.view, size)),
            Art::Icon => icon(size),
        };
        let made = ctx.load_texture("brand", image, egui::TextureOptions::LINEAR);
        let id = made.id();
        self.0.push((art, size, made));
        id
    }
}

/// white, its coverage as alpha: drawn tinted
fn alpha(size: [usize; 2], coverage: &[u8]) -> egui::ColorImage {
    egui::ColorImage::new(size, coverage.iter().map(|&a| Color32::from_white_alpha(a)).collect())
}

/// the program's icon brought down to `size` by averaging each pixel's share of the file: there are no mipmaps to do
/// it as it is drawn. empty if the file does not read.
fn icon(size: [usize; 2]) -> egui::ColorImage {
    static FILE: LazyLock<Option<(usize, usize, Vec<u8>)>> = LazyLock::new(|| {
        let mut reader = png::Decoder::new(std::io::Cursor::new(include_bytes!("../../assets/icon/eyepiece.png"))).read_info().ok()?;
        let mut rgba = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut rgba).ok()?;
        (info.color_type == png::ColorType::Rgba && info.bit_depth == png::BitDepth::Eight).then_some((info.width as usize, info.height as usize, rgba))
    });
    let Some((w, h, rgba)) = FILE.as_ref() else { return egui::ColorImage::filled(size, Color32::TRANSPARENT) };
    let mut out = Vec::with_capacity(size[0] * size[1] * 4);
    for y in 0..size[1] {
        let rows = y * h / size[1]..((y + 1) * h / size[1]).max(y * h / size[1] + 1);
        for x in 0..size[0] {
            let columns = x * w / size[0]..((x + 1) * w / size[0]).max(x * w / size[0] + 1);
            // summed premultiplied, so a clear pixel's color counts for nothing
            let mut sum = [0u64; 4];
            for row in rows.clone() {
                for p in rgba[(row * w + columns.start) * 4..(row * w + columns.end) * 4].as_chunks::<4>().0 {
                    let a = p[3] as u64;
                    sum = [sum[0] + p[0] as u64 * a / 255, sum[1] + p[1] as u64 * a / 255, sum[2] + p[2] as u64 * a / 255, sum[3] + a];
                }
            }
            let n = (rows.len() * columns.len()) as u64;
            out.extend(sum.map(|s| (s / n) as u8));
        }
    }
    egui::ColorImage::from_rgba_premultiplied(size, &out)
}

/// draws a texture made white into `rect`, in `color`, turned `degrees` clockwise about the rect's middle.
pub fn draw(painter: &egui::Painter, texture: egui::TextureId, rect: Rect, degrees: f32, color: Color32) {
    let mut mesh = Mesh::with_texture(texture);
    mesh.add_rect_with_uv(rect, Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)), color);
    if degrees != 0.0 {
        mesh.rotate(Rot2::from_angle(degrees.to_radians()), rect.center());
    }
    painter.add(mesh);
}

/// the card's star `t` seconds after the card came up (the design's fig. 32, simplified): it boots, turning in from
/// −150° as it grows from 55% to full, over 700 ms; then after each still hold of 1.6 to 4.4 s, drawn from `seed`, it
/// throws 120° in 400 ms. every move eases out and stops dead on a multiple of 120°, where the star looks as it did.
/// returns its turn in degrees, its scale, and how long it stands still from `t`: none while it moves.
pub fn motion(t: f64, seed: u64) -> (f32, f32, Option<f64>) {
    const BOOT: f64 = 0.7;
    const THROW: f64 = 0.4;
    // out-expo, as the site's moves
    let ease = |x: f64| (1.0 - (-10.0 * x).exp2()) as f32;
    if t < BOOT {
        let e = ease(t / BOOT);
        return (-150.0 * (1.0 - e), 0.55 + 0.45 * e, None);
    }
    let (mut from, mut k) = (BOOT, 0u64);
    loop {
        let rest = 1.6 + 2.8 * unit(seed, k);
        let at = (k % 3) as f32 * 120.0;
        if t < from + rest {
            return (at, 1.0, Some(from + rest - t));
        }
        from += rest;
        if t < from + THROW {
            return (at + 120.0 * ease((t - from) / THROW), 1.0, None);
        }
        (from, k) = (from + THROW, k + 1);
    }
}

/// a number from 0 to 1 for the `k`th hold of a card shown at `seed` (splitmix64)
fn unit(seed: u64, k: u64) -> f64 {
    let mut z = seed.wrapping_add(k.wrapping_add(1).wrapping_mul(0x9e37_79b9_7f4a_7c15));
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    (z ^ (z >> 31)) as f64 / u64::MAX as f64
}

/// a drawing read from an svg file: its view box, its outlines as closed polygons in the file's units, whether they
/// are filled, the width of the stroke drawn along their edges (0 for none), and the circle a clip path keeps it in
pub struct Drawing {
    view: Rect,
    outlines: Vec<Vec<Pos2>>,
    fill: bool,
    stroke: f32,
    clip: Option<(Pos2, f32)>,
}

impl Drawing {
    /// reads the view box, every path's outline, fill and stroke width, and a clip path's circle. what it does not
    /// know it leaves out.
    pub fn parse(svg: &str) -> Drawing {
        let view = match attribute(svg, "viewBox").map(numbers).as_deref() {
            Some(&[x, y, w, h]) => Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h)),
            _ => Rect::from_min_size(Pos2::ZERO, Vec2::splat(1.0)),
        };
        let number = |tag: &str, name: &str| attribute(tag, name).and_then(|v| v.parse::<f32>().ok());
        let clip = svg.split("<clipPath").nth(1).and_then(|c| c.split("<circle").nth(1)).and_then(|c| Some((Pos2::new(number(c, "cx")?, number(c, "cy")?), number(c, "r")?)));
        let mut drawing = Drawing { view, outlines: Vec::new(), fill: true, stroke: 0.0, clip };
        for path in svg.split("<path").skip(1) {
            drawing.outlines.extend(attribute(path, "d").map(outlines).unwrap_or_default());
            drawing.fill = attribute(path, "fill") != Some("none");
            if let Some(width) = number(path, "stroke-width") {
                drawing.stroke = width;
            }
        }
        drawing
    }

    /// how much of each pixel the drawing covers, 0 to 255, row by row: `window` (in the file's units) drawn into
    /// `size` pixels, scaled alike both ways to fit and centered, as an svg fits its view box.
    pub fn raster(&self, window: Rect, [w, h]: [usize; 2]) -> Vec<u8> {
        let k = (w as f32 / window.width()).min(h as f32 / window.height());
        let margin = (Vec2::new(w as f32, h as f32) - window.size() * k) / 2.0;
        let n = SAMPLES as f32;
        // in samples from here on
        let outlines: Vec<Vec<Pos2>> = self.outlines.iter().map(|o| o.iter().map(|&p| (((p - window.min) * k + margin) * n).to_pos2()).collect()).collect();
        let edges = || outlines.iter().flat_map(|o| o.iter().copied().zip(o.iter().copied().cycle().skip(1)));
        let (sw, sh) = (w * SAMPLES, h * SAMPLES);
        let mut inside = vec![false; sw * sh];
        // a sample's column from a place along a row: the first whose middle is at or past it
        let column = |x: f32| ((x - 0.5).ceil().max(0.0) as usize).min(sw);
        for row in (0..sh).filter(|_| self.fill) {
            let y = row as f32 + 0.5;
            let mut crossings: Vec<(f32, i32)> = edges().filter(|(a, b)| (a.y <= y) != (b.y <= y)).map(|(a, b)| (a.x + (y - a.y) * (b.x - a.x) / (b.y - a.y), if b.y > a.y { 1 } else { -1 })).collect();
            crossings.sort_by(|p, q| p.0.total_cmp(&q.0));
            let mut winding = 0;
            for pair in crossings.windows(2) {
                winding += pair[0].1;
                if winding != 0 {
                    inside[row * sw + column(pair[0].0)..row * sw + column(pair[1].0)].fill(true);
                }
            }
        }
        // the stroke: every sample within half its width of an edge
        let half = self.stroke / 2.0 * k * n;
        if half > 0.0 {
            for (a, b) in edges() {
                let (low, high) = (a.min(b) - Vec2::splat(half), a.max(b) + Vec2::splat(half));
                let rows = ((low.y - 0.5).ceil().max(0.0) as usize).min(sh)..((high.y - 0.5).ceil().max(0.0) as usize).min(sh);
                for y in rows {
                    for x in column(low.x)..column(high.x) {
                        if distance(Pos2::new(x as f32 + 0.5, y as f32 + 0.5), a, b) <= half {
                            inside[y * sw + x] = true;
                        }
                    }
                }
            }
        }
        // the clip: no sample outside its circle
        if let Some((middle, r)) = self.clip {
            let (middle, r) = ((((middle - window.min) * k + margin) * n).to_pos2(), r * k * n);
            for (i, sample) in inside.iter_mut().enumerate() {
                *sample &= Pos2::new((i % sw) as f32 + 0.5, (i / sw) as f32 + 0.5).distance(middle) <= r;
            }
        }
        let mut coverage = vec![0u8; w * h];
        for (i, pixel) in coverage.iter_mut().enumerate() {
            let (x, y) = (i % w * SAMPLES, i / w * SAMPLES);
            let count = (0..SAMPLES).map(|j| inside[(y + j) * sw + x..(y + j) * sw + x + SAMPLES].iter().filter(|s| **s).count()).sum::<usize>();
            *pixel = (count * 255 / (SAMPLES * SAMPLES)) as u8;
        }
        coverage
    }
}

/// how far `p` is from the segment from `a` to `b`
fn distance(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let along = b - a;
    let t = if along.length_sq() > 0.0 { ((p - a).dot(along) / along.length_sq()).clamp(0.0, 1.0) } else { 0.0 };
    (a + along * t).distance(p)
}

/// an attribute's value in a tag: ` name="value"`
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let from = tag.find(&key)? + key.len();
    tag[from..].split('"').next()
}

enum Token {
    Command(u8),
    Number(f32),
}

/// a path's letters and numbers. a number is a sign, figures with one point at most, and an exponent: a sign or a
/// second point after its start begins the next one, as svg writes "1-2" and "1.5.5"
fn tokens(d: &str) -> Vec<Token> {
    let b = d.as_bytes();
    let (mut out, mut i) = (Vec::new(), 0);
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_alphabetic() {
            out.push(Token::Command(c));
            i += 1;
            continue;
        }
        if !(c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.')) {
            i += 1;
            continue;
        }
        let (start, mut point) = (i, c == b'.');
        i += 1;
        while i < b.len() {
            match b[i] {
                b'0'..=b'9' => i += 1,
                b'.' if !point => (point, i) = (true, i + 1),
                b'e' | b'E' => i += if matches!(b.get(i + 1), Some(b'-' | b'+')) { 2 } else { 1 },
                _ => break,
            }
        }
        if let Ok(v) = d[start..i].parse() {
            out.push(Token::Number(v));
        }
    }
    out
}

fn numbers(text: &str) -> Vec<f32> {
    tokens(text).into_iter().filter_map(|t| if let Token::Number(v) = t { Some(v) } else { None }).collect()
}

/// a path's closed outlines, each a polygon, its cubics as short lines
fn outlines(d: &str) -> Vec<Vec<Pos2>> {
    let tokens = tokens(d);
    let (mut all, mut now): (Vec<Vec<Pos2>>, Vec<Pos2>) = (Vec::new(), Vec::new());
    let (mut at, mut start, mut command, mut i) = (Pos2::ZERO, Pos2::ZERO, b'M', 0);
    let close = |now: &mut Vec<Pos2>, all: &mut Vec<Vec<Pos2>>| {
        if now.len() > 2 {
            all.push(std::mem::take(now));
        }
        now.clear();
    };
    while i < tokens.len() {
        if let Token::Command(c) = tokens[i] {
            (command, i) = (c, i + 1);
            if c == b'Z' {
                close(&mut now, &mut all);
                at = start;
            }
            continue;
        }
        let wanted = match command {
            b'M' | b'L' => 2,
            b'C' => 6,
            _ => 1,
        };
        let n: Vec<f32> = tokens[i..].iter().take(wanted).map_while(|t| if let Token::Number(v) = t { Some(*v) } else { None }).collect();
        i += n.len().max(1);
        if n.len() < wanted {
            continue;
        }
        if command == b'M' {
            close(&mut now, &mut all);
            (at, start) = (Pos2::new(n[0], n[1]), Pos2::new(n[0], n[1]));
            now.push(at);
            // more pairs after a move are lines
            command = b'L';
            continue;
        }
        // a command these files do not use: its numbers are passed over
        if !matches!(command, b'L' | b'H' | b'V' | b'C') {
            continue;
        }
        // after a close, the next outline starts where the last one did
        if now.is_empty() {
            now.push(at);
        }
        match command {
            b'L' => at = Pos2::new(n[0], n[1]),
            b'H' => at.x = n[0],
            b'V' => at.y = n[0],
            b'C' => {
                let (c1, c2, end) = (Pos2::new(n[0], n[1]), Pos2::new(n[2], n[3]), Pos2::new(n[4], n[5]));
                for s in 1..CURVE {
                    let t = s as f32 / CURVE as f32;
                    let u = 1.0 - t;
                    now.push((at.to_vec2() * (u * u * u) + c1.to_vec2() * (3.0 * u * u * t) + c2.to_vec2() * (3.0 * u * t * t) + end.to_vec2() * (t * t * t)).to_pos2());
                }
                at = end;
            }
            _ => {}
        }
        now.push(at);
    }
    close(&mut now, &mut all);
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn drawn(svg: &str, size: usize) -> Vec<u8> {
        let drawing = Drawing::parse(svg);
        drawing.raster(drawing.view, [size, size])
    }

    /// a square on whole pixels covers them wholly and nothing else.
    #[test]
    fn a_square_covers_its_pixels_and_no_others() {
        let square = drawn(r#"<svg viewBox="0 0 4 4"><path d="M1 1 H3 V3 H1 Z"/></svg>"#, 4);
        let inside = |i: usize| (1..3).contains(&(i % 4)) && (1..3).contains(&(i / 4));
        assert!(square.iter().enumerate().all(|(i, &a)| a == if inside(i) { 255 } else { 0 }), "{square:?}");
        // half a pixel each way is half covered, and exponents are read
        let half = drawn(r#"<svg viewBox="0 0 2 2"><path d="M0.5 0 L 1.5e0 0 1.5 2 0.5 2Z"/></svg>"#, 2);
        assert_eq!(half, [127; 4]);
    }

    /// a ring of two circles drawn the same way round is filled through its middle by nonzero winding; drawn the
    /// other way round, its middle is a hole. either way the band between them is filled.
    #[test]
    fn a_ring_fills_by_its_winding() {
        let outer = "M9 5 C9 7.209 7.209 9 5 9 C2.791 9 1 7.209 1 5 C1 2.791 2.791 1 5 1 C7.209 1 9 2.791 9 5 Z";
        let same = "M7 5 C7 6.105 6.105 7 5 7 C3.895 7 3 6.105 3 5 C3 3.895 3.895 3 5 3 C6.105 3 7 3.895 7 5 Z";
        let other = "M7 5 C7 3.895 6.105 3 5 3 C3.895 3 3 3.895 3 5 C3 6.105 3.895 7 5 7 C6.105 7 7 6.105 7 5 Z";
        for (inner, middle) in [(same, 255), (other, 0)] {
            let ring = drawn(&format!(r#"<svg viewBox="0 0 10 10"><path d="{outer} {inner}"/></svg>"#), 10);
            // the pixels round the middle, the band at the left, and a corner outside both
            assert_eq!((ring[4 * 10 + 4], ring[4 * 10 + 1], ring[0]), (middle, 255, 0), "{inner}");
        }
    }

    /// the brand's files read as drawn: the star has its stroke and covers its middle and not its corners, and the
    /// wordmark has its seven letters.
    #[test]
    fn the_brand_kits_files_read() {
        assert_eq!((STAR.outlines.len(), STAR.stroke, WORDMARK.outlines.len(), WORDMARK.stroke), (1, 4.0, 7, 0.0));
        let star = STAR.raster(Rect::from_center_size(MIDDLE, Vec2::splat(TURNING)), [40, 40]);
        assert_eq!((star[20 * 40 + 20], star[0], star[39 * 40 + 39]), (255, 0, 0));
        let icon = icon([4, 4]);
        assert!(icon.pixels.iter().any(|p| p.a() == 255), "the icon read as nothing");
    }

    /// the strip's mark is the star of tridant.io's favicon, without its disc (the owner's choice, 2026-11-25, of the
    /// mockup's "d"): a stroke with no fill, so its middle is open; its long ray runs out to where the favicon's disc
    /// would end, and nothing is drawn past that edge, as the disc clips it.
    #[test]
    fn the_strips_mark_is_the_favicons_star() {
        assert_eq!((MARK.stroke, MARK.fill, MARK.clip), (11.34, false, Some((Pos2::new(50.0, 50.0), 47.8))));
        let mark = MARK.raster(MARK.view, [64, 64]);
        let at = |x: usize, y: usize| mark[y * 64 + x];
        // the open middle, the long ray to the right at the disc's edge, and the corners past the disc
        assert_eq!((at(32, 32), at(61, 32), at(0, 0), at(63, 63)), (0, 255, 0, 0));
        // nothing past the disc's edge: 47.8 of 100 is 30.6 of 64, from the middle
        let past = (0..64 * 64).filter(|&i| ((i % 64) as f32 + 0.5 - 32.0).hypot((i / 64) as f32 + 0.5 - 32.0) > 31.5).map(|i| mark[i]).max();
        assert_eq!(past, Some(0), "drawn past the disc's edge");
    }

    /// the card's star boots from −150° at 55% to upright at full size, holds, then throws a third of a turn and
    /// stops dead on it.
    #[test]
    fn the_cards_star_boots_then_holds_and_throws() {
        let (turn, scale, still) = motion(0.0, 7);
        assert_eq!((turn, scale, still), (-150.0, 0.55, None));
        let (turn, scale, still) = motion(0.7, 7);
        assert!((turn, scale) == (0.0, 1.0) && still.is_some_and(|s| (1.6..=4.4).contains(&s)), "{turn} {scale} {still:?}");
        let rest = still.unwrap();
        assert!(motion(0.7 + rest + 0.2, 7).2.is_none(), "it moves after its hold");
        assert_eq!(motion(0.7 + rest + 0.45, 7).0, 120.0);
    }
}
