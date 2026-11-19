//! The 3D player preview (src/js/gl.js `SkinViewer`), drawn on the CPU because GPUI has no 3D.
//!
//! Same model, camera, lights and animation as the three.js version: both skin layers, slim arms,
//! a cape, walking + idle sway, drag to turn, scroll to zoom, and a head that follows the cursor on Home.
use gpui::{
    canvas, div, prelude::*, Bounds, Context, MouseButton, MouseDownEvent, MouseMoveEvent, Pixels, Point, RenderImage, ScrollWheelEvent, Window,
};
use image::{Frame, RgbaImage};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Instant,
};

type V3 = [f32; 3];

fn sub(a: V3, b: V3) -> V3 { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn dot(a: V3, b: V3) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn cross(a: V3, b: V3) -> V3 { [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]] }
fn norm(a: V3) -> V3 {
    let l = dot(a, a).sqrt().max(1e-6);
    [a[0] / l, a[1] / l, a[2] / l]
}

/// 3×3 rotation, three.js Euler order XYZ (R = Rx · Ry · Rz).
#[derive(Clone, Copy)]
struct M3([f32; 9]);
impl M3 {
    fn euler(x: f32, y: f32, z: f32) -> Self {
        let (a, b) = (x.cos(), x.sin());
        let (cc, d) = (y.cos(), y.sin());
        let (e, f) = (z.cos(), z.sin());
        let (ae, af, be, bf) = (a * e, a * f, b * e, b * f);
        M3([cc * e, -cc * f, d, af + be * d, ae - bf * d, -b * cc, bf - ae * d, be + af * d, a * cc])
    }
    fn mul(&self, v: V3) -> V3 {
        let m = &self.0;
        [m[0] * v[0] + m[1] * v[1] + m[2] * v[2], m[3] * v[0] + m[4] * v[1] + m[5] * v[2], m[6] * v[0] + m[7] * v[1] + m[8] * v[2]]
    }
    fn then(&self, inner: &M3) -> M3 {
        // self · inner
        let (a, b) = (&self.0, &inner.0);
        let mut o = [0.; 9];
        for r in 0..3 {
            for col in 0..3 {
                o[r * 3 + col] = a[r * 3] * b[col] + a[r * 3 + 1] * b[3 + col] + a[r * 3 + 2] * b[6 + col];
            }
        }
        M3(o)
    }
}

/// A placed transform: world = rot · local + pos.
#[derive(Clone, Copy)]
struct Xf {
    rot: M3,
    pos: V3,
}
impl Xf {
    fn apply(&self, v: V3) -> V3 {
        let r = self.rot.mul(v);
        [r[0] + self.pos[0], r[1] + self.pos[1], r[2] + self.pos[2]]
    }
    /// self ∘ child (child transform is local to self).
    fn then(&self, child: &Xf) -> Xf {
        Xf { rot: self.rot.then(&child.rot), pos: self.apply(child.pos) }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mat {
    Base,
    Over,
    Cape,
    Block,
}

struct Tex<'a> {
    img: &'a RgbaImage,
    w: f32,
    h: f32,
}

/// Linear-space colour of a texel (sRGB → linear like three.js `sRGBEncoding`).
fn lin(c: u8) -> f32 {
    let v = c as f32 / 255.;
    if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}
fn srgb(v: f32) -> u8 {
    let v = v.clamp(0., 1.);
    let s = if v <= 0.0031308 { v * 12.92 } else { 1.055 * v.powf(1. / 2.4) - 0.055 };
    (s * 255. + 0.5) as u8
}

// Every triangle has a single lighting factor per channel. Skin textures contain
// few distinct colours, so calculate each shaded colour once per triangle.
struct ShadeCache {
    values: [[u16; 256]; 3],
    shade: [f32; 3],
    mat: Mat,
}

impl ShadeCache {
    fn new(shade: [f32; 3], mat: Mat) -> Self {
        Self { values: [[u16::MAX; 256]; 3], shade, mat }
    }

