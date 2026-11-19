//! The title-screen panorama (src/js/gl.js `Panorama`), sampled on the CPU.
//!
//! Same rig as the game's main menu: a unit box seen from inside with faces in the order
//! 1, 3, 4, 5, 0, 2 (three.js BoxGeometry +x, -x, +y, -y, +z, -z), scaled by -1, and a camera
//! rotated in ZYX order. Drawn at a reduced size and scaled up, like a blurred backdrop.
use gpui::{canvas, div, prelude::*, Context, RenderImage, Window};
use image::{Frame, RgbaImage};
use std::{cell::RefCell, rc::Rc, sync::Arc, time::Instant};

pub struct Pano {
    set: String,
    faces: Option<Arc<Vec<RgbaImage>>>,
    yaw: f32,
    start: Instant,
    last: Instant,
    pub reduce_motion: bool,
    frame: Rc<RefCell<Option<Arc<RenderImage>>>>,
    frame_clock: Rc<crate::frame_clock::FrameClock>,
    animation_pending: bool,
}

impl Pano {
    pub fn new() -> Self {
        Self { set: String::new(), faces: None, yaw: 0., start: Instant::now(), last: Instant::now(), reduce_motion: false, frame: Rc::new(RefCell::new(None)), frame_clock: Rc::new(crate::frame_clock::FrameClock::default()), animation_pending: false }
    }

    /// Release the six decoded panorama textures when the sign-in view closes.
    pub fn clear(&mut self) {
        self.set.clear();
        self.faces = None;
        // Keep the last small frame until the next paint can release its GPU image.
        self.frame_clock.invalidate();
    }

    /// "day", "sunset" or "nether".
    pub fn set(&mut self, set: &str) {
        if set == self.set {
            return;
        }
        self.set = set.to_string();
        self.frame_clock.invalidate();
        let faces: Option<Vec<RgbaImage>> = [1, 3, 4, 5, 0, 2]
            .iter()
            .map(|i| {
                let bytes = std::fs::read(crate::assets::path(&format!("assets/game/pano/{set}_{i}.webp"))).ok()?;
                image::load_from_memory(&bytes).ok().map(|i| i.thumbnail(512, 512).to_rgba8())
            })
            .collect();
        self.faces = faces.map(Arc::new);
    }
}

