//! 16×16 texture painting (port of src/js/creator/pixel.js). Textures are PNG data URLs,
//! the same format the web Mod Maker stores in project.json.
use base64::Engine;
use image::{Rgba, RgbaImage};

pub const SIZE: u32 = 16;

pub const PALETTE: [&str; 24] = [
    "#000000", "#3f3f3f", "#7f7f7f", "#bfbfbf", "#ffffff", "#5a3a1e", "#8b5a2b", "#c69c6d", "#7cc84a", "#3c8527", "#1f5e2a", "#9be3ee", "#3aa0d8", "#2a4f9e", "#7a3fb5", "#d84aa6", "#e23b3b", "#f08c2e", "#f2d13a", "#e8c39e", "#4ae2b1", "#a0a0ff", "#ff9fd0", "#00000000",
];

pub const COLORS: [&str; 8] = ["#7cc84a", "#e23b3b", "#3aa0d8", "#f2d13a", "#7a3fb5", "#8b5a2b", "#9be3ee", "#bfbfbf"];

pub fn rgba(hex: &str) -> [u8; 4] {
    let v = hex.trim_start_matches('#');
    let p = |i: usize| u8::from_str_radix(v.get(i..i + 2).unwrap_or("00"), 16).unwrap_or(0);
    [p(0), p(2), p(4), if v.len() == 8 { p(6) } else { 255 }]
}

pub fn hex(c: [u8; 4]) -> String {
    if c[3] == 0 {
        "#00000000".into()
    } else {
        format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
    }
}

pub fn decode(data_url: &str) -> RgbaImage {
    let b64 = data_url.split_once(',').map(|(_, b)| b).unwrap_or("");
    let img = base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok().and_then(|b| image::load_from_memory(&b).ok()).map(|i| i.to_rgba8());
    match img {
        Some(i) if i.width() == SIZE && i.height() == SIZE => i,
        Some(i) => image::imageops::resize(&i, SIZE, SIZE, image::imageops::FilterType::Nearest),
        None => RgbaImage::new(SIZE, SIZE),
    }
}

pub fn encode(img: &RgbaImage) -> String {
    let mut png = Vec::new();
    let _ = img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png);
    format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png))
}

/// A noisy starter texture in one colour (pixel.js `starterTexture`).
pub fn starter(base: &str, seed: u64) -> String {
    let [r, g, b, _] = rgba(base);
    let mut s = (seed * 9301 + 49297) as f64;
    let mut rnd = || {
        s = (s * 9301. + 49297.) % 233280.;
        s / 233280.
    };
    let mut img = RgbaImage::new(SIZE, SIZE);
    for y in 0..SIZE {
        for x in 0..SIZE {
            let edge = x == 0 || y == 0 || x == SIZE - 1 || y == SIZE - 1;
            let k = (if edge { 0.72 } else { 0.88 }) + rnd() * 0.24;
            let c = |v: u8| ((v as f64 * k).min(255.)) as u8;
            img.put_pixel(x, y, Rgba([c(r), c(g), c(b), 255]));
        }
    }
    encode(&img)
}

pub fn set_pixel(img: &mut RgbaImage, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
        return;
    }
    img.put_pixel(x as u32, y as u32, Rgba(color));
}

pub fn fill(img: &mut RgbaImage, x: i32, y: i32, color: [u8; 4]) {
    if x < 0 || y < 0 || x >= SIZE as i32 || y >= SIZE as i32 {
        return;
    }
    let target = img.get_pixel(x as u32, y as u32).0;
    if target == color {
        return;
    }
    let mut stack = vec![(x, y)];
    while let Some((px, py)) = stack.pop() {
        if px < 0 || py < 0 || px >= SIZE as i32 || py >= SIZE as i32 {
            continue;
        }
        if img.get_pixel(px as u32, py as u32).0 != target {
            continue;
        }
        img.put_pixel(px as u32, py as u32, Rgba(color));
        stack.extend([(px + 1, py), (px - 1, py), (px, py + 1), (px, py - 1)]);
    }
}

/// Editor state for one texture (tool, colour, history).
pub struct Editor {
    pub tool: &'static str,
    pub color: String,
    pub mirror: bool,
    pub undo: Vec<RgbaImage>,
    pub redo: Vec<RgbaImage>,
    pub last: Option<(i32, i32)>,
}

impl Default for Editor {
    fn default() -> Self {
        Self { tool: "pencil", color: "#7cc84a".into(), mirror: false, undo: Vec::new(), redo: Vec::new(), last: None }
    }
}

impl Editor {
    pub fn snapshot(&mut self, img: &RgbaImage) {
        self.undo.push(img.clone());
        self.redo.clear();
        if self.undo.len() > 40 {
            self.undo.remove(0);
        }
    }

    /// Paint at a cell; returns true when the image changed. The picker changes the colour instead.
    pub fn paint(&mut self, img: &mut RgbaImage, x: i32, y: i32) -> bool {
        match self.tool {
            "picker" => {
                let p = img.get_pixel(x.clamp(0, 15) as u32, y.clamp(0, 15) as u32).0;
                self.color = hex(p);
                self.tool = "pencil";
                false
            }
            "bucket" => {
                fill(img, x, y, rgba(&self.color));
                true
            }
            tool => {
                let color = if tool == "eraser" { [0, 0, 0, 0] } else { rgba(&self.color) };
                // Fill gaps between pointer events so fast strokes stay continuous.
                let from = self.last.unwrap_or((x, y));
                let steps = (x - from.0).abs().max((y - from.1).abs()).max(1);
                for i in 0..=steps {
                    let px = (from.0 as f32 + (x - from.0) as f32 * i as f32 / steps as f32).round() as i32;
                    let py = (from.1 as f32 + (y - from.1) as f32 * i as f32 / steps as f32).round() as i32;
                    set_pixel(img, px, py, color);
                    if self.mirror {
                        set_pixel(img, SIZE as i32 - 1 - px, py, color);
                    }
                }
                self.last = Some((x, y));
                true
            }
        }
    }
}