    fn channel(&mut self, component: usize, value: u8) -> u8 {
        let cached = &mut self.values[component][value as usize];
        if *cached == u16::MAX {
            let factor = self.shade[component];
            *cached = if self.mat == Mat::Block {
                (value as f32 * factor).clamp(0., 255.).round() as u16
            } else {
                srgb(lin(value) * factor) as u16
            };
        }
        *cached as u8
    }
}

struct Raster {
    w: usize,
    h: usize,
    color: Vec<u8>,
    depth: Vec<f32>,
    eye: V3,
    right: V3,
    up: V3,
    fwd: V3,
    focal: f32,
}

impl Raster {
    fn project(&self, p: V3) -> (f32, f32, f32) {
        let d = sub(p, self.eye);
        let z = dot(d, self.fwd);
        let x = dot(d, self.right);
        let y = dot(d, self.up);
        (self.w as f32 / 2. + x * self.focal / z, self.h as f32 / 2. - y * self.focal / z, z)
    }

    /// One textured triangle; `uv` in texels. Perspective-correct, nearest sampling, alpha test for overlays.
    fn tri(&mut self, p: [(f32, f32, f32); 3], uv: [(f32, f32); 3], tex: &Tex, mat: Mat, shade: [f32; 3]) {
        if p.iter().any(|v| v.2 <= 0.5) {
            return;
        }
        let (x0, y0) = (p[0].0, p[0].1);
        let (x1, y1) = (p[1].0, p[1].1);
        let (x2, y2) = (p[2].0, p[2].1);
        let area = (x1 - x0) * (y2 - y0) - (x2 - x0) * (y1 - y0);
        if area.abs() < 1e-6 {
            return;
        }
        let minx = x0.min(x1).min(x2).floor().max(0.) as i32;
        let maxx = x0.max(x1).max(x2).ceil().min(self.w as f32 - 1.) as i32;
        let miny = y0.min(y1).min(y2).floor().max(0.) as i32;
        let maxy = y0.max(y1).max(y2).ceil().min(self.h as f32 - 1.) as i32;
        let iz = [1. / p[0].2, 1. / p[1].2, 1. / p[2].2];
        let uz = [uv[0].0 * iz[0], uv[1].0 * iz[1], uv[2].0 * iz[2]];
        let vz = [uv[0].1 * iz[0], uv[1].1 * iz[1], uv[2].1 * iz[2]];
        let (tw, th) = (tex.img.width() as i32, tex.img.height() as i32);
        let raw = tex.img.as_raw();
        let mut colors = ShadeCache::new(shade, mat);
        for y in miny..=maxy {
            let py = y as f32 + 0.5;
            for x in minx..=maxx {
                let px_ = x as f32 + 0.5;
                let w0 = ((x1 - px_) * (y2 - py) - (x2 - px_) * (y1 - py)) / area;
                let w1 = ((x2 - px_) * (y0 - py) - (x0 - px_) * (y2 - py)) / area;
                let w2 = 1. - w0 - w1;
                if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                    continue;
                }
                let inv_z = w0 * iz[0] + w1 * iz[1] + w2 * iz[2];
                let z = 1. / inv_z;
                let i = y as usize * self.w + x as usize;
                if z >= self.depth[i] {
                    continue;
                }
                let u = (w0 * uz[0] + w1 * uz[1] + w2 * uz[2]) * z;
                let v = (w0 * vz[0] + w1 * vz[1] + w2 * vz[2]) * z;
                let tx = ((u / tex.w * tw as f32) as i32).clamp(0, tw - 1);
                let ty = ((v / tex.h * th as f32) as i32).clamp(0, th - 1);
                let t = ((ty * tw + tx) * 4) as usize;
                let a = raw[t + 3];
                if mat != Mat::Base && a < 128 {
                    continue;
                }
                self.depth[i] = z;
                let o = i * 4;
                // BGRA for GPUI.
                self.color[o] = colors.channel(2, raw[t + 2]);
                self.color[o + 1] = colors.channel(1, raw[t + 1]);
                self.color[o + 2] = colors.channel(0, raw[t]);
                self.color[o + 3] = 255;
            }
        }
    }
}

