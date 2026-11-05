// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod capture;
mod output;
mod overlay;
mod shapes;
mod text;
mod toolbar;

use std::rc::Rc;

use global_hotkey::{
    hotkey::{Code, HotKey, Modifiers},
    GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem},
    Icon, TrayIcon, TrayIconBuilder,
};
use winit::{
    application::ApplicationHandler,
    event::{StartCause, WindowEvent},
    event_loop::{ActiveEventLoop, EventLoop},
    monitor::MonitorHandle,
    window::WindowId,
};

use capture::ScreenRect;
use overlay::{Action, Overlay};
use text::TextRenderer;

enum UserEvent {
    HotKey(GlobalHotKeyEvent),
    Menu(MenuEvent),
}

struct App {
    _hotkeys: Option<GlobalHotKeyManager>,
    /// `--capture`: open overlay at once, exit when it closes (no hotkey/tray).
    oneshot: bool,
    hotkey_id: u32,
    tray: Option<TrayIcon>,
    capture_id: MenuId,
    quit_id: MenuId,
    menu: Option<Menu>,
    text: Option<Rc<TextRenderer>>,
    overlay: Option<Overlay>,
}

impl App {
    fn start_capture(&mut self, el: &ActiveEventLoop) {
        if self.overlay.is_some() {
            return;
        }
        let Some(monitor) = monitor_under_cursor(el) else { return };
        let (pos, size) = (monitor.position(), monitor.size());
        let rect = ScreenRect { x: pos.x, y: pos.y, w: size.width, h: size.height };
        let Some(img) = capture::capture(rect) else {
            eprintln!("znap: screen capture failed");
            return;
        };
        match Overlay::new(el, monitor, img, self.text.clone()) {
            Ok(o) => self.overlay = Some(o),
            Err(e) => eprintln!("znap: overlay failed: {e}"),
        }
    }

    fn finish(&mut self, el: &ActiveEventLoop, action: Action) {
        let Some(ov) = self.overlay.as_mut() else { return };
        match action {
            Action::None => return,
            Action::Close => {}
            Action::Copy => {
                if let Some(img) = ov.export() {
                    if let Err(e) = output::copy_to_clipboard(&img) {
                        eprintln!("znap: clipboard failed: {e}");
                    }
                }
            }
            Action::Save => {
                let Some(img) = ov.export() else { return };
                ov.window.set_visible(false); // dialog must not sit under the topmost overlay
                if output::save_with_dialog(&img).is_none() {
                    ov.window.set_visible(true); // cancelled: back to editing
                    ov.window.focus_window();
                    return;
                }
            }
        }
        self.overlay = None; // drops the window
        if self.oneshot {
            el.exit();
        }
    }
}

fn monitor_under_cursor(el: &ActiveEventLoop) -> Option<MonitorHandle> {
    if let Some((cx, cy)) = capture::cursor_pos() {
        let hit = el.available_monitors().find(|m| {
            let (p, s) = (m.position(), m.size());
            ScreenRect { x: p.x, y: p.y, w: s.width, h: s.height }.contains(cx, cy)
        });
        if hit.is_some() {
            return hit;
        }
    }
    el.primary_monitor().or_else(|| el.available_monitors().next())
}

impl ApplicationHandler<UserEvent> for App {
    fn new_events(&mut self, el: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init && self.oneshot {
            self.start_capture(el);
            if self.overlay.is_none() {
                el.exit();
            }
            return;
        }
        // Tray must be created after the loop starts (required on macOS, fine elsewhere).
        if cause == StartCause::Init && self.tray.is_none() {
            let builder = TrayIconBuilder::new().with_tooltip("znap — Ctrl+Alt+S to capture").with_icon(tray_icon_image());
            let builder = match self.menu.take() {
                Some(menu) => builder.with_menu(Box::new(menu)),
                None => builder,
            };
            match builder.build() {
                Ok(t) => self.tray = Some(t),
                Err(e) => eprintln!("znap: tray icon failed: {e}"),
            }
        }
    }

    fn resumed(&mut self, _el: &ActiveEventLoop) {}

    fn user_event(&mut self, el: &ActiveEventLoop, ev: UserEvent) {
        match ev {
            UserEvent::HotKey(e) if e.id == self.hotkey_id && e.state == HotKeyState::Pressed => self.start_capture(el),
            UserEvent::Menu(e) if e.id == self.capture_id => self.start_capture(el),
            UserEvent::Menu(e) if e.id == self.quit_id => el.exit(),
            _ => {}
        }
    }

    fn window_event(&mut self, el: &ActiveEventLoop, id: WindowId, ev: WindowEvent) {
        let Some(ov) = self.overlay.as_mut().filter(|o| o.window.id() == id) else { return };
        let action = ov.handle(&ev);
        self.finish(el, action);
    }
}

/// 32x32 tray icon: blue rounded square with white corner marks.
fn tray_icon_image() -> Icon {
    use tiny_skia::{FillRule, PathBuilder, Pixmap, Rect, Transform};
    let mut pm = Pixmap::new(32, 32).unwrap();
    let id = Transform::identity();
    if let Some(p) = Rect::from_xywh(1.0, 1.0, 30.0, 30.0).and_then(|r| shapes::rounded_rect(r, 7.0)) {
        pm.fill_path(&p, &shapes::paint([30, 144, 255, 255]), FillRule::Winding, id, None);
    }
    let mut pb = PathBuilder::new();
    for (x, y, dx, dy) in [(8.0, 8.0, 1.0, 1.0), (24.0, 8.0, -1.0, 1.0), (8.0, 24.0, 1.0, -1.0), (24.0, 24.0, -1.0, -1.0)] {
        pb.move_to(x, y + 6.0 * dy);
        pb.line_to(x, y);
        pb.line_to(x + 6.0 * dx, y);
    }
    if let Some(p) = pb.finish() {
        pm.stroke_path(&p, &shapes::paint([255, 255, 255, 255]), &shapes::stroke(2.5), id, None);
    }
    Icon::from_rgba(pm.take(), 32, 32).expect("valid icon")
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let event_loop = EventLoop::<UserEvent>::with_user_event().build()?;

    // Hotkey manager must live on the event-loop thread (Windows).
    let oneshot = std::env::args().any(|a| a == "--capture");
    let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyS);
    let hotkey_id = hotkey.id();
    let hotkeys = if oneshot {
        None
    } else {
        let m = GlobalHotKeyManager::new()?;
        if let Err(e) = m.register(hotkey) {
            let _ = rfd::MessageDialog::new()
                .set_title("znap")
                .set_description(format!("Cannot register Ctrl+Alt+S (used by another app?): {e}"))
                .show();
        }
        Some(m)
    };

    let proxy = event_loop.create_proxy();
    GlobalHotKeyEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::HotKey(e));
    }));
    let proxy = event_loop.create_proxy();
    MenuEvent::set_event_handler(Some(move |e| {
        let _ = proxy.send_event(UserEvent::Menu(e));
    }));

    let menu = Menu::new();
    let capture_item = MenuItem::new("Capture  (Ctrl+Alt+S)", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&capture_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let mut app = App {
        _hotkeys: hotkeys,
        oneshot,
        hotkey_id,
        tray: None,
        capture_id: capture_item.id().clone(),
        quit_id: quit_item.id().clone(),
        menu: Some(menu),
        text: TextRenderer::load().map(Rc::new),
        overlay: None,
    };
    event_loop.run_app(&mut app)?;
    Ok(())
}
