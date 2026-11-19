use image::{imageops, DynamicImage, ImageFormat, RgbaImage};
use std::io::Cursor;

/// Draw the front of a Minecraft-style 64x64 or legacy 64x32 skin.
pub fn front(png: &[u8], slim: bool) -> Result<Vec<u8>, String> {
    if png.len() > 256 * 1024 {
        return Err("Skin image is too large".into());
    }
    let (_, height) = match crate::util::png_size(png) {
        Some((64, h @ (32 | 64))) => (64, h),
        _ => return Err("Skin must be a 64x32 or 64x64 PNG".into()),
    };
    let skin = image::load_from_memory_with_format(png, ImageFormat::Png)
        .map_err(|e| e.to_string())?
        .to_rgba8();
    let mut result = RgbaImage::new(16, 32);
    let arm = if slim { 3 } else { 4 };
    let mut part = |sx, sy, w, h, dx, dy, mirror| {
        let crop = imageops::crop_imm(&skin, sx, sy, w, h).to_image();
        let crop = if mirror {
            imageops::flip_horizontal(&crop)
        } else {
            crop
        };
        imageops::overlay(&mut result, &crop, dx, dy);
    };
    let right_arm_x = 4 - arm;
    part(8, 8, 8, 8, 4, 0, false);
    part(20, 20, 8, 12, 4, 8, false);
    part(44, 20, arm, 12, right_arm_x.into(), 8, false);
    part(4, 20, 4, 12, 4, 20, false);
    if height == 32 {
        part(44, 20, arm, 12, 12, 8, true);
        part(4, 20, 4, 12, 8, 20, true);
    } else {
        part(36, 52, arm, 12, 12, 8, false);
        part(20, 52, 4, 12, 8, 20, false);
    }
    part(40, 8, 8, 8, 4, 0, false);
    if height == 64 {
        part(20, 36, 8, 12, 4, 8, false);
        part(44, 36, arm, 12, right_arm_x.into(), 8, false);
        part(4, 36, 4, 12, 4, 20, false);
        part(52, 52, arm, 12, 12, 8, false);
        part(4, 52, 4, 12, 8, 20, false);
    }
    let result = imageops::resize(&result, 128, 256, imageops::FilterType::Nearest);
    let mut out = Cursor::new(Vec::new());
    DynamicImage::ImageRgba8(result)
        .write_to(&mut out, ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    fn png(img: RgbaImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(img)
            .write_to(&mut out, ImageFormat::Png)
            .unwrap();
        out.into_inner()
    }
    #[test]
    fn renders_face_overlay_and_legacy_limbs() {
        let mut skin = RgbaImage::new(64, 32);
        for y in 8..16 {
            for x in 8..16 {
                skin.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        skin.put_pixel(40, 8, Rgba([0, 0, 255, 255]));
        for y in 20..32 {
            for x in 44..48 {
                skin.put_pixel(x, y, Rgba([0, 255, 0, 255]));
            }
        }
        let rendered = image::load_from_memory(&front(&png(skin), false).unwrap())
            .unwrap()
            .to_rgba8();
        assert_eq!(*rendered.get_pixel(32, 0), Rgba([0, 0, 255, 255]));
        assert_eq!(*rendered.get_pixel(40, 0), Rgba([255, 0, 0, 255]));
        assert_eq!(*rendered.get_pixel(0, 64), Rgba([0, 255, 0, 255]));
        assert_eq!(*rendered.get_pixel(96, 64), Rgba([0, 255, 0, 255]));
    }
    #[test]
    fn rejects_invalid_dimensions() {
        assert!(front(&png(RgbaImage::new(1024, 1024)), false).is_err());
        assert!(front(b"invalid", false).is_err());
    }
}