/// A box w×h×d with Minecraft-style UVs at (u, v), centred at `center` in its part's space.
#[allow(clippy::too_many_arguments)]
fn draw_box(r: &mut Raster, xf: &Xf, center: V3, size: (f32, f32, f32), grow: f32, uvo: (f32, f32), tex: &Tex, mat: Mat) {
    let (w, h, d) = size;
    let (u, v) = uvo;
    let (hx, hy, hz) = ((w + grow) / 2., (h + grow) / 2., (d + grow) / 2.);
    let c = center;
    let at = |x: f32, y: f32, z: f32| xf.apply([c[0] + x, c[1] + y, c[2] + z]);
    let e = 0.02;
    // (corners TL, TR, BR, BL), uv rect (x1, y1, x2, y2), local normal
    let faces: [([V3; 4], (f32, f32, f32, f32), V3); 6] = [
        ([at(-hx, hy, hz), at(hx, hy, hz), at(hx, -hy, hz), at(-hx, -hy, hz)], (u + d, v + d, u + d + w, v + d + h), [0., 0., 1.]),
        ([at(hx, hy, -hz), at(-hx, hy, -hz), at(-hx, -hy, -hz), at(hx, -hy, -hz)], (u + 2. * d + w, v + d, u + 2. * d + 2. * w, v + d + h), [0., 0., -1.]),
        ([at(-hx, hy, -hz), at(-hx, hy, hz), at(-hx, -hy, hz), at(-hx, -hy, -hz)], (u, v + d, u + d, v + d + h), [-1., 0., 0.]),
        ([at(hx, hy, hz), at(hx, hy, -hz), at(hx, -hy, -hz), at(hx, -hy, hz)], (u + d + w, v + d, u + 2. * d + w, v + d + h), [1., 0., 0.]),
        ([at(-hx, hy, -hz), at(hx, hy, -hz), at(hx, hy, hz), at(-hx, hy, hz)], (u + d, v, u + d + w, v + d), [0., 1., 0.]),
        ([at(-hx, -hy, hz), at(hx, -hy, hz), at(hx, -hy, -hz), at(-hx, -hy, -hz)], (u + d + w, v, u + d + 2. * w, v + d), [0., -1., 0.]),
    ];
    let sun = norm([0.7, 1., 1.2]);
    let rim = norm([-1., 0.4, -1.]);
    for (corners, (x1, y1, x2, y2), n) in faces {
        let nw = xf.rot.mul(n);
        let mid = [(corners[0][0] + corners[2][0]) / 2., (corners[0][1] + corners[2][1]) / 2., (corners[0][2] + corners[2][2]) / 2.];
        let facing = dot(nw, sub(r.eye, mid)) > 0.;
        if mat == Mat::Base && !facing {
            continue;
        }
        // Double-sided faces seen from inside are lit from the other side.
        let n_l = if facing { nw } else { [-nw[0], -nw[1], -nw[2]] };
        let s = dot(n_l, sun).max(0.) * 0.55;
        let rm = dot(n_l, rim).max(0.) * 0.18;
        let shade = [0.72 + s + rm * (0x9b as f32 / 255.), 0.72 + s + rm * (0xe3 as f32 / 255.), 0.72 + s + rm * (0xee as f32 / 255.)];
        let uv = [(x1 + e, y1 + e), (x2 - e, y1 + e), (x2 - e, y2 - e), (x1 + e, y2 - e)];
        let p: Vec<(f32, f32, f32)> = corners.iter().map(|c| r.project(*c)).collect();
        r.tri([p[0], p[1], p[2]], [uv[0], uv[1], uv[2]], tex, mat, shade);
        r.tri([p[0], p[2], p[3]], [uv[0], uv[2], uv[3]], tex, mat, shade);
    }
}

