//! Motion from src/styles.css: each pixel icon's own hover animation (`.ic-<name>` keyframes),
//! hover lifts, and a frame request when something is still moving.
//!
//! GPUI can't transform elements, so an animating icon is drawn as transformed polygons
//! (rotate / scale / skew / translate around its transform-origin) until the animation ends.
use gpui::{canvas, point, prelude::*, px, AnyElement, Hsla, PathBuilder, SharedString, Stateful, Window};
use std::{cell::RefCell, collections::HashMap, time::Instant};

thread_local! {
    /// When each hoverable element (by key) was last entered, and whether it's hovered now.
    static HOVERS: RefCell<HashMap<SharedString, (Instant, bool)>> = RefCell::new(HashMap::new());
    /// Last seen state of toggles (switch knobs, tab underlines) and when it changed.
    static TOGGLES: RefCell<HashMap<SharedString, (String, Instant)>> = RefCell::new(HashMap::new());
    static NEED_FRAME: RefCell<bool> = const { RefCell::new(false) };
    static DD_CLOSE: RefCell<HashMap<SharedString, Instant>> = RefCell::new(HashMap::new());
    static REDUCED: RefCell<bool> = const { RefCell::new(false) };
}

/// Settings → Reduce motion.
pub fn set_reduced(on: bool) {
    REDUCED.with(|r| *r.borrow_mut() = on);
}
pub fn reduced() -> bool {
    REDUCED.with(|r| *r.borrow())
}

/// Something is mid-animation: draw another frame.
pub fn request_frame() {
    NEED_FRAME.with(|f| *f.borrow_mut() = true);
}
/// Called once per window frame by the shell.
pub fn flush(window: &mut Window) {
    if NEED_FRAME.with(|f| std::mem::take(&mut *f.borrow_mut())) {
        window.request_animation_frame();
    }
}

/// Record hover starts for `key` (the trigger for icon animations and lifts).
pub fn hoverable(el: Stateful<gpui::Div>, key: impl Into<SharedString>) -> Stateful<gpui::Div> {
    let key = key.into();
    el.on_hover(move |hovered, window, _| {
        let k = key.clone();
        HOVERS.with(|h| {
            let mut h = h.borrow_mut();
            if *hovered {
                h.insert(k, (Instant::now(), true));
            } else if let Some(e) = h.get_mut(&k) {
                e.1 = false;
            }
        });
        window.refresh();
    })
}

/// Milliseconds since `key` was hovered, if it is hovered (or was, for up to `keep` ms).
fn since(key: &SharedString, keep: f32) -> Option<(f32, bool)> {
    HOVERS.with(|h| {
        let h = h.borrow();
        let (at, now) = h.get(key)?;
        let ms = at.elapsed().as_secs_f32() * 1000.;
        (*now || ms < keep).then_some((ms, *now))
    })
}

/// `.card:hover { transform: translateY(-3px) }` with its .18s spring: the offset to apply now.
pub fn lift(key: &SharedString, amount: f32) -> f32 {
    if reduced() {
        return 0.;
    }
    match since(key, 0.) {
        Some((ms, true)) => {
            let t = (ms / 180.).min(1.);
            if t < 1. {
                request_frame();
            }
            -amount * cubic(0.3, 1.3, 0.5, 1.0, t)
        }
        _ => 0.,
    }
}

pub fn dropdown_close_start(key: SharedString, at: Instant) {
    DD_CLOSE.with(|items| { items.borrow_mut().insert(key, at); });
}
pub fn dropdown_close_end(key: &SharedString) {
    DD_CLOSE.with(|items| { items.borrow_mut().remove(key); });
}
pub fn dropdown_exit(key: &SharedString) -> Option<f32> {
    let at = DD_CLOSE.with(|items| items.borrow().get(key).copied())?;
    let t = (at.elapsed().as_secs_f32() / 0.12).clamp(0., 1.);
    if t < 1. { request_frame(); }
    Some(t)
}

/// Gentle cape tilt while a cape card is hovered; returns a vertical scale.
pub fn cape_sway(key: &SharedString) -> f32 {
    if reduced() { return 1.; }
    match since(key, 0.) {
        Some((ms, true)) => {
            request_frame();
            let phase = ms / 900. * std::f32::consts::TAU;
            let angle = (1. - phase.cos()) * 0.5 * 16f32.to_radians();
            angle.cos()
        }
        _ => 1.,
    }
}

