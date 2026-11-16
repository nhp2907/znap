//! Fullscreen overlay: region select/resize, annotate, export.

use std::{error::Error, num::NonZeroU32, rc::Rc};

use softbuffer::{Context, Surface};
use tiny_skia::{FillRule, Pixmap, Point, Rect, StrokeDash, Transform};
use winit::{
    event::{ElementState, Ime, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::ActiveEventLoop,
    keyboard::{Key, KeyCode, ModifiersState, NamedKey, PhysicalKey},
    monitor::MonitorHandle,
    window::{CursorIcon, Fullscreen, Window, WindowLevel},
};

use crate::{
    output::Image,
    shapes::{paint, rounded_rect, stroke, Shape, Tool},
    text::TextRenderer,
    toolbar::{Btn, Toolbar, COLORS},
};

const ACCENT: [u8; 4] = [30, 144, 255, 255];

/// What the app should do after an event.
pub enum Action {
    None,
    Close,
    Copy,
    Save,
}

/// Selection rect, always normalized (l <= r, t <= b).
#[derive(Clone, Copy)]
struct Sel {
    l: f32,
    t: f32,
    r: f32,
    b: f32,
}

impl Sel {
    fn from_pts(a: Point, b: Point) -> Self {
        Self { l: a.x.min(b.x), t: a.y.min(b.y), r: a.x.max(b.x), b: a.y.max(b.y) }
    }
    fn contains(&self, p: Point) -> bool {
        p.x >= self.l && p.x <= self.r && p.y >= self.t && p.y <= self.b
    }
    fn clamp(&self, p: Point) -> Point {
        Point::from_xy(p.x.clamp(self.l, self.r), p.y.clamp(self.t, self.b))
    }
}

#[derive(Clone, Copy, Default)]
struct Edges {
    l: bool,
    t: bool,
    r: bool,
    b: bool,
}

impl Edges {
    fn any(&self) -> bool {
        self.l || self.t || self.r || self.b
    }
    fn cursor(&self) -> CursorIcon {
        match (self.l, self.t, self.r, self.b) {
            (true, true, _, _) | (_, _, true, true) => CursorIcon::NwseResize,
            (_, true, true, _) | (true, _, _, true) => CursorIcon::NeswResize,
            (true, _, _, _) | (_, _, true, _) => CursorIcon::EwResize,
            _ => CursorIcon::NsResize,
        }
    }
}

enum Drag {
    None,
    NewSel(Point),
    Move { start: Point, orig: Sel },
    Resize { start: Point, orig: Sel, edges: Edges },
    Draw,
}

struct TextEdit {
    pos: Point,
    text: String,
    color: [u8; 4],
    size: f32,
}

pub struct Overlay {
    pub window: Rc<Window>,
    surface: Surface<Rc<Window>, Rc<Window>>,
    _context: Context<Rc<Window>>,
    surface_size: (u32, u32),
    text: Option<Rc<TextRenderer>>,
    scale: f32,

    base: Pixmap,   // raw capture
    dim: Pixmap,    // darkened capture (outside selection)
    canvas: Pixmap, // capture + committed shapes
    canvas_dirty: bool,
    frame: Pixmap, // what is presented

    sel: Option<Sel>,
    drag: Drag,
    tool: Option<Tool>,
    color: usize,
    width: f32,
    font_size: f32,
    shapes: Vec<Shape>,
    current: Option<Shape>,
    editing: Option<TextEdit>,

    cursor: Point,
    cursor_icon: CursorIcon,
    mods: ModifiersState,
    hover: Option<Btn>,
}

impl Overlay {
    pub fn new(
        el: &ActiveEventLoop,
        monitor: MonitorHandle,
        base: Pixmap,
        text: Option<Rc<TextRenderer>>,
    ) -> Result<Self, Box<dyn Error>> {
        let attrs = Window::default_attributes()
            .with_title("znap")
            .with_decorations(false)
            .with_resizable(false)
            .with_visible(false)
            .with_window_level(WindowLevel::AlwaysOnTop)
            .with_position(monitor.position())
            .with_inner_size(monitor.size())
            .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        #[cfg(windows)]
        let attrs = {
            use winit::platform::windows::WindowAttributesExtWindows;
            attrs.with_skip_taskbar(true)
        };
        let window = Rc::new(el.create_window(attrs)?);
        let context = Context::new(window.clone())?;
        let surface = Surface::new(&context, window.clone())?;

        let mut dim = base.clone();
        for px in dim.data_mut().chunks_exact_mut(4) {
            for c in &mut px[..3] {
                *c = (*c as u16 * 110 / 255) as u8;
            }
        }
        let mut ov = Self {
            window,
            surface,
            _context: context,
            surface_size: (0, 0),
            text,
            scale: monitor.scale_factor() as f32,
            canvas: base.clone(),
            frame: base.clone(),
            dim,
            base,
            canvas_dirty: false,
            sel: None,
            drag: Drag::None,
            tool: None,
            color: 0,
            width: 3.0,
            font_size: 20.0 * monitor.scale_factor() as f32,
            shapes: Vec::new(),
            current: None,
            editing: None,
            cursor: Point::from_xy(0.0, 0.0),
            cursor_icon: CursorIcon::Default,
            mods: ModifiersState::empty(),
            hover: None,
        };
        // Paint before showing to avoid a blank flash.
        ov.render();
        ov.window.set_visible(true);
        ov.window.focus_window();
        ov.set_cursor(CursorIcon::Crosshair);
        ov.window.request_redraw();
        Ok(ov)
    }

    // ---------- events ----------

    pub fn handle(&mut self, ev: &WindowEvent) -> Action {
        let action = match ev {
            WindowEvent::RedrawRequested => {
                self.render();
                return Action::None;
            }
            WindowEvent::CloseRequested => return Action::Close,
            WindowEvent::ModifiersChanged(m) => {
                self.mods = m.state();
                return Action::None;
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.on_move(Point::from_xy(position.x as f32, position.y as f32));
                Action::None
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => self.on_left_down(),
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                self.on_left_up();
                Action::None
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } => self.on_right_down(),
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 / 40.0,
                };
                if dy != 0.0 {
                    self.on_wheel(dy.signum());
                }
                Action::None
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => self.on_key(event),
            WindowEvent::Ime(Ime::Commit(s)) => {
                if let Some(ed) = &mut self.editing {
                    ed.text.push_str(s);
                }
                Action::None
            }
            _ => return Action::None,
        };
        self.window.request_redraw();
        action
    }

    fn on_move(&mut self, p: Point) {
        self.cursor = p;
        match self.drag {
            Drag::NewSel(start) => self.sel = Some(Sel::from_pts(start, self.clamp_screen(p))),
            Drag::Move { start, orig } => {
                let (w, h) = (orig.r - orig.l, orig.b - orig.t);
                let (sw, sh) = self.screen();
                let l = (orig.l + p.x - start.x).clamp(0.0, (sw - w).max(0.0));
                let t = (orig.t + p.y - start.y).clamp(0.0, (sh - h).max(0.0));
                self.sel = Some(Sel { l, t, r: l + w, b: t + h });
            }
            Drag::Resize { start, orig, edges } => {
                let q = self.clamp_screen(p);
                let (dx, dy) = (q.x - start.x, q.y - start.y);
                let mut s = orig;
                if edges.l {
                    s.l += dx;
                }
                if edges.r {
                    s.r += dx;
                }
                if edges.t {
                    s.t += dy;
                }
                if edges.b {
                    s.b += dy;
                }
                self.sel = Some(Sel::from_pts(Point::from_xy(s.l, s.t), Point::from_xy(s.r, s.b)));
            }
            Drag::Draw => {
                if let (Some(sel), Some(shape)) = (self.sel, self.current.as_mut()) {
                    shape.drag_to(sel.clamp(p));
                }
            }
            Drag::None => {
                self.hover = self.toolbar().and_then(|tb| tb.hit(p));
                let icon = self.cursor_for(p);
                self.set_cursor(icon);
            }
        }
    }

    fn on_left_down(&mut self) -> Action {
        let p = self.cursor;
        if let Some(tb) = self.toolbar() {
            if let Some(btn) = tb.hit(p) {
                return self.press(btn);
            }
            if tb.contains(p) {
                return Action::None;
            }
        }
        self.commit_text();

        let Some(sel) = self.sel else {
            self.drag = Drag::NewSel(p);
            self.sel = Some(Sel::from_pts(p, p));
            return Action::None;
        };
        let edges = self.hit_edges(sel, p);
        if edges.any() {
            self.drag = Drag::Resize { start: p, orig: sel, edges };
        } else if sel.contains(p) {
            match self.tool {
                None => self.drag = Drag::Move { start: p, orig: sel },
                Some(Tool::Text) => self.start_text(p),
                Some(t) => {
                    self.current = Some(self.new_shape(t, p));
                    self.drag = Drag::Draw;
                }
            }
        } else if self.shapes.is_empty() {
            // Start over with a new region.
            self.tool = None;
            self.drag = Drag::NewSel(p);
            self.sel = Some(Sel::from_pts(p, p));
        }
        Action::None
    }

    fn on_left_up(&mut self) {
        match std::mem::replace(&mut self.drag, Drag::None) {
            Drag::NewSel(_) => {
                // Plain click (no drag) selects the whole screen.
                if let Some(s) = self.sel {
                    if s.r - s.l < 3.0 && s.b - s.t < 3.0 {
                        let (w, h) = self.screen();
                        self.sel = Some(Sel { l: 0.0, t: 0.0, r: w, b: h });
                    }
                }
                // Region chosen: default to Rect tool.
                self.tool = Some(Tool::Rect);
            }
            Drag::Draw => {
                if let Some(s) = self.current.take().filter(Shape::is_visible) {
                    self.shapes.push(s);
                    self.canvas_dirty = true;
                }
            }
            _ => {}
        }
        let icon = self.cursor_for(self.cursor);
        self.set_cursor(icon);
    }

    /// Right click: drop an empty selection, otherwise close.
    fn on_right_down(&mut self) -> Action {
        if !matches!(self.drag, Drag::None) {
            return Action::None;
        }
        if self.editing.is_some() {
            self.commit_text();
            return Action::None;
        }
        if self.sel.is_some() && self.shapes.is_empty() {
            self.sel = None;
            self.tool = None;
            return Action::None;
        }
        Action::Close
    }

    /// Wheel changes stroke width / font size.
    fn on_wheel(&mut self, step: f32) {
        if let Some(ed) = &mut self.editing {
            ed.size = (ed.size + step * 2.0).clamp(8.0, 160.0);
        } else if self.tool == Some(Tool::Text) {
            self.font_size = (self.font_size + step * 2.0).clamp(8.0, 160.0);
        } else {
            self.width = (self.width + step).clamp(1.0, 30.0);
        }
    }

    fn on_key(&mut self, ev: &KeyEvent) -> Action {
        let code = match ev.physical_key {
            PhysicalKey::Code(c) => Some(c),
            _ => None,
        };
        if self.mods.control_key() {
            return match code {
                Some(KeyCode::KeyC) => Action::Copy,
                Some(KeyCode::KeyS) => Action::Save,
                Some(KeyCode::KeyZ) => {
                    self.undo();
                    Action::None
                }
                Some(KeyCode::Enter | KeyCode::NumpadEnter) => {
                    self.commit_text();
                    Action::None
                }
                _ => Action::None,
            };
        }

        if self.editing.is_some() {
            if ev.logical_key == Key::Named(NamedKey::Escape) {
                self.commit_text();
            } else if let Some(ed) = self.editing.as_mut() {
                match &ev.logical_key {
                    Key::Named(NamedKey::Backspace) => {
                        ed.text.pop();
                    }
                    Key::Named(NamedKey::Enter) => ed.text.push('\n'),
                    _ => {
                        if let Some(t) = &ev.text {
                            ed.text.extend(t.chars().filter(|c| !c.is_control()));
                        }
                    }
                }
            }
            return Action::None;
        }

        match &ev.logical_key {
            Key::Named(NamedKey::Escape) => Action::Close,
            Key::Named(NamedKey::Enter) if self.sel.is_some() => Action::Copy,
            _ => Action::None,
        }
    }

    fn press(&mut self, btn: Btn) -> Action {
        match btn {
            Btn::Tool(t) => {
                self.commit_text();
                self.tool = if self.tool == Some(t) { None } else { Some(t) };
            }
            Btn::Color(i) => {
                self.color = i;
                if let Some(ed) = &mut self.editing {
                    ed.color = COLORS[i];
                }
            }
            Btn::Undo => self.undo(),
            Btn::Save => return Action::Save,
            Btn::Copy | Btn::Done => return Action::Copy,
            Btn::Cancel => return Action::Close,
        }
        Action::None
    }

    // ---------- editing helpers ----------

    fn new_shape(&self, t: Tool, p: Point) -> Shape {
        let (color, width) = (COLORS[self.color], self.width * self.scale);
        match t {
            Tool::Rect => Shape::Rect { a: p, b: p, color, width },
            Tool::Ellipse => Shape::Ellipse { a: p, b: p, color, width },
            Tool::Arrow => Shape::Arrow { a: p, b: p, color, width },
            Tool::Pen | Tool::Text => Shape::Pen { pts: vec![p], color, width },
            Tool::Mosaic => Shape::Mosaic { a: p, b: p, block: (4.0 + self.width * 2.0 * self.scale) as u32 },
        }
    }

    fn start_text(&mut self, p: Point) {
        let line_h = self.text.as_ref().map_or(self.font_size, |t| t.measure("", self.font_size).line_h);
        self.editing = Some(TextEdit {
            pos: Point::from_xy(p.x, p.y - line_h / 2.0),
            text: String::new(),
            color: COLORS[self.color],
            size: self.font_size,
        });
        self.window.set_ime_allowed(true);
    }

    fn commit_text(&mut self) {
        if let Some(ed) = self.editing.take() {
            self.window.set_ime_allowed(false);
            let shape = Shape::Text { pos: ed.pos, text: ed.text, color: ed.color, size: ed.size };
            if shape.is_visible() {
                self.shapes.push(shape);
                self.canvas_dirty = true;
            }
        }
    }

    fn undo(&mut self) {
        if self.editing.take().is_some() {
            self.window.set_ime_allowed(false);
        } else if self.shapes.pop().is_some() {
            self.canvas_dirty = true;
        }
    }

    // ---------- geometry ----------

    fn screen(&self) -> (f32, f32) {
        (self.base.width() as f32, self.base.height() as f32)
    }

    fn clamp_screen(&self, p: Point) -> Point {
        let (w, h) = self.screen();
        Point::from_xy(p.x.clamp(0.0, w), p.y.clamp(0.0, h))
    }

    fn toolbar(&self) -> Option<Toolbar> {
        let s = self.sel?;
        if matches!(self.drag, Drag::NewSel(_)) {
            return None;
        }
        Some(Toolbar::layout((s.l, s.t, s.r, s.b), self.screen(), self.scale))
    }

    fn hit_edges(&self, s: Sel, p: Point) -> Edges {
        let m = 6.0 * self.scale;
        let in_y = p.y >= s.t - m && p.y <= s.b + m;
        let in_x = p.x >= s.l - m && p.x <= s.r + m;
        let l = in_y && (p.x - s.l).abs() <= m;
        let t = in_x && (p.y - s.t).abs() <= m;
        Edges { l, t, r: !l && in_y && (p.x - s.r).abs() <= m, b: !t && in_x && (p.y - s.b).abs() <= m }
    }

    fn cursor_for(&self, p: Point) -> CursorIcon {
        if self.toolbar().is_some_and(|tb| tb.contains(p)) {
            return CursorIcon::Pointer;
        }
        let Some(sel) = self.sel else { return CursorIcon::Crosshair };
        let edges = self.hit_edges(sel, p);
        if edges.any() {
            return edges.cursor();
        }
        match (sel.contains(p), self.tool) {
            (true, None) => CursorIcon::Move,
            (true, Some(Tool::Text)) => CursorIcon::Text,
            _ => CursorIcon::Crosshair,
        }
    }

    fn set_cursor(&mut self, icon: CursorIcon) {
        if self.cursor_icon != icon {
            self.cursor_icon = icon;
            self.window.set_cursor(icon);
        }
    }

    /// Integer selection clamped to the screen; None if empty.
    fn sel_px(&self) -> Option<(u32, u32, u32, u32)> {
        let s = self.sel?;
        let (w, h) = (self.base.width() as f32, self.base.height() as f32);
        let (l, t) = (s.l.round().clamp(0.0, w) as u32, s.t.round().clamp(0.0, h) as u32);
        let (r, b) = (s.r.round().clamp(0.0, w) as u32, s.b.round().clamp(0.0, h) as u32);
        (r > l && b > t).then_some((l, t, r, b))
    }

    // ---------- output ----------

    /// Cropped selection (whole screen if none) with annotations.
    pub fn export(&mut self) -> Option<Image> {
        self.commit_text();
        self.refresh_canvas();
        let (l, t, r, b) = self.sel_px().unwrap_or((0, 0, self.base.width(), self.base.height()));
        let (w, h) = (r - l, b - t);
        let stride = self.canvas.width() as usize * 4;
        let mut rgba = Vec::with_capacity(w as usize * h as usize * 4);
        for y in t..b {
            let i = y as usize * stride + l as usize * 4;
            rgba.extend_from_slice(&self.canvas.data()[i..i + w as usize * 4]);
        }
        Some(Image { w, h, rgba })
    }

    // ---------- rendering ----------

    fn refresh_canvas(&mut self) {
        if !self.canvas_dirty {
            return;
        }
        self.canvas.data_mut().copy_from_slice(self.base.data());
        for s in &self.shapes {
            s.draw(&mut self.canvas, self.text.as_deref());
        }
        self.canvas_dirty = false;
    }

    pub fn render(&mut self) {
        self.refresh_canvas();
        self.frame.data_mut().copy_from_slice(self.dim.data());
        let id = Transform::identity();
        let s = self.scale;

        if let Some((l, t, r, b)) = self.sel_px() {
            // Undimmed, annotated selection.
            let stride = self.frame.width() as usize * 4;
            for y in t..b {
                let i = y as usize * stride + l as usize * 4;
                let j = i + (r - l) as usize * 4;
                self.frame.data_mut()[i..j].copy_from_slice(&self.canvas.data()[i..j]);
            }
            if let Some(shape) = &self.current {
                shape.draw(&mut self.frame, self.text.as_deref());
            }
            self.draw_text_edit();

            // Border + handles.
            let (lf, tf, rf, bf) = (l as f32, t as f32, r as f32, b as f32);
            if let Some(rect) = Rect::from_ltrb(lf, tf, rf, bf) {
                let path = tiny_skia::PathBuilder::from_rect(rect);
                self.frame.stroke_path(&path, &paint(ACCENT), &stroke(1.5 * s), id, None);
            }
            if !matches!(self.drag, Drag::Draw) {
                let (mx, my) = ((lf + rf) / 2.0, (tf + bf) / 2.0);
                let hs = 3.5 * s;
                for (x, y) in [(lf, tf), (mx, tf), (rf, tf), (rf, my), (rf, bf), (mx, bf), (lf, bf), (lf, my)] {
                    if let Some(hr) = Rect::from_ltrb(x - hs, y - hs, x + hs, y + hs) {
                        self.frame.fill_rect(hr, &paint([255, 255, 255, 255]), id, None);
                        let p = tiny_skia::PathBuilder::from_rect(hr);
                        self.frame.stroke_path(&p, &paint(ACCENT), &stroke(1.0 * s), id, None);
                    }
                }
            }
            self.draw_size_label(l, t, r, b);
        }

        if let Some(tb) = self.toolbar() {
            tb.draw(&mut self.frame, s, self.hover, self.tool, self.color);
        }
        self.present();
    }

    fn draw_text_edit(&mut self) {
        let (Some(ed), Some(tr)) = (&self.editing, self.text.as_deref()) else { return };
        let m = tr.draw(Some(&mut self.frame), &ed.text, ed.pos.x, ed.pos.y, ed.size, ed.color);
        let id = Transform::identity();
        // Caret.
        if let Some(r) = Rect::from_xywh(m.caret.0 + 1.0, m.caret.1, 1.5 * self.scale, m.line_h) {
            self.frame.fill_rect(r, &paint(ed.color), id, None);
        }
        // Dashed box.
        let pad = 4.0 * self.scale;
        let w = m.w.max(ed.size * 0.6);
        if let Some(r) = Rect::from_xywh(ed.pos.x - pad, ed.pos.y - pad, w + 2.0 * pad + 2.0, m.h + 2.0 * pad) {
            let mut st = stroke(1.0 * self.scale);
            st.dash = StrokeDash::new(vec![4.0 * self.scale, 3.0 * self.scale], 0.0);
            let p = tiny_skia::PathBuilder::from_rect(r);
            self.frame.stroke_path(&p, &paint([255, 255, 255, 200]), &st, id, None);
        }
    }

    fn draw_size_label(&mut self, l: u32, t: u32, r: u32, b: u32) {
        let Some(tr) = self.text.as_deref() else { return };
        let s = self.scale;
        let label = format!("{} × {}", r - l, b - t);
        let size = 12.0 * s;
        let m = tr.measure(&label, size);
        let pad = 5.0 * s;
        let h = m.h + pad;
        let x = l as f32;
        let y = if t as f32 >= h + 4.0 * s { t as f32 - h - 4.0 * s } else { t as f32 + 4.0 * s };
        if let Some(p) = Rect::from_xywh(x, y, m.w + 2.0 * pad, h).and_then(|r| rounded_rect(r, 4.0 * s)) {
            self.frame.fill_path(&p, &paint([0, 0, 0, 170]), FillRule::Winding, Transform::identity(), None);
        }
        tr.draw(Some(&mut self.frame), &label, x + pad, y + pad / 2.0, size, [255, 255, 255, 255]);
    }

    fn present(&mut self) {
        let size = self.window.inner_size();
        let (Some(w), Some(h)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
        if self.surface_size != (size.width, size.height) {
            if self.surface.resize(w, h).is_err() {
                return;
            }
            self.surface_size = (size.width, size.height);
        }
        let Ok(mut buf) = self.surface.buffer_mut() else { return };
        let (fw, sw) = (self.frame.width() as usize, size.width as usize);
        let cw = fw.min(sw);
        let ch = (self.frame.height() as usize).min(size.height as usize);
        let data = self.frame.data();
        for y in 0..ch {
            let src = &data[y * fw * 4..(y * fw + cw) * 4];
            let dst = &mut buf[y * sw..y * sw + cw];
            for (o, px) in dst.iter_mut().zip(src.chunks_exact(4)) {
                *o = (px[0] as u32) << 16 | (px[1] as u32) << 8 | px[2] as u32;
            }
        }
        let _ = buf.present();
    }
}