/// The Mod Maker's turning block (CSS `.cube`: rotateX(-24deg), a full turn every 14s, perspective 500px).
/// `side`/`top`/`bottom` are 16×16 textures; faces are shaded like the CSS filters.
pub fn render_cube(side: &RgbaImage, top: &RgbaImage, bottom: &RgbaImage, edge: f32, angle_y: f32, w: usize, h: usize, scale: f32) -> Vec<u8> {
    // CSS units in physical pixels: the cube is `edge` px wide, the viewer 500px in front of its centre.
    let persp = 500. * scale;
    let half = edge * scale / 2.;
    let mut r = Raster { w, h, color: vec![0; w * h * 4], depth: vec![f32::INFINITY; w * h], eye: [0., 0., persp], right: [1., 0., 0.], up: [0., 1., 0.], fwd: [0., 0., -1.], focal: persp };
    let rot = M3::euler((-24f32).to_radians() * -1., angle_y, 0.);
    let xf = Xf { rot, pos: [0., 0., 0.] };
    let at = |x: f32, y: f32, z: f32| xf.apply([x * half, y * half, z * half]);
    // (corners TL, TR, BR, BL), texture, brightness
    let faces: [([V3; 4], &RgbaImage, f32); 6] = [
        ([at(-1., 1., 1.), at(1., 1., 1.), at(1., -1., 1.), at(-1., -1., 1.)], side, 1.0),
        ([at(1., 1., 1.), at(1., 1., -1.), at(1., -1., -1.), at(1., -1., 1.)], side, 0.78),
        ([at(1., 1., -1.), at(-1., 1., -1.), at(-1., -1., -1.), at(1., -1., -1.)], side, 1.0),
        ([at(-1., 1., -1.), at(-1., 1., 1.), at(-1., -1., 1.), at(-1., -1., -1.)], side, 0.78),
        ([at(-1., 1., -1.), at(1., 1., -1.), at(1., 1., 1.), at(-1., 1., 1.)], top, 1.12),
        ([at(-1., -1., 1.), at(1., -1., 1.), at(1., -1., -1.), at(-1., -1., -1.)], bottom, 0.55),
    ];
    for (corners, img, k) in faces {
        let n = norm(cross(sub(corners[3], corners[0]), sub(corners[1], corners[0])));
        let mid = [(corners[0][0] + corners[2][0]) / 2., (corners[0][1] + corners[2][1]) / 2., (corners[0][2] + corners[2][2]) / 2.];
        if dot(n, sub(r.eye, mid)) <= 0. {
            continue;
        }
        let tex = Tex { img, w: 16., h: 16. };
        let p: Vec<(f32, f32, f32)> = corners.iter().map(|c| r.project(*c)).collect();
        let uv = [(0.02, 0.02), (15.98, 0.02), (15.98, 15.98), (0.02, 15.98)];
        // CSS filters work on sRGB values, so shade in sRGB space: lin() then srgb() of k·c.
        let shade = [k, k, k];
        r.tri([p[0], p[1], p[2]], [uv[0], uv[1], uv[2]], &tex, Mat::Block, shade);
        r.tri([p[0], p[2], p[3]], [uv[0], uv[2], uv[3]], &tex, Mat::Block, shade);
    }
    r.color
}

/// 64×32 skins have no left limbs; mirror the right ones like the game does (gl.js `toModernSkin`).
pub fn modern_skin(png: &[u8]) -> Option<RgbaImage> {
    let src = image::load_from_memory(png).ok()?.to_rgba8();
    if src.width() != 64 || !(src.height() == 32 || src.height() == 64) {
        return None;
    }
    let mut c = RgbaImage::new(64, 64);
    image::imageops::overlay(&mut c, &src, 0, 0);
    if src.height() == 32 {
        let copy = |c: &mut RgbaImage, sx: u32, sy: u32, w: u32, h: u32, dx: u32, dy: u32| {
            let part = image::imageops::flip_horizontal(&image::imageops::crop_imm(&*c, sx, sy, w, h).to_image());
            image::imageops::replace(c, &part, dx as i64, dy as i64);
        };
        for (sx, sy, w, h, dx, dy) in [(4, 16, 4, 4, 20, 48), (8, 16, 4, 4, 24, 48), (0, 20, 4, 12, 24, 52), (4, 20, 4, 12, 20, 52), (8, 20, 4, 12, 16, 52), (12, 20, 4, 12, 28, 52), (44, 16, 4, 4, 36, 48), (48, 16, 4, 4, 40, 48), (40, 20, 4, 12, 40, 52), (44, 20, 4, 12, 36, 52), (48, 20, 4, 12, 32, 52), (52, 20, 4, 12, 44, 52)] {
            copy(&mut c, sx, sy, w, h, dx, dy);
        }
    }
    Some(c)
}

/// Everything needed to draw one frame.
#[derive(Clone)]
struct Pose {
    yaw: f32,
    pitch: f32,
    dist: f32,
    t: f32,
    follow: bool,
    overlay: bool,
    head: (f32, f32),
    breathe: f32,
}