fn sample(img: &RgbaImage, u: f32, v: f32) -> [f32; 3] {
    let (w, h) = (img.width() as f32, img.height() as f32);
    let x = (u * w - 0.5).clamp(0., w - 1.);
    let y = (v * h - 0.5).clamp(0., h - 1.);
    let (x0, y0) = (x.floor() as u32, y.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width() - 1), (y0 + 1).min(img.height() - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let p = |x, y| img.get_pixel(x, y).0;
    let (a, b, c, d) = (p(x0, y0), p(x1, y0), p(x0, y1), p(x1, y1));
    let mut out = [0.; 3];
    for i in 0..3 {
        let top = a[i] as f32 * (1. - fx) + b[i] as f32 * fx;
        let bot = c[i] as f32 * (1. - fx) + d[i] as f32 * fx;
        out[i] = top * (1. - fy) + bot * fy;
    }
    out
}

/// One frame, BGRA. `dim` darkens (CSS `brightness(.7)`).
fn render(faces: &[RgbaImage], yaw_deg: f32, pitch_deg: f32, w: usize, h: usize, dim: f32) -> Vec<u8> {
    let aspect = w as f32 / h as f32;
    let fov = (2. * ((115f32.to_radians() / 2.).tan() / aspect).atan()).to_degrees().clamp(52., 85.);
    let t = (fov.to_radians() / 2.).tan();
    let rx = -(pitch_deg + 180.).to_radians();
    let ry = -(-yaw_deg - 180.).to_radians();
    let (cx, sx) = (rx.cos(), rx.sin());
    let (cy, sy) = (ry.cos(), ry.sin());
    let mut out = vec![0u8; w * h * 4];
    for py in 0..h {
        let ny = 1. - 2. * (py as f32 + 0.5) / h as f32;
        for px in 0..w {
            let nx = 2. * (px as f32 + 0.5) / w as f32 - 1.;
            // camera space → world: Ry · Rx · d
            let d = [nx * t * aspect, ny * t, -1.];
            let (x1, y1, z1) = (d[0], d[1] * cx - d[2] * sx, d[1] * sx + d[2] * cx);
            let world = [x1 * cy + z1 * sy, y1, -x1 * sy + z1 * cy];
            // the box is scaled by -1
            let l = [-world[0], -world[1], -world[2]];
            let (ax, ay, az) = (l[0].abs(), l[1].abs(), l[2].abs());
            let m = ax.max(ay).max(az);
            let p = [l[0] / m * 0.5, l[1] / m * 0.5, l[2] / m * 0.5];
            let (face, u, v) = if m == ax {
                if l[0] > 0. { (0, 0.5 - p[2], 0.5 - p[1]) } else { (1, 0.5 + p[2], 0.5 - p[1]) }
            } else if m == ay {
                if l[1] > 0. { (2, 0.5 + p[0], 0.5 + p[2]) } else { (3, 0.5 + p[0], 0.5 - p[2]) }
            } else if l[2] > 0. {
                (4, 0.5 + p[0], 0.5 - p[1])
            } else {
                (5, 0.5 - p[0], 0.5 - p[1])
            };
            let c = sample(&faces[face], u, v);
            let o = (py * w + px) * 4;
            out[o] = (c[2] * dim) as u8;
            out[o + 1] = (c[1] * dim) as u8;
            out[o + 2] = (c[0] * dim) as u8;
            out[o + 3] = 255;
        }
    }
    out
}

impl Render for Pano {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Instant::now();
        let dt = ((now - self.last).as_secs_f32() * 1000.).min(50.);
        self.last = now;
        if !self.reduce_motion {
            self.yaw += dt * 0.0045;
        }
        let pitch = 8. + if self.reduce_motion { 0. } else { ((now - self.start).as_secs_f32() * 1000. / 9000.).sin() * 5. };
        if !self.reduce_motion && !self.animation_pending {
            self.animation_pending = true;
            let timer = cx.background_executor().clone();
            cx.spawn(async move |view, cx| {
                timer.timer(std::time::Duration::from_millis(33)).await;
                let _ = view.update(cx, |this, cx| { this.animation_pending = false; cx.notify(); });
            }).detach();
        }
        let faces = self.faces.clone();
        let yaw = self.yaw;
        let cell = self.frame.clone();
        let clock = self.frame_clock.clone();
        div().size_full().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let Some(faces) = faces else { return };
                    // A third of the window size is plenty for a soft backdrop.
                    let w = ((f32::from(bounds.size.width) / 3.).round() as usize).clamp(2, 1024);
                    let h = ((f32::from(bounds.size.height) / 3.).round() as usize).clamp(2, 1024);
                    if clock.can_reuse(w, h, 1.) {
                        if let Some(image) = cell.borrow().as_ref() {
                            let _ = window.paint_image(bounds, gpui::Corners::default(), image.clone(), 0, false);
                            return;
                        }
                    }
                    let px = render(&faces, yaw, pitch, w, h, 0.7);
                    let Some(buf) = image::ImageBuffer::from_raw(w as u32, h as u32, px) else { return };
                    let image = Arc::new(RenderImage::new(vec![Frame::new(buf)]));
                    clock.rendered(w, h, 1.);
                    if let Some(old) = cell.borrow_mut().replace(image.clone()) {
                        let _ = window.drop_image(old);
                    }
                    let _ = window.paint_image(bounds, gpui::Corners::default(), image, 0, false);
                },
            )
            .size_full(),
        )
    }
}