// ── easing ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub enum Ease {
    Bezier(f32, f32, f32, f32),
    Steps(u32),
}
const EASE_OUT: Ease = Ease::Bezier(0., 0., 0.58, 1.);
const EASE_IN: Ease = Ease::Bezier(0.42, 0., 1., 1.);
const EASE_IN_OUT: Ease = Ease::Bezier(0.42, 0., 0.58, 1.);

/// CSS cubic-bezier(x1, y1, x2, y2) at progress `t`.
pub fn cubic(x1: f32, y1: f32, x2: f32, y2: f32, t: f32) -> f32 {
    let bx = |s: f32| 3. * x1 * s * (1. - s).powi(2) + 3. * x2 * s * s * (1. - s) + s * s * s;
    let by = |s: f32| 3. * y1 * s * (1. - s).powi(2) + 3. * y2 * s * s * (1. - s) + s * s * s;
    let (mut lo, mut hi) = (0f32, 1f32);
    for _ in 0..24 {
        let mid = (lo + hi) / 2.;
        if bx(mid) < t {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    by((lo + hi) / 2.)
}

fn ease(e: Ease, t: f32) -> f32 {
    match e {
        Ease::Bezier(a, b, c, d) => cubic(a, b, c, d, t),
        Ease::Steps(n) => ((t * n as f32).floor() / n as f32).min(1.),
    }
}

// ── icon keyframes ───────────────────────────────────────────────────────

/// One keyframe: translate (CSS px on a 16px icon), rotate (deg), scale, skewX (deg), opacity.
#[derive(Clone, Copy)]
pub struct Tf {
    pub tx: f32,
    pub ty: f32,
    pub rot: f32,
    pub sx: f32,
    pub sy: f32,
    pub skx: f32,
    pub op: f32,
}
const I: Tf = Tf { tx: 0., ty: 0., rot: 0., sx: 1., sy: 1., skx: 0., op: 1. };
const fn tr(tx: f32, ty: f32) -> Tf { Tf { tx, ty, ..I } }
const fn rot(r: f32) -> Tf { Tf { rot: r, ..I } }
const fn sc(s: f32) -> Tf { Tf { sx: s, sy: s, ..I } }

struct Anim {
    ms: f32,
    ease: Ease,
    origin: (f32, f32),
    keys: Vec<(f32, Tf)>,
}

fn anim(name: &str) -> Option<Anim> {
    let a = |ms: f32, ease: Ease, origin: (f32, f32), keys: &[(f32, Tf)]| Some(Anim { ms, ease, origin, keys: keys.to_vec() });
    let c = (0.5, 0.5);
    match name {
        "play" => a(450., EASE_OUT, c, &[(0.3, Tf { tx: -1., sx: 0.85, sy: 0.85, ..I }), (0.65, Tf { tx: 3., sx: 1.1, sy: 1.1, ..I })]),
        "stop" => a(350., Ease::Steps(2), c, &[(0.5, sc(0.65))]),
        "home" => a(450., EASE_OUT, c, &[(0.35, Tf { ty: -3., sy: 1.1, ..I }), (0.7, Tf { sy: 0.88, ..I })]),
        "grid" => a(500., EASE_IN_OUT, c, &[(0.5, Tf { rot: 90., sx: 0.8, sy: 0.8, ..I }), (1., rot(180.))]),
        "chest" => a(500., EASE_OUT, (0.5, 1.), &[(0.3, Tf { sy: 1.22, rot: -4., ..I }), (0.6, Tf { sy: 0.92, rot: 3., ..I })]),
        "server" => a(500., Ease::Steps(1), c, &[(0.2, Tf { op: 0.35, ..I }), (0.4, I), (0.6, Tf { op: 0.35, ..I }), (0.8, I)]),
        "shirt" => a(600., EASE_IN_OUT, (0.5, 0.), &[(0.25, rot(12.)), (0.6, rot(-9.)), (0.85, rot(4.))]),
        "bell" => a(600., EASE_IN_OUT, (0.5, 0.), &[(0.2, rot(-20.)), (0.45, rot(16.)), (0.7, rot(-9.)), (0.88, rot(4.))]),
        "gear" => a(600., Ease::Bezier(0.4, 0., 0.2, 1.), c, &[(1., rot(180.))]),
        "chevron" | "chevron-up" => a(400., EASE_OUT, c, &[(0.5, tr(0., 2.))]),
        "chevron-right" => a(400., EASE_OUT, c, &[(0.5, tr(3., 0.))]),
        "arrow-right" | "arrow-right@flipx" => a(500., EASE_IN_OUT, c, &[(0.4, Tf { tx: 4., op: 0., ..I }), (0.41, Tf { tx: -4., op: 0., ..I })]),
        "external" => a(450., EASE_IN_OUT, c, &[(0.4, Tf { tx: 3., ty: -3., op: 0., ..I }), (0.41, Tf { tx: -3., ty: 3., op: 0., ..I })]),
        "check" => a(400., Ease::Bezier(0.2, 0.9, 0.3, 1.6), c, &[(0., Tf { sx: 0.3, sy: 0.3, rot: -20., ..I })]),
        "search" => a(600., EASE_IN_OUT, c, &[(0.25, Tf { tx: -1., ty: -1., sx: 1.15, sy: 1.15, ..I }), (0.5, Tf { tx: 2., sx: 1.15, sy: 1.15, ..I }), (0.75, Tf { ty: 1., sx: 1.15, sy: 1.15, ..I })]),
        "plus" => a(400., Ease::Bezier(0.3, 1.5, 0.5, 1.), c, &[(0., Tf { rot: -90., sx: 0.7, sy: 0.7, ..I })]),
        "down" => a(500., EASE_IN, c, &[(0.45, Tf { ty: 4., op: 0., ..I }), (0.46, Tf { ty: -5., op: 0., ..I })]),
        "upload" => a(500., EASE_IN, c, &[(0.45, Tf { ty: -5., op: 0., ..I }), (0.46, Tf { ty: 4., op: 0., ..I })]),
        "folder" => a(400., EASE_OUT, (0., 1.), &[(0.5, Tf { skx: -12., sy: 0.9, ..I })]),
        "copy" => a(400., Ease::Steps(2), c, &[(0.5, tr(2., -2.))]),
        "trash" => a(450., EASE_IN_OUT, c, &[(0.2, rot(-12.)), (0.4, rot(10.)), (0.6, rot(-7.)), (0.8, rot(4.))]),
        "pin" => a(400., EASE_IN, c, &[(0.4, Tf { ty: -3., rot: -15., ..I }), (0.7, tr(0., 1.))]),
        "rotate" => a(600., Ease::Bezier(0.4, 0., 0.2, 1.), c, &[(1., rot(-360.))]),
        "close" => a(350., EASE_OUT, c, &[(0.5, Tf { rot: 90., sx: 0.75, sy: 0.75, ..I }), (1., rot(90.))]),
        "times" => a(300., EASE_OUT, c, &[(0.5, sc(0.6))]),
        "min" => a(300., EASE_OUT, c, &[(0.5, Tf { ty: 2., sx: 0.6, ..I })]),
        "max" => a(300., EASE_OUT, c, &[(0.5, sc(1.25))]),
        "edit" => a(500., EASE_IN_OUT, c, &[(0.25, Tf { tx: -1., ty: 1., rot: -10., ..I }), (0.5, Tf { tx: 1., rot: 4., ..I }), (0.75, Tf { ty: -1., rot: -4., ..I })]),
        "pencil" => a(400., Ease::Steps(4), c, &[(0.25, tr(1., -1.)), (0.5, tr(-1., 1.)), (0.75, tr(1., 0.))]),
        "sliders" => a(450., EASE_IN_OUT, c, &[(0.35, Tf { sx: 0.75, ..I }), (0.7, Tf { sx: 1.1, ..I })]),
        "monitor" => a(450., Ease::Steps(1), c, &[(0.15, Tf { op: 0.2, ..I }), (0.3, I), (0.45, Tf { op: 0.5, ..I }), (0.6, I)]),
        "package" => a(500., EASE_OUT, c, &[(0.25, Tf { ty: -3., rot: -6., ..I }), (0.55, Tf { sx: 1.1, sy: 0.85, ..I }), (0.8, Tf { sx: 0.96, sy: 1.05, ..I })]),
        "paint" => a(500., EASE_IN_OUT, c, &[(0.3, Tf { rot: -18., tx: -1., ..I }), (0.7, Tf { rot: 14., tx: 1., ..I })]),
        "keys" => a(350., Ease::Steps(2), c, &[(0.5, Tf { ty: 1.5, sy: 0.8, ..I })]),
        "user" => a(500., EASE_IN_OUT, (0.5, 1.), &[(0.3, rot(-8.)), (0.65, rot(8.))]),
        "info" => a(500., EASE_OUT, c, &[(0.3, sc(1.25)), (0.6, sc(0.92))]),
        "star" => a(600., EASE_IN_OUT, c, &[(0.5, Tf { rot: 72., sx: 1.3, sy: 1.3, ..I }), (1., rot(144.))]),
        "cube" => a(550., EASE_IN_OUT, c, &[(0.4, Tf { ty: -3., rot: 45., ..I }), (0.75, rot(90.)), (1., rot(90.))]),
        "block" => a(400., EASE_OUT, c, &[(0., Tf { ty: -5., op: 0., ..I }), (0.6, Tf { sy: 0.8, ..I })]),
        "map" => a(500., EASE_IN_OUT, c, &[(0.5, Tf { sx: 0.64, ..I })]),
        "eye" => a(400., EASE_IN_OUT, c, &[(0.5, Tf { sy: 0.15, ..I })]),
        "cape" => a(700., EASE_IN_OUT, (0.5, 0.), &[(0.3, Tf { skx: -14., sy: 1.05, ..I }), (0.65, Tf { skx: 10., ..I })]),
        "gift" => a(550., EASE_IN_OUT, (0.5, 1.), &[(0.2, Tf { rot: -10., sx: 1.1, sy: 1.1, ..I }), (0.4, Tf { rot: 10., sx: 1.1, sy: 1.1, ..I }), (0.6, Tf { rot: -6., sx: 1.1, sy: 1.1, ..I }), (0.8, Tf { rot: 6., sx: 1.1, sy: 1.1, ..I })]),
        "hammer" => a(500., EASE_IN, (0.5, 1.), &[(0.35, rot(-35.)), (0.6, rot(18.)), (0.75, rot(6.)), (0.88, rot(14.))]),
        "code" => a(450., EASE_OUT, c, &[(0.5, Tf { sx: 1.35, ..I })]),
        "item" => a(600., EASE_IN_OUT, c, &[(0.5, Tf { ty: -2., rot: 180., sx: 1.15, sy: 1.15, ..I }), (1., rot(360.))]),
        "recipe" => a(500., Ease::Steps(4), c, &[(0.25, tr(-1., -1.)), (0.5, tr(1., -1.)), (0.75, tr(1., 1.))]),
        "test" => a(600., EASE_IN_OUT, (0.5, 0.2), &[(0.25, rot(-22.)), (0.5, rot(18.)), (0.75, rot(-10.))]),
        "bucket" => a(500., EASE_IN_OUT, c, &[(0.4, Tf { rot: -40., ty: -1., ..I }), (0.7, Tf { rot: -40., ty: -1., ..I })]),
        "picker" => a(450., EASE_IN_OUT, c, &[(0.4, tr(-2., 2.)), (0.7, tr(1., -1.))]),
        "eraser" => a(450., EASE_IN_OUT, c, &[(0.25, tr(-2., 0.)), (0.5, tr(2., 0.)), (0.75, tr(-1., 0.))]),
        "leaf" => a(700., EASE_IN_OUT, (0., 1.), &[(0.3, rot(-16.)), (0.65, rot(10.))]),
        "player" => a(700., Ease::Steps(1), c, &[(0.2, tr(0., -1.)), (0.4, Tf { ty: -2., rot: -6., ..I }), (0.6, Tf { ty: -1., rot: 6., ..I }), (0.8, tr(0., -1.))]),
        // The default hop (`ic-hop`) for anything else.
        _ => a(350., Ease::Steps(3), c, &[(0.5, tr(0., -2.))]),
    }
}

fn lerp(a: Tf, b: Tf, k: f32) -> Tf {
    let l = |x: f32, y: f32| x + (y - x) * k;
    Tf { tx: l(a.tx, b.tx), ty: l(a.ty, b.ty), rot: l(a.rot, b.rot), sx: l(a.sx, b.sx), sy: l(a.sy, b.sy), skx: l(a.skx, b.skx), op: l(a.op, b.op) }
}

/// Keyframes at progress `t` (0..1). Missing 0% / 100% frames are the resting icon, like CSS.
fn sample(a: &Anim, t: f32) -> Tf {
    let mut keys: Vec<(f32, Tf)> = a.keys.clone();
    if keys.first().map_or(true, |k| k.0 > 0.) {
        keys.insert(0, (0., I));
    }
    if keys.last().map_or(true, |k| k.0 < 1.) {
        keys.push((1., I));
    }
    for w in keys.windows(2) {
        let ((t0, a0), (t1, a1)) = (w[0], w[1]);
        if t <= t1 {
            let local = if t1 > t0 { (t - t0) / (t1 - t0) } else { 1. };
            return lerp(a0, a1, ease(a.ease, local.clamp(0., 1.)));
        }
    }
    keys.last().unwrap().1
}

/// The icon as it looks `ms` into its hover animation, or None once it's over.
fn pose(name: &str, ms: f32) -> Option<Tf> {
    let a = anim(name)?;
    let t = ms / a.ms;
    (t < 1.).then(|| sample(&a, t))
}

/// An icon whose hover animation is triggered by the element registered under `key`.
pub fn animated_icon(name: &'static str, size: f32, color: Hsla, key: &SharedString) -> AnyElement {
    let ms = if reduced() { None } else { since(key, 2000.).map(|(ms, _)| ms) };
    let Some(tf) = ms.and_then(|ms| pose(name, ms)) else {
        return super::kit::icon(name, size, color).into_any_element();
    };
    request_frame();
    let origin = anim(name).map(|a| a.origin).unwrap_or((0.5, 0.5));
    let (base, flip, flip_x) = super::kit::icon_parts(name);
    let rects = super::kit::icon_rects(base);
    let unit = size / 8.;
    let k = size / 16.;
    let (rs, rc) = tf.rot.to_radians().sin_cos();
    let sk = tf.skx.to_radians().tan();
    let color = Hsla { a: color.a * tf.op.clamp(0., 1.), ..color };
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            let (ox, oy) = (origin.0 * size, origin.1 * size);
            let map = |x: f32, y: f32| {
                // scale, then skewX, then rotate, around the origin; then translate.
                let (dx, dy) = ((x - ox) * tf.sx, (y - oy) * tf.sy);
                let dx = dx + sk * dy;
                let (rx, ry) = (dx * rc - dy * rs, dx * rs + dy * rc);
                point(bounds.origin.x + px(ox + rx + tf.tx * k), bounds.origin.y + px(oy + ry + tf.ty * k))
            };
            for &(x, y, w, h) in rects {
                let y = if flip { 8 - y - h } else { y };
                let x = if flip_x { 8 - x - w } else { x };
                let (l, t) = (x as f32 * unit, y as f32 * unit);
                let (r, b) = ((x + w) as f32 * unit, (y + h) as f32 * unit);
                let mut p = PathBuilder::fill();
                p.move_to(map(l, t));
                p.line_to(map(r, t));
                p.line_to(map(r, b));
                p.line_to(map(l, b));
                p.close();
                if let Ok(path) = p.build() {
                    window.paint_path(path, color);
                }
            }
        },
    )
    .size(px(size))
    .flex_none()
    .into_any_element()
}

