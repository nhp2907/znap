//! Toolbar layout, hit-testing and icon drawing.

use tiny_skia::{FillRule, PathBuilder, Pixmap, Point, Rect, Transform};

use crate::shapes::{paint, rounded_rect, stroke, Rgba, Tool, TOOLS};

pub const COLORS: [Rgba; 6] = [
    [239, 68, 68, 255],   // red
    [250, 204, 21, 255],  // yellow
    [34, 197, 94, 255],   // green
    [59, 130, 246, 255],  // blue
    [255, 255, 255, 255], // white
    [17, 17, 17, 255],    // black
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Btn {
    Tool(Tool),
    Color(usize),
    Undo,
    Save,
    Copy,
    Cancel,
    Done,
}

pub struct Toolbar {
    pub bg: Rect,
    pub items: Vec<(Btn, Rect)>,
    seps: Vec<f32>,
}

impl Toolbar {
    /// Place the bar under the selection (l, t, r, b), flipping above / inside if no room.
    pub fn layout(sel: (f32, f32, f32, f32), screen: (f32, f32), s: f32) -> Self {
        let (bsz, csz, gap, sep, pad) = (32.0 * s, 22.0 * s, 2.0 * s, 10.0 * s, 6.0 * s);

        let mut slots: Vec<Option<(Btn, f32)>> = TOOLS.iter().map(|t| Some((Btn::Tool(*t), bsz))).collect();
        slots.push(None);
        slots.extend((0..COLORS.len()).map(|i| Some((Btn::Color(i), csz))));
        slots.push(None);
        slots.push(Some((Btn::Undo, bsz)));
        slots.push(None);
        slots.extend([Btn::Save, Btn::Copy, Btn::Cancel, Btn::Done].map(|b| Some((b, bsz))));

        let (mut x, mut rel, mut seps) = (pad, Vec::new(), Vec::new());
        for slot in slots {
            match slot {
                Some((btn, w)) => {
                    rel.push((btn, x, w));
                    x += w + gap;
                }
                None => {
                    seps.push(x + sep / 2.0 - gap / 2.0);
                    x += sep;
                }
            }
        }
        let (w, h) = (x - gap + pad, bsz + 2.0 * pad);
        let (_, t, r, b) = sel;
        let bx = (r - w).min(screen.0 - w).max(0.0);
        let mut by = b + 6.0 * s;
        if by + h > screen.1 {
            by = t - 6.0 * s - h;
        }
        if by < 0.0 {
            by = (b - h - 6.0 * s).max(0.0);
        }
        Self {
            bg: Rect::from_xywh(bx, by, w, h).unwrap(),
            items: rel.into_iter().map(|(btn, rx, w)| (btn, Rect::from_xywh(bx + rx, by + pad, w, bsz).unwrap())).collect(),
            seps: seps.into_iter().map(|sx| bx + sx).collect(),
        }
    }

    pub fn contains(&self, p: Point) -> bool {
        in_rect(&self.bg, p)
    }

    pub fn hit(&self, p: Point) -> Option<Btn> {
        self.items.iter().find(|(_, r)| in_rect(r, p)).map(|(b, _)| *b)
    }

    pub fn draw(&self, pm: &mut Pixmap, s: f32, hover: Option<Btn>, tool: Option<Tool>, color: usize) {
        let id = Transform::identity();
        if let Some(p) = rounded_rect(self.bg, 8.0 * s) {
            pm.fill_path(&p, &paint([32, 33, 36, 240]), FillRule::Winding, id, None);
        }
        for &x in &self.seps {
            let mut pb = PathBuilder::new();
            pb.move_to(x, self.bg.top() + 12.0 * s);
            pb.line_to(x, self.bg.bottom() - 12.0 * s);
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &paint([255, 255, 255, 60]), &stroke(1.0 * s), id, None);
            }
        }
        for &(btn, r) in &self.items {
            let active = matches!(btn, Btn::Tool(t) if Some(t) == tool);
            let bg = if active {
                Some([59, 130, 246, 220])
            } else if hover == Some(btn) {
                Some([255, 255, 255, 36])
            } else {
                None
            };
            if let (Some(c), Some(p)) = (bg, rounded_rect(r, 6.0 * s)) {
                pm.fill_path(&p, &paint(c), FillRule::Winding, id, None);
            }
            let c = Point::from_xy(r.left() + r.width() / 2.0, r.top() + r.height() / 2.0);
            draw_icon(pm, btn, c, s, color);
        }
    }
}

fn in_rect(r: &Rect, p: Point) -> bool {
    p.x >= r.left() && p.x <= r.right() && p.y >= r.top() && p.y <= r.bottom()
}

