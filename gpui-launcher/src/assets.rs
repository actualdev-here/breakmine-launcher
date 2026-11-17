//! Launcher assets in the project-level asset directory and pixel-art scaling.
//!
//! GPUI smooths scaled images, so pixel art (icons, skins, textures) is enlarged here with
//! nearest-neighbour first, like `image-rendering: pixelated` in the web UI.
use gpui::{Image, ImageFormat};
use std::{
    cell::RefCell,
    collections::HashMap,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
};

/// A file under the project's `src/` folder, e.g. `assets/game/launcher_logo.png`.
pub fn path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src").join(rel)
}

thread_local! {
    static CACHE: RefCell<crate::memory_cache::MemoryCache<Arc<Image>>> = RefCell::new(crate::memory_cache::MemoryCache::new(16 * 1024 * 1024, 256));
    static FILE_CACHE: RefCell<HashMap<PathBuf, (u64, std::time::SystemTime, Vec<u8>)>> = RefCell::new(HashMap::new());
}

fn encode(img: &image::RgbaImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    img.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).ok()?;
    Some(out)
}

fn cached(key: String, make: impl FnOnce() -> Option<Vec<u8>>) -> Option<Arc<Image>> {
    if let Some(hit) = CACHE.with(|c| c.borrow_mut().get(&key)) {
        return Some(hit);
    }
    let bytes = make()?;
    let size = bytes.len();
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
    CACHE.with(|c| { c.borrow_mut().insert(key, image.clone(), size); });
    Some(image)
}

fn upscale(img: &image::RgbaImage, scale: u32) -> image::RgbaImage {
    image::imageops::resize(img, img.width() * scale, img.height() * scale, image::imageops::FilterType::Nearest)
}

/// A PNG file enlarged `scale`× with hard pixel edges.
pub fn pixel_file(file: &Path, scale: u32) -> Option<Arc<Image>> {
    cached(format!("f:{}:{scale}", file.display()), || {
        let img = image::load_from_memory(&std::fs::read(file).ok()?).ok()?.to_rgba8();
        encode(&upscale(&img, scale))
    })
}

/// A launcher asset (relative to src/) enlarged `scale`×.
pub fn pixel(rel: &str, scale: u32) -> Option<Arc<Image>> {
    pixel_file(&path(rel), scale)
}

/// PNG bytes (e.g. a painted texture or a downloaded skin) enlarged `scale`×. `key` identifies the bytes.
pub fn pixel_bytes(key: &str, png: &[u8], scale: u32) -> Option<Arc<Image>> {
    cached(format!("b:{key}:{scale}"), || {
        let img = image::load_from_memory(png).ok()?.to_rgba8();
        encode(&upscale(&img, scale))
    })
}

/// Cache the rendered full-body preview as well as its decoded image.
pub fn skin_front(key: &str, png: &[u8], slim: bool) -> Option<Arc<Image>> {
    cached(format!("front:{key}:{slim}"), || {
        let preview = crate::skin_preview::front(png, slim).ok()?;
        let image = image::load_from_memory(&preview).ok()?.to_rgba8();
        encode(&upscale(&image, 4))
    })
}

/// The face of a skin (8×8 at 8,8 with the hat layer on top), enlarged `scale`×.
pub fn skin_head(key: &str, png: &[u8], scale: u32) -> Option<Arc<Image>> {
    cached(format!("h:{key}:{scale}"), || {
        let skin = image::load_from_memory(png).ok()?.to_rgba8();
        if skin.width() < 64 || skin.height() < 32 {
            return None;
        }
        let mut head = image::imageops::crop_imm(&skin, 8, 8, 8, 8).to_image();
        let hat = image::imageops::crop_imm(&skin, 40, 8, 8, 8).to_image();
        image::imageops::overlay(&mut head, &hat, 0, 0);
        encode(&upscale(&head, scale))
    })
}

/// An asset recoloured to one colour, keeping its alpha (CSS `mask` + `background: currentColor`).
pub fn tinted(rel: &str, rgb: u32) -> Option<Arc<Image>> {
    cached(format!("t:{rel}:{rgb:06x}"), || {
        let mut img = image::load_from_memory(&std::fs::read(path(rel)).ok()?).ok()?.to_rgba8();
        let (r, g, b) = ((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8);
        for p in img.pixels_mut() {
            p.0 = [r, g, b, p.0[3]];
        }
        encode(&img)
    })
}

/// Reuse unchanged skin files during animated frames, while noticing external edits.
pub fn read_cached(path: &Path) -> Option<Vec<u8>> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?;
    let size = metadata.len();
    if let Some(bytes) = FILE_CACHE.with(|cache| cache.borrow().get(path)
        .filter(|(old_size, old_modified, _)| *old_size == size && *old_modified == modified)
        .map(|(_, _, bytes)| bytes.clone())) {
        return Some(bytes);
    }
    let bytes = std::fs::read(path).ok()?;
    FILE_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if cache.len() >= 128 { cache.clear(); }
        cache.insert(path.to_owned(), (size, modified, bytes.clone()));
    });
    Some(bytes)
}