fn render(skin: &RgbaImage, slim: bool, cape: Option<&RgbaImage>, pose: &Pose, w: usize, h: usize) -> Vec<u8> {
    let cy = 16.;
    let eye = [0., cy + pose.pitch.sin() * pose.dist, pose.pitch.cos() * pose.dist];
    let fwd = norm(sub([0., cy, 0.], eye));
    let right = norm(cross(fwd, [0., 1., 0.]));
    let up = cross(right, fwd);
    let focal = (h as f32 / 2.) / (34f32.to_radians() / 2.).tan();
    let mut r = Raster { w, h, color: vec![0; w * h * 4], depth: vec![f32::INFINITY; w * h], eye, right, up, fwd, focal };

    let s = if pose.follow { 0. } else { pose.t.sin() };
    let bob = if pose.follow { 0. } else { pose.t.cos().abs() * 0.45 };
    let player = Xf { rot: M3::euler(0., pose.yaw, 0.), pos: [0., bob, 0.] };
    let a = if slim { 3. } else { 4. };
    let tex = Tex { img: skin, w: 64., h: 64. };
    let part = |pos: V3, rx: f32, ry: f32, rz: f32| player.then(&Xf { rot: M3::euler(rx, ry, rz), pos });
    let (arm_z_r, arm_z_l) = if pose.follow { (-0.05 - pose.breathe * 0.012, 0.05 + pose.breathe * 0.012) } else { (-0.05 - s.abs() * 0.02, 0.05 + s.abs() * 0.02) };
    let head = part([0., 24., 0.], pose.head.1, pose.head.0, 0.);
    let body = player.then(&Xf { rot: M3::euler(0., 0., 0.), pos: [0., 18., 0.] });
    let arm_r = part([-(4. + a / 2.), 22., 0.], -s * 0.42, 0., arm_z_r);
    let arm_l = part([4. + a / 2., 22., 0.], s * 0.42, 0., arm_z_l);
    let leg_r = part([-1.98, 12., 0.], s * 0.5, 0., 0.);
    let leg_l = part([1.98, 12., 0.], -s * 0.5, 0., 0.);
    let body_h = 12. * (1. + pose.breathe * 0.008);

    let parts: [(&Xf, V3, (f32, f32, f32), (f32, f32), (f32, f32), f32); 6] = [
        (&head, [0., 4., 0.], (8., 8., 8.), (0., 0.), (32., 0.), 1.),
        (&body, [0., 0., 0.], (8., body_h, 4.), (16., 16.), (16., 32.), 0.5),
        (&arm_r, [0., -4., 0.], (a, 12., 4.), (40., 16.), (40., 32.), 0.5),
        (&arm_l, [0., -4., 0.], (a, 12., 4.), (32., 48.), (48., 48.), 0.5),
        (&leg_r, [0., -6., 0.], (4., 12., 4.), (0., 16.), (0., 32.), 0.5),
        (&leg_l, [0., -6., 0.], (4., 12., 4.), (16., 48.), (0., 48.), 0.5),
    ];
    for (xf, center, size, base_uv, _, _) in parts.iter() {
        draw_box(&mut r, xf, *center, *size, 0., *base_uv, &tex, Mat::Base);
    }
    if let Some(cape) = cape {
        let legacy = ((cape.width() as f32 / cape.height() as f32) - 22. / 17.).abs() < 0.08;
        let ctex = Tex { img: cape, w: if legacy { 22. } else { 64. }, h: if legacy { 17. } else { 32. } };
        let angle = 0.12 + s.abs() * 0.18 + (pose.t * 1.7).sin() * 0.03;
        let pivot = player.then(&Xf { rot: M3::euler(angle, std::f32::consts::PI, 0.), pos: [0., 24., -2.] });
        draw_box(&mut r, &pivot, [0., -8., 0.5], (10., 16., 1.), 0., (0., 0.), &ctex, Mat::Cape);
    }
    if pose.overlay {
        for (xf, center, size, _, over_uv, grow) in parts.iter() {
            draw_box(&mut r, xf, *center, *size, *grow, *over_uv, &tex, Mat::Over);
        }
    }
    r.color
}

/// The turning block view (`.cube-stage`): set its textures, it spins on its own.
pub struct CubeView {
    key: String,
    faces: Option<Arc<(RgbaImage, RgbaImage, RgbaImage)>>,
    edge: f32,
    start: Instant,
    pub reduce_motion: bool,
    frame: Rc<RefCell<Option<Arc<RenderImage>>>>,
    frame_clock: Rc<crate::frame_clock::FrameClock>,
    animation_pending: bool,
}

