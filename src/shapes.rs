//! Annotation shapes + drawing helpers.

use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Point, Rect, Stroke, Transform};

use crate::text::TextRenderer;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Rect,
    Ellipse,
    Arrow,
    Pen,
    Text,
    Mosaic,
}

pub const TOOLS: [Tool; 6] = [Tool::Rect, Tool::Ellipse, Tool::Arrow, Tool::Pen, Tool::Text, Tool::Mosaic];

pub type Rgba = [u8; 4];

#[derive(Clone, Debug)]
pub enum Shape {
    Rect { a: Point, b: Point, color: Rgba, width: f32 },
    Ellipse { a: Point, b: Point, color: Rgba, width: f32 },
    Arrow { a: Point, b: Point, color: Rgba, width: f32 },
    Pen { pts: Vec<Point>, color: Rgba, width: f32 },
    Text { pos: Point, text: String, color: Rgba, size: f32 },
    Mosaic { a: Point, b: Point, block: u32 },
}

impl Shape {
    /// Update the "drag end" of a shape being drawn.
    pub fn drag_to(&mut self, p: Point) {
        match self {
            Shape::Rect { b, .. } | Shape::Ellipse { b, .. } | Shape::Arrow { b, .. } | Shape::Mosaic { b, .. } => *b = p,
            Shape::Pen { pts, .. } => {
                if pts.last().map_or(true, |l| dist(*l, p) > 1.0) {
                    pts.push(p);
                }
            }
            Shape::Text { .. } => {}
        }
    }

    /// False for accidental clicks that produce nothing visible.
    pub fn is_visible(&self) -> bool {
        match self {
            Shape::Rect { a, b, .. } | Shape::Ellipse { a, b, .. } | Shape::Mosaic { a, b, .. } => {
                (a.x - b.x).abs() >= 2.0 && (a.y - b.y).abs() >= 2.0
            }
            Shape::Arrow { a, b, .. } => dist(*a, *b) >= 4.0,
            Shape::Pen { pts, .. } => !pts.is_empty(),
            Shape::Text { text, .. } => !text.trim().is_empty(),
        }
    }

    pub fn draw(&self, pm: &mut Pixmap, text: Option<&TextRenderer>) {
        let id = Transform::identity();
        match self {
            Shape::Rect { a, b, color, width } => {
                if let Some(r) = rect_of(*a, *b) {
                    pm.stroke_path(&PathBuilder::from_rect(r), &paint(*color), &stroke(*width), id, None);
                }
            }
            Shape::Ellipse { a, b, color, width } => {
                if let Some(p) = rect_of(*a, *b).and_then(PathBuilder::from_oval) {
                    pm.stroke_path(&p, &paint(*color), &stroke(*width), id, None);
                }
            }
            Shape::Arrow { a, b, color, width } => draw_arrow(pm, *a, *b, *color, *width),
            Shape::Pen { pts, color, width } => {
                if pts.len() == 1 {
                    if let Some(c) = PathBuilder::from_circle(pts[0].x, pts[0].y, width / 2.0) {
                        pm.fill_path(&c, &paint(*color), FillRule::Winding, id, None);
                    }
                    return;
                }
                let mut pb = PathBuilder::new();
                pb.move_to(pts[0].x, pts[0].y);
                for p in &pts[1..] {
                    pb.line_to(p.x, p.y);
                }
                if let Some(p) = pb.finish() {
                    pm.stroke_path(&p, &paint(*color), &stroke(*width), id, None);
                }
            }
            Shape::Text { pos, text: s, color, size } => {
                if let Some(t) = text {
                    t.draw(Some(pm), s, pos.x, pos.y, *size, *color);
                }
            }
            Shape::Mosaic { a, b, block } => {
                pixelate(pm, a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y), *block as i32)
            }
        }
    }
}

fn draw_arrow(pm: &mut Pixmap, a: Point, b: Point, color: Rgba, width: f32) {
    let len = dist(a, b);
    if len < 1.0 {
        return;
    }
    let (ux, uy) = ((b.x - a.x) / len, (b.y - a.y) / len);
    let head = (width * 4.0).max(12.0).min(len);
    let (bx, by) = (b.x - ux * head, b.y - uy * head);
    let (nx, ny) = (-uy * head * 0.5, ux * head * 0.5);

    let mut pb = PathBuilder::new();
    pb.move_to(a.x, a.y);
    pb.line_to(bx + ux, by + uy);
    if let Some(p) = pb.finish() {
        pm.stroke_path(&p, &paint(color), &stroke(width), Transform::identity(), None);
    }
    let mut pb = PathBuilder::new();
    pb.move_to(b.x, b.y);
    pb.line_to(bx + nx, by + ny);
    pb.line_to(bx - nx, by - ny);
    pb.close();
    if let Some(p) = pb.finish() {
        pm.fill_path(&p, &paint(color), FillRule::Winding, Transform::identity(), None);
    }
}

/// Replace area with block-averaged colors.
pub fn pixelate(pm: &mut Pixmap, l: f32, t: f32, r: f32, b: f32, block: i32) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    let (l, t) = ((l as i32).clamp(0, w), (t as i32).clamp(0, h));
    let (r, b) = ((r as i32).clamp(0, w), (b as i32).clamp(0, h));
    let block = block.max(2);
    let data = pm.data_mut();
    let mut by = t;
    while by < b {
        let by1 = (by + block).min(b);
        let mut bx = l;
        while bx < r {
            let bx1 = (bx + block).min(r);
            let mut sum = [0u32; 4];
            for y in by..by1 {
                for x in bx..bx1 {
                    let i = (y * w + x) as usize * 4;
                    for c in 0..4 {
                        sum[c] += data[i + c] as u32;
                    }
                }
            }
            let n = ((by1 - by) * (bx1 - bx)) as u32;
            let avg = sum.map(|s| (s / n) as u8);
            for y in by..by1 {
                for x in bx..bx1 {
                    let i = (y * w + x) as usize * 4;
                    data[i..i + 4].copy_from_slice(&avg);
                }
            }
            bx = bx1;
        }
        by = by1;
    }
}

pub fn rect_of(a: Point, b: Point) -> Option<Rect> {
    Rect::from_ltrb(a.x.min(b.x), a.y.min(b.y), a.x.max(b.x), a.y.max(b.y))
}

pub fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c[0], c[1], c[2], c[3]);
    p.anti_alias = true;
    p
}

pub fn stroke(width: f32) -> Stroke {
    Stroke { width, line_cap: LineCap::Round, line_join: LineJoin::Round, ..Stroke::default() }
}

pub fn rounded_rect(r: Rect, rad: f32) -> Option<Path> {
    let (l, t, rr, b) = (r.left(), r.top(), r.right(), r.bottom());
    let k = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    let mut pb = PathBuilder::new();
    pb.move_to(l + k, t);
    pb.line_to(rr - k, t);
    pb.quad_to(rr, t, rr, t + k);
    pb.line_to(rr, b - k);
    pb.quad_to(rr, b, rr - k, b);
    pb.line_to(l + k, b);
    pb.quad_to(l, b, l, b - k);
    pb.line_to(l, t + k);
    pb.quad_to(l, t, l + k, t);
    pb.close();
    pb.finish()
}

pub fn dist(a: Point, b: Point) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}
