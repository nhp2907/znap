//! Clipboard + PNG save.

use std::{borrow::Cow, path::PathBuf, time::SystemTime};

use tiny_skia::{IntSize, Pixmap};

/// Opaque RGBA image.
pub struct Image {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

pub fn copy_to_clipboard(img: &Image) -> Result<(), arboard::Error> {
    arboard::Clipboard::new()?.set_image(arboard::ImageData {
        width: img.w as usize,
        height: img.h as usize,
        bytes: Cow::Borrowed(&img.rgba),
    })
}

/// Ask for a path and write a PNG. `None` if cancelled or failed.
pub fn save_with_dialog(img: &Image) -> Option<PathBuf> {
    let ts = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let path = rfd::FileDialog::new()
        .add_filter("PNG image", &["png"])
        .set_file_name(format!("znap_{ts}.png"))
        .save_file()?;
    let pm = Pixmap::from_vec(img.rgba.clone(), IntSize::from_wh(img.w, img.h)?)?;
    pm.save_png(&path).ok()?;
    Some(path)
}
