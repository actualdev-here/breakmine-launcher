//! Design tokens from src/styles.css (:root) and the fonts the launcher ships.
use gpui::{rgb, rgba, App, Hsla};
use std::borrow::Cow;

pub const ACCENT: u32 = 0xfafafa;
pub const INK: u32 = 0x000000;
pub const BG: u32 = 0x000000;
pub const FG: u32 = 0xf5f5f5;
pub const DIM: u32 = 0x8f8f8f;
pub const DIM2: u32 = 0x7d7d7d;
pub const MUTED: u32 = 0xa3a3a3;
pub const LINE: u32 = 0x161616;
pub const LINE2: u32 = 0x262626;
pub const CARD: u32 = 0x050505;
pub const CARD2: u32 = 0x070707;
pub const FIELD: u32 = 0x0a0a0a;
pub const DANGER: u32 = 0xff8a8a;

pub const SANS: &str = "Inter";
pub const MONO: &str = "JetBrains Mono";
pub const PIXEL: &str = "Breakmine";

pub fn c(hex: u32) -> Hsla {
    rgb(hex).into()
}
/// `0xRRGGBBAA`.
pub fn ca(hex: u32) -> Hsla {
    rgba(hex).into()
}

/// Inter (four weights cut from the variable font), JetBrains Mono and the Breakmine pixel font.
pub fn load_fonts(cx: &mut App) {
    let dir = crate::assets::launcher_path("fonts");
    let fonts: Vec<Cow<'static, [u8]>> = std::fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "ttf"))
        .filter_map(|e| std::fs::read(e.path()).ok())
        .map(Cow::Owned)
        .collect();
    let n = fonts.len();
    if let Err(e) = cx.text_system().add_fonts(fonts) {
        eprintln!("fonts: {e}");
    }
    if std::env::var_os("BM_FONT_DEBUG").is_some() {
        let names = cx.text_system().all_font_names();
        eprintln!("loaded {n} font files; known: {:?}", names.iter().filter(|f| ["Inter", "Breakmine", "JetBrains"].iter().any(|k| f.contains(k))).collect::<Vec<_>>());
    }
}