impl CubeView {
    pub fn new(edge: f32) -> Self {
        Self { key: String::new(), faces: None, edge, start: Instant::now(), reduce_motion: false, frame: Rc::new(RefCell::new(None)), frame_clock: Rc::new(crate::frame_clock::FrameClock::default()), animation_pending: false }
    }
    /// PNG bytes for the sides, top and bottom; `key` says whether they changed.
    pub fn set(&mut self, key: &str, side: &[u8], top: &[u8], bottom: &[u8]) {
        if key == self.key {
            return;
        }
        self.key = key.to_string();
        self.frame_clock.invalidate();
        let load = |b: &[u8]| {
            let texture = image::load_from_memory(b).ok().map(|i| i.to_rgba8()).unwrap_or_else(|| RgbaImage::new(16, 16));
            let mut background = RgbaImage::from_pixel(texture.width(), texture.height(), image::Rgba([26, 26, 26, 255]));
            image::imageops::overlay(&mut background, &texture, 0, 0);
            background
        };
        self.faces = Some(Arc::new((load(side), load(top), load(bottom))));
    }
}

impl Render for CubeView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let angle = if self.reduce_motion { 35f32.to_radians() } else { (self.start.elapsed().as_secs_f32() / 14.).fract() * std::f32::consts::TAU };
        if !self.reduce_motion && !self.animation_pending {
            self.animation_pending = true;
            let timer = cx.background_executor().clone();
            cx.spawn(async move |view, cx| {
                timer.timer(std::time::Duration::from_millis(33)).await;
                let _ = view.update(cx, |this, cx| { this.animation_pending = false; cx.notify(); });
            }).detach();
        }
        let faces = self.faces.clone();
        let edge = self.edge;
        let cell = self.frame.clone();
        let clock = self.frame_clock.clone();
        canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let Some(f) = faces else { return };
                let scale = window.scale_factor();
                let w = (f32::from(bounds.size.width) * scale).round().max(2.) as usize;
                let h = (f32::from(bounds.size.height) * scale).round().max(2.) as usize;
                if w > 2048 || h > 2048 {
                    return;
                }
                if clock.can_reuse(w, h, scale) {
                    if let Some(image) = cell.borrow().as_ref() {
                        let _ = window.paint_image(bounds, gpui::Corners::default(), image.clone(), 0, false);
                        return;
                    }
                }
                let px_ = render_cube(&f.0, &f.1, &f.2, edge, angle, w, h, scale);
                let Some(buf) = image::ImageBuffer::from_raw(w as u32, h as u32, px_) else { return };
                let image = Arc::new(RenderImage::new(vec![Frame::new(buf)]));
                clock.rendered(w, h, scale);
                if let Some(old) = cell.borrow_mut().replace(image.clone()) {
                    let _ = window.drop_image(old);
                }
                let _ = window.paint_image(bounds, gpui::Corners::default(), image, 0, false);
            },
        )
        .size_full()
    }
}

struct Drag {
    start: Point<Pixels>,
    yaw: f32,
    pitch: f32,
}

/// The `SkinViewer` view. `set()` changes the skin; the parent embeds the entity.
pub struct SkinViewer {
    skin: Option<RgbaImage>,
    key: String,
    slim: bool,
    cape: Option<RgbaImage>,
    cape_key: String,
    follow: bool,
    overlay: bool,
    pub reduce_motion: bool,
    yaw: f32,
    pitch: f32,
    dist: f32,
    t: f32,
    spin: f32,
    idle_at: Instant,
    last: Instant,
    head: (f32, f32),
    drag: Option<Drag>,
    last_pointer: Option<Point<Pixels>>,
    pointer_moved_at: Instant,
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    frame: Rc<RefCell<Option<Arc<RenderImage>>>>,
    frame_clock: Rc<crate::frame_clock::FrameClock>,
    animation_pending: bool,
}

impl SkinViewer {
    pub fn new(follow_cursor: bool) -> Self {
        Self {
            skin: None,
            key: String::new(),
            slim: false,
            cape: None,
            cape_key: String::new(),
            follow: follow_cursor,
            overlay: false,
            reduce_motion: false,
            yaw: if follow_cursor { 0. } else { -0.5 },
            pitch: if follow_cursor { 0. } else { 0.05 },
            dist: 70.,
            t: 0.,
            spin: 0.,
            idle_at: Instant::now(),
            last: Instant::now(),
            head: (0., 0.),
            drag: None,
            last_pointer: None,
            pointer_moved_at: Instant::now(),
            bounds: Rc::new(Cell::new(None)),
            frame: Rc::new(RefCell::new(None)),
            frame_clock: Rc::new(crate::frame_clock::FrameClock::default()),
            animation_pending: false,
        }
    }