/// Icons are authored in a -10..10 unit box around `c`.
fn draw_icon(pm: &mut Pixmap, btn: Btn, c: Point, s: f32, selected_color: usize) {
    let id = Transform::identity();
    let u = s;
    let at = |x: f32, y: f32| (c.x + x * u, c.y + y * u);
    let white = [235, 235, 235, 255];

    // Polyline strokes: each inner Vec is one line strip.
    let strips = |pm: &mut Pixmap, lines: &[&[(f32, f32)]], color: Rgba, width: f32| {
        let mut pb = PathBuilder::new();
        for line in lines {
            let (x, y) = at(line[0].0, line[0].1);
            pb.move_to(x, y);
            for &(lx, ly) in &line[1..] {
                let (x, y) = at(lx, ly);
                pb.line_to(x, y);
            }
        }
        if let Some(p) = pb.finish() {
            pm.stroke_path(&p, &paint(color), &stroke(width * u), id, None);
        }
    };

    match btn {
        Btn::Tool(Tool::Rect) => strips(pm, &[&[(-8., -6.), (8., -6.), (8., 6.), (-8., 6.), (-8., -6.)]], white, 1.8),
        Btn::Tool(Tool::Ellipse) => {
            let (x, y) = at(-9., -7.);
            if let Some(p) = Rect::from_xywh(x, y, 18. * u, 14. * u).and_then(PathBuilder::from_oval) {
                pm.stroke_path(&p, &paint(white), &stroke(1.8 * u), id, None);
            }
        }
        Btn::Tool(Tool::Arrow) => strips(pm, &[&[(-7., 7.), (7., -7.)], &[(0., -7.), (7., -7.), (7., 0.)]], white, 1.8),
        Btn::Tool(Tool::Pen) => {
            let mut pb = PathBuilder::new();
            let (x0, y0) = at(-9., 4.);
            let (x1, y1) = at(-4., -10.);
            let (x2, y2) = at(0., 0.);
            let (x3, y3) = at(4., 10.);
            let (x4, y4) = at(9., -4.);
            pb.move_to(x0, y0);
            pb.quad_to(x1, y1, x2, y2);
            pb.quad_to(x3, y3, x4, y4);
            if let Some(p) = pb.finish() {
                pm.stroke_path(&p, &paint(white), &stroke(1.8 * u), id, None);
            }
        }
        Btn::Tool(Tool::Text) => strips(pm, &[&[(-7., -7.), (7., -7.)], &[(0., -7.), (0., 8.)]], white, 2.2),
        Btn::Tool(Tool::Mosaic) => {
            for gy in 0..3 {
                for gx in 0..3 {
                    let (x, y) = at(-9. + gx as f32 * 6., -9. + gy as f32 * 6.);
                    let alpha = if (gx + gy) % 2 == 0 { 255 } else { 110 };
                    if let Some(r) = Rect::from_xywh(x, y, 5. * u, 5. * u) {
                        pm.fill_rect(r, &paint([235, 235, 235, alpha]), id, None);
                    }
                }
            }
        }
        Btn::Color(i) => {
            if let Some(p) = PathBuilder::from_circle(c.x, c.y, 6.5 * u) {
                pm.fill_path(&p, &paint(COLORS[i]), FillRule::Winding, id, None);
                if i == COLORS.len() - 1 {
                    pm.stroke_path(&p, &paint([120, 120, 120, 255]), &stroke(1.0 * u), id, None);
                }
            }
            if i == selected_color {
                if let Some(p) = PathBuilder::from_circle(c.x, c.y, 9.5 * u) {
                    pm.stroke_path(&p, &paint(white), &stroke(1.6 * u), id, None);
                }
            }
        }
        Btn::Undo => strips(
            pm,
            &[&[(-3., -8.), (-8., -3.), (-3., 2.)], &[(-8., -3.), (3., -3.), (8., 1.), (8., 4.), (4., 8.), (-2., 8.)]],
            white,
            1.8,
        ),
        Btn::Save => strips(pm, &[&[(0., -9.), (0., 4.)], &[(-5., -1.), (0., 4.), (5., -1.)], &[(-8., 9.), (8., 9.)]], white, 1.8),
        Btn::Copy => strips(
            pm,
            &[&[(-4., -4.), (8., -4.), (8., 9.), (-4., 9.), (-4., -4.)], &[(-8., 5.), (-8., -8.), (4., -8.)]],
            white,
            1.6,
        ),
        Btn::Cancel => strips(pm, &[&[(-7., -7.), (7., 7.)], &[(7., -7.), (-7., 7.)]], [248, 113, 113, 255], 2.2),
        Btn::Done => strips(pm, &[&[(-8., 0.), (-3., 6.), (8., -7.)]], [74, 222, 128, 255], 2.6),
    }
}