/// 0→1 over `ms` after `state` of `key` changes (1 when it hasn't changed lately or on first sight).
pub fn since_change(key: impl Into<SharedString>, state: &str, ms: f32) -> f32 {
    if reduced() {
        return 1.;
    }
    let key = key.into();
    let t = TOGGLES.with(|t| {
        let mut t = t.borrow_mut();
        match t.get(&key) {
            Some((s, at)) if s == state => (at.elapsed().as_secs_f32() * 1000. / ms).min(1.),
            Some(_) => {
                t.insert(key, (state.to_string(), Instant::now()));
                0.
            }
            None => {
                t.insert(key, (state.to_string(), Instant::now() - std::time::Duration::from_secs(10)));
                1.
            }
        }
    });
    if t < 1. {
        request_frame();
    }
    t
}

/// 0→1 over `ms` from the first time `key` is seen (enter animations), with `delay` ms first.
pub fn enter(key: impl Into<SharedString>, ms: f32, delay: f32) -> f32 {
    if reduced() {
        return 1.;
    }
    let key = key.into();
    let at = TOGGLES.with(|t| {
        let mut t = t.borrow_mut();
        t.entry(key).or_insert_with(|| (String::new(), Instant::now())).1
    });
    let t = ((at.elapsed().as_secs_f32() * 1000. - delay) / ms).clamp(0., 1.);
    if t < 1. {
        request_frame();
    }
    t
}

/// Forget enter animations whose key starts with `prefix` (so a page animates in again next visit).
pub fn reset(prefix: &str) {
    TOGGLES.with(|t| t.borrow_mut().retain(|k, _| !k.starts_with(prefix)));
}