    /// Change the skin (PNG bytes) and/or cape. `key` identifies the bytes so unchanged skins aren't re-decoded.
    pub fn set(&mut self, key: &str, png: &[u8], slim: bool, cape_key: &str, cape: Option<&[u8]>) {
        if key != self.key || slim != self.slim {
            self.frame_clock.invalidate();
            self.key = key.to_string();
            self.slim = slim;
            self.skin = modern_skin(png);
        }
        if cape_key != self.cape_key {
            self.frame_clock.invalidate();
            self.cape_key = cape_key.to_string();
            self.cape = cape.and_then(|b| image::load_from_memory(b).ok()).map(|i| i.to_rgba8());
        }
    }

    /// The "Turn around" button: half a spin.
    pub fn turn(&mut self) {
        self.frame_clock.invalidate();
        self.spin = std::f32::consts::PI;
        self.idle_at = Instant::now() + std::time::Duration::from_secs(3);
    }

    fn step(&mut self, window: &Window) {
        let now = Instant::now();
        let dt = (now - self.last).as_secs_f32() * 1000.;
        let dt = dt.min(120.);
        self.last = now;
        let still = self.reduce_motion;
        if !self.follow && self.drag.is_none() && !still && now > self.idle_at {
            self.yaw += dt * 0.00035;
        }
        if self.spin > 0. {
            let step = self.spin.min(dt * 0.008);
            self.yaw += step;
            self.spin -= step;
        }
        if !still {
            self.t += dt * 0.0042;
        }
        if self.follow {
            // Look towards the cursor, anywhere in the window.
            let size = window.viewport_size();
            let (vw, vh) = (f32::from(size.width), f32::from(size.height));
            let m = window.mouse_position();
            if self.last_pointer != Some(m) {
                self.last_pointer = Some(m);
                self.pointer_moved_at = now;
            }
            let (cx, cy) = match self.bounds.get() {
                Some(b) => (f32::from(b.origin.x) + f32::from(b.size.width) / 2., f32::from(b.origin.y) + f32::from(b.size.height) * 0.32),
                None => (vw / 2., vh / 2.),
            };
            let lx = ((f32::from(m.x) - cx) / (vw / 2.).max(1.)).clamp(-1., 1.);
            let ly = ((f32::from(m.y) - cy) / (vh / 2.).max(1.)).clamp(-1., 1.);
            let ease = 1. - (-dt / 130.).exp();
            self.head.0 += (lx * 0.8 - self.head.0) * ease;
            self.head.1 += (ly * 0.45 - self.head.1) * ease;
        } else {
            self.head = ((self.t * 0.31).sin() * 0.25, (self.t * 0.23).sin() * 0.08);
        }
    }
}

