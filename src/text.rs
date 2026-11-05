//! Minimal text rasterizer (ab_glyph) drawing straight into a Pixmap.

use ab_glyph::{point, Font, FontVec, PxScale, ScaleFont};
use tiny_skia::Pixmap;

pub struct TextRenderer {
    font: FontVec,
}

/// Layout result. `caret` = (x, top-y) after the last char.
pub struct Metrics {
    pub w: f32,
    pub h: f32,
    pub line_h: f32,
    pub caret: (f32, f32),
}

impl TextRenderer {
    pub fn load() -> Option<Self> {
        const FONTS: &[&str] = &[
            r"C:\Windows\Fonts\segoeui.ttf",
            r"C:\Windows\Fonts\arial.ttf",
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ];
        let bytes = FONTS.iter().find_map(|p| std::fs::read(p).ok())?;
        FontVec::try_from_vec(bytes).ok().map(|font| Self { font })
    }

    pub fn measure(&self, text: &str, size: f32) -> Metrics {
        self.draw(None, text, 0.0, 0.0, size, [0; 4])
    }

    /// Draws multi-line `text` with top-left at (x, y). Pass `None` to only measure.
    pub fn draw(&self, mut pm: Option<&mut Pixmap>, text: &str, x: f32, y: f32, size: f32, color: [u8; 4]) -> Metrics {
        let sf = self.font.as_scaled(PxScale::from(size));
        let line_h = sf.height() + sf.line_gap();
        let (mut w, mut cx, mut top) = (0f32, x, y);

        for (i, line) in text.split('\n').enumerate() {
            if i > 0 {
                top += line_h;
            }
            cx = x;
            let mut prev = None;
            for ch in line.chars() {
                let id = self.font.glyph_id(ch);
                if let Some(p) = prev {
                    cx += sf.kern(p, id);
                }
                let glyph = id.with_scale_and_position(size, point(cx, top + sf.ascent()));
                if let (Some(pm), Some(og)) = (pm.as_deref_mut(), self.font.outline_glyph(glyph)) {
                    let bb = og.px_bounds();
                    og.draw(|gx, gy, cov| {
                        blend(pm, bb.min.x as i32 + gx as i32, bb.min.y as i32 + gy as i32, color, cov)
                    });
                }
                cx += sf.h_advance(id);
                prev = Some(id);
            }
            w = w.max(cx - x);
        }
        Metrics { w, h: top - y + line_h, line_h, caret: (cx, top) }
    }
}

/// Alpha-blend one pixel of straight `color` with coverage into a premultiplied pixmap.
pub fn blend(pm: &mut Pixmap, x: i32, y: i32, color: [u8; 4], cov: f32) {
    let (w, h) = (pm.width() as i32, pm.height() as i32);
    if x < 0 || y < 0 || x >= w || y >= h {
        return;
    }
    let a = cov.clamp(0.0, 1.0) * color[3] as f32 / 255.0;
    let i = (y * w + x) as usize * 4;
    let d = &mut pm.data_mut()[i..i + 4];
    for c in 0..3 {
        d[c] = (color[c] as f32 * a + d[c] as f32 * (1.0 - a)).round() as u8;
    }
    d[3] = (255.0 * a + d[3] as f32 * (1.0 - a)).round() as u8;
}
