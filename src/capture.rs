//! Platform screen capture. Windows uses GDI; other platforms are TODO.

use tiny_skia::Pixmap;

/// Screen area in physical pixels (virtual-desktop coordinates).
#[derive(Clone, Copy, Debug)]
pub struct ScreenRect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl ScreenRect {
    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w as i32 && py < self.y + self.h as i32
    }
}

#[cfg(windows)]
pub fn cursor_pos() -> Option<(i32, i32)> {
    use windows_sys::Win32::{Foundation::POINT, UI::WindowsAndMessaging::GetCursorPos};
    let mut p = POINT { x: 0, y: 0 };
    let ok = unsafe { GetCursorPos(&mut p) };
    (ok != 0).then_some((p.x, p.y))
}

/// Grab `r` from the screen as an opaque RGBA pixmap.
#[cfg(windows)]
pub fn capture(r: ScreenRect) -> Option<Pixmap> {
    use std::{mem, ptr};
    use windows_sys::Win32::Graphics::Gdi::*;

    let (w, h) = (r.w as i32, r.h as i32);
    let mut buf = vec![0u8; r.w as usize * r.h as usize * 4];
    unsafe {
        let screen = GetDC(ptr::null_mut());
        if screen.is_null() {
            return None;
        }
        let mem_dc = CreateCompatibleDC(screen);
        let bmp = CreateCompatibleBitmap(screen, w, h);
        let old = SelectObject(mem_dc, bmp);
        let blt = BitBlt(mem_dc, 0, 0, w, h, screen, r.x, r.y, SRCCOPY | CAPTUREBLT);
        SelectObject(mem_dc, old); // bitmap must be deselected before GetDIBits

        let mut info: BITMAPINFO = mem::zeroed();
        info.bmiHeader.biSize = mem::size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = w;
        info.bmiHeader.biHeight = -h; // top-down rows
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = 0; // BI_RGB
        let lines = GetDIBits(mem_dc, bmp, 0, r.h, buf.as_mut_ptr().cast(), &mut info, DIB_RGB_COLORS);

        DeleteObject(bmp);
        DeleteDC(mem_dc);
        ReleaseDC(ptr::null_mut(), screen);
        if blt == 0 || lines == 0 {
            return None;
        }
    }
    // BGRA -> RGBA, force opaque.
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
        px[3] = 255;
    }
    Pixmap::from_vec(buf, tiny_skia::IntSize::from_wh(r.w, r.h)?)
}

#[cfg(not(windows))]
pub fn cursor_pos() -> Option<(i32, i32)> {
    None // TODO: macOS / X11 / Wayland
}

#[cfg(not(windows))]
pub fn capture(_r: ScreenRect) -> Option<Pixmap> {
    None // TODO: macOS / X11 / Wayland
}