/// The built-in Breakmine skin.
pub fn default_skin() -> Vec<u8> {
    read_cached(&path("assets/game/default_skin.png")).unwrap_or_default()
}

fn cover_rect(sw: u32, sh: u32, w: u32, h: u32, x_pos: f32, y_pos: f32) -> Option<(u32, u32, u32, u32)> {
    if sw == 0 || sh == 0 || w == 0 || h == 0 { return None; }
    let scale = (w as f64 / sw as f64).max(h as f64 / sh as f64);
    let cw = ((w as f64 / scale).round() as u32).clamp(1, sw);
    let ch = ((h as f64 / scale).round() as u32).clamp(1, sh);
    let x = ((sw - cw) as f32 * x_pos.clamp(0., 1.)).round() as u32;
    let y = ((sh - ch) as f32 * y_pos.clamp(0., 1.)).round() as u32;
    Some((x, y, cw, ch))
}

/// Paint a centre-cropped image into the actual rounded viewport. GPUI's built-in
/// Cover rounds the enlarged image bounds, which can leave the viewport square.
pub fn rounded_cover(file: PathBuf, radius: f32, x_pos: f32, y_pos: f32) -> impl gpui::IntoElement {
    let key = format!("file:{}", file.display());
    cover_canvas(key, move || image::open(&file).ok().map(|image| image.to_rgba8()), radius, false, x_pos, y_pos)
}

pub fn rounded_cover_bytes(bytes: Vec<u8>, radius: f32, x_pos: f32, y_pos: f32) -> impl gpui::IntoElement {
    use sha2::{Digest, Sha256};
    let key = format!("bytes:{:x}", Sha256::digest(&bytes));
    cover_canvas(key, move || image::load_from_memory(&bytes).ok().map(|image| image.to_rgba8()), radius, false, x_pos, y_pos)
}

pub fn rounded_cover_top(file: PathBuf, radius: f32, x_pos: f32, y_pos: f32) -> impl gpui::IntoElement {
    let key = format!("file:{}", file.display());
    cover_canvas(key, move || image::open(&file).ok().map(|image| image.to_rgba8()), radius, true, x_pos, y_pos)
}

pub fn rounded_cover_top_bytes(bytes: Vec<u8>, radius: f32, x_pos: f32, y_pos: f32) -> impl gpui::IntoElement {
    use sha2::{Digest, Sha256};
    let key = format!("bytes:{:x}", Sha256::digest(&bytes));
    cover_canvas(key, move || image::load_from_memory(&bytes).ok().map(|image| image.to_rgba8()), radius, true, x_pos, y_pos)
}

fn cover_canvas(key: String, source: impl Fn() -> Option<image::RgbaImage> + 'static, radius: f32, top_only: bool, x_pos: f32, y_pos: f32) -> impl gpui::IntoElement {
    use gpui::{canvas, prelude::*, px, Corners, RenderImage};
    use image::Frame;
    thread_local! {
        static COVER_CACHE: RefCell<crate::memory_cache::MemoryCache<Arc<RenderImage>>> = RefCell::new(crate::memory_cache::MemoryCache::new(48 * 1024 * 1024, 64));
    }
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let scale = window.scale_factor();
            let w = (f32::from(bounds.size.width) * scale).round() as u32;
            let h = (f32::from(bounds.size.height) * scale).round() as u32;
            if w == 0 || h == 0 || w > 4096 || h > 4096 { return; }
            let key = format!("{key}:{w}:{h}:{x_pos}:{y_pos}");
            let cached = COVER_CACHE.with(|c| c.borrow_mut().get(&key));
            let image = if let Some(image) = cached { image } else {
                let Some(source) = source() else { return };
                let (sw, sh) = source.dimensions();
                let Some((x, y, cw, ch)) = cover_rect(sw, sh, w, h, x_pos, y_pos) else { return };
                let crop = image::imageops::crop_imm(&source, x, y, cw, ch).to_image();
                let mut pixels = image::imageops::resize(&crop, w, h, image::imageops::FilterType::Triangle);
                for p in pixels.pixels_mut() { p.0.swap(0, 2); } // GPUI RenderImage uses BGRA.
                let image = Arc::new(RenderImage::new(vec![Frame::new(pixels)]));
                let evicted = COVER_CACHE.with(|c| c.borrow_mut().insert(key, image.clone(), w as usize * h as usize * 4));
                for old in evicted { let _ = window.drop_image(old); }
                image
            };
            let corners = if top_only {
                Corners { top_left: px(radius), top_right: px(radius), bottom_left: px(0.), bottom_right: px(0.) }
            } else { Corners::all(px(radius)) };
            let _ = window.paint_image(bounds, corners, image, 0, false);
        },
    ).size_full()
}

#[cfg(test)]
mod cover_tests {
    use super::cover_rect;
    #[test]
    fn cover_crop_is_positioned_and_within_source() {
        assert_eq!(cover_rect(400, 200, 100, 100, 0.5, 0.5), Some((100, 0, 200, 200)));
        assert_eq!(cover_rect(200, 400, 100, 100, 0.5, 1.0), Some((0, 200, 200, 200)));
        assert!(cover_rect(0, 400, 100, 100, 0.5, 0.5).is_none());
    }
}