impl Render for SkinViewer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.step(window);
        if (!self.reduce_motion || self.follow || self.spin > 0.) && !self.animation_pending {
            self.animation_pending = true;
            // The Home avatar only breathes very slightly when the pointer is still.
            // Keep cursor tracking responsive and use fewer frames during idle.
            let interval = if self.follow && self.drag.is_none() && self.spin <= 0.
                && self.pointer_moved_at.elapsed() > std::time::Duration::from_secs(1) { 100 } else { 33 };
            let timer = cx.background_executor().clone();
            cx.spawn(async move |view, cx| {
                timer.timer(std::time::Duration::from_millis(interval)).await;
                let _ = view.update(cx, |this, cx| { this.animation_pending = false; cx.notify(); });
            }).detach();
        }
        let pose = Pose {
            yaw: self.yaw,
            pitch: self.pitch,
            dist: self.dist,
            t: self.t,
            follow: self.follow,
            overlay: self.overlay,
            head: self.head,
            breathe: if self.reduce_motion || !self.follow { 0. } else { (self.t * 0.55).sin() },
        };
        let skin = self.skin.clone();
        let cape = self.cape.clone();
        let slim = self.slim;
        let bounds_cell = self.bounds.clone();
        let frame_cell = self.frame.clone();
        let clock = self.frame_clock.clone();
        let drawing = canvas(
            move |bounds, _, _| bounds,
            move |bounds, _, window, _| {
                bounds_cell.set(Some(bounds));
                let Some(skin) = skin else { return };
                let scale = window.scale_factor();
                let w = (f32::from(bounds.size.width) * scale).round().max(1.) as usize;
                let h = (f32::from(bounds.size.height) * scale).round().max(1.) as usize;
                if w < 2 || h < 2 || w > 4096 || h > 4096 {
                    return;
                }
                if clock.can_reuse(w, h, scale) {
                    if let Some(image) = frame_cell.borrow().as_ref() {
                        let _ = window.paint_image(bounds, gpui::Corners::default(), image.clone(), 0, false);
                        return;
                    }
                }
                let px_ = render(&skin, slim, cape.as_ref(), &pose, w, h);
                let Some(buf) = image::ImageBuffer::from_raw(w as u32, h as u32, px_) else { return };
                let image = Arc::new(RenderImage::new(vec![Frame::new(buf)]));
                clock.rendered(w, h, scale);
                if let Some(old) = frame_cell.borrow_mut().replace(image.clone()) {
                    let _ = window.drop_image(old);
                }
                let _ = window.paint_image(bounds, gpui::Corners::default(), image, 0, false);
            },
        )
        .size_full();
        div()
            .id("skin3d")
            .size_full()
            .cursor(if self.drag.is_some() { gpui::CursorStyle::ClosedHand } else { gpui::CursorStyle::OpenHand })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, e: &MouseDownEvent, _, cx| {
                    this.drag = Some(Drag { start: e.position, yaw: this.yaw, pitch: this.pitch });
                    cx.notify();
                }),
            )
            .on_mouse_move(cx.listener(|this, e: &MouseMoveEvent, _, cx| {
                if let Some(d) = &this.drag {
                    if e.pressed_button != Some(MouseButton::Left) {
                        this.drag = None;
                        this.idle_at = Instant::now() + std::time::Duration::from_millis(2500);
                        return;
                    }
                    this.frame_clock.invalidate();
                    this.yaw = d.yaw + f32::from(e.position.x - d.start.x) * 0.012;
                    if !this.follow {
                        this.pitch = (d.pitch + f32::from(e.position.y - d.start.y) * 0.006).clamp(-0.5, 0.6);
                    }
                    cx.notify();
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    this.drag = None;
                    this.idle_at = Instant::now() + std::time::Duration::from_millis(2500);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, _| {
                    if this.drag.take().is_some() {
                        this.idle_at = Instant::now() + std::time::Duration::from_millis(2500);
                    }
                }),
            )
            .on_scroll_wheel(cx.listener(|this, e: &ScrollWheelEvent, window, cx| {
                let dy = f32::from(e.delta.pixel_delta(window.line_height()).y);
                this.frame_clock.invalidate();
                this.dist = (this.dist - dy * 0.05).clamp(40., 110.);
                cx.notify();
            }))
            .child(drawing)
    }
}

#[cfg(test)]
mod parity_tests {
    use super::SkinViewer;

    #[test]
    fn cached_shading_matches_original_for_every_channel() {
        for mat in [super::Mat::Base, super::Mat::Block] {
            let shade = [0.72, 1.05, 1.35];
            let mut cache = super::ShadeCache::new(shade, mat);
            for (component, factor) in shade.into_iter().enumerate() {
                for value in 0..=255u8 {
                    let expected = if mat == super::Mat::Block {
                        (value as f32 * factor).clamp(0., 255.).round() as u8
                    } else {
                        super::srgb(super::lin(value) * factor)
                    };
                    assert_eq!(cache.channel(component, value), expected);
                    assert_eq!(cache.channel(component, value), expected);
                }
            }
        }
    }

    #[test]
    fn cube_renders_front_and_top_not_back_faces() {
        let solid = |p| image::RgbaImage::from_pixel(16, 16, image::Rgba(p));
        let pixels = super::render_cube(&solid([255, 0, 0, 255]), &solid([0, 255, 0, 255]), &solid([0, 0, 255, 255]), 64., 0., 112, 112, 1.);
        let colors: Vec<_> = pixels.chunks_exact(4).filter(|p| p[3] != 0).collect();
        assert!(colors.iter().any(|p| p[2] > 200 && p[1] == 0 && p[0] == 0), "front visible");
        assert!(colors.iter().any(|p| p[1] > 200 && p[2] == 0 && p[0] == 0), "top visible");
        assert!(!colors.iter().any(|p| p[0] > 0), "bottom hidden");
    }

    #[test]
    fn home_and_skins_hide_the_extra_overlay() {
        assert!(!SkinViewer::new(true).overlay);
        assert!(!SkinViewer::new(false).overlay);
    }
}
